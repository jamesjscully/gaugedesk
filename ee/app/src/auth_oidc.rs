//! OIDC auth-code + PKCE **shell** (M3 `ID-3`) — the two browser-facing control-plane
//! routes that wrap the verified OIDC core ([`crate::identity_oidc`]) into a real login:
//!
//! - `GET /auth/login` — read the org's configured SSO connection (`/admin/sso`, the
//!   one home for "which IdP"), discover the OP's endpoints, mint a PKCE verifier plus
//!   a CSRF `state`, stash them server-side keyed by `state`, and **302-redirect** the
//!   browser to the OP's authorize endpoint.
//! - `GET /auth/callback` — the OP redirects the browser back here with `code` and
//!   `state`. Look up the pending PKCE verifier by `state` (single-use; an unknown
//!   `state` is refused — the CSRF guard, `INV-20`), redeem the code at the token
//!   endpoint ([`exchange_code`], presenting the verifier so an intercepted code is
//!   useless), then verify the returned id-token against the issuer's live JWKS.
//!
//! The verified **id-token is the bearer** the control plane already accepts
//! (`Workbench::authorize` → `idp.authenticate`), so the shell hands it back to the
//! client rather than minting a second credential — one home for the session truth
//! (the signed, self-expiring token), no parallel session table to keep in sync.
//!
//! The HTTP-touching logic lives in two seam-generic functions ([`start_login`],
//! [`finish_callback`]) tested against a mock OP; the axum handlers are the thin
//! wiring that supplies the real [`net_http::HttpClient`](gaugewright_app::net_http)
//! (off the async runtime via [`tokio::task::spawn_blocking`], since the seam is
//! blocking) and the server-side [`PendingAuthStore`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::{
    extract::{Extension, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect},
    Json,
};
use jsonwebtoken::decode_header;
use serde::Deserialize;
use serde_json::json;

use gaugewright_core::abac::AuthorityAttributes;
use gaugewright_core::ids::AuthorityId;

use crate::identity_oidc::{
    authorize_url, discover_endpoints, discover_jwks, exchange_code, ClaimMapping, HttpForm,
    HttpGet, OidcIdentityProvider, Pkce,
};
use base64::Engine as _;
use gaugewright_app::identity::IdentityProvider;
use gaugewright_app::net_http::HttpClient;
use gaugewright_app::org::{
    MembershipRecord, MembershipStatus, Org, RecordOp, SsoConnectionRecord, SsoProtocol, ORG_ID,
};
use gaugewright_app::{LockUnpoisoned, SharedWorkbench, Workbench};

/// Server-side state carried from `/auth/login` to `/auth/callback` for one
/// auth-code + PKCE exchange, keyed by the CSRF `state` (`ID-3`). Holds the PKCE
/// verifier (never sent on the authorize leg, only on the token exchange) plus the
/// endpoints + verification parameters the login leg already discovered, so the
/// callback need not re-discover. Single-use: the callback consumes it.
#[derive(Clone, Debug)]
pub struct PendingAuth {
    /// The PKCE verifier whose S256 challenge went to the OP; presented at exchange.
    /// Held [`Secret`](gaugewright_app::secret::Secret) so this `Debug`-deriving
    /// struct never leaks it to a log (`SECAUD-10`).
    pub verifier: gaugewright_app::secret::Secret,
    /// The OP token endpoint the code is redeemed at.
    pub token_endpoint: String,
    /// The OP JWKS endpoint the returned id-token is verified against.
    pub jwks_uri: String,
    /// The issuer the verifier pins (`iss` must match).
    pub issuer: String,
    /// The accepted audiences (`aud` must contain one); the first is the client id.
    pub audiences: Vec<String>,
    /// The exact `redirect_uri` sent on authorize — must match on exchange (RFC 6749).
    pub redirect_uri: String,
    /// How the returned id-token's claims map onto ABAC attributes (`ID-3`).
    pub mapping: ClaimMapping,
}

/// In-flight `/auth/login` → `/auth/callback` PKCE state, keyed by CSRF `state`
/// (`ID-3`). Single-process, held behind the [`EnterpriseAuthState`] mutex. A
/// `state` authorizes exactly one callback: [`take`](Self::take) removes it,
/// so a replayed or forged `state` finds nothing (fail-closed, `INV-20`). Loopback
/// scaffold: a real multi-node deployment backs this with shared, TTL-bounded
/// storage behind the same seam (mirroring
/// [`SessionStore`](gaugewright_app::session::SessionStore)).
#[derive(Default)]
pub struct PendingAuthStore {
    by_state: BTreeMap<String, PendingAuth>,
}

impl PendingAuthStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the pending PKCE state a login leg minted, keyed by its CSRF `state`.
    pub fn begin(&mut self, state: impl Into<String>, pending: PendingAuth) {
        self.by_state.insert(state.into(), pending);
    }

    /// Consume the pending state for a callback's `state` (single-use). `None` for an
    /// unknown / already-redeemed / forged `state` — the CSRF guard.
    pub fn take(&mut self, state: &str) -> Option<PendingAuth> {
        self.by_state.remove(state)
    }

    /// How many logins are awaiting their callback.
    pub fn len(&self) -> usize {
        self.by_state.len()
    }

    /// Whether any login is awaiting its callback.
    pub fn is_empty(&self) -> bool {
        self.by_state.is_empty()
    }
}

/// Enterprise-owned OIDC auth-code login state (`ID-3`) — **ee axum state**, not a
/// [`Workbench`] field. The route builder ([`crate::org_routes::routes`]) mints one
/// per composition and carries it to the `/auth/login` + `/auth/callback` handlers
/// as an [`Extension`], so the pending-login store's lifetime spans requests exactly
/// as the pre-split workbench field did (created once per composition, never
/// per-request). A cheap-to-clone shared handle; its own mutex keeps
/// [`PendingAuthStore::take`]'s single-use CSRF consumption atomic.
#[derive(Clone, Default)]
pub struct EnterpriseAuthState {
    pending_auth: Arc<Mutex<PendingAuthStore>>,
}

impl EnterpriseAuthState {
    /// Empty enterprise auth state.
    pub fn new() -> Self {
        Self::default()
    }

    /// In-flight OIDC login store (`ID-3`), mutable: `/auth/login` records a
    /// pending PKCE state and `/auth/callback` consumes it (one lock hold per
    /// operation — the store's single-use `take` stays atomic).
    pub fn pending_auth_mut(&self) -> MutexGuard<'_, PendingAuthStore> {
        self.pending_auth.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Why `/auth/login` could not begin a flow.
#[derive(Debug)]
pub enum LoginError {
    /// No SSO connection, or it is not an OIDC connection / has no client id.
    NotConfigured,
    /// The OIDC connection has no issuer URL.
    NoIssuer,
    /// OIDC discovery (`.well-known/openid-configuration`) failed.
    Discovery(String),
    /// The OS CSPRNG was unavailable for PKCE/state generation.
    Pkce(String),
}

/// Begin the auth-code + PKCE flow: discover the OP endpoints, mint a PKCE pair + a
/// random CSRF `state`, and build the authorize-endpoint redirect. Returns the URL to
/// send the browser to, the `state` key, and the [`PendingAuth`] for the callback to
/// stash. Free of axum/IO except the injected discovery [`HttpGet`] — so it is tested
/// against a mock OP.
pub fn start_login(
    sso: &SsoConnectionRecord,
    redirect_uri: &str,
    scope: &str,
    mapping: ClaimMapping,
    http: &impl HttpGet,
) -> Result<(String, String, PendingAuth), LoginError> {
    if sso.protocol != SsoProtocol::Oidc {
        return Err(LoginError::NotConfigured);
    }
    if sso.issuer.trim().is_empty() {
        return Err(LoginError::NoIssuer);
    }
    let client_id = sso.audiences.first().ok_or(LoginError::NotConfigured)?;
    let endpoints = discover_endpoints(&sso.issuer, http).map_err(LoginError::Discovery)?;
    let pkce = Pkce::generate().map_err(LoginError::Pkce)?;
    // 16 CSPRNG bytes hex-encoded — unguessable, so a forged `state` cannot collide
    // with a live login (the CSRF binding the OP echoes back).
    let state = hex::encode(gaugewright_app::session::random_bytes::<16>());
    let url = authorize_url(
        &endpoints.authorization_endpoint,
        client_id,
        redirect_uri,
        scope,
        &state,
        &pkce.challenge,
    );
    let pending = PendingAuth {
        verifier: pkce.verifier.into(),
        token_endpoint: endpoints.token_endpoint,
        jwks_uri: endpoints.jwks_uri,
        issuer: sso.issuer.clone(),
        audiences: sso.audiences.clone(),
        redirect_uri: redirect_uri.to_string(),
        mapping,
    };
    Ok((url, state, pending))
}

/// Why `/auth/callback` could not finish a flow.
#[derive(Debug)]
pub enum CallbackError {
    /// The authorization code did not redeem at the token endpoint.
    Exchange(String),
    /// The issuer's JWKS could not be fetched or parsed.
    Jwks(String),
    /// The returned id-token failed signature / claim verification (fail-closed).
    NotVerified,
}

/// Complete the flow: redeem `code` at the token endpoint with the stashed PKCE
/// verifier, fetch the issuer's live JWKS, and verify the returned id-token. Returns
/// the authenticated [`AuthorityId`] and the verified **id-token** (the bearer the
/// control plane accepts). Fail-closed: a code that does not redeem, or a token that
/// does not verify against the live keys, yields an error and no authority (`INV-20`).
pub fn finish_callback(
    pending: &PendingAuth,
    code: &str,
    http: &(impl HttpForm + HttpGet),
) -> Result<(AuthorityId, String), CallbackError> {
    let client_id = pending
        .audiences
        .first()
        .map(String::as_str)
        .unwrap_or_default();
    let id_token = exchange_code(
        &pending.token_endpoint,
        client_id,
        &pending.redirect_uri,
        code,
        pending.verifier.expose(),
        None, // public PKCE client — no client_secret
        http,
    )
    .map_err(CallbackError::Exchange)?;

    // Verify against the issuer's *live* JWKS — the token from the exchange is not yet
    // trusted (it could be anything the token endpoint returned).
    let jwks = http.get(&pending.jwks_uri).map_err(CallbackError::Jwks)?;
    let idp = OidcIdentityProvider::new(pending.issuer.clone(), pending.audiences.clone())
        .with_mapping(pending.mapping.clone())
        .with_jwks(&jwks)
        .map_err(CallbackError::Jwks)?;
    let authority = idp
        .authenticate(&id_token)
        .ok_or(CallbackError::NotVerified)?;
    Ok((authority, id_token))
}

// ---- enterprise-mode activation (wb.idp from the SSO connection) ---------

/// How id-token claims map onto ABAC attributes for a connection (`ID-3`). The home
/// is the SSO connection record (`/admin/sso`); each field falls back to its
/// `GAUGEWRIGHT_OIDC_*_CLAIM` env knob (the legacy operator path) and then to unmapped
/// (fail-closed: no attribute is safer than a wrong one). The subject defaults to `sub`.
/// RBAC console gating reads the member's role from the org directory, not the token —
/// so this only feeds the *attribute* path (roles/region/tenant the ABAC evaluator reads).
pub fn claim_mapping_for(sso: &SsoConnectionRecord) -> ClaimMapping {
    let env_opt = |k: &str| std::env::var(k).ok().filter(|s| !s.trim().is_empty());
    let m = &sso.claim_mapping;
    ClaimMapping {
        subject_claim: m
            .subject_claim
            .clone()
            .or_else(|| env_opt("GAUGEWRIGHT_OIDC_SUBJECT_CLAIM"))
            .unwrap_or_else(|| "sub".to_string()),
        roles_claim: m
            .roles_claim
            .clone()
            .or_else(|| env_opt("GAUGEWRIGHT_OIDC_ROLES_CLAIM")),
        region_claim: m
            .region_claim
            .clone()
            .or_else(|| env_opt("GAUGEWRIGHT_OIDC_REGION_CLAIM")),
        tenant_claim: m
            .tenant_claim
            .clone()
            .or_else(|| env_opt("GAUGEWRIGHT_OIDC_TENANT_CLAIM")),
    }
}

/// JWKS refresh cooldown: at most one discovery fetch per window — so a flood of
/// unknown-`kid` tokens can't stampede the OP, and a persistent outage is retried
/// (not hammered). Also the worst-case heal latency after the IdP recovers.
const JWKS_REFRESH_COOLDOWN: Duration = Duration::from_secs(30);

/// The mutable half of a [`RefreshingOidcProvider`]: the loaded verifier plus what we
/// need to decide whether a verification miss warrants a JWKS refresh.
struct RefreshState {
    provider: OidcIdentityProvider,
    /// The `kid`s of the signing keys currently loaded — a token whose `kid` is here
    /// is verifiable, so a miss for it is a bad token, not a stale key set.
    known_kids: BTreeSet<String>,
    /// Whether any signing key is loaded (the IdP has been reached at least once).
    has_keys: bool,
    /// When we last *attempted* a refresh (success or failure) — the cooldown anchor.
    last_refresh: Option<Instant>,
}

/// An [`IdentityProvider`] that verifies OIDC id-tokens and **self-refreshes** its
/// signing keys from the issuer's JWKS (`ID-3`). Wraps the pure [`OidcIdentityProvider`]
/// (which deliberately speaks no HTTP) with the discovery seam, so:
///
/// - a verifier that started **cold** (the IdP was unreachable at startup) heals on
///   the first login once the IdP is back — no restart, no brick; and
/// - **key rotation** is handled: a token signed by a newly-published key (an unknown
///   `kid`) triggers a refresh and then verifies.
///
/// Refreshes are bounded by [`JWKS_REFRESH_COOLDOWN`] and fire only on a genuine
/// cache-miss (an unknown `kid`, or no keys at all) — never for a token whose `kid` we
/// already hold (a bad signature is just rejected), so invalid tokens cannot stampede
/// the OP. Fail-closed throughout (`INV-20`): until keys load, nothing authenticates.
///
/// The refresh runs synchronously on the verifying call (which holds the workbench
/// lock); it is rare (cache-miss only) and uses a short HTTP timeout, so the stall is
/// bounded. A fully off-lock async refresh is a later refinement.
pub struct RefreshingOidcProvider<H: HttpGet> {
    issuer: String,
    audiences: Vec<String>,
    mapping: ClaimMapping,
    http: H,
    /// Minimum spacing between on-request JWKS refreshes ([`JWKS_REFRESH_COOLDOWN`] in
    /// production; tunable so tests can drive the heal path without real time).
    cooldown: Duration,
    state: Mutex<RefreshState>,
}

impl<H: HttpGet> RefreshingOidcProvider<H> {
    /// Build a verifier for `issuer`, doing a **best-effort** initial JWKS load. If the
    /// IdP is unreachable the verifier is cold (authenticates nothing) but heals on
    /// first use once the IdP is back. `cooldown` bounds on-request refreshes.
    pub fn new(
        issuer: impl Into<String>,
        audiences: Vec<String>,
        mapping: ClaimMapping,
        http: H,
        cooldown: Duration,
    ) -> Self {
        let issuer = issuer.into();
        let me = Self {
            issuer: issuer.clone(),
            audiences: audiences.clone(),
            mapping: mapping.clone(),
            http,
            cooldown,
            state: Mutex::new(RefreshState {
                provider: OidcIdentityProvider::new(issuer, audiences).with_mapping(mapping),
                known_kids: BTreeSet::new(),
                has_keys: false,
                last_refresh: None,
            }),
        };
        let _ = me.refresh(); // warm up; cold is fine (heals on first use)
        me
    }

    /// Whether at least one signing key is loaded (the IdP was reachable). Used by the
    /// activation path to report whether a connection went live or is "saved, pending".
    pub fn is_warm(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .has_keys
    }

    /// Re-fetch the issuer's JWKS and rebuild the inner verifier. A failed fetch leaves
    /// the existing keys intact (a transient outage never *drops* working keys). Does
    /// not touch the cooldown anchor — the warm-up call must not spend the budget, so
    /// the first login after the IdP recovers heals immediately; the cooldown is
    /// anchored by the on-request path in [`authenticate`](Self#impl-IdentityProvider).
    fn refresh(&self) -> Result<(), String> {
        let jwks = discover_jwks(&self.issuer, &self.http)?;
        let provider = OidcIdentityProvider::new(self.issuer.clone(), self.audiences.clone())
            .with_mapping(self.mapping.clone())
            .with_jwks(&jwks)?; // errors unless ≥1 usable signing key
        let kids = jwks_kids(&jwks);
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.provider = provider;
        st.known_kids = kids;
        st.has_keys = true;
        Ok(())
    }
}

impl<H: HttpGet + Send + Sync> IdentityProvider for RefreshingOidcProvider<H> {
    fn authenticate(&self, credential: &str) -> Option<AuthorityId> {
        // Fast path: the cached keys verify it (the common case, no network).
        if let Some(authority) = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .provider
            .authenticate(credential)
        {
            return Some(authority);
        }
        // Miss. Refresh only on a genuine key gap (unknown `kid` / no keys), bounded by
        // the cooldown — a token whose `kid` we already hold is simply invalid.
        let header_kid = decode_header(credential).ok().and_then(|h| h.kid);
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let key_gap = match header_kid.as_deref() {
                Some(kid) => !st.known_kids.contains(kid),
                None => !st.has_keys,
            };
            let cooled = st.last_refresh.is_none_or(|t| t.elapsed() >= self.cooldown);
            if !(key_gap && cooled) {
                return None;
            }
            // Anchor the cooldown here (not in the warm-up): so the budget is spent by
            // on-request refreshes, and a persistent outage can't stampede the OP.
            st.last_refresh = Some(Instant::now());
        } // release the lock before the network fetch
        if self.refresh().is_err() {
            return None;
        }
        // Retry once against the refreshed keys.
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .provider
            .authenticate(credential)
    }

    fn claims(&self, authority: &AuthorityId) -> AuthorityAttributes {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .provider
            .claims(authority)
    }
}

/// The `kid`s of the usable signing keys in a JWKS document (RSA, not `use:"enc"`) —
/// what [`OidcIdentityProvider::with_jwks`] would load. Tracking them lets the
/// refreshing verifier tell "unknown key, refresh" from "known key, just a bad token".
fn jwks_kids(jwks_json: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(jwks_json) else {
        return out;
    };
    let Some(keys) = doc.get("keys").and_then(|k| k.as_array()) else {
        return out;
    };
    for jwk in keys {
        if jwk.get("use").and_then(|u| u.as_str()) == Some("enc") {
            continue;
        }
        if jwk.get("kty").and_then(|v| v.as_str()) != Some("RSA") {
            continue;
        }
        if let Some(kid) = jwk.get("kid").and_then(|v| v.as_str()) {
            out.insert(kid.to_string());
        }
    }
    out
}

/// Build the [`IdentityProvider`] the control plane authenticates bearers against,
/// from the org's stored SSO connection (`ID-3` enterprise-mode activation). This is
/// what makes the id-token `/auth/callback` returns *honored* on `/admin/*`
/// (`Workbench::authorize` → `idp.authenticate`).
///
/// - No connection / not OIDC / no issuer or audiences ⇒ `None`: single-user local
///   mode (admin ungated) — the product's default.
/// - A configured OIDC connection ⇒ a self-refreshing verifier ([`RefreshingOidcProvider`])
///   plus a `warm` flag = whether the issuer's JWKS loaded on this attempt. A cold
///   verifier (IdP unreachable) is still returned — it heals on first use — so the
///   caller decides whether to attach it (startup: yes, fail-closed + healing) or hold
///   off (a runtime reconfigure: keep the working verifier until the new one is warm).
///
/// Touches the network (the initial JWKS load) — call off the async runtime.
pub fn build_oidc_idp(
    sso: Option<&SsoConnectionRecord>,
) -> Option<(Arc<dyn IdentityProvider + Send + Sync>, bool)> {
    let sso = sso?;
    if sso.protocol != SsoProtocol::Oidc || sso.issuer.trim().is_empty() || sso.audiences.is_empty()
    {
        return None;
    }
    let provider = RefreshingOidcProvider::new(
        sso.issuer.clone(),
        sso.audiences.clone(),
        claim_mapping_for(sso),
        HttpClient::with_timeout(Duration::from_secs(5)),
        JWKS_REFRESH_COOLDOWN,
    );
    let warm = provider.is_warm();
    Some((Arc::new(provider), warm))
}

/// Enterprise-mode activation (`ID-3`): if an OIDC SSO connection is configured,
/// attach a self-refreshing id-token verifier so the bearer `/auth/callback`
/// returns is honored on `/admin/*` (`Workbench::authorize` →
/// `idp.authenticate`).
///
/// No connection means single-user local mode, the default. A cold verifier (IdP
/// unreachable at startup) is still attached: it is fail-closed until keys load
/// and self-heals on the first login once the IdP is reachable.
///
/// Runs at **startup**, matching the pre-split workbench-open activation timing:
/// the ee composition setup ([`crate::org_routes::enterprise_control_plane`])
/// calls it before serving, and the hosted shell (`gaugewright-cloud-server`)
/// calls it right after workbench open. Installs the verifier via the open
/// `Workbench::set_identity_provider` seam.
pub fn activate_configured_idp(wb: &mut Workbench) {
    let sso = Org::rebuild(wb.store_ref()).ok().and_then(|o| o.sso);
    if let Some((idp, warm)) = build_oidc_idp(sso.as_ref()) {
        if !warm {
            eprintln!(
                "[gaugewright] WARNING: OIDC SSO is configured but the IdP was unreachable at \
                 startup; /admin/* is fail-closed and the verifier will self-heal on the first \
                 login once the IdP is reachable (no restart needed)."
            );
        }
        wb.set_identity_provider(Some(idp));
    }
}

/// Extract the `email` claim from an **already-verified** id-token (the caller verified
/// signature + claims via [`finish_callback`]) — used only for JIT domain matching, so
/// decoding the payload without re-checking the signature is safe here. `None` if the
/// token has no readable `email`.
fn email_claim(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("email")
        .and_then(|e| e.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

/// JIT provisioning (`ONB-2`): a successful SSO login whose verified subject is **not
/// yet a member** auto-creates an active `member` — *iff* the subject's email domain is
/// a **verified** org domain (the same basis as domain-capture, `ID-6`). Fail-closed
/// (`INV-20`): an unverified domain (or no email claim) provisions nothing — the user
/// must be invited or SCIM-provisioned. No-op if already an active member. Returns
/// whether a member was newly provisioned. JIT seeds `member`; SCIM/group-mapping or an
/// admin elevates (the directory stays the role authority).
pub fn jit_provision(wb: &mut Workbench, scope: &str, authority: &str, id_token: &str) -> bool {
    let Ok(org) = Org::rebuild_in(wb.store_ref(), scope) else {
        return false;
    };
    if org.role_of(authority).is_some() {
        return false; // already an active member
    }
    let Some(email) = email_claim(id_token) else {
        return false; // no email ⇒ cannot match a verified domain (fail-closed)
    };
    if !org.domain_is_verified(&email) {
        return false; // unverified domain ⇒ no auto-join (fail-closed)
    }
    let record = MembershipRecord {
        id: authority.to_string(),
        op: RecordOp::Upsert,
        org_id: ORG_ID.to_string(),
        authority: authority.to_string(),
        email,
        role: "member".to_string(),
        status: MembershipStatus::Active,
        managed_by_scim: false,
        team: None,
    };
    crate::org_routes::write_membership(wb, scope, &record);
    gaugewright_app::audit::record(wb, authority, "member.jit-provision", authority);
    true
}

// ---- axum handlers -------------------------------------------------------

/// The `redirect_uri` this control plane registers with the OP. An explicit
/// `GAUGEWRIGHT_OIDC_REDIRECT_URI` wins (the value registered at the IdP); otherwise it
/// is derived from the request `Host` so a default loopback dev run works unconfigured.
fn callback_redirect_uri(headers: &HeaderMap) -> String {
    if let Ok(uri) = std::env::var("GAUGEWRIGHT_OIDC_REDIRECT_URI") {
        if !uri.trim().is_empty() {
            return uri;
        }
    }
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    format!("http://{host}/auth/callback")
}

fn login_err(e: LoginError) -> axum::response::Response {
    let (code, msg) = match e {
        LoginError::NotConfigured => (
            StatusCode::CONFLICT,
            "SSO is not configured for OIDC (set an OIDC connection + client id at /admin/sso)"
                .to_string(),
        ),
        LoginError::NoIssuer => (
            StatusCode::CONFLICT,
            "the OIDC SSO connection has no issuer".to_string(),
        ),
        LoginError::Discovery(m) => (
            StatusCode::BAD_GATEWAY,
            format!("OIDC discovery failed: {m}"),
        ),
        LoginError::Pkce(m) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("PKCE generation failed: {m}"),
        ),
    };
    (code, msg).into_response()
}

/// `GET /auth/login` — begin OIDC login: discover, mint PKCE + state, stash, and
/// redirect the browser to the IdP. See the module docs. The pending-login store
/// arrives as the composition-scoped [`EnterpriseAuthState`] extension.
pub async fn get_login(
    State(wb): State<SharedWorkbench>,
    Extension(auth): Extension<EnterpriseAuthState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let sso = {
        let wb = wb.lock_unpoisoned();
        match Org::rebuild_in(wb.store_ref(), &crate::org_routes::req_scope(&headers)) {
            Ok(org) => org.sso,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
        }
    };
    let Some(sso) = sso else {
        return (StatusCode::CONFLICT, "no SSO connection configured").into_response();
    };

    let redirect_uri = callback_redirect_uri(&headers);
    let scope = std::env::var("GAUGEWRIGHT_OIDC_SCOPE")
        .unwrap_or_else(|_| "openid profile email".to_string());
    // The claim mapping comes from the connection record (env-fallback) — the same
    // resolution the durable verifier uses, so the shell and `wb.idp` agree (`ID-3`).
    let mapping = claim_mapping_for(&sso);

    // Discovery touches the network — run it off the async runtime (ureq is blocking).
    let started = tokio::task::spawn_blocking(move || {
        let http = HttpClient::new();
        start_login(&sso, &redirect_uri, &scope, mapping, &http)
    })
    .await;
    let (url, state, pending) = match started {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return login_err(e),
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "login task panicked").into_response()
        }
    };

    auth.pending_auth_mut().begin(state, pending);
    Redirect::to(&url).into_response()
}

/// The OP's redirect-back query: a success carries `code` + `state`; a denial carries
/// `error` (+ optional `error_description`) per RFC 6749 §4.1.2.1.
#[derive(Deserialize)]
pub struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

fn callback_err(e: CallbackError) -> axum::response::Response {
    let (code, msg) = match e {
        CallbackError::Exchange(m) => (
            StatusCode::BAD_GATEWAY,
            format!("token exchange failed: {m}"),
        ),
        CallbackError::Jwks(m) => (
            StatusCode::BAD_GATEWAY,
            format!("JWKS fetch/parse failed: {m}"),
        ),
        CallbackError::NotVerified => (
            StatusCode::UNAUTHORIZED,
            "the id-token did not verify".to_string(),
        ),
    };
    (code, msg).into_response()
}

/// `GET /auth/callback` — finish OIDC login: match the CSRF `state`, redeem the code,
/// verify the id-token, audit the login, and hand the verified id-token (the bearer)
/// back to the client. See the module docs.
pub async fn get_callback(
    State(wb): State<SharedWorkbench>,
    Extension(auth): Extension<EnterpriseAuthState>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> impl IntoResponse {
    // SECAUD-8: per-tenant failed-callback lockout (429 when locked) — defense-in-depth
    // behind the edge rate-limit, mirroring the SCIM guard. A bad/replayed state or a failed
    // token exchange records a failure; a completed login clears the tenant's count.
    let tenant = crate::org_routes::req_scope(&headers);
    let throttle = wb.lock_unpoisoned().oidc_throttle().clone();
    let now = throttle.now_ms();
    if !throttle.allowed(&tenant, now) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "too many failed SSO callbacks; retry later",
        )
            .into_response();
    }
    if let Some(err) = q.error {
        let desc = q.error_description.unwrap_or_default();
        return (
            StatusCode::UNAUTHORIZED,
            format!("the IdP denied the login: {err} {desc}")
                .trim()
                .to_string(),
        )
            .into_response();
    }
    let (Some(code), Some(state)) = (q.code, q.state) else {
        throttle.record_failure(&tenant, now);
        return (StatusCode::BAD_REQUEST, "missing code or state").into_response();
    };

    // Single-use take: an unknown / replayed `state` finds nothing (CSRF guard).
    let pending = auth.pending_auth_mut().take(&state);
    let Some(pending) = pending else {
        throttle.record_failure(&tenant, now);
        return (StatusCode::BAD_REQUEST, "unknown or expired state").into_response();
    };

    let finished = tokio::task::spawn_blocking(move || {
        let http = HttpClient::new();
        finish_callback(&pending, &code, &http)
    })
    .await;
    let (authority, id_token) = match finished {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            throttle.record_failure(&tenant, now);
            return callback_err(e);
        }
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "callback task panicked").into_response()
        }
    };

    // A completed exchange clears the tenant's failed-callback count (SECAUD-8).
    throttle.record_success(&tenant);

    // Attribute the login to the authenticated authority (`AUD-1` / `INV-21`), and
    // JIT-provision a verified-domain newcomer as a member (`ONB-2`, fail-closed).
    {
        let mut wb = wb.lock_unpoisoned();
        let actor = authority.as_str().to_string();
        gaugewright_app::audit::record(&mut wb, &actor, "auth.login", authority.as_str());
        jit_provision(
            &mut wb,
            &crate::org_routes::req_scope(&headers),
            authority.as_str(),
            &id_token,
        );
    }

    // Deliver the bearer. With a configured client URL, 302 there with the token in
    // the URL *fragment* (not a query param — fragments are never sent to servers, so
    // the token stays out of access logs / `Referer`); otherwise return JSON.
    if let Ok(url) = std::env::var("GAUGEWRIGHT_OIDC_POST_LOGIN_URL") {
        if !url.trim().is_empty() {
            // A JWT is base64url + `.` — all URL-fragment-safe, no escaping needed.
            let target = format!("{url}#id_token={id_token}&token_type=Bearer");
            return Redirect::to(&target).into_response();
        }
    }
    (
        StatusCode::OK,
        Json(json!({
            "authority": authority.as_str(),
            "id_token": id_token,
            "token_type": "Bearer",
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // A real RSA-2048 test keypair (generated for tests only, never used in
    // production). The OP "signs" id-tokens with the private half; the shell verifies
    // against the JWKS the mock serves (the public modulus), exercising the real
    // asymmetric RS256 path — the algorithm Okta / Entra / Google all default to.
    const RSA_PRIVATE_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDDcxUtTJr7p7iI
vTn0SpwErh4ghN/ebHcz75T3xX9S5WxdaP2RRXzvO9tSC71dI+sGvjj1h5NlJSFg
v6DJ28ZbZdqh8rNRgtjvoQbzJLPcnGm15Qoxndu59csA8lMLv/dce4jx/XEpcNEK
3PsN7iyQuIM3maURPrmkJR0BNgVQ4UaNU/sbRHZqbFWUad2t49WooG8CsY5ITSMu
9gJQaT2aXY1JMqfq+SCiSDnw0FZhpcuYiFz9vRHQI+d4hHxIhN/lDg5CAJYQaKuX
01hormXh9Ra57INW6D9Afs9vF8Eh6aSngPbmCgfS29FAEzINrPOmtw1PH8tXTUc1
jjG5IXjtAgMBAAECggEAQ+escGAgqpVjkjJ4O61eVmvuMKsponv50FQJZCo8ad8m
zq9nBb1oQjAAK5nDkWQkyGN3o6qWZbpIRfZeFTPjzyZslv6dGZFF8L94DCrwyJGZ
UqaAa6umRw4kGTCX9Mmd1gZfln/Q/K5jGoybNwRMfH12rW8WwA6UbfitApozr5zw
jCVef7sBNvUw7s9n8x/OAmuzzRGwOX7vNBh/FkeIv5zYoCAeNDCejpoSBCp1PDUb
0ryev+LTi7WlYXGkwYCFLpzUie2GrAgnzHg9h4tuuNdrn5ZKCB3Bo6+65ENNFOla
xdh77h8g1ooGDAV/k7I2bQWX0k05UVR4nGninsT8OwKBgQD88EouhaQ+cu1qki/M
vI4Ct+gJzfurq8atfup3be8SZIiNSnllIiZIM0c7/ulPG5mTn6f3xenQlQay7wMB
uQzIJEGjj/2u+nRgKrhYswD4zn4lrDH5ySQGBlNkHCLU1CtZqtGQLwQ4jO3sVDr/
q9RLzwR66XYK8wkOa7GDTbrISwKBgQDF0KrmahY8+Gs0VhRqa7DvyC++fADPxYKc
wdRWOAZRyKNMEPOewsm9ymLt67xj2PgFIe/glrGX/Ouwhm+mirXN3KXFwvtp5KCH
nWIIaJyqTByGYQByFbh3S6Mijwg5PldK7ygkvTptiPCUkmZCDYw+/3hHMXGnFqQM
KnlgTPhwpwKBgQC29mHSkR0jhyKxihlFcccPtFQGc5dusIzAhyO3TDA5D7uu6IYz
X6ZtZ5pJjbTaYk6O+FgZ5HGjTYlQ+Y8lOeRDCebpF4kbf1ObFIvQrXswfr3FJm/o
DVUffofnzGptpSPOcr+wGjJlbZvU7YDX3EVuqMrG1gVrGi4c3k3DewB3TQKBgB0H
3KzoEM9t3b3WjDR6DYODK46XAD99ywdaYuEsY7EI8v4s1rQL/jN+SjqEiCdXJj8K
lfut4e5eTfCgKi6U2M2XfjShwufth6mfbU2ynJtZhC4sejZD/ch0L0LZHunXvlPe
+VM6+iItILGNMriq6FQuheZc2UMeTYEDksCRSzytAoGAb//H+J3Q73ulQKY0ydF9
fwnv+jEOksgeG3wM+fQkqTqWyBYZLOQhc47xGFMBnY46Qcagq1VzRidTQkACZpRP
Ml6HHZjRK98Vq4rtCrAPJ3f8Vth24MkZ9VlXSmo4L9WGI14ao54uWtp9h+EXfumO
iqlTEKVISscuchxZtKQJ4k8=
-----END PRIVATE KEY-----";
    const JWK_N: &str = "w3MVLUya-6e4iL059EqcBK4eIITf3mx3M--U98V_UuVsXWj9kUV87zvbUgu9XSPrBr449YeTZSUhYL-gydvGW2XaofKzUYLY76EG8ySz3JxpteUKMZ3bufXLAPJTC7_3XHuI8f1xKXDRCtz7De4skLiDN5mlET65pCUdATYFUOFGjVP7G0R2amxVlGndrePVqKBvArGOSE0jLvYCUGk9ml2NSTKn6vkgokg58NBWYaXLmIhc_b0R0CPneIR8SITf5Q4OQgCWEGirl9NYaK5l4fUWueyDVug_QH7PbxfBIemkp4D25goH0tvRQBMyDazzprcNTx_LV01HNY4xuSF47Q";
    const KID: &str = "shell-test-rsa";
    const ISSUER: &str = "https://idp.example.test";
    const CLIENT_ID: &str = "gaugewright-shell";
    const TOKEN_ENDPOINT: &str = "https://idp.example.test/token";
    const AUTHZ_ENDPOINT: &str = "https://idp.example.test/authorize";
    const JWKS_URI: &str = "https://idp.example.test/keys";

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn mint_id_token() -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(KID.to_string());
        let claims = json!({
            "iss": ISSUER,
            "aud": CLIENT_ID,
            "sub": "alice@example.test",
            "exp": now() + 3600,
            "iat": now(),
            "roles": ["admin"],
        });
        let key = EncodingKey::from_rsa_pem(RSA_PRIVATE_PEM).expect("test signing key");
        encode(&header, &claims, &key).expect("encode id-token")
    }

    fn jwks() -> String {
        format!(
            r#"{{"keys":[{{"kty":"RSA","use":"sig","kid":"{KID}","n":"{n}","e":"AQAB"}}]}}"#,
            n = JWK_N.trim()
        )
    }

    fn discovery() -> String {
        json!({
            "issuer": ISSUER,
            "authorization_endpoint": AUTHZ_ENDPOINT,
            "token_endpoint": TOKEN_ENDPOINT,
            "jwks_uri": JWKS_URI,
        })
        .to_string()
    }

    /// A mock OP: canned GETs (discovery, JWKS) and a token endpoint that records the
    /// posted form and returns a minted id-token.
    struct MockOp {
        gets: BTreeMap<String, String>,
        token_response: String,
        seen_form: Mutex<Vec<(String, String)>>,
    }
    impl HttpGet for MockOp {
        fn get(&self, url: &str) -> Result<String, String> {
            self.gets
                .get(url)
                .cloned()
                .ok_or_else(|| format!("404 {url}"))
        }
    }
    impl HttpForm for MockOp {
        fn post_form(&self, _url: &str, fields: &[(&str, &str)]) -> Result<String, String> {
            *self.seen_form.lock().unwrap() = fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            Ok(self.token_response.clone())
        }
    }

    fn mock_op(token_response: String) -> MockOp {
        let mut gets = BTreeMap::new();
        gets.insert(
            format!("{ISSUER}/.well-known/openid-configuration"),
            discovery(),
        );
        gets.insert(JWKS_URI.to_string(), jwks());
        MockOp {
            gets,
            token_response,
            seen_form: Mutex::new(vec![]),
        }
    }

    fn oidc_sso() -> SsoConnectionRecord {
        SsoConnectionRecord {
            protocol: SsoProtocol::Oidc,
            issuer: ISSUER.to_string(),
            audiences: vec![CLIENT_ID.to_string()],
            ..Default::default()
        }
    }

    #[test]
    fn pending_store_take_is_single_use() {
        let mut store = PendingAuthStore::new();
        let pending = PendingAuth {
            verifier: "v".into(),
            token_endpoint: TOKEN_ENDPOINT.into(),
            jwks_uri: JWKS_URI.into(),
            issuer: ISSUER.into(),
            audiences: vec![CLIENT_ID.into()],
            redirect_uri: "http://localhost/auth/callback".into(),
            mapping: ClaimMapping::default(),
        };
        store.begin("state-1", pending);
        assert_eq!(store.len(), 1);
        assert!(store.take("state-1").is_some(), "first take redeems");
        assert!(
            store.take("state-1").is_none(),
            "second take is empty (single-use)"
        );
        assert!(store.is_empty());
    }

    #[test]
    fn unknown_state_finds_nothing() {
        let mut store = PendingAuthStore::new();
        // The CSRF guard: a forged `state` the server never minted finds no verifier.
        assert!(store.take("forged").is_none());
    }

    #[test]
    fn claim_mapping_prefers_the_record_and_defaults_the_subject() {
        // The record is the home (ID-3): its claim names win, and the subject defaults
        // to `sub` when unset — independent of any GAUGEWRIGHT_OIDC_*_CLAIM env fallback.
        let mut sso = oidc_sso();
        sso.claim_mapping = gaugewright_app::org::SsoClaimMapping {
            roles_claim: Some("groups".into()),
            region_claim: Some("locale".into()),
            ..Default::default()
        };
        let m = claim_mapping_for(&sso);
        assert_eq!(m.roles_claim.as_deref(), Some("groups"));
        assert_eq!(m.region_claim.as_deref(), Some("locale"));
        assert_eq!(m.subject_claim, "sub");
    }

    /// A JWT-shaped token carrying the given claims (signature irrelevant — JIT decodes
    /// the payload of an already-verified token).
    fn token_with_claims(claims: serde_json::Value) -> String {
        let b64 = |v: &serde_json::Value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(v).unwrap())
        };
        format!("{}.{}.sig", b64(&json!({ "alg": "none" })), b64(&claims))
    }

    #[test]
    fn jit_provisions_a_verified_domain_subject_and_skips_others() {
        use gaugewright_app::org::{Org, OrgRecord, ORG_ID, ORG_SCOPE};
        let store = gaugewright_store::Store::open_in_memory().unwrap();
        let mut wb = Workbench::new(store);
        // Seed an org with a verified domain (the JIT basis).
        let org_rec = OrgRecord {
            id: ORG_ID.to_string(),
            op: RecordOp::Upsert,
            display_name: "Acme".into(),
            verified_domains: vec!["acme.com".into()],
            default_region: None,
            kind: Default::default(),
        };
        wb.store_mut()
            .append_record(ORG_SCOPE, "org", &serde_json::to_string(&org_rec).unwrap())
            .unwrap();

        // Verified-domain subject → provisioned as an active member.
        let tok = token_with_claims(json!({ "sub": "sub-alice", "email": "alice@acme.com" }));
        assert!(
            jit_provision(&mut wb, gaugewright_app::org::ORG_SCOPE, "sub-alice", &tok),
            "verified domain provisions"
        );
        assert!(Org::rebuild(wb.store_ref())
            .unwrap()
            .role_of("sub-alice")
            .is_some());
        // Idempotent: already a member → no-op.
        assert!(!jit_provision(
            &mut wb,
            gaugewright_app::org::ORG_SCOPE,
            "sub-alice",
            &tok
        ));

        // Unverified domain → fail-closed, no provision.
        let evil = token_with_claims(json!({ "sub": "sub-eve", "email": "eve@evil.com" }));
        assert!(!jit_provision(
            &mut wb,
            gaugewright_app::org::ORG_SCOPE,
            "sub-eve",
            &evil
        ));
        assert!(Org::rebuild(wb.store_ref())
            .unwrap()
            .role_of("sub-eve")
            .is_none());

        // No email claim → cannot match a verified domain → no provision.
        let anon = token_with_claims(json!({ "sub": "sub-anon" }));
        assert!(!jit_provision(
            &mut wb,
            gaugewright_app::org::ORG_SCOPE,
            "sub-anon",
            &anon
        ));
        assert!(Org::rebuild(wb.store_ref())
            .unwrap()
            .role_of("sub-anon")
            .is_none());
    }

    #[test]
    fn start_login_builds_a_pkce_authorize_url() {
        let op = mock_op(String::new());
        let (url, state, pending) = start_login(
            &oidc_sso(),
            "http://localhost:1421/auth/callback",
            "openid profile email",
            ClaimMapping {
                roles_claim: Some("roles".into()),
                ..ClaimMapping::default()
            },
            &op,
        )
        .expect("login starts");

        assert!(
            url.starts_with(AUTHZ_ENDPOINT),
            "redirects to the discovered authorize endpoint"
        );
        assert!(url.contains("response_type=code"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(
            url.contains(&format!("state={state}")),
            "the CSRF state is on the URL"
        );
        assert!(url.contains("scope=openid%20profile%20email"));
        // The verifier is kept server-side, never on the authorize URL.
        assert!(!url.contains(pending.verifier.expose()));
        assert_eq!(pending.token_endpoint, TOKEN_ENDPOINT);
        assert_eq!(pending.jwks_uri, JWKS_URI);
    }

    #[test]
    fn start_login_refuses_a_non_oidc_or_unconfigured_connection() {
        let op = mock_op(String::new());
        // SAML connection → not this shell's job.
        let mut saml = oidc_sso();
        saml.protocol = SsoProtocol::Saml;
        assert!(matches!(
            start_login(&saml, "http://x/cb", "openid", ClaimMapping::default(), &op),
            Err(LoginError::NotConfigured)
        ));
        // OIDC but no issuer.
        let mut no_issuer = oidc_sso();
        no_issuer.issuer = String::new();
        assert!(matches!(
            start_login(
                &no_issuer,
                "http://x/cb",
                "openid",
                ClaimMapping::default(),
                &op
            ),
            Err(LoginError::NoIssuer)
        ));
        // OIDC, issuer, but no client id.
        let mut no_client = oidc_sso();
        no_client.audiences.clear();
        assert!(matches!(
            start_login(
                &no_client,
                "http://x/cb",
                "openid",
                ClaimMapping::default(),
                &op
            ),
            Err(LoginError::NotConfigured)
        ));
    }

    #[test]
    fn finish_callback_exchanges_then_verifies_the_id_token() {
        let id_token = mint_id_token();
        let op = mock_op(json!({ "id_token": id_token, "token_type": "Bearer" }).to_string());

        // The pending state a prior login leg would have stashed.
        let (_url, _state, pending) = start_login(
            &oidc_sso(),
            "http://localhost:1421/auth/callback",
            "openid",
            ClaimMapping {
                roles_claim: Some("roles".into()),
                ..ClaimMapping::default()
            },
            &op,
        )
        .unwrap();

        let (authority, returned) =
            finish_callback(&pending, "auth-code-xyz", &op).expect("callback verifies");
        assert_eq!(authority, AuthorityId::new("alice@example.test"));
        assert_eq!(
            returned, id_token,
            "the verified id-token is the bearer handed back"
        );

        // The PKCE verifier was presented on the exchange (so an intercepted code is useless).
        let seen = op.seen_form.lock().unwrap();
        assert!(seen
            .iter()
            .any(|(k, v)| k == "code_verifier" && v.as_str() == pending.verifier.expose()));
        assert!(seen
            .iter()
            .any(|(k, v)| k == "grant_type" && v == "authorization_code"));
        assert!(seen
            .iter()
            .any(|(k, v)| k == "code" && v == "auth-code-xyz"));
    }

    #[test]
    fn finish_callback_rejects_a_token_for_the_wrong_audience() {
        // The token endpoint returns a token minted for a *different* client — the
        // shell's verification (aud check) must reject it (fail-closed).
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(KID.to_string());
        let claims = json!({
            "iss": ISSUER, "aud": "some-other-client", "sub": "mallory",
            "exp": now() + 3600, "iat": now(),
        });
        let key = EncodingKey::from_rsa_pem(RSA_PRIVATE_PEM).unwrap();
        let bad = encode(&header, &claims, &key).unwrap();
        let op = mock_op(json!({ "id_token": bad }).to_string());
        let (_u, _s, pending) = start_login(
            &oidc_sso(),
            "http://x/cb",
            "openid",
            ClaimMapping::default(),
            &op,
        )
        .unwrap();
        assert!(matches!(
            finish_callback(&pending, "code", &op),
            Err(CallbackError::NotVerified)
        ));
    }

    #[test]
    fn build_oidc_idp_is_none_for_local_or_non_oidc() {
        // These return before any network — no connection / SAML / no issuer ⇒ single-
        // user local (None). (A configured-OIDC build touches the network, so the
        // construction + self-heal behaviour is exercised via RefreshingOidcProvider.)
        assert!(build_oidc_idp(None).is_none());
        let mut saml = oidc_sso();
        saml.protocol = SsoProtocol::Saml;
        assert!(build_oidc_idp(Some(&saml)).is_none());
        let mut no_issuer = oidc_sso();
        no_issuer.issuer = String::new();
        assert!(build_oidc_idp(Some(&no_issuer)).is_none());
    }

    /// An OP whose reachability can be toggled, counting GETs — to drive the cold →
    /// recover → heal path and the refresh cooldown deterministically.
    struct ToggleOp {
        online: Arc<AtomicBool>,
        get_calls: Arc<AtomicUsize>,
    }
    impl HttpGet for ToggleOp {
        fn get(&self, url: &str) -> Result<String, String> {
            self.get_calls.fetch_add(1, Ordering::SeqCst);
            if !self.online.load(Ordering::SeqCst) {
                return Err("offline".to_string());
            }
            if url.ends_with("/.well-known/openid-configuration") {
                return Ok(discovery());
            }
            if url == JWKS_URI {
                return Ok(jwks());
            }
            Err(format!("404 {url}"))
        }
    }

    #[test]
    fn refreshing_provider_heals_after_a_cold_start_when_the_idp_recovers() {
        let online = Arc::new(AtomicBool::new(false));
        let op = ToggleOp {
            online: online.clone(),
            get_calls: Arc::new(AtomicUsize::new(0)),
        };
        // Cold start (IdP unreachable): fail-closed — nothing authenticates.
        let idp = RefreshingOidcProvider::new(
            ISSUER,
            vec![CLIENT_ID.to_string()],
            ClaimMapping::default(),
            op,
            Duration::ZERO, // no cooldown wait in the test
        );
        assert!(!idp.is_warm(), "cold start has no keys");
        assert_eq!(
            idp.authenticate(&mint_id_token()),
            None,
            "cold ⇒ fail-closed"
        );

        // The IdP comes back. The next login triggers a JWKS refresh and verifies — no
        // restart, no brick.
        online.store(true, Ordering::SeqCst);
        assert_eq!(
            idp.authenticate(&mint_id_token()),
            Some(AuthorityId::new("alice@example.test")),
            "self-heals on first use once the IdP is reachable"
        );
        assert!(idp.is_warm());
    }

    #[test]
    fn refreshing_provider_does_not_refetch_for_a_known_key_or_within_cooldown() {
        let online = Arc::new(AtomicBool::new(true));
        let calls = Arc::new(AtomicUsize::new(0));
        let op = ToggleOp {
            online: online.clone(),
            get_calls: calls.clone(),
        };
        // Warm start loads the keys (discovery + JWKS = 2 GETs).
        let idp = RefreshingOidcProvider::new(
            ISSUER,
            vec![CLIENT_ID.to_string()],
            ClaimMapping::default(),
            op,
            Duration::from_secs(3600), // long cooldown
        );
        assert!(idp.is_warm());
        let after_warmup = calls.load(Ordering::SeqCst);

        // A valid token verifies off the cached keys — no refetch.
        assert!(idp.authenticate(&mint_id_token()).is_some());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            after_warmup,
            "known key ⇒ no refetch"
        );

        // A token whose `kid` we already hold but with a broken signature is just
        // rejected — it must NOT stampede the OP with refreshes.
        let mut tampered: Vec<char> = mint_id_token().chars().collect();
        let last = tampered.len() - 1;
        tampered[last] = if tampered[last] == 'a' { 'b' } else { 'a' };
        let tampered: String = tampered.into_iter().collect();
        assert_eq!(idp.authenticate(&tampered), None);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            after_warmup,
            "bad signature for a known kid ⇒ no refetch (no stampede)"
        );
    }

    #[test]
    fn finish_callback_surfaces_a_token_response_without_an_id_token() {
        // An OAuth2-only token response (no id_token) is an exchange error.
        let op = mock_op(json!({ "access_token": "at", "token_type": "Bearer" }).to_string());
        let (_u, _s, pending) = start_login(
            &oidc_sso(),
            "http://x/cb",
            "openid",
            ClaimMapping::default(),
            &op,
        )
        .unwrap();
        assert!(matches!(
            finish_callback(&pending, "code", &op),
            Err(CallbackError::Exchange(_))
        ));
    }
}

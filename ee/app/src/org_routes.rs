//! The organization admin surface (`ORG-1`, B10/B11): org settings + the member
//! directory. CRUD here writes durable `org`-scope records (latest-wins / tombstone)
//! and pushes a workspace-change reference so connected clients refresh live — the
//! same record discipline as [`library_routes`](gaugewright_app::library_routes). The org/membership
//! projection is folded **on demand** from the log per request (`INV-5`); the
//! directory is small and these routes are low-frequency, so no hot-path projection
//! is held on the `Workbench`.
//!
//! These routes are **ungated** today (like the rest of the single-authority app);
//! `RBAC-5` wires the actor's role gate (owner/admin may administer) onto them.
//! The one invariant enforced here regardless is structural: an org always retains
//! at least one active `owner` (the break-glass account, `ID-5`).

use axum::routing::{get, patch, post};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;

use gaugewright_core::abac::Policy;
use gaugewright_core::rbac::Capability;

use gaugewright_app::org::{
    is_valid_role, ArchetypeApprovalPolicyRecord, BillingRecord, GroupMappingRecord,
    MemberGrantRecord, MembershipRecord, MembershipStatus, Org, OrgRecord, PolicyRecord, RecordOp,
    SecurityPolicyRecord, SsoConnectionRecord, ORG_ID,
};
use gaugewright_app::{LockUnpoisoned, SharedWorkbench, Workbench};

/// The ee enterprise composition (SPLIT-1): the open route surface plus this
/// module's governance routes, wrapped in the same ENTSEC-1 auth route-layer /
/// CORS / HSTS ordering the hosted cross-band composition
/// (`gaugewright-cloud-server`) uses — minus the settlement, attestation, and
/// embed planes, which live in their own `cloud/` crates. Enterprise
/// integration tests compose against this.
///
/// Composition setup also runs enterprise-mode activation
/// ([`crate::auth_oidc::activate_configured_idp`]): the persisted Org SSO
/// connection attaches the OIDC verifier before any request is served — the
/// same pre-request timing the pre-split workbench-open activation had.
pub fn enterprise_control_plane(wb: SharedWorkbench) -> Router {
    crate::auth_oidc::activate_configured_idp(&mut wb.lock_unpoisoned());
    // ENTSEC-1: the middleware needs its own handle to the workbench (the router
    // moves `wb` into `.with_state`).
    let auth_wb = wb.clone();
    let federation_on = {
        let g = wb.lock_unpoisoned();
        g.is_federation_enabled()
    };
    Router::new()
        .merge(gaugewright_app::local_routes::routes(federation_on))
        .merge(routes())
        .merge(gaugewright_app::account_routes::routes())
        .route_layer(axum::middleware::from_fn_with_state(
            auth_wb,
            enterprise_auth,
        ))
        .layer(gaugewright_app::net_http::cors_layer())
        .with_state(wb)
        .layer(axum::middleware::from_fn(
            gaugewright_app::net_http::security_headers,
        ))
}

/// Source-available enterprise governance route surface: SSO/OIDC/SAML, SCIM,
/// RBAC, org policy, audit, security, and billing-seat administration. Always
/// the real routes — the `ee/` crate boundary is the band gate (ADR 0069), so
/// the old `featured_routes()` feature on/off wrapper collapsed with the
/// extraction. Mints one composition-scoped
/// [`EnterpriseAuthState`](crate::auth_oidc::EnterpriseAuthState) (the OIDC
/// pending-login store) and hands it to the `/auth/*` handlers as an
/// `Extension`, so the pending-login lifetime spans requests.
pub fn routes() -> Router<SharedWorkbench> {
    let enterprise_auth_state = crate::auth_oidc::EnterpriseAuthState::new();
    Router::new()
        // SP integration details (M3 ONB-1): the values an admin pastes into their IdP
        // (`/admin/integration`, console-gated) + the public SP metadata descriptor.
        .route("/admin/integration", get(get_integration))
        .route("/saml/metadata", get(get_saml_metadata))
        // SSO test-connection (M3 ONB-3): live OIDC discovery + JWKS reachability check.
        .route("/admin/sso/test", post(post_sso_test))
        // DNS-TXT domain verification (M3 ONB-5): the proof behind auto-join/JIT.
        .route(
            "/admin/domains/verify-token",
            post(post_domain_verify_token),
        )
        .route("/admin/domains/verify", post(post_domain_verify))
        // Organization admin surface (M3 ORG-1, B10/B11): org settings + the member
        // directory. Ungated today; RBAC-5 adds the role gate.
        .route("/admin/org", get(get_org).post(post_org))
        .route("/admin/members", get(get_members).post(post_member))
        .route("/admin/sessions", get(get_sessions))
        // Per-actor audit timeline (M3 B14 / AUD-1, AUD-2): filterable + CSV export.
        .route("/admin/audit", get(get_audit_log))
        .route("/admin/audit/verify", get(get_audit_verify))
        // Domain-capture auto-join (M3 ID-6): verified-domain email -> active member.
        .route("/admin/members/auto-join", post(post_auto_join))
        // Member->project scope grants (ENTSEC-2 / ADR 0065): how a scoped member (a consultant)
        // is given access to their engagement's project. ManageMembers to write; console-read to
        // list. Owner/admin bypass scoping, so they need no grant.
        .route(
            "/admin/grants",
            get(get_grants).post(post_grant).delete(delete_grant),
        )
        .route("/admin/members/:id/role", post(post_member_role))
        .route(
            "/admin/members/:id/deactivate",
            post(post_member_deactivate),
        )
        // Org resource-floor policy (M3 RBAC-6, B15): the per-org Policy the export
        // gate reads. Read needs console access; write needs ConfigureSecurity.
        .route("/admin/policy", get(get_policy).post(post_policy))
        // Placement policy (DEPLOY-2): admissible (operator, attested) deployment modes for
        // engagements touching this org's data. Read needs console access; write needs
        // ConfigureSecurity. The pairing gate (DEPLOY-3) consults it at the client's accept.
        .route(
            "/admin/placement-policy",
            get(get_placement_policy).post(post_placement_policy),
        )
        // SSO connection (M3 B12 / ID-5): which IdP, and the enforce-SSO flag.
        .route("/admin/sso", get(get_sso).post(post_sso))
        // OIDC auth-code + PKCE login shell (M3 ID-3): `/auth/login` redirects the
        // browser to the configured IdP; `/auth/callback` redeems the code and hands
        // back the verified id-token (the bearer the admin routes accept).
        .route("/auth/login", get(crate::auth_oidc::get_login))
        .route("/auth/callback", get(crate::auth_oidc::get_callback))
        // Session refresh (ADR 0077): a still-valid session mints a fresh id-token cookie from the
        // stored refresh token, so a hosted session outlives the ~1h id-token without re-login.
        .route("/auth/refresh", get(crate::auth_oidc::get_refresh))
        // Security policy (M3 B15 / SEC-1/2/3): MFA, session, residency default.
        .route("/admin/security", get(get_security).post(post_security))
        // Archetype-approval policy (ADR 0063): the org default for whether adding an
        // archetype to a project requires owner approval (pending) or is frictionless.
        .route(
            "/admin/archetype-approval",
            get(get_archetype_approval).post(post_archetype_approval),
        )
        // Billing & seats (M3 B16 / BILL-1/3): operational only, never authority.
        .route("/admin/billing", get(get_billing).post(post_billing))
        // SCIM provisioning (M3 B13): admin issues/rotates the SCIM token; the IdP
        // drives the token-authenticated SCIM Users endpoints.
        .route(
            "/admin/scim/token",
            post(crate::scim_routes::post_scim_token),
        )
        // SCIM group -> role/team mapping (M3 SCIM-3).
        .route("/admin/scim/group-mapping", post(post_group_mapping))
        .route("/scim/v2/Users", post(crate::scim_routes::post_scim_user))
        .route(
            "/scim/v2/Users/:id",
            patch(crate::scim_routes::patch_scim_user).delete(crate::scim_routes::delete_scim_user),
        )
        .layer(axum::Extension(enterprise_auth_state))
}

// ---- helpers -------------------------------------------------------------

fn op_str(op: RecordOp) -> &'static str {
    match op {
        RecordOp::Upsert => "upsert",
        RecordOp::Tombstone => "tombstone",
    }
}

/// The bearer credential from the `Authorization: Bearer <token>` header — the
/// open neutral parser, re-exported for this module's handlers and tests.
pub use gaugewright_app::net_http::bearer;

/// The open admin-gate and tenant-scope substrate ([`gaugewright_app::workbench_auth`]),
/// re-exported so the ee route surface and its sibling modules keep one import home.
/// The settlement plane (`gaugewright-cloud-settlement`) consumes the same seams
/// directly from the open crate.
pub use gaugewright_app::workbench_auth::{deny, req_scope};

/// ENTSEC-1: paths that bypass the enterprise data-route auth gate — the pre-auth and
/// own-auth flows. `/health` (readiness), `/auth/*` (the OIDC login/callback that *mints* the
/// bearer), `/scim/*` (its own SCIM bearer token), `/saml/*` (IdP metadata/ACS), `/federation/*`
/// (cross-machine signed-envelope auth, not org-member bearers), and `/test/*` (env-gated reset).
/// The public embed/audience plane is merged AFTER this layer, so it is never wrapped and keeps
/// its own audience auth.
fn entsec_exempt(path: &str) -> bool {
    path == "/health"
        || path.starts_with("/auth/")
        || path.starts_with("/scim/")
        || path.starts_with("/saml/")
        || path.starts_with("/federation/")
        || path.starts_with("/test/")
}

/// ENTSEC-1 middleware ([ADR 0065]): in **enterprise mode** (an `IdentityProvider` is attached
/// and the directory is provisioned) every consultant route requires an authenticated active
/// member; **solo / loopback passes through** (the control-plane API is the local operator's own
/// channel). Exempt paths keep their own auth / pre-auth flow. Fail-closed (`INV-20`).
pub async fn enterprise_auth(
    axum::extract::State(wb): axum::extract::State<SharedWorkbench>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    // CORS preflight carries no credentials — let it through (the CORS layer answers it).
    if req.method() == axum::http::Method::OPTIONS || entsec_exempt(req.uri().path()) {
        return next.run(req).await;
    }
    let bearer = bearer(req.headers());
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    {
        let mut guard = wb.lock_unpoisoned();
        // ENTSEC-1 + ENTSEC-2 + SECAUD-7: one fold-once admission — authenticate the bearer,
        // confirm active membership, and (if the path is project-scoped) enforce the grant
        // (owner/admin bypass), all against a single consistent directory read. Returns the
        // resolved actor so the audit record below reuses it (no re-authenticate). Folding the
        // org twice opened a TOCTOU window between membership and scope (CC6.1).
        let project = guard.scope_project_of_path(&path);
        let actor = match guard.admit_data_request(bearer, project.as_deref()) {
            Ok(actor) => actor,
            Err((code, msg)) => return (code, Json(json!({ "error": msg }))).into_response(),
        };
        // ENTSEC-4 (ADR 0065): audit data-route *actions* (mutating methods) to the org trail —
        // the "what did this consultant do" record (references only, `INV-10`). `/admin/*` audits
        // itself semantically, so it is not double-logged here. Solo (no IdP) writes nothing.
        // SECAUD-4 (CC7.2): when sensitive-read auditing is enabled, GET reads of *project-scoped*
        // data (transcripts/files/diffs/resource content — i.e. `project` resolved Some) are
        // recorded too, so "who read this client's data" is answerable; off by default (reads are
        // high-volume), so the nav/listing GETs (project None) are never logged.
        let is_admin = path.starts_with("/admin/");
        let mutating = method != axum::http::Method::GET;
        let sensitive_read = !mutating && project.is_some() && guard.audits_reads();
        if guard.has_idp() && !is_admin && (mutating || sensitive_read) {
            gaugewright_app::audit::record(&mut guard, &actor, &format!("{method} {path}"), &path);
        }
    }
    next.run(req).await
}

fn write_org(wb: &mut Workbench, scope: &str, r: &OrgRecord) {
    let op = op_str(r.op);
    let _ = wb
        .store_mut()
        .append_record(scope, "org", &serde_json::to_string(r).unwrap());
    wb.notify_library_changed("org", &r.id, op);
}

pub(crate) fn write_membership(wb: &mut Workbench, scope: &str, r: &MembershipRecord) {
    let op = op_str(r.op);
    let _ = wb
        .store_mut()
        .append_record(scope, "membership", &serde_json::to_string(r).unwrap());
    wb.notify_library_changed("membership", &r.id, op);
}

fn write_policy(wb: &mut Workbench, scope: &str, r: &PolicyRecord) {
    let op = op_str(r.op);
    let _ = wb
        .store_mut()
        .append_record(scope, "policy", &serde_json::to_string(r).unwrap());
    wb.notify_library_changed("policy", &r.id, op);
}

fn unprocessable(msg: &str) -> axum::response::Response {
    (StatusCode::UNPROCESSABLE_ENTITY, msg.to_string()).into_response()
}

// ---- org settings (B10) --------------------------------------------------

pub async fn get_org(State(wb): State<SharedWorkbench>, headers: HeaderMap) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, None) {
        return resp;
    }
    match Org::rebuild_in(wb.store_ref(), &req_scope(&headers)) {
        Ok(org) => (StatusCode::OK, Json(json!({ "org": org.org }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    }
}

#[derive(Deserialize)]
pub struct OrgSettingsBody {
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    verified_domains: Vec<String>,
    #[serde(default)]
    default_region: Option<String>,
    #[serde(default)]
    kind: gaugewright_app::org::OrgKind,
}

pub async fn post_org(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(body): Json<OrgSettingsBody>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::EditOrgSettings)) {
        return resp;
    }
    let record = OrgRecord {
        id: ORG_ID.to_string(),
        op: RecordOp::Upsert,
        display_name: body.display_name,
        verified_domains: body.verified_domains,
        default_region: body.default_region,
        kind: body.kind,
    };
    write_org(&mut wb, &req_scope(&headers), &record);
    (StatusCode::OK, Json(json!({ "org": record }))).into_response()
}

// ---- members (B11) -------------------------------------------------------

pub async fn get_members(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, None) {
        return resp;
    }
    match Org::rebuild_in(wb.store_ref(), &req_scope(&headers)) {
        Ok(org) => {
            let members: Vec<&MembershipRecord> = org.members.values().collect();
            (StatusCode::OK, Json(json!({ "members": members }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    }
}

/// `GET /admin/sessions` (`ITGOV-2`) — the live **IT session roster**: which members are
/// currently active (authority, age, idle), recorded by the data-route admission. Console-read
/// gated; never exposes a bearer. Empty until members make authenticated data requests.
pub async fn get_sessions(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, None) {
        return resp;
    }
    (
        StatusCode::OK,
        Json(json!({ "sessions": wb.session_roster() })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct InviteBody {
    /// Stable member id; defaults to the authority string.
    #[serde(default)]
    id: Option<String>,
    authority: String,
    #[serde(default)]
    email: String,
    role: String,
    #[serde(default)]
    status: Option<MembershipStatus>,
    #[serde(default)]
    managed_by_scim: bool,
    #[serde(default)]
    team: Option<String>,
}

pub async fn post_member(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(body): Json<InviteBody>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ManageMembers)) {
        return resp;
    }
    if !is_valid_role(&body.role) {
        return unprocessable(&format!("unknown role {:?}", body.role));
    }
    if body.authority.trim().is_empty() {
        return unprocessable("authority is required");
    }
    let record = MembershipRecord {
        id: body.id.unwrap_or_else(|| body.authority.clone()),
        op: RecordOp::Upsert,
        org_id: ORG_ID.to_string(),
        authority: body.authority,
        email: body.email,
        role: body.role,
        // Default a freshly-added member to Invited (operational, not yet truth);
        // an explicit status (e.g. SCIM creating an Active member) overrides.
        status: body.status.unwrap_or(MembershipStatus::Invited),
        managed_by_scim: body.managed_by_scim,
        team: body.team,
    };
    write_membership(&mut wb, &req_scope(&headers), &record);
    let actor = wb.actor(bearer(&headers));
    gaugewright_app::audit::record(&mut wb, &actor, "member.invite", &record.id);
    (StatusCode::OK, Json(json!({ "member": record }))).into_response()
}

#[derive(Deserialize)]
pub struct RoleBody {
    role: String,
}

pub async fn post_member_role(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<RoleBody>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ManageMembers)) {
        return resp;
    }
    if !is_valid_role(&body.role) {
        return unprocessable(&format!("unknown role {:?}", body.role));
    }
    let org = match Org::rebuild_in(wb.store_ref(), &req_scope(&headers)) {
        Ok(org) => org,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    };
    let Some(existing) = org.members.get(&id) else {
        return (StatusCode::NOT_FOUND, "no such member").into_response();
    };
    if !wb.team_scope_ok(bearer(&headers), existing.team.as_deref()) {
        return (StatusCode::FORBIDDEN, "outside your team scope").into_response();
    }
    // An org must always retain a break-glass owner: refuse demoting the last active
    // one (ID-5 / INV-1).
    if existing.role == "owner"
        && existing.status == MembershipStatus::Active
        && body.role != "owner"
        && org.active_count_with_role("owner") <= 1
    {
        return (StatusCode::CONFLICT, "cannot demote the last owner").into_response();
    }
    let mut record = existing.clone();
    record.op = RecordOp::Upsert;
    record.role = body.role;
    write_membership(&mut wb, &req_scope(&headers), &record);
    let actor = wb.actor(bearer(&headers));
    gaugewright_app::audit::record(&mut wb, &actor, "member.role", &record.id);
    (StatusCode::OK, Json(json!({ "member": record }))).into_response()
}

pub async fn post_member_deactivate(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ManageMembers)) {
        return resp;
    }
    let org = match Org::rebuild_in(wb.store_ref(), &req_scope(&headers)) {
        Ok(org) => org,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    };
    let Some(existing) = org.members.get(&id) else {
        return (StatusCode::NOT_FOUND, "no such member").into_response();
    };
    if !wb.team_scope_ok(bearer(&headers), existing.team.as_deref()) {
        return (StatusCode::FORBIDDEN, "outside your team scope").into_response();
    }
    if existing.role == "owner"
        && existing.status == MembershipStatus::Active
        && org.active_count_with_role("owner") <= 1
    {
        return (StatusCode::CONFLICT, "cannot deactivate the last owner").into_response();
    }
    // Deprovision in place (keep the record so the audit shows the offboarding,
    // INV-18); not a tombstone.
    let mut record = existing.clone();
    record.op = RecordOp::Upsert;
    record.status = MembershipStatus::Deprovisioned;
    write_membership(&mut wb, &req_scope(&headers), &record);
    let actor = wb.actor(bearer(&headers));
    gaugewright_app::audit::record(&mut wb, &actor, "member.deactivate", &record.id);
    (StatusCode::OK, Json(json!({ "member": record }))).into_response()
}

// ---- member → project scope grants (ENTSEC-2 / ADR 0065) -----------------

fn write_grant(wb: &mut Workbench, scope: &str, r: &MemberGrantRecord) {
    let op = op_str(r.op);
    let _ = wb
        .store_mut()
        .append_record(scope, "member_grant", &serde_json::to_string(r).unwrap());
    wb.notify_library_changed("member_grant", &r.id, op);
}

#[derive(Deserialize)]
pub struct GrantBody {
    authority: String,
    project_id: String,
}

/// `GET /admin/grants` (`ENTSEC-2`) — the member→project scope grants. Console-read gated.
pub async fn get_grants(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, None) {
        return resp;
    }
    match Org::rebuild_in(wb.store_ref(), &req_scope(&headers)) {
        Ok(org) => {
            let grants: Vec<&MemberGrantRecord> = org.grants.values().collect();
            (StatusCode::OK, Json(json!({ "grants": grants }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    }
}

/// `POST /admin/grants` (`ENTSEC-2`, [ADR 0065]) — grant a member access to a project's data.
/// `ManageMembers`-gated (a directory-administration action). Owner/admin already see every
/// project; this is how a scoped member (a consultant) is given their engagement's project.
pub async fn post_grant(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(body): Json<GrantBody>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ManageMembers)) {
        return resp;
    }
    if body.authority.trim().is_empty() || body.project_id.trim().is_empty() {
        return unprocessable("authority and project_id are required");
    }
    let record = MemberGrantRecord {
        id: MemberGrantRecord::make_id(&body.authority, &body.project_id),
        op: RecordOp::Upsert,
        authority: body.authority,
        project_id: body.project_id,
    };
    write_grant(&mut wb, &req_scope(&headers), &record);
    let actor = wb.actor(bearer(&headers));
    gaugewright_app::audit::record(&mut wb, &actor, "grant.add", &record.id);
    (StatusCode::OK, Json(json!({ "grant": record }))).into_response()
}

/// `DELETE /admin/grants` (`ENTSEC-2`) — revoke a member's access to a project (tombstone;
/// future-only revocation, `INV-18`). `ManageMembers`-gated. Body carries the `(authority,
/// project_id)` pair (the grant has no standalone id a client would already hold).
pub async fn delete_grant(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(body): Json<GrantBody>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ManageMembers)) {
        return resp;
    }
    let record = MemberGrantRecord {
        id: MemberGrantRecord::make_id(&body.authority, &body.project_id),
        op: RecordOp::Tombstone,
        authority: body.authority,
        project_id: body.project_id,
    };
    write_grant(&mut wb, &req_scope(&headers), &record);
    let actor = wb.actor(bearer(&headers));
    gaugewright_app::audit::record(&mut wb, &actor, "grant.revoke", &record.id);
    (StatusCode::OK, Json(json!({ "grant": record }))).into_response()
}

// ---- SCIM group → role/team mappings (B13 / SCIM-3) ----------------------

#[derive(Deserialize)]
pub struct GroupMappingBody {
    group: String,
    role: String,
    #[serde(default)]
    team: Option<String>,
}

/// Configure an IdP-group → workspace-role (and optional team) mapping (`SCIM-3`).
/// Admin-gated (`ConfigureProvisioning`). The SCIM Users endpoint applies it when a
/// provisioned user carries the group.
pub async fn post_group_mapping(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(body): Json<GroupMappingBody>,
) -> impl IntoResponse {
    if !is_valid_role(&body.role) {
        return unprocessable(&format!("unknown role {:?}", body.role));
    }
    let mut wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ConfigureProvisioning)) {
        return resp;
    }
    let record = GroupMappingRecord {
        id: body.group.clone(),
        op: RecordOp::Upsert,
        group: body.group,
        role: body.role,
        team: body.team,
    };
    let _ = wb.store_mut().append_record(
        &req_scope(&headers),
        "group_mapping",
        &serde_json::to_string(&record).unwrap(),
    );
    wb.notify_library_changed("group_mapping", &record.id, "upsert");
    (StatusCode::OK, Json(json!({ "mapping": record }))).into_response()
}

// ---- audit timeline (B14 / AUD-1, AUD-2) ---------------------------------

#[derive(Deserialize)]
pub struct AuditQuery {
    #[serde(default)]
    actor: Option<String>,
    #[serde(default)]
    action: Option<String>,
    /// `csv` exports CSV; otherwise JSON (the default).
    #[serde(default)]
    format: Option<String>,
}

/// The per-actor audit timeline (`AUD-1`), filterable by `?actor=`/`?action=`, and
/// exportable as `?format=csv` (`AUD-2`). Gated by `ViewAudit`. Returns references
/// only, never payloads (`INV-10`).
pub async fn get_audit_log(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<AuditQuery>,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ViewAudit)) {
        return resp;
    }
    let entries: Vec<gaugewright_app::audit::AuditEntry> =
        gaugewright_app::audit::list(wb.store_ref())
            .into_iter()
            .filter(|e| q.actor.as_ref().is_none_or(|a| &e.actor == a))
            .filter(|e| q.action.as_ref().is_none_or(|a| &e.action == a))
            .collect();
    if q.format.as_deref() == Some("csv") {
        (
            StatusCode::OK,
            [("content-type", "text/csv")],
            gaugewright_app::audit::to_csv(&entries),
        )
            .into_response()
    } else {
        // AUD-3: publish the minimum-retention guarantee alongside the timeline. The log is
        // append-only/forever (`INV-6`); this is the contractual floor surfaced to the buyer.
        let retention_min_days = Org::rebuild_in(wb.store_ref(), &req_scope(&headers))
            .map(|o| o.audit_retention_min_days())
            .unwrap_or(gaugewright_app::org::DEFAULT_AUDIT_RETENTION_MIN_DAYS);
        (
            StatusCode::OK,
            Json(json!({ "entries": entries, "retention_min_days": retention_min_days })),
        )
            .into_response()
    }
}

/// Verify the audit log's hash-chain integrity (`SECAUD-2`, SOC 2 CC7.2/CC7.3): a
/// public, queryable tamper-evidence check. Walks the chain and reports `ok`, the
/// entry count, the current head hash (anchor/sign it externally to also catch tail
/// truncation), and the first broken link if any. Gated by `ViewAudit`. References
/// only, never payloads (`INV-10`).
pub async fn get_audit_verify(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ViewAudit)) {
        return resp;
    }
    // SECAUD-2: pass the workbench's own governance public key as the trusted verifier
    // of the signed checkpoint — never the key embedded in the checkpoint record.
    let pubkey = wb.governance_public_key();
    (
        StatusCode::OK,
        Json(gaugewright_app::audit::verify(
            wb.store_ref(),
            Some(&pubkey),
        )),
    )
        .into_response()
}

// ---- domain-capture auto-join (B10 / ID-6) -------------------------------

#[derive(Deserialize)]
pub struct AutoJoinBody {
    authority: String,
    email: String,
}

/// Domain-capture auto-join (`ID-6`): a user whose email is on a **verified domain**
/// (B10) joins the org as an active `member`. The verified domain *is* the
/// authorization basis (no admin capability required); in production the
/// authenticated email comes from the IdP. A non-verified domain is refused (`403`),
/// fail-closed. Idempotent: re-joining upserts the same active membership.
pub async fn post_auto_join(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(body): Json<AutoJoinBody>,
) -> impl IntoResponse {
    if body.authority.trim().is_empty() {
        return unprocessable("authority is required");
    }
    let mut wb = wb.lock_unpoisoned();
    let org = match Org::rebuild_in(wb.store_ref(), &req_scope(&headers)) {
        Ok(org) => org,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    };
    if !org.domain_is_verified(&body.email) {
        return (
            StatusCode::FORBIDDEN,
            "email domain is not verified for auto-join",
        )
            .into_response();
    }
    let record = MembershipRecord {
        id: body.authority.clone(),
        op: RecordOp::Upsert,
        org_id: ORG_ID.to_string(),
        authority: body.authority,
        email: body.email,
        role: "member".to_string(),
        status: MembershipStatus::Active,
        managed_by_scim: false,
        team: None,
    };
    write_membership(&mut wb, &req_scope(&headers), &record);
    (StatusCode::OK, Json(json!({ "member": record }))).into_response()
}

// ---- org policy (B15 / RBAC-6) -------------------------------------------

pub async fn get_policy(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, None) {
        return resp;
    }
    match Org::rebuild_in(wb.store_ref(), &req_scope(&headers)) {
        Ok(org) => (StatusCode::OK, Json(json!({ "policy": org.policy() }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    }
}

pub async fn post_policy(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(policy): Json<Policy>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ConfigureSecurity)) {
        return resp;
    }
    let record = PolicyRecord {
        id: ORG_ID.to_string(),
        op: RecordOp::Upsert,
        policy,
    };
    write_policy(&mut wb, &req_scope(&headers), &record);
    let actor = wb.actor(bearer(&headers));
    gaugewright_app::audit::record(&mut wb, &actor, "policy.update", "policy");
    (StatusCode::OK, Json(json!({ "policy": record.policy }))).into_response()
}

// ---- placement policy (DEPLOY-2) -----------------------------------------

fn write_placement_policy(
    wb: &mut Workbench,
    scope: &str,
    r: &gaugewright_app::org::PlacementPolicyRecord,
) {
    let op = op_str(r.op);
    let _ = wb.store_mut().append_record(
        scope,
        "placement_policy",
        &serde_json::to_string(r).unwrap(),
    );
    wb.notify_library_changed("placement_policy", &r.id, op);
}

pub async fn get_placement_policy(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, None) {
        return resp;
    }
    match Org::rebuild_in(wb.store_ref(), &req_scope(&headers)) {
        Ok(org) => (
            StatusCode::OK,
            Json(json!({ "placement_policy": org.effective_placement_policy() })),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    }
}

pub async fn post_placement_policy(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(policy): Json<gaugewright_core::boundary_lifecycle::PlacementPolicy>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ConfigureSecurity)) {
        return resp;
    }
    let record = gaugewright_app::org::PlacementPolicyRecord {
        id: ORG_ID.to_string(),
        op: RecordOp::Upsert,
        policy,
    };
    write_placement_policy(&mut wb, &req_scope(&headers), &record);
    let actor = wb.actor(bearer(&headers));
    gaugewright_app::audit::record(
        &mut wb,
        &actor,
        "placement_policy.update",
        "placement_policy",
    );
    (
        StatusCode::OK,
        Json(json!({ "placement_policy": record.policy })),
    )
        .into_response()
}

// ---- billing & seats (B16 / BILL-1, BILL-3) ------------------------------

fn write_billing(wb: &mut Workbench, scope: &str, r: &BillingRecord) {
    let op = op_str(r.op);
    let _ = wb
        .store_mut()
        .append_record(scope, "billing", &serde_json::to_string(r).unwrap());
    wb.notify_library_changed("billing", &r.id, op);
}

pub async fn get_billing(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ManageBilling)) {
        return resp;
    }
    match Org::rebuild_in(wb.store_ref(), &req_scope(&headers)) {
        Ok(org) => (
            StatusCode::OK,
            Json(json!({ "billing": org.billing, "seats_used": org.seats_used() })),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    }
}

pub async fn post_billing(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(mut record): Json<BillingRecord>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ManageBilling)) {
        return resp;
    }
    record.id = ORG_ID.to_string();
    record.op = RecordOp::Upsert;
    write_billing(&mut wb, &req_scope(&headers), &record);
    let actor = wb.actor(bearer(&headers));
    gaugewright_app::audit::record(&mut wb, &actor, "billing.update", "billing");
    (StatusCode::OK, Json(json!({ "billing": record }))).into_response()
}

// ---- security policy (B15 / SEC-1/2/3) -----------------------------------

fn write_security(wb: &mut Workbench, scope: &str, r: &SecurityPolicyRecord) {
    let op = op_str(r.op);
    let _ = wb
        .store_mut()
        .append_record(scope, "security", &serde_json::to_string(r).unwrap());
    wb.notify_library_changed("security", &r.id, op);
}

pub async fn get_security(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, None) {
        return resp;
    }
    match Org::rebuild_in(wb.store_ref(), &req_scope(&headers)) {
        Ok(org) => (StatusCode::OK, Json(json!({ "security": org.security }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    }
}

pub async fn post_security(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(mut record): Json<SecurityPolicyRecord>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ConfigureSecurity)) {
        return resp;
    }
    record.id = ORG_ID.to_string();
    record.op = RecordOp::Upsert;
    write_security(&mut wb, &req_scope(&headers), &record);
    let actor = wb.actor(bearer(&headers));
    gaugewright_app::audit::record(&mut wb, &actor, "security.update", "security");
    (StatusCode::OK, Json(json!({ "security": record }))).into_response()
}

// ---- archetype-approval policy (ADR 0063) --------------------------------
// The org default a project inherits: when set, an added archetype's placement is
// **pending** until the owner accepts; unset = frictionless (active at once).

fn write_archetype_approval(wb: &mut Workbench, scope: &str, r: &ArchetypeApprovalPolicyRecord) {
    let op = op_str(r.op);
    let _ = wb.store_mut().append_record(
        scope,
        "archetype_approval",
        &serde_json::to_string(r).unwrap(),
    );
    wb.notify_library_changed("archetype_approval", &r.id, op);
}

pub async fn get_archetype_approval(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, None) {
        return resp;
    }
    match Org::rebuild_in(wb.store_ref(), &req_scope(&headers)) {
        Ok(org) => (
            StatusCode::OK,
            Json(json!({ "require_approval": org.effective_require_archetype_approval() })),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    }
}

pub async fn post_archetype_approval(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(mut record): Json<ArchetypeApprovalPolicyRecord>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, Some(Capability::ConfigureSecurity)) {
        return resp;
    }
    record.id = ORG_ID.to_string();
    record.op = RecordOp::Upsert;
    write_archetype_approval(&mut wb, &req_scope(&headers), &record);
    let actor = wb.actor(bearer(&headers));
    gaugewright_app::audit::record(
        &mut wb,
        &actor,
        "archetype_approval.update",
        "archetype_approval",
    );
    (
        StatusCode::OK,
        Json(json!({ "require_approval": record.require_approval })),
    )
        .into_response()
}

// ---- SSO connection (B12 / ID-5) -----------------------------------------

fn write_sso(wb: &mut Workbench, scope: &str, r: &SsoConnectionRecord) {
    let op = op_str(r.op);
    let _ = wb
        .store_mut()
        .append_record(scope, "sso", &serde_json::to_string(r).unwrap());
    wb.notify_library_changed("sso", &r.id, op);
}

pub async fn get_sso(State(wb): State<SharedWorkbench>, headers: HeaderMap) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, None) {
        return resp;
    }
    match Org::rebuild_in(wb.store_ref(), &req_scope(&headers)) {
        Ok(org) => (StatusCode::OK, Json(json!({ "sso": org.sso }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    }
}

pub async fn post_sso(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(mut record): Json<SsoConnectionRecord>,
) -> impl IntoResponse {
    {
        let mut wbg = wb.lock_unpoisoned();
        if let Some(resp) = deny(&wbg, &headers, Some(Capability::ConfigureSso)) {
            return resp;
        }
        record.id = ORG_ID.to_string();
        record.op = RecordOp::Upsert;
        write_sso(&mut wbg, &req_scope(&headers), &record);
        let actor = wbg.actor(bearer(&headers));
        gaugewright_app::audit::record(&mut wbg, &actor, "sso.configure", "sso");
    }
    // Activate OIDC verification from the just-saved connection (`ID-3`) without a
    // restart, so the bearer `/auth/callback` returns is honored on `/admin/*`. The
    // initial JWKS load touches the network → off the async runtime. A connection only
    // takes effect once its issuer is reachable (warm): a cold one (unreachable / bad
    // issuer) is "saved, not activated" and the existing verifier is left **untouched**
    // — so a bad runtime edit can't lock admins out. (Startup differs: it attaches a
    // cold verifier to fail closed + self-heal, since no operator is in the loop.)
    let sso = record.clone();
    let built =
        tokio::task::spawn_blocking(move || crate::auth_oidc::build_oidc_idp(Some(&sso))).await;
    let (oidc_active, activation_error) = match built {
        // A non-OIDC / cleared connection ⇒ deactivate OIDC verification.
        Ok(None) => {
            wb.lock_unpoisoned().set_identity_provider(None);
            (false, None)
        }
        // A reachable OIDC connection ⇒ activate it.
        Ok(Some((idp, true))) => {
            wb.lock_unpoisoned().set_identity_provider(Some(idp));
            (true, None)
        }
        // A cold OIDC connection ⇒ keep the existing verifier; tell the operator why.
        Ok(Some((_idp, false))) => (
            wb.lock_unpoisoned().has_idp(),
            Some(
                "OIDC discovery failed (issuer unreachable?); connection saved but not activated — \
                 the existing verifier is unchanged"
                    .to_string(),
            ),
        ),
        Err(_) => (
            wb.lock_unpoisoned().has_idp(),
            Some("activation task panicked".to_string()),
        ),
    };
    (
        StatusCode::OK,
        Json(json!({
            "sso": record,
            "oidc_active": oidc_active,
            "activation_error": activation_error,
        })),
    )
        .into_response()
}

// ---- SP integration details (ONB-1) --------------------------------------

/// The control plane's public base URL — what the admin's IdP must reach. An explicit
/// `GAUGEWRIGHT_PUBLIC_URL` wins (the deployment's canonical externally-visible URL);
/// otherwise it is derived from the request (`X-Forwarded-Proto` + `Host`), so a
/// default loopback run works unconfigured.
fn public_base(headers: &HeaderMap) -> String {
    if let Ok(u) = std::env::var("GAUGEWRIGHT_PUBLIC_URL") {
        if !u.trim().is_empty() {
            return u.trim_end_matches('/').to_string();
        }
    }
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:7878");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("http");
    format!("{scheme}://{host}")
}

fn sp_entity_id(base: &str) -> String {
    std::env::var("GAUGEWRIGHT_SP_ENTITY_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{base}/saml/metadata"))
}

/// `GET /admin/integration` (`ONB-1`) — the **SP-side values** an IT admin pastes into
/// their IdP to connect us: the OIDC redirect URI + login URL, the SAML SP entity id /
/// ACS / metadata URL, and the SCIM base URL. Console-read gated. The admin no longer
/// has to reverse-engineer our endpoints (the biggest onboarding friction, ADR 0058).
pub async fn get_integration(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    if let Some(resp) = deny(&wb, &headers, None) {
        return resp;
    }
    let base = public_base(&headers);
    let sp = sp_entity_id(&base);
    (
        StatusCode::OK,
        Json(json!({
            "base_url": base,
            "oidc": {
                "redirect_uri": format!("{base}/auth/callback"),
                "login_url": format!("{base}/auth/login"),
            },
            "saml": {
                "sp_entity_id": sp,
                "acs_url": format!("{base}/auth/saml/acs"),
                "metadata_url": format!("{base}/saml/metadata"),
                // SP metadata is publishable now (pre-register the SP); the SP-initiated
                // ACS receiver + SAML session is the ADR 0058 follow-on.
                "status": "metadata available; SP-initiated ACS is a follow-on (ADR 0058)",
            },
            "scim": {
                "base_url": format!("{base}/scim/v2"),
            },
        })),
    )
        .into_response()
}

/// `GET /saml/metadata` (`ONB-1`) — the SP metadata descriptor an IdP consumes to
/// pre-register us (entity id + the HTTP-POST ACS location + WantAssertionsSigned).
/// Public (SP metadata carries no secret); served as `application/samlmetadata+xml`.
pub async fn get_saml_metadata(headers: HeaderMap) -> impl IntoResponse {
    let base = public_base(&headers);
    let sp = sp_entity_id(&base);
    let acs = format!("{base}/auth/saml/acs");
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<EntityDescriptor xmlns="urn:oasis:names:tc:SAML:2.0:metadata" entityID="{sp}">
  <SPSSODescriptor AuthnRequestsSigned="false" WantAssertionsSigned="true" protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
    <NameIDFormat>urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress</NameIDFormat>
    <AssertionConsumerService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST" Location="{acs}" index="0" isDefault="true"/>
  </SPSSODescriptor>
</EntityDescriptor>"#
    );
    (
        StatusCode::OK,
        [("content-type", "application/samlmetadata+xml")],
        xml,
    )
        .into_response()
}

// ---- SSO test-connection (ONB-3) -----------------------------------------

/// `POST /admin/sso/test` (`ONB-3`) — a real connectivity test of an OIDC SSO
/// connection (the one in the body, so the wizard can test before saving): runs the
/// live discovery + JWKS load via [`crate::auth_oidc::build_oidc_idp`] and reports
/// whether the issuer is reachable and its signing keys load. The result is
/// **operational evidence, never an admitted "connected" fact** (`INV-2`) — nothing is
/// stored. `ConfigureSso`-gated; the network fetch runs off the async runtime.
pub async fn post_sso_test(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(record): Json<SsoConnectionRecord>,
) -> impl IntoResponse {
    {
        let wb = wb.lock_unpoisoned();
        if let Some(resp) = deny(&wb, &headers, Some(Capability::ConfigureSso)) {
            return resp;
        }
    }
    if record.protocol != gaugewright_app::org::SsoProtocol::Oidc {
        return (
            StatusCode::OK,
            Json(json!({
                "ok": false,
                "detail": "live test is supported for OIDC; SAML connects via SP metadata + the ACS flow",
            })),
        )
            .into_response();
    }
    let built =
        tokio::task::spawn_blocking(move || crate::auth_oidc::build_oidc_idp(Some(&record))).await;
    let (ok, detail) = match built {
        Ok(Some((_idp, true))) => (
            true,
            "issuer reachable and signing keys loaded — the connection can verify tokens",
        ),
        Ok(Some((_idp, false))) => (
            false,
            "issuer or JWKS endpoint unreachable — check the issuer URL is correct and public",
        ),
        Ok(None) => (
            false,
            "incomplete OIDC connection — an issuer and at least one audience (client id) are required",
        ),
        Err(_) => (false, "the test task failed unexpectedly"),
    };
    (StatusCode::OK, Json(json!({ "ok": ok, "detail": detail }))).into_response()
}

// ---- DNS-TXT domain verification (ONB-5) ---------------------------------

/// The deterministic per-(org, domain) challenge token an admin publishes as a TXT
/// record to prove control of the domain. Deterministic so no pending state need be
/// stored; unguessable enough that the real gate is DNS control (the point of the proof).
fn domain_challenge_token(domain: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(
        format!(
            "gaugewright-domain-verification:{ORG_ID}:{}",
            domain.trim().to_lowercase()
        )
        .as_bytes(),
    );
    hex::encode(h.finalize())
}

/// The full TXT value to publish (`gaugewright-domain-verification=<token>`).
fn expected_txt(domain: &str) -> String {
    format!(
        "gaugewright-domain-verification={}",
        domain_challenge_token(domain)
    )
}

/// Whether any of the DNS TXT `values` (DoH may quote them) matches the challenge.
fn txt_matches(values: &[String], domain: &str) -> bool {
    let want = expected_txt(domain);
    values
        .iter()
        .any(|v| v.trim().trim_matches('"').trim() == want)
}

/// Look up the TXT records at `name` via DNS-over-HTTPS (reusing the shared HTTP
/// client — no resolver dependency). Returns the record strings (empty on any error).
fn doh_txt(name: &str) -> Vec<String> {
    use crate::identity_oidc::HttpGet;
    let http = gaugewright_app::net_http::HttpClient::new();
    let url = format!("https://dns.google/resolve?name={name}&type=TXT");
    let Ok(body) = http.get(&url) else {
        return vec![];
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
        return vec![];
    };
    v.get("Answer")
        .and_then(|a| a.as_array())
        .map(|ans| {
            ans.iter()
                .filter_map(|r| r.get("data").and_then(|d| d.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Deserialize)]
pub struct DomainBody {
    domain: String,
}

/// `POST /admin/domains/verify-token` (`ONB-5`) — the TXT record the admin must publish
/// to prove control of a domain (the hosted-mode basis for auto-join/JIT). `EditOrgSettings`-gated.
pub async fn post_domain_verify_token(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(body): Json<DomainBody>,
) -> impl IntoResponse {
    {
        let wb = wb.lock_unpoisoned();
        if let Some(resp) = deny(&wb, &headers, Some(Capability::EditOrgSettings)) {
            return resp;
        }
    }
    let domain = body.domain.trim().to_lowercase();
    if domain.is_empty() {
        return unprocessable("domain is required");
    }
    (
        StatusCode::OK,
        Json(json!({
            "domain": domain,
            "record_name": format!("_gaugewright-challenge.{domain}"),
            "record_type": "TXT",
            "value": expected_txt(&domain),
        })),
    )
        .into_response()
}

/// `POST /admin/domains/verify` (`ONB-5`) — look up the TXT challenge over DoH; on a
/// match, **admit** the domain into the org's verified set (the verifying event, B10),
/// which then powers domain-capture auto-join (`ID-6`) and JIT (`ONB-2`). On no match,
/// returns the expected record so the admin can fix it. `EditOrgSettings`-gated; the
/// DNS lookup runs off the async runtime.
pub async fn post_domain_verify(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(body): Json<DomainBody>,
) -> impl IntoResponse {
    {
        let wb = wb.lock_unpoisoned();
        if let Some(resp) = deny(&wb, &headers, Some(Capability::EditOrgSettings)) {
            return resp;
        }
    }
    let domain = body.domain.trim().to_lowercase();
    if domain.is_empty() {
        return unprocessable("domain is required");
    }
    let name = format!("_gaugewright-challenge.{domain}");
    let values = tokio::task::spawn_blocking(move || doh_txt(&name))
        .await
        .unwrap_or_default();
    if !txt_matches(&values, &domain) {
        return (
            StatusCode::OK,
            Json(json!({
                "verified": false,
                "expected": { "record_name": format!("_gaugewright-challenge.{domain}"), "value": expected_txt(&domain) },
                "seen": values,
            })),
        )
            .into_response();
    }
    // Admit the domain into the org's verified set (preserving the rest of the record).
    let mut wbg = wb.lock_unpoisoned();
    let mut record = match Org::rebuild(wbg.store_ref()) {
        Ok(o) => o.org.unwrap_or_default(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response(),
    };
    record.id = ORG_ID.to_string();
    record.op = RecordOp::Upsert;
    if !record
        .verified_domains
        .iter()
        .any(|d| d.eq_ignore_ascii_case(&domain))
    {
        record.verified_domains.push(domain.clone());
    }
    write_org(&mut wbg, &req_scope(&headers), &record);
    let actor = wbg.actor(bearer(&headers));
    gaugewright_app::audit::record(&mut wbg, &actor, "domain.verified", &domain);
    (
        StatusCode::OK,
        Json(json!({ "verified": true, "domain": domain })),
    )
        .into_response()
}

#[cfg(test)]
mod onb5_tests {
    use super::{expected_txt, txt_matches};

    #[test]
    fn txt_matches_the_expected_challenge_and_is_domain_specific() {
        let want = expected_txt("acme.com");
        // DoH returns TXT values quoted; bare and quoted both match, case-insensitive domain.
        assert!(txt_matches(&[format!("\"{want}\"")], "acme.com"));
        assert!(txt_matches(std::slice::from_ref(&want), "Acme.com"));
        // wrong / missing → no match (fail-closed).
        assert!(!txt_matches(
            &["gaugewright-domain-verification=nope".into()],
            "acme.com"
        ));
        assert!(!txt_matches(&[], "acme.com"));
        // the token is domain-bound — acme's value does not verify evil.com.
        assert_ne!(expected_txt("acme.com"), expected_txt("evil.com"));
    }
}

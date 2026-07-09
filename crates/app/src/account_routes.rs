//! The account surface (ACCT-1): the operator's own device registry, settings, and
//! linked-credential management, over the reserved `account` scope. These act on the
//! operator's *own* account (not org admin), so they are ungated on the loopback
//! desktop — the operator is the account owner. CRUD writes durable records (latest-
//! wins / tombstone) and pushes a workspace-change reference so clients refresh live.
//!
//! The linked credential's **plaintext** is never returned over HTTP — only the
//! provider list and link/unlink. Decryption is the internal
//! [`crate::account::resolve_token`] API the local runtime uses.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;

use crate::account::{seal_token, DeviceRecord, DeviceStatus, RecordOp, SettingRecord};
use crate::codex_oauth;
use crate::{err_response, net_http, LockUnpoisoned, SharedWorkbench};

/// Split account route surface. Local device/settings/credential ownership stays
/// open; hosted token brokerage and account-ledger operation are split later.
pub fn routes() -> Router<SharedWorkbench> {
    Router::new()
        // Account (ACCT-1): the operator's own device registry, settings, and
        // sealed linked-credentials. Ungated (the operator owns their account).
        .route("/account/devices", get(get_devices).post(post_device))
        .route("/account/devices/:id/revoke", post(post_device_revoke))
        // Device-enrollment handshake drive (ACCT-1, ADR 0055): the holder hosts +
        // authorizes; the new device joins. Both dial the rendezvous broker; status
        // GETs surface the phase + SAS for the out-of-band human compare. The account
        // key crosses only as ECIES ciphertext over the broker, never over HTTP.
        .route("/account/devices/enroll/host", post(post_enroll_host))
        .route(
            "/account/devices/enroll/host/:session",
            get(get_enroll_host),
        )
        .route(
            "/account/devices/enroll/authorize",
            post(post_enroll_authorize),
        )
        .route("/account/devices/enroll/join", post(post_enroll_join))
        .route(
            "/account/devices/enroll/join/:session",
            get(get_enroll_join),
        )
        .route("/account/settings", get(get_settings))
        .route("/account/settings/:key", put(put_setting))
        .route(
            "/account/credentials",
            get(get_credentials).post(post_credential),
        )
        .route("/account/credentials/:provider", delete(delete_credential))
        // First-run gate signal (ADR 0075 Phase 0): whether the default runtime
        // actually needs an LLM credential. False under the scripted fake agent
        // (dev/e2e), so the first-run overlay never blocks a no-credential test.
        .route("/account/onboarding-status", get(get_onboarding_status))
        // OpenAI codex OAuth link (LLM-1, ADR 0062): status + start-the-flow. The
        // credential lands in Pi's own auth store (read by the engine's default
        // provider), distinct from the BYOK sealed credentials above.
        .route(
            "/account/oauth/openai-codex",
            get(codex_oauth::get_codex_status),
        )
        .route(
            "/account/oauth/openai-codex/start",
            post(codex_oauth::post_codex_login_start),
        )
        // Library sync (ADR 0054 / the library_sync facility, ADR 0077): publish this account's
        // sealed state to the blind directory / pull + merge from it. No-op unless the facility is
        // active and GAUGEWRIGHT_DIRECTORY_URL is set. Signs with the workbench's root key (desktop).
        .route("/account/library-sync", post(post_library_sync_publish))
        .route("/account/library-sync/pull", post(post_library_sync_pull))
}

/// Publish the current account state to the blind directory (the `library_sync` facility). Builds
/// the signed record under the lock, then PUTs it off the lock. `409` if sync is off / unconfigured.
pub async fn post_library_sync_publish(State(wb): State<SharedWorkbench>) -> impl IntoResponse {
    let Some(base) = crate::directory_sync::directory_url_from_env() else {
        return (StatusCode::CONFLICT, "GAUGEWRIGHT_DIRECTORY_URL not set").into_response();
    };
    let put = wb.lock_unpoisoned().library_sync_signed_put();
    let Some(put) = put else {
        return (StatusCode::CONFLICT, "library sync is not active").into_response();
    };
    let published = tokio::task::spawn_blocking(move || {
        crate::directory_sync::publish(&crate::net_http::HttpClient::new(), &base, &put)
    })
    .await;
    match published {
        Ok(Ok(())) => (StatusCode::OK, Json(json!({ "published": true }))).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_GATEWAY, e).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "publish task panicked").into_response(),
    }
}

/// Pull the account state from the blind directory and merge it locally (the `library_sync`
/// facility). Fetches off the lock, then merges under it. `409` if sync is off / unconfigured.
pub async fn post_library_sync_pull(State(wb): State<SharedWorkbench>) -> impl IntoResponse {
    let Some(base) = crate::directory_sync::directory_url_from_env() else {
        return (StatusCode::CONFLICT, "GAUGEWRIGHT_DIRECTORY_URL not set").into_response();
    };
    let (active, root) = {
        let wb = wb.lock_unpoisoned();
        (wb.library_sync_active(), wb.library_sync_root())
    };
    if !active {
        return (StatusCode::CONFLICT, "library sync is not active").into_response();
    }
    let fetched = tokio::task::spawn_blocking(move || {
        crate::directory_sync::fetch(&crate::net_http::HttpClient::new(), &base, &root)
    })
    .await;
    let entry = match fetched {
        Ok(Ok(Some(e))) => e,
        Ok(Ok(None)) => {
            return (StatusCode::OK, Json(json!({ "found": false, "merged": 0 }))).into_response()
        }
        Ok(Err(e)) => return (StatusCode::BAD_GATEWAY, e).into_response(),
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "fetch task panicked").into_response()
        }
    };
    match wb.lock_unpoisoned().library_sync_apply(&entry) {
        Ok(merged) => (
            StatusCode::OK,
            Json(json!({ "found": true, "merged": merged })),
        )
            .into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
    }
}

/// Whether a first-run user must connect an LLM credential before the runtime
/// can run a turn (ADR 0075 Phase 0). Mirrors the runtime selection in
/// `harness_select::factory_for_turn`: the scripted fake agent (selected by
/// `GAUGEWRIGHT_FAKE_AGENT`, used in dev/e2e) needs no credential, so the gate is
/// off there; the real Pi runtime needs one, so it's on.
pub async fn get_onboarding_status() -> impl IntoResponse {
    let credential_required = std::env::var("GAUGEWRIGHT_FAKE_AGENT").is_err();
    (
        StatusCode::OK,
        Json(json!({ "credential_required": credential_required })),
    )
        .into_response()
}

// ---- devices (the trusted-devices registry) ------------------------------

pub async fn get_devices(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    let scope = wb.account_scope_for(net_http::bearer(&headers));
    match wb.account_devices_in(&scope) {
        Ok(devices) => (StatusCode::OK, Json(json!({ "devices": devices }))).into_response(),
        Err(e) => err_response(e),
    }
}

#[derive(Deserialize)]
pub struct EnrollBody {
    id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    subkey_pubkey: String,
}

/// Register (enroll) a device into the registry. The full enrollment *handshake*
/// (an existing device authorizing a new one + transferring the account key) is a
/// follow-on; this records the device fact.
pub async fn post_device(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(body): Json<EnrollBody>,
) -> impl IntoResponse {
    if body.id.trim().is_empty() {
        return (StatusCode::UNPROCESSABLE_ENTITY, "device id is required").into_response();
    }
    let record = DeviceRecord {
        id: body.id,
        op: RecordOp::Upsert,
        label: body.label,
        subkey_pubkey: body.subkey_pubkey,
        status: DeviceStatus::Active,
    };
    let mut wb = wb.lock_unpoisoned();
    let scope = wb.account_scope_for(net_http::bearer(&headers));
    if let Err(e) = wb.upsert_account_device_in(&scope, &record) {
        return err_response(e);
    }
    (StatusCode::OK, Json(json!({ "device": record }))).into_response()
}

pub async fn post_device_revoke(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    let scope = wb.account_scope_for(net_http::bearer(&headers));
    let record = match wb.revoke_account_device_in(&scope, &id) {
        Ok(Some(record)) => record,
        Ok(None) => return (StatusCode::NOT_FOUND, "no such device").into_response(),
        Err(e) => return err_response(e),
    };
    (StatusCode::OK, Json(json!({ "device": record }))).into_response()
}

// ---- device enrollment handshake (ACCT-1, ADR 0055) ----------------------
//
// The full handshake `post_device` deferred: an existing (holder) device authorizes
// a new one over the rendezvous broker and hands over the account key, sealed to the
// new device's subkey. Two roles, each a background leg the status GETs poll:
//
//   holder:      POST /enroll/host        -> mint + show the ticket
//                GET  /enroll/host/:sess   -> phase + SAS (compare with the new device)
//                POST /enroll/authorize    -> the human confirmed the SAS matches
//   new device:  POST /enroll/join {ticket} -> consume the ticket, start the leg
//                GET  /enroll/join/:sess    -> phase + SAS (compare with the holder)
//
// Fail-closed: no authorize without the explicit confirm; a substituted subkey shows
// up as a mismatched SAS the human catches; a lapsed pairing times the leg out.

/// A leg's phase + SAS for a status poll (never the account key — `INV-10`).
fn enroll_status_json(
    snapshot: Option<(
        crate::device_enroll_drive::EnrollPhase,
        Option<String>,
        Option<String>,
    )>,
) -> axum::response::Response {
    match snapshot {
        Some((phase, sas, error)) => (
            StatusCode::OK,
            Json(json!({ "phase": phase, "sas": sas, "error": error })),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "no such enrollment session").into_response(),
    }
}

/// Holder: start the enrollment host leg and return the out-of-band ticket to show
/// (QR + code). The leg waits at the broker for the new device, then for the human's
/// SAS confirm.
pub async fn post_enroll_host(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let scope = wb
        .lock_unpoisoned()
        .account_scope_for(net_http::bearer(&headers));
    match crate::device_enroll_drive::start_host(&wb, scope) {
        Some(ticket) => (StatusCode::OK, Json(json!({ "ticket": ticket }))).into_response(),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not start enrollment",
        )
            .into_response(),
    }
}

/// Holder: the current phase + SAS of a host leg (poll after showing the ticket).
pub async fn get_enroll_host(
    State(wb): State<SharedWorkbench>,
    Path(session): Path<String>,
) -> impl IntoResponse {
    let drive = wb.lock_unpoisoned().enroll_drive();
    enroll_status_json(drive.host_snapshot(&session))
}

#[derive(Deserialize)]
pub struct EnrollAuthorizeBody {
    session: String,
}

/// Holder: the human confirmed the SAS matches the new device's — release the host
/// leg to authorize. `409` if the leg is not awaiting confirmation (fail-closed).
pub async fn post_enroll_authorize(
    State(wb): State<SharedWorkbench>,
    Json(body): Json<EnrollAuthorizeBody>,
) -> impl IntoResponse {
    let drive = wb.lock_unpoisoned().enroll_drive();
    match drive.confirm_host(&body.session) {
        Ok(()) => (StatusCode::OK, Json(json!({ "authorized": true }))).into_response(),
        Err(e) => (StatusCode::CONFLICT, e).into_response(),
    }
}

#[derive(Deserialize)]
pub struct EnrollJoinBody {
    ticket: crate::device_enroll_drive::EnrollmentTicket,
}

/// New device: consume a ticket and start the join leg. Returns the session id to
/// poll for the SAS + outcome.
pub async fn post_enroll_join(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(body): Json<EnrollJoinBody>,
) -> impl IntoResponse {
    let scope = wb
        .lock_unpoisoned()
        .account_scope_for(net_http::bearer(&headers));
    match crate::device_enroll_drive::start_join(&wb, scope, body.ticket) {
        Ok(session) => (StatusCode::OK, Json(json!({ "session": session }))).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
    }
}

/// New device: the current phase + SAS of a join leg (poll for the outcome).
pub async fn get_enroll_join(
    State(wb): State<SharedWorkbench>,
    Path(session): Path<String>,
) -> impl IntoResponse {
    let drive = wb.lock_unpoisoned().enroll_drive();
    enroll_status_json(drive.join_snapshot(&session))
}

// ---- settings ------------------------------------------------------------

pub async fn get_settings(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    let scope = wb.account_scope_for(net_http::bearer(&headers));
    match wb.account_settings_in(&scope) {
        Ok(settings) => (StatusCode::OK, Json(json!({ "settings": settings }))).into_response(),
        Err(e) => err_response(e),
    }
}

#[derive(Deserialize)]
pub struct SettingBody {
    value: String,
}

pub async fn put_setting(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(body): Json<SettingBody>,
) -> impl IntoResponse {
    let record = SettingRecord {
        id: key,
        op: RecordOp::Upsert,
        value: body.value,
    };
    let mut wb = wb.lock_unpoisoned();
    let scope = wb.account_scope_for(net_http::bearer(&headers));
    if let Err(e) = wb.upsert_account_setting_in(&scope, &record) {
        return err_response(e);
    }
    (StatusCode::OK, Json(json!({ "setting": record }))).into_response()
}

// ---- linked credentials (sealed; plaintext never returned) ---------------

pub async fn get_credentials(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    let scope = wb.account_scope_for(net_http::bearer(&headers));
    match wb.account_credential_providers_in(&scope) {
        Ok(provider_ids) => {
            // Providers + linked-flag only — never the token (sealed or otherwise).
            let providers: Vec<serde_json::Value> = provider_ids
                .iter()
                .map(|p| json!({ "provider": p, "linked": true }))
                .collect();
            (StatusCode::OK, Json(json!({ "credentials": providers }))).into_response()
        }
        Err(e) => err_response(e),
    }
}

#[derive(Deserialize)]
pub struct LinkBody {
    provider: String,
    token: String,
}

/// Link a provider account: seal the OAuth token (`SEC-4`) and store the ciphertext.
pub async fn post_credential(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Json(body): Json<LinkBody>,
) -> impl IntoResponse {
    if body.provider.trim().is_empty() || body.token.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "provider and token are required",
        )
            .into_response();
    }
    let mut wb = wb.lock_unpoisoned();
    let scope = wb.account_scope_for(net_http::bearer(&headers));
    // The seal is at-rest encryption keyed by the control-plane authority; the per-person
    // access boundary is the scope (INV-1), so a person only ever reads their own credentials.
    let authority = wb.authority().as_str().to_string();
    let Some(sealed) = seal_token(&authority, &body.token) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "seal failed").into_response();
    };
    let provider = body.provider;
    if let Err(e) = wb.upsert_account_credential_in(&scope, provider.clone(), sealed) {
        return err_response(e);
    }
    // Advance the onboarding checklist (ADR 0075 Phase 2). Best-effort — the
    // credential is already saved; the provider name is not a secret.
    wb.advance_onboarding("credential", &json!({ "provider": provider }).to_string());
    (
        StatusCode::OK,
        Json(json!({ "provider": provider, "linked": true })),
    )
        .into_response()
}

pub async fn delete_credential(
    State(wb): State<SharedWorkbench>,
    headers: HeaderMap,
    Path(provider): Path<String>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    let scope = wb.account_scope_for(net_http::bearer(&headers));
    if let Err(e) = wb.tombstone_account_credential_in(&scope, provider.clone()) {
        return err_response(e);
    }
    (
        StatusCode::OK,
        Json(json!({ "provider": provider, "linked": false })),
    )
        .into_response()
}

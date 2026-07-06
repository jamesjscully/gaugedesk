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
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;

use crate::account::{seal_token, DeviceRecord, DeviceStatus, RecordOp, SettingRecord};
use crate::codex_oauth;
use crate::{err_response, LockUnpoisoned, SharedWorkbench};

/// Split account route surface. Local device/settings/credential ownership stays
/// open; hosted token brokerage and account-ledger operation are split later.
pub fn routes() -> Router<SharedWorkbench> {
    Router::new()
        // Account (ACCT-1): the operator's own device registry, settings, and
        // sealed linked-credentials. Ungated (the operator owns their account).
        .route("/account/devices", get(get_devices).post(post_device))
        .route("/account/devices/:id/revoke", post(post_device_revoke))
        .route("/account/settings", get(get_settings))
        .route("/account/settings/:key", put(put_setting))
        .route(
            "/account/credentials",
            get(get_credentials).post(post_credential),
        )
        .route("/account/credentials/:provider", delete(delete_credential))
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
}

// ---- devices (the trusted-devices registry) ------------------------------

pub async fn get_devices(State(wb): State<SharedWorkbench>) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    match wb.account_devices() {
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
    if let Err(e) = wb.upsert_account_device(&record) {
        return err_response(e);
    }
    (StatusCode::OK, Json(json!({ "device": record }))).into_response()
}

pub async fn post_device_revoke(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    let record = match wb.revoke_account_device(&id) {
        Ok(Some(record)) => record,
        Ok(None) => return (StatusCode::NOT_FOUND, "no such device").into_response(),
        Err(e) => return err_response(e),
    };
    (StatusCode::OK, Json(json!({ "device": record }))).into_response()
}

// ---- settings ------------------------------------------------------------

pub async fn get_settings(State(wb): State<SharedWorkbench>) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    match wb.account_settings() {
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
    Path(key): Path<String>,
    Json(body): Json<SettingBody>,
) -> impl IntoResponse {
    let record = SettingRecord {
        id: key,
        op: RecordOp::Upsert,
        value: body.value,
    };
    let mut wb = wb.lock_unpoisoned();
    if let Err(e) = wb.upsert_account_setting(&record) {
        return err_response(e);
    }
    (StatusCode::OK, Json(json!({ "setting": record }))).into_response()
}

// ---- linked credentials (sealed; plaintext never returned) ---------------

pub async fn get_credentials(State(wb): State<SharedWorkbench>) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    match wb.account_credential_providers() {
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
    let authority = wb.authority().as_str().to_string();
    let Some(sealed) = seal_token(&authority, &body.token) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "seal failed").into_response();
    };
    let provider = body.provider;
    if let Err(e) = wb.upsert_account_credential(provider.clone(), sealed) {
        return err_response(e);
    }
    (
        StatusCode::OK,
        Json(json!({ "provider": provider, "linked": true })),
    )
        .into_response()
}

pub async fn delete_credential(
    State(wb): State<SharedWorkbench>,
    Path(provider): Path<String>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Err(e) = wb.tombstone_account_credential(provider.clone()) {
        return err_response(e);
    }
    (
        StatusCode::OK,
        Json(json!({ "provider": provider, "linked": false })),
    )
        .into_response()
}

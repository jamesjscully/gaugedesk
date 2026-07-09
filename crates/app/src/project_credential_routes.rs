//! Per-project LLM-access credential overrides (`LLM-2`, [ADR 0062]). A project may pin
//! its own sealed BYOK credential in its coordination scope, overriding the account
//! default for chats in that project (nearest-scope-wins at run time — see
//! [`crate::account::resolved_credential_envs`]). Same discipline as
//! [`crate::account_routes`]: the **plaintext** token is never returned over HTTP — only
//! the sealed ciphertext lives at rest (`SEC-4`/`INV-10`), and the surface lists provider
//! names + a linked flag, never the secret.
//!
//! "Or a managed plan" (ADR 0062) is the LLM-3 managed-execution axis and rides that
//! item; this surface covers the buildable BYOK-credential override.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::account::seal_token;
use crate::{err_response, LockUnpoisoned, SharedWorkbench};

/// `GET /projects/:id/credentials` — the providers this project pins (names + linked
/// flag only; never the token).
pub async fn get_project_credentials(
    State(wb): State<SharedWorkbench>,
    Path(project): Path<String>,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    let providers: Vec<serde_json::Value> = wb
        .project_credential_providers(&project)
        .iter()
        .map(|p| json!({ "provider": p, "linked": true }))
        .collect();
    (StatusCode::OK, Json(json!({ "credentials": providers }))).into_response()
}

#[derive(Deserialize)]
pub struct LinkBody {
    provider: String,
    token: String,
}

/// `POST /projects/:id/credentials` — pin a provider for this project: seal the token
/// (`SEC-4`) and store the ciphertext in the project's coordination scope.
pub async fn post_project_credential(
    State(wb): State<SharedWorkbench>,
    Path(project): Path<String>,
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
    let Some(sealed) = seal_token(wb.account_key(), &body.token) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "seal failed").into_response();
    };
    let provider = body.provider;
    if let Err(e) = wb.upsert_project_credential(&project, provider.clone(), sealed) {
        return err_response(e);
    }
    (
        StatusCode::OK,
        Json(json!({ "provider": provider, "linked": true })),
    )
        .into_response()
}

/// `DELETE /projects/:id/credentials/:provider` — drop this project's pin (tombstone),
/// so the project falls back to the account default again.
pub async fn delete_project_credential(
    State(wb): State<SharedWorkbench>,
    Path((project, provider)): Path<(String, String)>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if let Err(e) = wb.tombstone_project_credential(&project, provider.clone()) {
        return err_response(e);
    }
    (
        StatusCode::OK,
        Json(json!({ "provider": provider, "linked": false })),
    )
        .into_response()
}

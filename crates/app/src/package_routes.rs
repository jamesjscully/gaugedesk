use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use gaugewright_store::AdmitError;
use serde::Deserialize;

use crate::{
    err_response,
    package_store::{PackageRecord, VersionRecord},
    LockUnpoisoned, SharedWorkbench,
};

#[derive(Deserialize)]
pub(crate) struct PublishBody {
    id: String,
    version: String,
    #[serde(default)]
    agent_ref: String,
}

#[derive(Deserialize)]
pub(crate) struct ContextQuery {
    #[serde(default)]
    context: String,
}

/// The package catalog projection (`data.md`): every package record joined with its
/// live distribution status (folded from PD-1). Blocking events (withdrawal) change
/// the folded status immediately; readiness is always derived live, never cached.
pub(crate) async fn get_packages(State(wb): State<SharedWorkbench>) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    let recs = wb.package_catalog().unwrap_or_default();
    let views: Vec<serde_json::Value> = recs
        .iter()
        .map(|(r, status)| {
            serde_json::json!({
                "id": r.id,
                "version": r.version,
                "source_authority": r.source_authority,
                "protection_posture": r.protection_posture,
                "status": status,
                "tombstoned": r.tombstoned,
            })
        })
        .collect();
    (StatusCode::OK, Json(views)).into_response()
}

/// Source: withdraw a package — future-only; immediately drops availability + readiness.
pub(crate) async fn post_package_withdraw(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    match wb.withdraw_package(&id) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "withdrawn": id }))).into_response(),
        Err(AdmitError::Rejected(r)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "rejected": r.reason })),
        )
            .into_response(),
        Err(e) => err_response(e),
    }
}

/// Source: freeze an agent version and publish a package referencing it.
pub(crate) async fn post_package_publish(
    State(wb): State<SharedWorkbench>,
    Json(body): Json<PublishBody>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    let vrec = VersionRecord {
        id: body.version.clone(),
        agent_ref: body.agent_ref.clone(),
        method_handles: vec![],
        config: "{}".into(),
        protection_posture: "local".into(),
        provenance: vec![],
        content_hashes: vec![],
        tombstoned: false,
    };
    let prec = PackageRecord {
        id: body.id.clone(),
        version: body.version.clone(),
        source_authority: "source".into(),
        agent_ref: body.agent_ref,
        method_handles: vec![],
        protection_posture: "local".into(),
        source_basis: true,
        tombstoned: false,
    };
    match wb.publish_package(&vrec, &prec) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "published": body.id })),
        )
            .into_response(),
        Err(AdmitError::Rejected(r)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "rejected": r.reason })),
        )
            .into_response(),
        Err(e) => err_response(e),
    }
}

/// Target: install a published package.
pub(crate) async fn post_package_install(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    match wb.install_package(&id) {
        Ok(phase) => (
            StatusCode::OK,
            Json(serde_json::json!({ "phase": format!("{phase:?}") })),
        )
            .into_response(),
        Err(AdmitError::Rejected(r)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "rejected": r.reason })),
        )
            .into_response(),
        Err(e) => err_response(e),
    }
}

pub(crate) async fn post_package_entitle(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
    Query(q): Query<ContextQuery>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    match wb.entitle_package(&id, &q.context) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "entitled": true })),
        )
            .into_response(),
        Err(AdmitError::Rejected(r)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "rejected": r.reason })),
        )
            .into_response(),
        Err(e) => err_response(e),
    }
}

/// Governed run readiness for a context: installed AND entitled (install alone is
/// never runnable). The package catalog's run-readiness hint reads this (PK-3).
pub(crate) async fn get_package_readiness(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
    Query(q): Query<ContextQuery>,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    let (installed, entitled, run_ready) = wb.package_readiness(&id, &q.context);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "installed": installed, "entitled": entitled, "run_ready": run_ready,
        })),
    )
        .into_response()
}

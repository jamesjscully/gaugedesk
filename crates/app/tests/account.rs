//! The account surface end to end (ACCT-1): the operator's device registry, settings,
//! and sealed linked-credentials over the mounted `control_plane`.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use gaugewright_app::open_control_plane;
use gaugewright_app::Workbench;
use gaugewright_store::Store;
use gaugewright_workspace::Instance;

fn workbench() -> (tempfile::TempDir, Router) {
    let dir = tempfile::tempdir().unwrap();
    let instance = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
    let store = Store::open_in_memory().unwrap();
    let wb = Workbench::with_instance("inst-test", instance, store);
    (dir, open_control_plane(Arc::new(Mutex::new(wb))))
}

async fn send(app: &Router, method: &str, uri: &str, body: Option<&str>) -> (StatusCode, Value) {
    let req = match body {
        Some(b) => Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn device_registry_enroll_list_revoke() {
    let (_dir, app) = workbench();

    let (s, body) = send(
        &app,
        "POST",
        "/account/devices",
        Some(r#"{"id":"phone","label":"My phone","subkey_pubkey":"ab12"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["device"]["status"], "active");

    let (s, body) = send(&app, "GET", "/account/devices", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["devices"].as_array().unwrap().len(), 1);

    // Revoke keeps the record but flips status (INV-6).
    let (s, body) = send(&app, "POST", "/account/devices/phone/revoke", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["device"]["status"], "revoked");

    let (_s, body) = send(&app, "GET", "/account/devices", None).await;
    assert_eq!(body["devices"].as_array().unwrap().len(), 1);
    assert_eq!(body["devices"][0]["status"], "revoked");

    // Revoking an unknown device is a 404.
    let (s, _) = send(&app, "POST", "/account/devices/ghost/revoke", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn settings_round_trip() {
    let (_dir, app) = workbench();
    let (s, _) = send(
        &app,
        "PUT",
        "/account/settings/theme",
        Some(r#"{"value":"dark"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = send(&app, "GET", "/account/settings", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["settings"]["theme"], "dark");
}

#[tokio::test]
async fn linked_credential_is_sealed_and_token_never_leaves() {
    let (_dir, app) = workbench();

    // Link an OpenAI account — the token goes in sealed.
    let (s, body) = send(
        &app,
        "POST",
        "/account/credentials",
        Some(r#"{"provider":"openai","token":"sk-super-secret"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["provider"], "openai");
    assert_eq!(body["linked"], true);

    // The list exposes the provider — never the token (sealed or otherwise).
    let (s, body) = send(&app, "GET", "/account/credentials", None).await;
    assert_eq!(s, StatusCode::OK);
    let raw = body.to_string();
    assert!(
        !raw.contains("sk-super-secret"),
        "token must never be returned over HTTP"
    );
    assert!(!raw.contains("token"), "no token field at all");
    assert_eq!(body["credentials"][0]["provider"], "openai");

    // Unlink.
    let (s, body) = send(&app, "DELETE", "/account/credentials/openai", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["linked"], false);
    let (_s, body) = send(&app, "GET", "/account/credentials", None).await;
    assert_eq!(body["credentials"].as_array().unwrap().len(), 0);
}

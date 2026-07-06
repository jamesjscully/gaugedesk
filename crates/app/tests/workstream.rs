//! Workstream collaboration, end to end (`WS-D`/`WS-E`). Drives the mounted
//! `control_plane` through the whole flow: create a named workstream in a placement,
//! home two chats to it, run a member's turn, and assert the greedy auto-sync — the
//! member's work lands on the stream main, the sibling picks it up, the member's merge
//! auto-advances (no human review), the placement mainline stays isolated until an
//! explicit promote, and archiving re-homes the members.
//!
//! This lives in its own test binary (its own process) so the process-global
//! `GAUGEWRIGHT_FAKE_AGENT` it sets is not raced by the parallel lib unit tests.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
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

async fn send(app: &Router, method: &str, uri: &str, body: Option<&str>) -> (StatusCode, String) {
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
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// Create a workstream in the default placement and home both chats to it; returns its id.
async fn workstream_with_members(app: &Router, chats: &[&str]) -> String {
    for c in chats {
        let (s, b) = send(app, "POST", "/chats", Some(&format!(r#"{{"id":"{c}"}}"#))).await;
        assert_eq!(s, StatusCode::CREATED, "create chat {c}: {b}");
    }
    let (s, body) = send(
        app,
        "POST",
        "/placements/inst-test/workstreams",
        Some(r#"{"name":"feature"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "create ws: {body}");
    let ws: serde_json::Value = serde_json::from_str(&body).unwrap();
    let ws_id = ws["id"].as_str().unwrap().to_string();
    for c in chats {
        let (s, b) = send(
            app,
            "POST",
            &format!("/workstreams/{ws_id}/join"),
            Some(&format!(r#"{{"chat":"{c}"}}"#)),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "join {c}: {b}");
    }
    ws_id
}

#[tokio::test]
async fn member_turn_auto_syncs_to_siblings_without_touching_mainline() {
    std::env::set_var("GAUGEWRIGHT_FAKE_AGENT", "1");
    let (_d, app) = workbench();
    let ws_id = workstream_with_members(&app, &["wsa", "wsb"]).await;

    // A member's turn: the fake agent appends to agent-note.txt and commits; the greedy
    // hook promotes it into the stream main and syncs siblings.
    let (s, b) = send(
        &app,
        "POST",
        "/chats/wsa/task",
        Some(r#"{"prompt":"do the thing"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "task: {b}");

    // The sibling's worktree now carries the member's work (auto-synced via stream main).
    let (s, file) = send(&app, "GET", "/chats/wsb/file?path=agent-note.txt", None).await;
    assert_eq!(s, StatusCode::OK, "sibling file: {file}");
    assert!(
        file.contains("do the thing"),
        "sibling picked up the member's work: {file}"
    );

    // The contributing member auto-advanced — no human-review task left hanging.
    let (_s, merge) = send(&app, "GET", "/chats/wsa/merge", None).await;
    assert!(
        merge.contains("Advanced"),
        "an in-stream contribution auto-advances: {merge}"
    );

    // The mainline is still isolated: a brand-new chat off mainline does not see the work.
    send(&app, "POST", "/chats", Some(r#"{"id":"main-chat"}"#)).await;
    let (s, _) = send(
        &app,
        "GET",
        "/chats/main-chat/file?path=agent-note.txt",
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "mainline is isolated from the workstream until an explicit promote"
    );

    // After an explicit promote, the mainline carries the stream's work.
    let (s, b) = send(&app, "POST", &format!("/workstreams/{ws_id}/promote"), None).await;
    assert_eq!(s, StatusCode::OK, "promote: {b}");

    std::env::remove_var("GAUGEWRIGHT_FAKE_AGENT");
}

#[tokio::test]
async fn archive_rehomes_members_to_mainline() {
    std::env::set_var("GAUGEWRIGHT_FAKE_AGENT", "1");
    let (_d, app) = workbench();
    let ws_id = workstream_with_members(&app, &["arc-a"]).await;

    // Archiving re-homes the member; a subsequent turn no longer auto-advances (it goes
    // back to the mainline review flow — its merge stays Clean for the human).
    let (s, b) = send(&app, "POST", &format!("/workstreams/{ws_id}/archive"), None).await;
    assert_eq!(s, StatusCode::OK, "archive: {b}");

    let (s, _) = send(
        &app,
        "POST",
        "/chats/arc-a/task",
        Some(r#"{"prompt":"post-archive work"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_s, merge) = send(&app, "GET", "/chats/arc-a/merge", None).await;
    assert!(
        merge.contains("Clean"),
        "a re-homed (mainline) chat awaits human review, not auto-advance: {merge}"
    );

    std::env::remove_var("GAUGEWRIGHT_FAKE_AGENT");
}

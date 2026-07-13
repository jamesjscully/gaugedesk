use std::sync::{Arc, Mutex};

use axum::Router;
use gaugewright_core::instance::InstanceState;
use gaugewright_store::Store;
use gaugewright_workspace::Instance;

use super::*;
use crate::test_support::fake_agent_env;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use gaugewright_core::instance::InstanceCommand;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[test]
fn context_attributes_map_labels_and_fail_closed_on_unknown() {
    use gaugewright_core::abac::{Classification, Region};
    // SECAUD-5: known labels map; region is carried.
    let a = resource_store::context_attributes(Some("pii"), Some("eu"));
    assert_eq!(a.classification, Classification::Pii);
    assert_eq!(a.region, Some(Region::new("eu")));
    assert_eq!(
        resource_store::context_attributes(Some("public"), None).classification,
        Classification::Public
    );
    assert_eq!(
        resource_store::context_attributes(Some("internal"), None).classification,
        Classification::Internal
    );
    // Unknown / typo / absent ⇒ fail-closed most-protected, never under-protect.
    assert_eq!(
        resource_store::context_attributes(Some("seekrit"), None).classification,
        Classification::Regulated
    );
    assert_eq!(
        resource_store::context_attributes(None, None).classification,
        Classification::Regulated
    );
    // Blank region is dropped (not an empty-string tag).
    assert_eq!(
        resource_store::context_attributes(Some("pii"), Some("   ")).region,
        None
    );
}

#[test]
fn authorize_resource_export_enforces_pii_classification_on_egress() {
    // SECAUD-5 / CORE-6: the live export gate denies a PII-labeled resource at an
    // unattested ceiling, lets an unlabeled (Regulated) resource through, and is
    // ungated in solo mode.
    use crate::identity::LoopbackIdentityProvider;
    use crate::library::RecordOp;
    use crate::org::{MembershipRecord, MembershipStatus, ORG_SCOPE};
    use gaugewright_core::abac::{AuthorityAttributes, Classification, Region, ResourceAttributes};
    use gaugewright_core::ids::AuthorityId;

    let idp = LoopbackIdentityProvider::new().enroll(
        "member-token",
        AuthorityId::new("member-auth"),
        AuthorityAttributes::default(),
    );
    let mut wb =
        Workbench::new(Store::open_in_memory().unwrap()).with_identity_provider(Arc::new(idp));
    // Provision an active member so the gate engages (past bootstrap).
    let member = MembershipRecord {
        id: "member-auth".into(),
        op: RecordOp::Upsert,
        org_id: "org".into(),
        authority: "member-auth".into(),
        email: "m@e.com".into(),
        role: "member".into(),
        status: MembershipStatus::Active,
        managed_by_scim: false,
        team: None,
    };
    wb.store_mut()
        .append_record(
            ORG_SCOPE,
            "membership",
            &serde_json::to_string(&member).unwrap(),
        )
        .unwrap();

    let pii = resource_store::mint_context_with(
        wb.store_mut(),
        "eng",
        "client",
        "pii-doc",
        "c1",
        ResourceAttributes {
            classification: Classification::Pii,
            region: Some(Region::new("eu")),
            purpose: Default::default(),
        },
    )
    .unwrap();
    let plain =
        resource_store::mint_context(wb.store_mut(), "eng", "client", "plain-doc", "c1").unwrap();

    // PII at an unattested ceiling is refused egress...
    let err = wb
        .authorize_resource_export(Some("member-token"), "eng", &pii.resource.id)
        .unwrap_err();
    assert_eq!(err.0, StatusCode::FORBIDDEN);
    // ...the unlabeled (Regulated) resource exports freely (policy constrains only PII).
    assert!(wb
        .authorize_resource_export(Some("member-token"), "eng", &plain.resource.id)
        .is_ok());

    // CORE-6: the same floor gates a resource-access *grant* — the PII resource is refused,
    // the unlabeled one is allowed.
    assert_eq!(
        wb.authorize_resource_access(Some("member-token"), "eng", &pii.resource.id)
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN,
        "PII access grant refused at an unattested ceiling",
    );
    assert!(
        wb.authorize_resource_access(Some("member-token"), "eng", &plain.resource.id)
            .is_ok(),
        "unlabeled resource access is allowed",
    );

    // Solo (no IdP) is ungated — PII included.
    let mut solo = Workbench::new(Store::open_in_memory().unwrap());
    let pii2 = resource_store::mint_context_with(
        solo.store_mut(),
        "eng",
        "client",
        "pii-doc",
        "c1",
        ResourceAttributes {
            classification: Classification::Pii,
            region: None,
            purpose: Default::default(),
        },
    )
    .unwrap();
    assert!(solo
        .authorize_resource_export(Some("member-token"), "eng", &pii2.resource.id)
        .is_ok());
    assert!(
        solo.authorize_resource_access(Some("member-token"), "eng", &pii2.resource.id)
            .is_ok(),
        "solo resource-access grant is ungated (the operator's own workspace)",
    );
}

#[test]
fn content_vault_wired_into_the_store_encrypts_transcripts_and_crypto_erases() {
    // SECAUD-9/6 end-to-end at the workbench: with the vault as the store codec,
    // transcript content round-trips (encrypted at rest), and crypto_erase_content
    // makes a chat's transcript unrecoverable while leaving another chat's intact.
    let dir = tempfile::tempdir().unwrap();
    let vault = Arc::new(content_vault::ContentVault::new(
        dir.path().join("ckeys"),
        Box::new(at_rest::LoopbackKeyWrap::new([3u8; 32])),
    ));
    let store = Store::open_in_memory().unwrap().with_codec(vault.clone());
    let mut wb = Workbench::new(store).with_content_vault(vault);

    wb.store_mut()
        .append_record("chat-1", "transcript", r#"{"line":"private"}"#)
        .unwrap();
    wb.store_mut()
        .append_record("chat-2", "transcript", r#"{"line":"keep"}"#)
        .unwrap();
    // Reads decrypt transparently.
    assert_eq!(
        wb.store_ref().records("chat-1", "transcript").unwrap(),
        vec![r#"{"line":"private"}"#]
    );

    // Deleting chat-1 crypto-erases its content; chat-2 is untouched (per-unit keys).
    assert!(wb.crypto_erase_content("chat-1"));
    assert!(
        wb.store_ref()
            .records("chat-1", "transcript")
            .unwrap()
            .is_empty(),
        "erased transcript is unrecoverable"
    );
    assert_eq!(
        wb.store_ref().records("chat-2", "transcript").unwrap(),
        vec![r#"{"line":"keep"}"#],
        "another chat's content is intact"
    );
}

/// ENTSEC-7: every control-plane response carries an HSTS header, so a browser that reaches
/// it over HTTPS will refuse a later plain-HTTP downgrade. Asserted on the always-open
/// `/health` route (no auth/state needed).
#[tokio::test]
async fn responses_carry_an_hsts_header() {
    let (_dir, wb) = workbench();
    let app = open_control_plane(wb);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let hsts = resp
        .headers()
        .get(axum::http::header::STRICT_TRANSPORT_SECURITY)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(hsts.contains("max-age="), "HSTS header present: {hsts:?}");
    assert!(hsts.contains("includeSubDomains"));
}

/// ADR 0065 gate: the cross-authority federation surface is PARKED off by default. A workbench
/// with no federation configured (the single-authority product shape) mounts **none** of the
/// `/federation/*` relay routes — they 404, not 503 — so the unauthenticated relay surface is
/// genuinely absent, not merely dormant.
#[tokio::test]
async fn federation_routes_are_absent_when_federation_is_off() {
    let (_dir, wb) = workbench(); // with_instance → no federation attached
    let app = open_control_plane(wb);
    for path in [
        "/federation/peers",
        "/federation/handoff/incoming",
        "/federation/run/queue",
    ] {
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{path} must be unmounted when federation is off"
        );
    }
    // A product route (always mounted) still answers — the gate removed only federation.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

/// RF-A4: a handler that panics while holding the workbench lock poisons
/// the mutex; `lock_unpoisoned` recovers it so the next request still
/// works, instead of every later lock panicking too.
#[test]
fn a_poisoned_lock_recovers_instead_of_cascading() {
    let m = Arc::new(Mutex::new(0_i32));
    let m2 = Arc::clone(&m);
    let _ = std::thread::spawn(move || {
        let _guard = m2.lock().unwrap();
        panic!("simulated handler panic while holding the lock");
    })
    .join();
    assert!(m.is_poisoned(), "the panic poisoned the mutex");
    *m.lock_unpoisoned() += 1;
    assert_eq!(*m.lock_unpoisoned(), 1);
}

fn workbench() -> (tempfile::TempDir, SharedWorkbench) {
    let dir = tempfile::tempdir().unwrap();
    let instance = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
    let store = Store::open_in_memory().unwrap();
    (
        dir,
        Arc::new(Mutex::new(Workbench::with_instance(
            "inst-test",
            instance,
            store,
        ))),
    )
}

async fn send(app: &Router, method: &str, uri: &str, body: Option<&str>) -> (StatusCode, String) {
    let b = match body {
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
    let resp = app.clone().oneshot(b).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// `send` for GETs, also returning the `x-workspace-cut` header — the
/// addressable base a cut-carrying save sends back (SUB-6 §12).
async fn send_with_cut(app: &Router, uri: &str) -> (StatusCode, Option<String>, String) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let cut = resp
        .headers()
        .get("x-workspace-cut")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, cut, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn transcript_is_durable_across_a_fresh_read() {
    let _fake_agent = fake_agent_env();
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);
    send(&app, "POST", "/chats", Some(r#"{"id":"t1"}"#)).await;
    send(
        &app,
        "POST",
        "/chats/t1/task",
        Some(r#"{"prompt":"do the thing"}"#),
    )
    .await;

    // a *fresh* GET (no client state) rebuilds the chat from durable records.
    let (s, body) = send(&app, "GET", "/chats/t1/transcript", None).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    assert!(
        body.contains(r#""type":"user""#) && body.contains("do the thing"),
        "user msg: {body}"
    );
    assert!(
        body.contains(r#""type":"assistant""#),
        "assistant msg: {body}"
    );
    assert!(
        body.contains(r#""type":"admitted""#) && body.contains("run → Completed"),
        "run: {body}"
    );
}

#[tokio::test]
async fn explicit_resource_access_request_approve_revoke_routes() {
    // CORE-3: the multi-party request → approve → grant → revoke lifecycle over HTTP.
    let (_dir, wb) = workbench();
    let app = open_control_plane(wb);
    send(&app, "POST", "/chats", Some(r#"{"id":"c1"}"#)).await;

    // Request access naming "alice" as the required approver → pending (Requested).
    let (s, b) = send(
        &app,
        "POST",
        "/chats/c1/resources/doc-1/access/request",
        Some(r#"{"required":["alice"]}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "got {b}");
    assert!(
        b.contains(r#""phase":"Requested""#),
        "request → Requested: {b}"
    );

    // The read route reflects the pending request (payload not yet accessible).
    let (_, b) = send(&app, "GET", "/chats/c1/resources/doc-1/access", None).await;
    assert!(b.contains(r#""phase":"Requested""#), "read: {b}");

    // alice approves → all required approved → Granted.
    let (s, b) = send(
        &app,
        "POST",
        "/chats/c1/resources/doc-1/access/approve",
        Some(r#"{"approver":"alice"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "got {b}");
    assert!(b.contains(r#""phase":"Granted""#), "approve → Granted: {b}");

    // Revoke the grant (INV-18, future-only) → Revoked.
    let (s, b) = send(
        &app,
        "POST",
        "/chats/c1/resources/doc-1/access/revoke",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "got {b}");
    assert!(b.contains(r#""phase":"Revoked""#), "revoke → Revoked: {b}");
}

#[tokio::test]
async fn opening_a_folder_mints_a_granted_context_resource() {
    let (dir, wb) = workbench();
    let app = open_control_plane(wb);
    send(&app, "POST", "/chats", Some(r#"{"id":"c1"}"#)).await;

    // a real folder to open as context
    let src = dir.path().join("docs");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("notes.txt"), "secret-bytes").unwrap();
    let body = serde_json::json!({ "path": src.to_str().unwrap() }).to_string();
    let (s, b) = send(&app, "POST", "/chats/c1/context", Some(&body)).await;
    assert_eq!(s, StatusCode::OK, "got {b}");
    assert!(
        b.contains(r#""resource":"ctx-"#),
        "returns the minted handle: {b}"
    );

    // a fresh GET (no client state) rebuilds the resources projection from the
    // store: a granted `context` handle owned by the local authority…
    let (s, b) = send(&app, "GET", "/chats/c1/resources", None).await;
    assert_eq!(s, StatusCode::OK, "got {b}");
    assert!(b.contains(r#""kind":"context""#), "context kind: {b}");
    assert!(
        b.contains(r#""access":"Granted""#),
        "auto-granted (trust-by-default): {b}"
    );
    assert!(b.contains(r#""owner":"local-user""#), "local owner: {b}");
    assert!(b.contains(r#""tombstoned":false"#), "not tombstoned: {b}");
    // …rendering metadata only — the payload bytes never enter the projection (INV-10).
    assert!(
        !b.contains("secret-bytes"),
        "payload not in the projection: {b}"
    );
}

#[tokio::test]
async fn content_resolves_through_a_granted_handle_then_tombstone_blocks_it() {
    let (dir, wb) = workbench();
    let app = open_control_plane(wb);
    send(&app, "POST", "/chats", Some(r#"{"id":"c2"}"#)).await;

    let src = dir.path().join("docs");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("notes.txt"), "secret-bytes").unwrap();
    let body = serde_json::json!({ "path": src.to_str().unwrap() }).to_string();
    send(&app, "POST", "/chats/c2/context", Some(&body)).await;
    let rid = resource_store::context_id(src.to_str().unwrap());
    let rid = rid.as_str();

    // the manifest resolves through the granted handle…
    let (s, b) = send(
        &app,
        "GET",
        &format!("/chats/c2/resources/{rid}/content"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "manifest resolves: {b}");
    assert!(
        b.contains("notes.txt"),
        "manifest lists the ingested file: {b}"
    );
    // …and so do the file's bytes.
    let (s, b) = send(
        &app,
        "GET",
        &format!("/chats/c2/resources/{rid}/content?path=notes.txt"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        b, "secret-bytes",
        "payload bytes resolve for a granted handle"
    );

    // tombstone the payload → future resolution is GONE (INV-18)…
    let (s, _) = send(
        &app,
        "POST",
        &format!("/chats/c2/resources/{rid}/tombstone"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(
        &app,
        "GET",
        &format!("/chats/c2/resources/{rid}/content?path=notes.txt"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::GONE, "tombstoned payload no longer resolves");
    // …while the handle/record survive in the projection, marked tombstoned (INV-6).
    let (_, b) = send(&app, "GET", "/chats/c2/resources", None).await;
    assert!(
        b.contains(r#""tombstoned":true"#),
        "handle/record remain: {b}"
    );
}

#[tokio::test]
async fn export_source_required_is_derived_from_the_resource_stakeholders() {
    let _fake_agent = fake_agent_env();
    let (dir, wb) = workbench();
    let app = open_control_plane(wb);
    send(&app, "POST", "/chats", Some(r#"{"id":"x1"}"#)).await;

    // open a context folder (owner = local authority), then run a turn: the
    // engine mints the derived output resource, tainted by the granted context.
    let src = dir.path().join("docs");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("notes.txt"), "data").unwrap();
    let body = serde_json::json!({ "path": src.to_str().unwrap() }).to_string();
    send(&app, "POST", "/chats/x1/context", Some(&body)).await;
    send(&app, "POST", "/chats/x1/task", Some(r#"{"prompt":"go"}"#)).await;

    let out = resource_store::output_id("x1");
    let out = out.as_str();
    let (_, b) = send(&app, "GET", "/chats/x1/resources", None).await;
    assert!(
        b.contains(out) && b.contains(r#""kind":"output""#),
        "output resource minted: {b}"
    );

    // propose export — source_required comes from the RESOURCE, not the caller.
    let (s, b) = send(
        &app,
        "POST",
        &format!("/chats/x1/resources/{out}/export"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "got {b}");
    assert!(b.contains(r#""phase":"Requested""#), "export proposed: {b}");
    assert!(
        b.contains("local-user"),
        "source_required derived from the resource owner: {b}"
    );

    // drive the gate to Exported through the returned scope: only once the
    // resource's stakeholder consents (and the target admits) does it cross.
    let scope = resource_store::export_scope("x1", &resource_store::output_id("x1"));
    let uri = format!("/scopes/{scope}/export/command");
    send(
        &app,
        "POST",
        &uri,
        Some(r#"{"SourceConsent":"local-user"}"#),
    )
    .await;
    send(&app, "POST", &uri, Some(r#""TargetAdmit""#)).await;
    let (s, b) = send(&app, "POST", &uri, Some(r#""Export""#)).await;
    assert_eq!(s, StatusCode::OK, "got {b}");
    assert!(
        b.contains(r#""phase":"Exported""#),
        "crosses once the resource stakeholder consented: {b}"
    );
}

/// RF-A9: export-to-disk is gated on the export lifecycle being `Exported`
/// and then actually writes the resolved bytes to disk + records the egress.
#[tokio::test]
async fn export_to_disk_is_gated_then_writes_bytes_and_records_egress() {
    let _fake_agent = fake_agent_env();
    let (dir, wb) = workbench();
    let app = open_control_plane(wb);
    send(&app, "POST", "/chats", Some(r#"{"id":"d1"}"#)).await;
    // A turn produces the engagement's output (the fake agent writes a note).
    send(&app, "POST", "/chats/d1/task", Some(r#"{"prompt":"go"}"#)).await;

    let out = resource_store::output_id("d1");
    let out = out.as_str();
    let dest = dir.path().join("delivered");
    std::fs::create_dir_all(&dest).unwrap();
    let dest_body = serde_json::json!({ "dest": dest.to_str().unwrap() }).to_string();

    // Before the export lifecycle clears, export-to-disk fails closed.
    let (s, b) = send(
        &app,
        "POST",
        &format!("/chats/d1/resources/{out}/export-to-disk"),
        Some(&dest_body),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "must fail closed before Exported: {b}"
    );
    assert!(
        std::fs::read_dir(&dest).unwrap().next().is_none(),
        "nothing leaves before the export is cleared"
    );

    // Clear the export lifecycle (propose → consent → target-admit → export).
    send(
        &app,
        "POST",
        &format!("/chats/d1/resources/{out}/export"),
        None,
    )
    .await;
    let scope = resource_store::export_scope("d1", &resource_store::output_id("d1"));
    let uri = format!("/scopes/{scope}/export/command");
    send(
        &app,
        "POST",
        &uri,
        Some(r#"{"SourceConsent":"local-user"}"#),
    )
    .await;
    send(&app, "POST", &uri, Some(r#""TargetAdmit""#)).await;
    send(&app, "POST", &uri, Some(r#""Export""#)).await;

    // Now export-to-disk writes the deliverable and records the egress.
    let (s, b) = send(
        &app,
        "POST",
        &format!("/chats/d1/resources/{out}/export-to-disk"),
        Some(&dest_body),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "exports once cleared: {b}");
    assert!(
        b.contains("agent-note.txt"),
        "the deliverable file is reported: {b}"
    );
    assert!(
        dest.join("agent-note.txt").exists(),
        "the bytes actually landed on disk"
    );
}

#[tokio::test]
async fn review_required_is_derived_from_the_resource_stakeholders() {
    let _fake_agent = fake_agent_env();
    let (dir, wb) = workbench();
    let app = open_control_plane(wb);
    send(&app, "POST", "/chats", Some(r#"{"id":"r2"}"#)).await;

    let src = dir.path().join("docs");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("notes.txt"), "data").unwrap();
    let body = serde_json::json!({ "path": src.to_str().unwrap() }).to_string();
    send(&app, "POST", "/chats/r2/context", Some(&body)).await;
    send(&app, "POST", "/chats/r2/task", Some(r#"{"prompt":"go"}"#)).await;
    let out = resource_store::output_id("r2");
    let out = out.as_str();

    // propose review — `required` comes from the resource, not the caller.
    let (s, b) = send(
        &app,
        "POST",
        &format!("/chats/r2/resources/{out}/review"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "got {b}");
    assert!(b.contains(r#""phase":"Proposed""#), "review proposed: {b}");
    assert!(
        b.contains("local-user"),
        "required derived from the resource stakeholder: {b}"
    );

    // the resource's stakeholder consenting clears the review, then it releases.
    let scope = resource_store::review_scope("r2", &resource_store::output_id("r2"));
    let uri = format!("/scopes/{scope}/review/command");
    let (_, b) = send(&app, "POST", &uri, Some(r#"{"Consent":"local-user"}"#)).await;
    assert!(
        b.contains(r#""phase":"Cleared""#),
        "clears once the stakeholder consents: {b}"
    );
    let (_, b) = send(&app, "POST", &uri, Some(r#""Release""#)).await;
    assert!(b.contains(r#""phase":"Released""#), "released: {b}");
}

#[tokio::test]
async fn package_publish_install_entitle_flow_over_http() {
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);

    // source: publish a package referencing a frozen version.
    let (s, b) = send(
        &app,
        "POST",
        "/packages",
        Some(r#"{"id":"p1","version":"v1","agent_ref":"agent-default"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "got {b}");
    assert!(b.contains(r#""published":"p1""#), "{b}");

    // target: install it. Installed — but a governed run is NOT yet ready.
    let (_, b) = send(&app, "POST", "/packages/p1/install", None).await;
    assert!(b.contains(r#""phase":"Installed""#), "{b}");
    let (_, b) = send(&app, "GET", "/packages/p1/readiness?context=ctx", None).await;
    assert!(
        b.contains(r#""installed":true"#) && b.contains(r#""run_ready":false"#),
        "install alone ≠ runnable: {b}"
    );

    // target authority: entitle the context → now run-ready.
    send(&app, "POST", "/packages/p1/entitle?context=ctx", None).await;
    let (_, b) = send(&app, "GET", "/packages/p1/readiness?context=ctx", None).await;
    assert!(
        b.contains(r#""run_ready":true"#),
        "installed + entitled ⇒ runnable: {b}"
    );

    // installing an unpublished package is rejected by the distribution lifecycle.
    let (s, _) = send(&app, "POST", "/packages/ghost/install", None).await;
    assert_eq!(s, StatusCode::CONFLICT, "unpublished install rejected");

    // the catalog projection shows p1 installed.
    let (_, b) = send(&app, "GET", "/packages", None).await;
    assert!(
        b.contains(r#""id":"p1""#) && b.contains(r#""status":"Installed""#),
        "catalog: {b}"
    );

    // withdrawal is a blocking event: it immediately drops availability AND readiness.
    send(&app, "POST", "/packages/p1/withdraw", None).await;
    let (_, b) = send(&app, "GET", "/packages", None).await;
    assert!(
        b.contains(r#""status":"Withdrawn""#),
        "catalog reflects withdrawal: {b}"
    );
    let (_, b) = send(&app, "GET", "/packages/p1/readiness?context=ctx", None).await;
    assert!(
        b.contains(r#""run_ready":false"#),
        "withdrawal immediately drops readiness: {b}"
    );
}

#[tokio::test]
async fn merge_lifecycle_clean_admit_advances_main() {
    let _fake_agent = fake_agent_env();
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);

    send(&app, "POST", "/chats", Some(r#"{"id":"m1"}"#)).await;
    // a (fake) turn leaves the merge awaiting review: Clean.
    let (s, body) = send(&app, "POST", "/chats/m1/task", Some(r#"{"prompt":"go"}"#)).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    assert!(body.contains("\"merge_phase\":\"Clean\""), "got {body}");

    let (_, body) = send(&app, "GET", "/chats/m1/merge", None).await;
    assert!(body.contains("\"phase\":\"Clean\""), "got {body}");

    // the human admits → the real merge runs → main advances.
    let (s, body) = send(
        &app,
        "POST",
        "/chats/m1/merge/command",
        Some(r#"{"action":"admit"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    assert!(body.contains("\"phase\":\"Advanced\""), "got {body}");

    // WS-1: integrate the advanced workstream into the shared mainline. The hop
    // admits the boundary command then integrates — MAINLINE_INTEGRATION_REQUIRES_
    // BOUNDARY, driven live (the reducer proptest verifies the gate).
    let (s, body) = send(
        &app,
        "POST",
        "/chats/m1/merge/command",
        Some(r#"{"action":"integrate"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    assert!(body.contains("\"phase\":\"Integrated\""), "got {body}");
}

#[tokio::test]
async fn revert_discards_chat_work() {
    let _fake_agent = fake_agent_env();
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);

    send(&app, "POST", "/chats", Some(r#"{"id":"rv1"}"#)).await;
    // a fake turn leaves work awaiting review (Clean).
    let (s, body) = send(&app, "POST", "/chats/rv1/task", Some(r#"{"prompt":"go"}"#)).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    assert!(body.contains("\"merge_phase\":\"Clean\""), "got {body}");

    // revert (UX-5) discards it.
    let (s, body) = send(&app, "POST", "/chats/rv1/revert", None).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    assert!(body.contains("\"reverted\":true"), "got {body}");

    // an unknown chat fails closed (404).
    let (s, _) = send(&app, "POST", "/chats/nope/revert", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn end_to_end_run_command_flow() {
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);

    for cmd in ["\"RequestRun\"", "\"AdmitRun\"", "\"StartRun\""] {
        let (s, _) = send(&app, "POST", "/scopes/run-1/run/command", Some(cmd)).await;
        assert_eq!(s, StatusCode::OK);
    }
    let (s, body) = send(&app, "GET", "/scopes/run-1/run", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.contains("Running"), "got {body}");

    // INV-11: start without admission is rejected (409), no fact appended.
    let (_, _) = send(
        &app,
        "POST",
        "/scopes/run-2/run/command",
        Some("\"RequestRun\""),
    )
    .await;
    let (s, body) = send(
        &app,
        "POST",
        "/scopes/run-2/run/command",
        Some("\"StartRun\""),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(body.contains("rejected"), "got {body}");
    let (_, body) = send(&app, "GET", "/scopes/run-2/run", None).await;
    assert!(body.contains("Requested"), "got {body}");
}

/// MOB-012: the projection shim wraps the folded value in a `ProjectionCarriage`
/// (mirrors `web/src/api/projection-carriage.ts`) — a default read is `live`,
/// a declared non-live read carries its caveat + a repair hint, and an unknown
/// kind is a 404. The basis grows as truth is admitted (the append-only clock).
#[tokio::test]
async fn fork_tree_route_returns_a_forest() {
    // UX-8: the fork-forest projection is reachable (and the router builds — no route
    // conflict with /chats/*).
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);
    let (s, body) = send(&app, "GET", "/fork-tree", None).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["forest"].is_array(), "forest is an array: {body}");
}

#[tokio::test]
async fn merge_projection_has_freshness_carriage() {
    // UX-13: the `merge` kind now has a carriage read like run/review/export/boundary.
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);
    let (s, body) = send(&app, "GET", "/projections/m-9/merge", None).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["freshness"]["marker"], "live", "got {body}");
    assert!(
        v["value"].is_object(),
        "value is the folded merge projection: {body}"
    );
    // an unknown kind still fails closed with a 404.
    let (s, _) = send(&app, "GET", "/projections/m-9/bogus", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn health_probe_is_a_fixed_ok() {
    // The readiness probe the SERVE-2 sandbox warm-check (and any host) polls: a 200
    // once the control plane is serving, no store access.
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);
    let (s, body) = send(&app, "GET", "/health", None).await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["ok"], true);
}

#[tokio::test]
async fn project_home_rolls_up_runs_outputs_and_audit() {
    // UX-2: the project-home rollup aggregates the project's work chats' run/merge/audit
    // state from data (INV-5), across all its placements; 404 on an unknown project.
    use library::{Admission, ChatRecord, InstanceKind, InstanceRecord, ProjectRecord, RecordOp};
    let (_d, wb) = workbench();
    {
        let mut g = wb.lock_unpoisoned();
        g.write_project_record(ProjectRecord {
            id: "proj-1".into(),
            op: RecordOp::Upsert,
            name: "Acme".into(),
            is_default: false,
            network_isolated: false,
            run_purpose: None,
            deployment_mode: None,
        });
        g.write_instance_record(InstanceRecord {
            id: "inst-1".into(),
            op: RecordOp::Upsert,
            kind: InstanceKind::Using,
            agent_id: "a1".into(),
            project_id: Some("proj-1".into()),
            version: 1,
            admission: Admission::Active,
        });
        g.write_chat_record(ChatRecord {
            id: "chat-1".into(),
            op: RecordOp::Upsert,
            instance_id: "inst-1".into(),
            title: "Build the landing page".into(),
            created_position: 1,
            forked_from: None,
        });
    }
    let app = open_control_plane(wb);

    let (s, body) = send(&app, "GET", "/projects/proj-1/home", None).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["project_id"], "proj-1");
    assert_eq!(v["audit"]["placements"], 1);
    assert_eq!(v["audit"]["chats"], 1);
    // The work chat appears in recent_runs, derived from its (fresh) RunState — never run.
    let runs = v["recent_runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["chat"], "chat-1");
    assert_eq!(runs[0]["title"], "Build the landing page");
    assert_eq!(runs[0]["ran"], false);
    // No live merge ⇒ no output/review summary yet.
    assert!(v["outputs"].as_array().unwrap().is_empty());

    // Fail-closed on an unknown project.
    let (s, _) = send(&app, "GET", "/projects/nope/home", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn projection_shim_wraps_every_read_in_a_freshness_carriage() {
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);

    // Nothing admitted yet: a live carriage over an empty run projection, basis 0.
    let (s, body) = send(&app, "GET", "/projections/run-9/run", None).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["freshness"]["marker"], "live",
        "default read is live: {body}"
    );
    assert_eq!(
        v["freshness"]["generated_at"], 0,
        "empty scope basis is 0: {body}"
    );
    assert!(
        v["freshness"]["repair_hint"].is_null(),
        "live carries no hint: {body}"
    );
    assert!(v["client_request_id"].is_null(), "no reconcile id: {body}");
    assert!(
        v["value"].is_object(),
        "value is the folded projection: {body}"
    );

    // Admit some run truth — the basis (last admitted position) advances.
    for cmd in ["\"RequestRun\"", "\"AdmitRun\"", "\"StartRun\""] {
        let (s, _) = send(&app, "POST", "/scopes/run-9/run/command", Some(cmd)).await;
        assert_eq!(s, StatusCode::OK);
    }
    let (_, body) = send(&app, "GET", "/projections/run-9/run", None).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(body.contains("Running"), "wraps the live run state: {body}");
    assert!(
        v["freshness"]["generated_at"].as_u64().unwrap() > 0,
        "basis advanced as truth was admitted: {body}"
    );

    // A declared non-live read keeps its caveat + a repair hint (never silently live).
    let (s, body) = send(&app, "GET", "/projections/run-9/run?freshness=stale", None).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["freshness"]["marker"], "stale",
        "honours the declared marker: {body}"
    );
    assert!(
        v["freshness"]["repair_hint"].is_string(),
        "stale carries a repair hint: {body}"
    );

    // An unknown marker is a client error, not a silent fallback to live.
    let (s, _) = send(&app, "GET", "/projections/run-9/run?freshness=bogus", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "unknown marker rejected");

    // An unknown kind is a 404, not an empty carriage.
    let (s, _) = send(&app, "GET", "/projections/run-9/nope", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "unknown kind is 404");

    // The shim projects the other admission-spine kinds too (e.g. boundary).
    let (s, body) = send(&app, "GET", "/projections/run-9/boundary", None).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    assert!(
        body.contains("ceiling"),
        "boundary carriage carries its ceiling: {body}"
    );
}

#[tokio::test]
async fn engagement_lifecycle_over_http() {
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);

    let (s, body) = send(&app, "POST", "/chats", Some(r#"{"id":"e1"}"#)).await;
    assert_eq!(s, StatusCode::CREATED, "got {body}");
    assert!(body.contains("engagement/e1"), "branch in response: {body}");

    // duplicate id is a conflict
    let (s, _) = send(&app, "POST", "/chats", Some(r#"{"id":"e1"}"#)).await;
    assert_eq!(s, StatusCode::CONFLICT);

    let (s, body) = send(&app, "GET", "/chats", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.contains("e1"));

    // a fresh engagement has an empty diff against main
    let (s, body) = send(&app, "GET", "/chats/e1/diff", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.contains("\"diff\""), "got {body}");
}

#[tokio::test]
async fn agent_authoring_config_round_trips_and_rejects_garbage() {
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);
    send(&app, "POST", "/chats", Some(r#"{"id":"a1"}"#)).await;

    // empty config initially
    let (s, body) = send(&app, "GET", "/chats/a1/config", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body.trim(), "{}");

    // valid config is parsed-then-persisted
    let cfg = r#"{"provider":"openai-codex","model":"gpt-5.5","thinking":"high"}"#;
    let (s, _) = send(&app, "PUT", "/chats/a1/config", Some(cfg)).await;
    assert_eq!(s, StatusCode::OK);
    let (_, body) = send(&app, "GET", "/chats/a1/config", None).await;
    assert!(body.contains("openai-codex"), "got {body}");

    // Package capabilities cannot be smuggled back into host runtime settings.
    let (s, body) = send(
        &app,
        "PUT",
        "/chats/a1/config",
        Some(r#"{"policy":{"block_tools":["bash"]}}"#),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(body.contains("package-owned"), "got {body}");

    // malformed config is rejected at the boundary, not written
    let (s, _) = send(&app, "PUT", "/chats/a1/config", Some("{ not json")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn review_and_export_negative_paths_reachable_over_http() {
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);
    // review: propose then cancel → withheld (the INV-23 escape, now exposed).
    send(
        &app,
        "POST",
        "/scopes/r1/review/command",
        Some(r#"{"Propose":{"required":["A"]}}"#),
    )
    .await;
    let (s, body) = send(
        &app,
        "POST",
        "/scopes/r1/review/command",
        Some(r#""Cancel""#),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    assert!(body.contains("Withheld"), "canceled → withheld: {body}");
    // export: propose then a source reject → denied.
    send(
        &app,
        "POST",
        "/scopes/e1/export/command",
        Some(r#"{"ProposeExport":{"source_required":["A","B"]}}"#),
    )
    .await;
    let (_, body) = send(
        &app,
        "POST",
        "/scopes/e1/export/command",
        Some(r#"{"Reject":"B"}"#),
    )
    .await;
    assert!(body.contains("Denied"), "rejected → denied: {body}");
}

#[tokio::test]
async fn review_shelf_drives_conjunctive_consent_over_http() {
    let (_d, wb) = workbench();
    let app = open_control_plane(wb);
    let scope = "/scopes/out-1";

    // propose review requiring two stakeholders
    let (s, _) = send(
        &app,
        "POST",
        &format!("{scope}/review/command"),
        Some(r#"{"Propose":{"required":["A","B"]}}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // one consent is not enough — still Proposed
    let (_, body) = send(
        &app,
        "POST",
        &format!("{scope}/review/command"),
        Some(r#"{"Consent":"A"}"#),
    )
    .await;
    assert!(body.contains("Proposed"), "got {body}");

    // the second consent auto-clears (conjunctive consent), then release
    let (_, body) = send(
        &app,
        "POST",
        &format!("{scope}/review/command"),
        Some(r#"{"Consent":"B"}"#),
    )
    .await;
    assert!(body.contains("Cleared"), "got {body}");
    let (_, body) = send(
        &app,
        "POST",
        &format!("{scope}/review/command"),
        Some(r#""Release""#),
    )
    .await;
    assert!(body.contains("Released"), "got {body}");

    // the audit timeline records the whole lifecycle in order
    let (s, body) = send(&app, "GET", &format!("{scope}/audit"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        body.contains("Proposed") && body.contains("Released"),
        "got {body}"
    );
}

/// A workbench seeded like the live server (builder agent + authoring instance)
/// so the library routes have an instance to work against.
fn seeded_workbench() -> (tempfile::TempDir, SharedWorkbench) {
    let dir = tempfile::tempdir().unwrap();
    let wb = open_workbench(dir.path()).unwrap();
    (dir, wb)
}

/// Seed the org's archetype-approval policy (`APPROVE-1`, ADR 0064) — the record the
/// ee Admin Console writes — directly into the org scope before the control plane opens.
fn seed_require_archetype_approval(wb: &SharedWorkbench) {
    use crate::org::{ArchetypeApprovalPolicyRecord, ORG_SCOPE};
    let mut guard = wb.lock_unpoisoned();
    let rec = ArchetypeApprovalPolicyRecord {
        id: String::new(),
        op: crate::library::RecordOp::Upsert,
        require_approval: true,
    };
    guard
        .store_mut()
        .append_record(
            ORG_SCOPE,
            "archetype_approval",
            &serde_json::to_string(&rec).unwrap(),
        )
        .unwrap();
}

/// Find a placement's projected JSON in `GET /workspace` by its instance id.
async fn placement_json(app: &Router, iid: &str) -> Option<serde_json::Value> {
    let (_, body) = send(app, "GET", "/workspace", None).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    for project in v["projects"].as_array()? {
        for placement in project["placements"].as_array()? {
            if placement["placement_id"] == iid {
                return Some(placement.clone());
            }
        }
    }
    None
}

/// APPROVE-1 (ADR 0064): under an approval-required policy an explicitly-added
/// archetype's placement lands **pending** — it can't host a work chat and is flagged in
/// the nav — until the owner **accepts** it, whereupon it goes active and hosts chats.
#[tokio::test]
async fn approval_policy_holds_a_placement_pending_until_the_owner_accepts() {
    let (_d, wb) = seeded_workbench();
    seed_require_archetype_approval(&wb);
    let app = open_control_plane(wb);

    // an archetype to place, and a project (its built-in general placement stays active).
    let (_, body) = send(&app, "POST", "/archetypes", Some(r#"{"name":"reviewer"}"#)).await;
    let agent_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, body) = send(&app, "POST", "/projects", Some(r#"{"name":"acme"}"#)).await;
    let pid = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // explicitly add the archetype → a *pending* placement under the policy.
    let (s, body) = send(
        &app,
        "POST",
        &format!("/projects/{pid}/placements"),
        Some(&format!(r#"{{"agent_id":"{agent_id}"}}"#)),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "bound: {body}");
    let iid = serde_json::from_str::<serde_json::Value>(&body).unwrap()["instance_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        placement_json(&app, &iid).await.expect("in workspace")["pending"],
        true,
        "an explicitly-added placement starts pending under the policy",
    );

    // a work chat can't root on a pending placement — fail closed with an actionable reason.
    let (s, body) = send(
        &app,
        "POST",
        &format!("/projects/{pid}/placements/{iid}/chats"),
        Some(r#"{"title":"go"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR, "refused: {body}");
    assert!(
        body.contains("pending approval"),
        "actionable reason: {body}"
    );

    // the owner's second act accepts it → active.
    let (s, body) = send(&app, "POST", &format!("/placements/{iid}/accept"), None).await;
    assert_eq!(s, StatusCode::OK, "accepted: {body}");
    assert_eq!(
        placement_json(&app, &iid).await.unwrap()["pending"],
        false,
        "accepted placement is active",
    );

    // now a work chat is allowed.
    let (s, body) = send(
        &app,
        "POST",
        &format!("/projects/{pid}/placements/{iid}/chats"),
        Some(r#"{"title":"go"}"#),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CREATED,
        "active placement hosts a chat: {body}"
    );
}

/// APPROVE-1: the default (frictionless) policy admits an explicitly-added placement
/// **active at once** — no accept step, a chat roots immediately.
#[tokio::test]
async fn frictionless_default_admits_a_placement_active_at_once() {
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);

    let (_, body) = send(&app, "POST", "/archetypes", Some(r#"{"name":"reviewer"}"#)).await;
    let agent_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, body) = send(&app, "POST", "/projects", Some(r#"{"name":"acme"}"#)).await;
    let pid = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (s, body) = send(
        &app,
        "POST",
        &format!("/projects/{pid}/placements"),
        Some(&format!(r#"{{"agent_id":"{agent_id}"}}"#)),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "bound: {body}");
    let iid = serde_json::from_str::<serde_json::Value>(&body).unwrap()["instance_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        placement_json(&app, &iid).await.unwrap()["pending"],
        false,
        "frictionless default is active immediately",
    );
    let (s, body) = send(
        &app,
        "POST",
        &format!("/projects/{pid}/placements/{iid}/chats"),
        Some(r#"{"title":"go"}"#),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CREATED,
        "active placement hosts a chat: {body}"
    );
}

/// ENTSEC-5: context can be ingested from an **upload** (not just a server-local path) — the
/// enterprise thin-client's context-in. The uploaded file lands in the engagement worktree
/// and a context resource is minted.
#[tokio::test]
async fn context_upload_ingests_files_into_the_engagement() {
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);

    // a live work chat (an engagement with a worktree).
    let (s, _) = send(&app, "POST", "/chats", Some(r#"{"id":"up-chat"}"#)).await;
    assert_eq!(s, StatusCode::CREATED);

    // upload context files.
    let (s, body) = send(
        &app,
        "POST",
        "/chats/up-chat/context/upload",
        Some(r##"{"files":[{"name":"brief.md","content":"# the brief"}],"classification":"internal"}"##),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "upload accepted: {body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["ingested"], 1);
    assert!(
        v["resource"].as_str().is_some(),
        "a resource was minted: {body}"
    );

    // the uploaded file is now in the engagement worktree.
    let (_, tree) = send(&app, "GET", "/chats/up-chat/tree", None).await;
    assert!(
        tree.contains("brief.md"),
        "uploaded file present in the tree: {tree}"
    );
}

#[tokio::test]
async fn workspace_seeds_a_default_agent_then_agents_and_chats_appear() {
    let (d, wb) = seeded_workbench();
    let app = open_control_plane(wb);

    let package = gaugewright_whip_runtime::AuthoredAgentPackage::load(
        d.path()
            .join("instances")
            .join(DEFAULT_INSTANCE)
            .join("repo/.whipple/versions/1"),
    )
    .expect("seeded version is a native WhippleScript package");
    assert!(package.version_ref().starts_with("whip:agent-package:"));

    // fresh root seeds the default agent under the Agents facet.
    let (s, body) = send(&app, "GET", "/workspace", None).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    assert!(
        body.contains("\"name\":\"assistant\""),
        "default agent seeded: {body}"
    );
    assert!(body.contains("\"is_default\":true"), "got {body}");

    // create an agent → it shows up.
    let (s, body) = send(&app, "POST", "/archetypes", Some(r#"{"name":"reviewer"}"#)).await;
    assert_eq!(s, StatusCode::CREATED, "got {body}");
    let agent_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // new chat under that agent → appears under it + in recent.
    let (s, body) = send(
        &app,
        "POST",
        &format!("/archetypes/{agent_id}/chats"),
        Some(r#"{"title":"first chat"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "got {body}");
    let chat_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (_, body) = send(&app, "GET", "/workspace", None).await;
    assert!(
        body.contains("reviewer") && body.contains("first chat"),
        "got {body}"
    );

    // the chat is a real engagement: it can be tasked/diffed.
    let (s, _) = send(&app, "GET", &format!("/chats/{chat_id}/diff"), None).await;
    assert_eq!(s, StatusCode::OK);

    // delete the chat → tombstoned, gone from the workspace.
    let (s, _) = send(&app, "DELETE", &format!("/chats/{chat_id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, body) = send(&app, "GET", "/workspace", None).await;
    assert!(!body.contains("first chat"), "deleted chat is gone: {body}");
}

#[tokio::test]
async fn publish_freezes_native_package_and_upgrade_installs_that_exact_ref() {
    let (dir, wb) = seeded_workbench();
    let edit_draft = |wb: &SharedWorkbench, body: &str| {
        let guard = wb.lock_unpoisoned();
        let workspace = guard
            .instances
            .get(DEFAULT_INSTANCE)
            .expect("authoring workspace");
        let id = library::gen_id("test-edit");
        let edit = workspace.create_engagement(&id).expect("edit engagement");
        edit.write_file(".whipple/draft/persona.md", body)
            .expect("edit persona");
        edit.commit_turn("edit package draft")
            .expect("commit draft");
        assert_eq!(
            edit.merge_into_main().expect("merge draft"),
            gaugewright_workspace::MergeOutcome::Clean
        );
        workspace.remove_engagement(&id).expect("remove edit");
    };
    edit_draft(&wb, "published persona");

    let app = open_control_plane(wb.clone());
    let (status, body) = send(
        &app,
        "POST",
        &format!("/archetypes/{DEFAULT_AGENT}/publish"),
        Some("{}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    assert!(body.contains("\"version\":2"), "publish: {body}");

    let frozen_root = dir
        .path()
        .join("instances")
        .join(DEFAULT_INSTANCE)
        .join("repo/.whipple/versions/2");
    let frozen =
        gaugewright_whip_runtime::AuthoredAgentPackage::load(&frozen_root).expect("frozen package");
    let frozen_ref = frozen.version_ref().to_owned();
    assert_eq!(
        std::fs::read_to_string(frozen_root.join("persona.md")).unwrap(),
        "published persona"
    );

    // Further draft work cannot mutate version 2 or its content address.
    edit_draft(&wb, "unpublished persona");
    assert_eq!(
        gaugewright_whip_runtime::AuthoredAgentPackage::load(&frozen_root)
            .unwrap()
            .version_ref(),
        frozen_ref
    );

    let (status, body) = send(
        &app,
        "POST",
        &format!("/placements/{DEFAULT_PLACEMENT}/upgrade"),
        Some("{}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upgrade: {body}");
    let installed_root = dir
        .path()
        .join("instances")
        .join(DEFAULT_PLACEMENT)
        .join("repo/.whipple/versions/2");
    let installed = gaugewright_whip_runtime::AuthoredAgentPackage::load(installed_root)
        .expect("installed package");
    assert_eq!(installed.version_ref(), frozen_ref);
}

#[tokio::test]
async fn base_carrying_save_merges_concurrent_edits_and_folds_conflicts() {
    // SUB-6: the editor's save carries the content it loaded (the
    // three-way base). Concurrent disjoint edits merge through whip's
    // token-level engine; overlapping rewrites 409 with the fold payload
    // and write nothing; the merge fact reaches the transcript while the
    // piece-level provenance lands on the audit plane.
    let (_dir, wb) = seeded_workbench();
    let app = open_control_plane(wb);
    let (status, body) = send(&app, "POST", "/chats", Some(r#"{"id":"sub6"}"#)).await;
    assert_eq!(status, StatusCode::CREATED, "chat: {body}");
    let base = "The quick brown fox jumps over the lazy dog tonight.";
    let (status, _) = send(&app, "PUT", "/chats/sub6/file?path=notes.md", Some(base)).await;
    assert_eq!(status, StatusCode::OK);
    // An "agent" write moves the file (legacy unconditional PUT stands in
    // for the turn's mediated write).
    let (status, _) = send(
        &app,
        "PUT",
        "/chats/sub6/file?path=notes.md",
        Some("The swift brown fox jumps over the lazy dog tonight."),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // The editor saves a draft based on the ORIGINAL content, editing a
    // distant word: the save merges, keeping both edits.
    let save = serde_json::json!({
        "content": "The quick brown fox jumps over the lazy dog today.",
        "base_content": base,
    })
    .to_string();
    let (status, body) = send(&app, "PUT", "/chats/sub6/file?path=notes.md", Some(&save)).await;
    assert_eq!(status, StatusCode::OK, "merged save: {body}");
    let merged: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(merged["merged"], true);
    assert_eq!(
        merged["content"],
        "The swift brown fox jumps over the lazy dog today."
    );
    let (_, file) = send(&app, "GET", "/chats/sub6/file?path=notes.md", None).await;
    assert_eq!(file, "The swift brown fox jumps over the lazy dog today.");
    // Provenance is audit evidence; the transcript states only the fact.
    let (_, audit) = send(&app, "GET", "/chats/sub6/audit", None).await;
    assert!(audit.contains("save_merged"), "audit provenance: {audit}");
    let (_, transcript) = send(&app, "GET", "/chats/sub6/transcript", None).await;
    assert!(
        transcript.contains("merged with concurrent changes"),
        "the fact reaches the conversation: {transcript}"
    );
    // Overlapping rewrites: 409 with the fold payload, nothing written.
    let head = "The swift brown fox jumps over the lazy dog today.";
    let (status, _) = send(
        &app,
        "PUT",
        "/chats/sub6/file?path=notes.md",
        Some("The swift brown fox jumps over the lazy TIGER today."),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let save = serde_json::json!({
        "content": "The swift brown fox jumps over the lazy LION today.",
        "base_content": head,
    })
    .to_string();
    let (status, body) = send(&app, "PUT", "/chats/sub6/file?path=notes.md", Some(&save)).await;
    assert_eq!(status, StatusCode::CONFLICT, "fold payload: {body}");
    let conflict: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(conflict["conflict"], true);
    assert!(
        conflict["pieces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|piece| piece["kind"] == "conflict"),
        "structured regions: {body}"
    );
    let (_, file) = send(&app, "GET", "/chats/sub6/file?path=notes.md", None).await;
    assert_eq!(
        file, "The swift brown fox jumps over the lazy TIGER today.",
        "a conflicted save writes nothing"
    );
}

#[tokio::test]
async fn cut_carrying_saves_mint_region_memory_and_preview_folds() {
    // The §12 endgame over HTTP: GET names the state it serves
    // (x-workspace-cut), the save carries that cut back, a fold-settled
    // region rides the resolve as durable memory, and the SAME divergence
    // in ANOTHER file later folds cleanly through the read-only preview —
    // resolved provenance, no re-ask.
    let (_dir, wb) = seeded_workbench();
    let app = open_control_plane(wb);
    let (status, body) = send(&app, "POST", "/chats", Some(r#"{"id":"cut1"}"#)).await;
    assert_eq!(status, StatusCode::CREATED, "chat: {body}");
    let base = "Alpha beta gamma delta epsilon zeta eta theta.";
    let agent = "Alpha beta AGENT-GAMMA delta epsilon zeta eta theta.";
    let editor = "Alpha beta EDITOR-GAMMA delta epsilon zeta eta theta.";

    send(&app, "PUT", "/chats/cut1/file?path=one.md", Some(base)).await;
    let (status, cut, body) = send_with_cut(&app, "/chats/cut1/file?path=one.md").await;
    assert_eq!(status, StatusCode::OK, "read: {body}");
    let cut = cut.expect("the read names its cut");
    // The agent moves the file; the editor saves an overlapping draft
    // against the cut it loaded → 409, structured regions, re-save base.
    send(&app, "PUT", "/chats/cut1/file?path=one.md", Some(agent)).await;
    let save = serde_json::json!({ "content": editor, "base_cut": cut }).to_string();
    let (status, body) = send(&app, "PUT", "/chats/cut1/file?path=one.md", Some(&save)).await;
    assert_eq!(status, StatusCode::CONFLICT, "fold payload: {body}");
    let conflict: serde_json::Value = serde_json::from_str(&body).unwrap();
    let resave_cut = conflict["current_cut"]
        .as_str()
        .expect("re-save base")
        .to_owned();
    let pieces = conflict["pieces"].as_array().unwrap().clone();
    let region = pieces
        .iter()
        .find(|piece| piece["kind"] == "conflict")
        .expect("a conflict region")
        .clone();
    // The user settles the region; the composed document re-saves with
    // the settled triple riding along.
    let composed: String = pieces
        .iter()
        .map(|piece| {
            if piece["kind"] == "merged" {
                piece["text"].as_str().unwrap().to_owned()
            } else {
                "SETTLED-GAMMA".to_owned()
            }
        })
        .collect();
    let resolve = serde_json::json!({
        "content": composed,
        "base_cut": resave_cut,
        "resolutions": [{
            "base_text": region["base_text"],
            "ours_text": region["ours_text"],
            "theirs_text": region["theirs_text"],
            "resolution_text": "SETTLED-GAMMA",
        }],
    })
    .to_string();
    let (status, body) = send(&app, "PUT", "/chats/cut1/file?path=one.md", Some(&resolve)).await;
    assert_eq!(status, StatusCode::OK, "resolved save: {body}");
    let saved: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(saved["cut"].is_string(), "the save names its cut: {body}");
    let (_, file) = send(&app, "GET", "/chats/cut1/file?path=one.md", None).await;
    assert!(
        file.contains("SETTLED-GAMMA"),
        "settled text landed: {file}"
    );
    // Minted memory is audit-plane evidence.
    let (_, audit) = send(&app, "GET", "/chats/cut1/audit", None).await;
    assert!(
        audit.contains("region_resolutions_recorded"),
        "memory minting is recorded: {audit}"
    );

    // Pay-forward through the read-only preview: same divergence, other
    // file — folds clean with resolved provenance, nothing moves.
    send(&app, "PUT", "/chats/cut1/file?path=two.md", Some(base)).await;
    let (_, cut2, _) = send_with_cut(&app, "/chats/cut1/file?path=two.md").await;
    let cut2 = cut2.expect("second read names its cut");
    send(&app, "PUT", "/chats/cut1/file?path=two.md", Some(agent)).await;
    let preview =
        serde_json::json!({ "path": "two.md", "draft": editor, "base_cut": cut2 }).to_string();
    let (status, body) = send(&app, "POST", "/chats/cut1/merge-preview", Some(&preview)).await;
    assert_eq!(status, StatusCode::OK, "preview: {body}");
    let preview: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(preview["known_base"], true, "known base: {body}");
    assert_eq!(preview["clean"], true, "memory folds it clean: {body}");
    assert!(
        preview["merged"]
            .as_str()
            .unwrap()
            .contains("SETTLED-GAMMA"),
        "the fold carries the remembered text: {body}"
    );
    assert!(
        preview["pieces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|piece| piece["provenance"] == "resolved"),
        "remembered regions are honestly tagged: {body}"
    );
    // Preview moved nothing: the file still holds the agent's body.
    let (_, file) = send(&app, "GET", "/chats/cut1/file?path=two.md", None).await;
    assert_eq!(file, agent, "read-only preview");
}

#[tokio::test]
async fn file_edits_respect_draft_version_and_host_control_ownership() {
    let (_dir, wb) = seeded_workbench();
    let app = open_control_plane(wb);
    let (status, body) = send(
        &app,
        "POST",
        &format!("/archetypes/{DEFAULT_AGENT}/chats"),
        Some(r#"{"title":"edit package"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "edit chat: {body}");
    let edit_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _) = send(
        &app,
        "PUT",
        &format!("/chats/{edit_id}/file?path=.whipple%2Fdraft%2Fpersona.md"),
        Some("new draft"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(
        &app,
        "PUT",
        &format!("/chats/{edit_id}/file?path=.whipple%2Fversions%2F1%2Fpersona.md"),
        Some("tamper"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "frozen write: {body}");
    let (status, body) = send(
        &app,
        "PUT",
        &format!("/chats/{edit_id}/file?path=.agent-config.json"),
        Some("{}"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "host config write: {body}");

    let (status, body) = send(&app, "POST", "/chats", Some("{}")).await;
    assert_eq!(status, StatusCode::CREATED);
    let work_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, body) = send(
        &app,
        "PUT",
        &format!("/chats/{work_id}/file?path=.whipple%2Fdraft%2Fpersona.md"),
        Some("tamper"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "work package write: {body}");
}

/// The All-chats "+ new chat" quick-start: `POST /chats` with no id mints one
/// server-side and roots on the hidden Personal default placement (a work chat).
#[tokio::test]
async fn post_chats_without_id_mints_a_work_chat_on_the_default_placement() {
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);

    // No id in the body ⇒ the server mints one (the UI never mints ids).
    let (s, body) = send(&app, "POST", "/chats", Some("{}")).await;
    assert_eq!(s, StatusCode::CREATED, "got {body}");
    let v = serde_json::from_str::<serde_json::Value>(&body).unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("chat-"), "minted a chat id: {id}");

    // It is a real engagement (diffable) rooted on the default placement, and it
    // carries the "new chat" placeholder title so the nav renders it "Untitled".
    let (s, _) = send(&app, "GET", &format!("/chats/{id}/diff"), None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, body) = send(&app, "GET", "/workspace", None).await;
    assert!(
        body.contains(&id) && body.contains("\"new chat\""),
        "got {body}"
    );

    // A second quick-start mints a distinct id — no collision, no client id.
    let (s, body2) = send(&app, "POST", "/chats", Some("{}")).await;
    assert_eq!(s, StatusCode::CREATED, "got {body2}");
    let id2 = serde_json::from_str::<serde_json::Value>(&body2).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(id, id2, "each quick-start gets its own id");
}

/// Regression: a store seeded before activate-on-seed left `inst-default` in
/// `Created` (never pinned ⇒ not runnable), so "new chat" 500'd with "instance
/// is not runnable". Opening such a store must self-heal the default instance.
#[tokio::test]
async fn legacy_store_with_unactivated_default_instance_self_heals_on_open() {
    use library::{Admission, AgentRecord, InstanceKind, InstanceRecord, RecordOp, LIBRARY_SCOPE};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let instances_dir = root.join("instances");
    std::fs::create_dir_all(&instances_dir).unwrap();

    // Lay down a *legacy* store: the default agent/instance records exist and the
    // instance repo is seeded, but the instance was never activated.
    {
        let mut store = Store::open(root.join("gaugewright.db").to_str().unwrap()).unwrap();
        let inst = Instance::init_at(instances_dir.join(DEFAULT_INSTANCE)).unwrap();
        inst.seed_main(&[
            (".pi/SYSTEM.md", app_support::DEFAULT_AGENT_SYSTEM_MD),
            ("AGENTS.md", app_support::DEFAULT_AGENT_AGENTS_MD),
        ])
        .unwrap();
        let inst_rec = InstanceRecord {
            id: DEFAULT_INSTANCE.into(),
            op: RecordOp::Upsert,
            kind: InstanceKind::Authoring,
            agent_id: DEFAULT_AGENT.into(),
            project_id: None,
            version: 1,
            admission: Admission::Active,
        };
        store
            .append_record(
                LIBRARY_SCOPE,
                "instance",
                &serde_json::to_string(&inst_rec).unwrap(),
            )
            .unwrap();
        let agent = AgentRecord {
            id: DEFAULT_AGENT.into(),
            op: RecordOp::Upsert,
            name: "assistant".into(),
            instance_id: DEFAULT_INSTANCE.into(),
            config: "{}".into(),
            current_version: 1,
            package_versions: Default::default(),
            auto_upgrade: false,
            forked_from: None,
        };
        store
            .append_record(
                LIBRARY_SCOPE,
                "agent",
                &serde_json::to_string(&agent).unwrap(),
            )
            .unwrap();
        // No activate_instance — this is exactly the bug we heal.
        assert!(
            !store
                .fold::<InstanceState>(DEFAULT_INSTANCE)
                .map(|s| s.runnable)
                .unwrap_or(false),
            "precondition: legacy default instance is not runnable"
        );
    }

    // Opening the workbench heals it.
    let wb = open_workbench(root).unwrap();
    {
        let w = wb.lock_unpoisoned();
        let st = w
            .store
            .fold::<InstanceState>(DEFAULT_INSTANCE)
            .expect("instance state");
        assert!(st.runnable, "default instance healed to runnable on open");
        assert_eq!(
            st.pinned_version.as_deref(),
            Some("v0"),
            "healed via activate (pin)"
        );
    }

    // And "new chat" now succeeds — the 500 the user hit is gone.
    let app = open_control_plane(wb);
    let (s, body) = send(
        &app,
        "POST",
        &format!("/archetypes/{DEFAULT_AGENT}/chats"),
        Some(r#"{"title":"after heal"}"#),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CREATED,
        "chat creation works after heal: {body}"
    );
}

/// The heal is narrow: a *deliberately* suspended default instance (pinned, then
/// suspended) must stay suspended across a reopen — never silently auto-resumed.
#[tokio::test]
async fn reopening_a_suspended_default_instance_is_not_auto_resumed() {
    let dir = tempfile::tempdir().unwrap();
    {
        let wb = open_workbench(dir.path()).unwrap(); // seeds + activates
        let mut w = wb.lock_unpoisoned();
        w.store_mut()
            .admit::<InstanceState>(DEFAULT_INSTANCE, InstanceCommand::Suspend)
            .unwrap();
        assert!(
            !w.store_ref()
                .fold::<InstanceState>(DEFAULT_INSTANCE)
                .unwrap()
                .runnable,
            "suspended"
        );
    }
    // Reopen: pinned_version is Some, so the heal skips it — the suspend stands.
    let wb = open_workbench(dir.path()).unwrap();
    let w = wb.lock_unpoisoned();
    let st = w
        .store_ref()
        .fold::<InstanceState>(DEFAULT_INSTANCE)
        .unwrap();
    assert!(
        !st.runnable,
        "a deliberately suspended instance is not auto-resumed on reopen"
    );
    assert_eq!(
        st.phase,
        gaugewright_core::instance::InstancePhase::Suspended
    );
}

/// Archetype fork (ADR 0035/0038): copies the source's config + method into a
/// fresh, independent archetype that is itself usable.
#[tokio::test]
async fn forking_an_archetype_copies_config_and_is_independent() {
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);
    // a distinctive config on the source so we can prove the copy
    let (s, _) = send(
        &app,
        "PUT",
        &format!("/archetypes/{DEFAULT_AGENT}"),
        Some(r#"{"config":"{\"model\":\"src-model\"}"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    // fork it
    let (s, body) = send(
        &app,
        "POST",
        &format!("/archetypes/{DEFAULT_AGENT}/fork"),
        Some(r#"{"name":"forked"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "fork: {body}");
    let fork_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    // the fork carries the copied config
    let (_, fb) = send(&app, "GET", &format!("/archetypes/{fork_id}"), None).await;
    assert!(fb.contains("src-model"), "fork copied the config: {fb}");
    // independence: reconfigure the fork, the source is untouched
    let _ = send(
        &app,
        "PUT",
        &format!("/archetypes/{fork_id}"),
        Some(r#"{"config":"{\"model\":\"fork-model\"}"}"#),
    )
    .await;
    let (_, srcb) = send(&app, "GET", &format!("/archetypes/{DEFAULT_AGENT}"), None).await;
    assert!(
        srcb.contains("src-model") && !srcb.contains("fork-model"),
        "source unchanged: {srcb}"
    );
    // the fork is a real, runnable archetype: it can host an edit chat
    let (s, _) = send(
        &app,
        "POST",
        &format!("/archetypes/{fork_id}/chats"),
        Some(r#"{"title":"edit the fork"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
}

/// Chat fork (ADR 0038): clone a chat into a linked new chat that inherits the
/// parent's worktree files. Runtime thread continuity is covered by the
/// WhippleScript adapter and the @live real-model fork scenario.
#[tokio::test]
async fn forking_a_chat_links_it_and_inherits_the_parent_worktree() {
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);
    // a work chat (back-compat path roots it on the default placement)
    let (s, _) = send(&app, "POST", "/chats", Some(r#"{"id":"fork-src"}"#)).await;
    assert_eq!(s, StatusCode::CREATED);
    // a distinctive file in the parent's worktree
    let (s, _) = send(
        &app,
        "PUT",
        "/chats/fork-src/file?path=note.txt",
        Some("parent work"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    // fork it
    let (s, body) = send(&app, "POST", "/chats/fork-src/fork", None).await;
    assert_eq!(s, StatusCode::CREATED, "fork: {body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let fork_id = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["forked_from"], "fork-src", "the fork records its parent");
    // the fork's worktree inherited the parent's file
    let (s, fb) = send(
        &app,
        "GET",
        &format!("/chats/{fork_id}/file?path=note.txt"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        fb.contains("parent work"),
        "fork inherited the parent's worktree: {fb}"
    );
    // and the projection surfaces the fork lineage
    let (_, ws) = send(&app, "GET", "/workspace", None).await;
    assert!(
        ws.contains("\"forked_from\":\"fork-src\""),
        "projection shows forked_from: {ws}"
    );
}

#[tokio::test]
async fn stop_is_a_no_op_when_nothing_is_running() {
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);
    let (_, body) = send(
        &app,
        "POST",
        "/archetypes/agent-default/chats",
        Some(r#"{"title":"s"}"#),
    )
    .await;
    let chat = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    // no turn running → stop is a clean no-op.
    let (s, body) = send(&app, "POST", &format!("/chats/{chat}/stop"), None).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    assert!(body.contains("\"stopped\":false"), "got {body}");
}

#[test]
fn running_turn_registry_round_trips() {
    // the out-of-band registry the Stop route reads (no workbench lock): the
    // registered interrupt handle comes back invokable, and clear removes it.
    let fired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = fired.clone();
    engine::register_running_turn(
        "eng-x",
        std::sync::Arc::new(move || flag.store(true, std::sync::atomic::Ordering::SeqCst)),
    );
    let interrupt = engine::running_turn_interrupt("eng-x").expect("handle registered");
    interrupt();
    assert!(fired.load(std::sync::atomic::Ordering::SeqCst));
    engine::clear_running_turn("eng-x");
    assert!(engine::running_turn_interrupt("eng-x").is_none());
}

#[tokio::test]
async fn workstream_sync_route_is_clean_with_nothing_to_pull() {
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);
    let (_, body) = send(
        &app,
        "POST",
        "/archetypes/agent-default/chats",
        Some(r#"{"title":"w"}"#),
    )
    .await;
    let chat = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    // syncing with nothing promoted to main is a clean no-op (WC-1 route).
    let (s, body) = send(&app, "POST", &format!("/chats/{chat}/sync"), None).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    assert!(body.contains("\"conflict\":false"), "got {body}");
}

#[tokio::test]
async fn instance_lifecycle_suspend_blocks_new_chats_then_resume_allows() {
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);

    // the seeded instance is active (pinned) and runnable.
    let (s, body) = send(&app, "GET", "/placements/inst-default", None).await;
    assert_eq!(s, StatusCode::OK, "got {body}");
    assert!(
        body.contains("\"runnable\":true") && body.contains("\"phase\":\"active\""),
        "got {body}"
    );

    // suspend → a new chat is rejected (SUSPEND_BLOCKS_RUN)…
    let (s, _) = send(
        &app,
        "POST",
        "/placements/inst-default/command",
        Some(r#""Suspend""#),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = send(
        &app,
        "POST",
        "/archetypes/agent-default/chats",
        Some(r#"{"title":"nope"}"#),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::INTERNAL_SERVER_ERROR,
        "suspended instance rejects: {body}"
    );

    // …resume → chats work again.
    send(
        &app,
        "POST",
        "/placements/inst-default/command",
        Some(r#""Resume""#),
    )
    .await;
    let (s, _) = send(
        &app,
        "POST",
        "/archetypes/agent-default/chats",
        Some(r#"{"title":"ok"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // double-pin is rejected (PIN_IMMUTABLE).
    let (s, body) = send(
        &app,
        "POST",
        "/placements/inst-default/command",
        Some(r#"{"PinVersion":"v9"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "re-pin rejected: {body}");
}

#[tokio::test]
async fn project_credential_override_pins_seals_and_lists() {
    // LLM-2 (ADR 0062): the per-project credential surface pins a sealed BYOK token
    // in the project scope, lists provider+linked only (never the token), and unpins.
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);

    let (_, body) = send(&app, "POST", "/projects", Some(r#"{"name":"client-site"}"#)).await;
    let pid = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // No pins yet.
    let (s, body) = send(&app, "GET", &format!("/projects/{pid}/credentials"), None).await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["credentials"].as_array().unwrap().len(),
        0,
        "starts empty: {body}"
    );

    // Pin a provider for the project.
    let (s, _) = send(
        &app,
        "POST",
        &format!("/projects/{pid}/credentials"),
        Some(r#"{"provider":"openai","token":"proj-secret"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // It lists the provider but NEVER the token (sealed at rest, INV-10).
    let (_, body) = send(&app, "GET", &format!("/projects/{pid}/credentials"), None).await;
    assert!(
        !body.contains("proj-secret"),
        "sealed token must never be returned: {body}"
    );
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["credentials"][0]["provider"], "openai", "got {body}");

    // Unpin → empty again (fall back to the account default).
    let (s, _) = send(
        &app,
        "DELETE",
        &format!("/projects/{pid}/credentials/openai"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, body) = send(&app, "GET", &format!("/projects/{pid}/credentials"), None).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["credentials"].as_array().unwrap().len(),
        0,
        "unpinned: {body}"
    );
}

#[tokio::test]
async fn project_binds_an_agent_and_hosts_a_chat() {
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);

    let (_, body) = send(&app, "POST", "/projects", Some(r#"{"name":"client-site"}"#)).await;
    let pid = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // bind the default agent into the project → a using instance.
    let (s, body) = send(
        &app,
        "POST",
        &format!("/projects/{pid}/placements"),
        Some(r#"{"agent_id":"agent-default"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "got {body}");
    let iid = serde_json::from_str::<serde_json::Value>(&body).unwrap()["instance_id"]
        .as_str()
        .unwrap()
        .to_string();

    // chat under the binding.
    let (s, _) = send(
        &app,
        "POST",
        &format!("/projects/{pid}/placements/{iid}/chats"),
        Some(r#"{"title":"triage"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, body) = send(&app, "GET", "/workspace", None).await;
    assert!(
        body.contains("client-site") && body.contains("triage"),
        "got {body}"
    );

    // deleting the project cascades the using instance + its chats.
    let (s, _) = send(&app, "DELETE", &format!("/projects/{pid}"), None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, body) = send(&app, "GET", "/workspace", None).await;
    assert!(
        !body.contains("client-site") && !body.contains("triage"),
        "got {body}"
    );
}

#[tokio::test]
async fn delete_agent_refuses_default_and_survives_restart_rehydration() {
    let (dir, wb) = seeded_workbench();
    let app = open_control_plane(wb);

    // the seed default agent can't be deleted.
    let (s, body) = send(&app, "DELETE", "/archetypes/agent-default", None).await;
    assert_eq!(s, StatusCode::CONFLICT, "got {body}");

    // create an agent + a chat, then reopen the workbench from disk.
    let (_, body) = send(&app, "POST", "/archetypes", Some(r#"{"name":"persisted"}"#)).await;
    let agent_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    send(
        &app,
        "POST",
        &format!("/archetypes/{agent_id}/chats"),
        Some(r#"{"title":"durable"}"#),
    )
    .await;

    let wb2 = open_workbench(dir.path()).unwrap();
    let app2 = open_control_plane(wb2);
    let (_, body) = send(&app2, "GET", "/workspace", None).await;
    assert!(
        body.contains("persisted") && body.contains("durable"),
        "rehydrated: {body}"
    );
}

#[tokio::test]
async fn task_queue_lists_a_pending_review_then_clears_on_keep() {
    let _fake_agent = fake_agent_env();
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);
    send(&app, "POST", "/chats", Some(r#"{"id":"q1"}"#)).await;

    // no work yet → no review task. (A fresh workbench seeds onboarding `issue`
    // tasks, ADR 0075; this lifecycle is about `review` tasks specifically.)
    let (s, body) = send(&app, "GET", "/tasks", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        !body.contains(r#""kind":"review""#),
        "no review yet: {body}"
    );

    // a finished turn leaves a clean diff → a review task queues.
    send(&app, "POST", "/chats/q1/task", Some(r#"{"prompt":"go"}"#)).await;
    let (_, body) = send(&app, "GET", "/tasks", None).await;
    assert!(
        body.contains(r#""id":"q1""#) && body.contains(r#""kind":"review""#),
        "queued: {body}"
    );

    // keeping the work (admit→advance) clears it from the queue.
    send(
        &app,
        "POST",
        "/chats/q1/merge/command",
        Some(r#"{"action":"admit"}"#),
    )
    .await;
    let (_, body) = send(&app, "GET", "/tasks", None).await;
    assert!(
        !body.contains(r#""kind":"review""#),
        "review cleared: {body}"
    );
}

/// ATTN-1 (ADR 0082 §4, the shipped no-op rule): a settled turn that changed no
/// files auto-advances — recorded and explained — instead of queuing "needs
/// review"; a turn that did change a file still holds for the human.
#[tokio::test]
async fn noop_turn_auto_advances_instead_of_queuing_review() {
    let _fake_agent = fake_agent_env();
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);
    send(&app, "POST", "/chats", Some(r#"{"id":"noop1"}"#)).await;

    // A no-op turn (`[no-write]` keeps the fake agent's hands off the worktree):
    // no review task; the merge advanced without a human.
    send(
        &app,
        "POST",
        "/chats/noop1/task",
        Some(r#"{"prompt":"[no-write] just think"}"#),
    )
    .await;
    let (_, body) = send(&app, "GET", "/tasks", None).await;
    assert!(
        !body.contains(r#""kind":"review""#),
        "no review for a no-op turn: {body}"
    );
    let (_, merge) = send(&app, "GET", "/chats/noop1/merge", None).await;
    assert!(merge.contains("Advanced"), "auto-advanced: {merge}");

    // The advance is durable, admitted evidence that explains itself — on
    // the AUDIT record (ADR 0082 §4), not in the user's conversation: a
    // no-op advance is invisible to the user, and rule citations never
    // reach the transcript.
    let (_, audit) = send(&app, "GET", "/chats/noop1/audit", None).await;
    assert!(
        audit.contains("no-op rule"),
        "the audit trail cites the rule: {audit}"
    );
    let (_, transcript) = send(&app, "GET", "/chats/noop1/transcript", None).await;
    assert!(
        !transcript.contains("advanced automatically") && !transcript.contains("ADR 0082"),
        "internal advancement rationale never reaches the user transcript: {transcript}"
    );

    // The rule is narrow: the next turn writes a file → review queues as before.
    send(
        &app,
        "POST",
        "/chats/noop1/task",
        Some(r#"{"prompt":"now write"}"#),
    )
    .await;
    let (_, body) = send(&app, "GET", "/tasks", None).await;
    assert!(
        body.contains(r#""id":"noop1""#) && body.contains(r#""kind":"review""#),
        "a real change still queues: {body}"
    );
}

/// ADR 0082 §2: each chat task is typed by its **ask** — a conflicted merge
/// queues `repair` (not `review`), and a turn suspended on a human question
/// queues `answer` (outranking merge state).
#[tokio::test]
async fn task_queue_types_asks_repair_and_answer() {
    let _fake_agent = fake_agent_env();
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);

    // Two chats off the same base: both fake turns create `agent-note.txt`, so
    // keeping the first makes the second's addition an add/add conflict.
    send(&app, "POST", "/chats", Some(r#"{"id":"ra"}"#)).await;
    send(&app, "POST", "/chats", Some(r#"{"id":"rb"}"#)).await;
    send(
        &app,
        "POST",
        "/chats/ra/task",
        Some(r#"{"prompt":"alpha"}"#),
    )
    .await;
    send(
        &app,
        "POST",
        "/chats/ra/merge/command",
        Some(r#"{"action":"admit"}"#),
    )
    .await;
    send(&app, "POST", "/chats/rb/task", Some(r#"{"prompt":"beta"}"#)).await;
    let (_, body) = send(&app, "GET", "/tasks", None).await;
    assert!(
        body.contains(r#""id":"rb""#) && body.contains(r#""kind":"repair""#),
        "conflicted chat queues repair: {body}"
    );
    assert!(
        !body.contains(r#""kind":"review""#),
        "the conflict is not mislabeled review: {body}"
    );

    // Suspend rb's run on a human question: `answer` outranks its merge state.
    for cmd in [
        "\"RetryRun\"",
        "\"AdmitRun\"",
        "\"StartRun\"",
        "\"AwaitHuman\"",
    ] {
        send(&app, "POST", "/scopes/rb/run/command", Some(cmd)).await;
    }
    let (_, body) = send(&app, "GET", "/tasks", None).await;
    assert!(
        body.contains(r#""kind":"answer""#),
        "suspended turn queues answer: {body}"
    );
    assert!(
        !body.contains(r#""kind":"repair""#),
        "answer outranks repair for the same chat: {body}"
    );
}

/// ATTN-2 (ADR 0082 §3): the operator's attention rules re-shape the queue —
/// muting `changes` drops the review task *and* its nav badge, while opting
/// `turn-settled` into the queue raises the `reply` ask the defaults never show
/// (the muted signal falls through; it does not silence the chat).
#[tokio::test]
async fn attention_rules_reshape_queue_and_badges() {
    let _fake_agent = fake_agent_env();
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);
    send(&app, "POST", "/chats", Some(r#"{"id":"at1"}"#)).await;
    send(&app, "POST", "/chats/at1/task", Some(r#"{"prompt":"go"}"#)).await;

    // Defaults: the clean merge queues `review`; no `reply` pill.
    let (_, body) = send(&app, "GET", "/tasks", None).await;
    assert!(
        body.contains(r#""kind":"review""#) && !body.contains(r#""kind":"reply""#),
        "defaults hold: {body}"
    );

    // Mute `changes`, opt `turn-settled` into the queue.
    let rules = serde_json::json!({
        "value": r#"{"version":1,"rules":[{"signal":"changes","attention":"mute"},{"signal":"turn-settled","attention":"queue"}]}"#
    })
    .to_string();
    let (s, _) = send(
        &app,
        "PUT",
        "/account/settings/attention.rules",
        Some(&rules),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (_, body) = send(&app, "GET", "/tasks", None).await;
    assert!(!body.contains(r#""kind":"review""#), "review muted: {body}");
    assert!(
        body.contains(r#""id":"at1""#) && body.contains(r#""kind":"reply""#),
        "reply queued via fall-through: {body}"
    );

    // The muted signal's nav badge goes with it (badge surface, same rules).
    let (_, ws) = send(&app, "GET", "/workspace", None).await;
    let v: serde_json::Value = serde_json::from_str(&ws).unwrap();
    let chat = v["recent"]
        .as_array()
        .and_then(|chats| chats.iter().find(|c| c["id"] == "at1"))
        .expect("at1 in recent")
        .clone();
    assert_eq!(
        chat["changes"], false,
        "muted changes shows no badge: {chat}"
    );
}

/// ATTN-3 (ADR 0082 §4): an operator advancement rule auto-advances a covered
/// turn — recorded with a citation — while an uncovered scope holds for review
/// (and with no rules configured everything holds, per the other tests).
#[tokio::test]
async fn advancement_rules_auto_advance_covered_turns_only() {
    let _fake_agent = fake_agent_env();
    let (_d, wb) = seeded_workbench();
    let app = open_control_plane(wb);

    // A rule covering the fake agent's write (`agent-note.txt` at the root).
    let rules = serde_json::json!({
        "value": r#"{"version":1,"rules":[{"advance":"writes-within","paths":["*.txt"]}]}"#
    })
    .to_string();
    send(
        &app,
        "PUT",
        "/account/settings/advancement.rules",
        Some(&rules),
    )
    .await;

    send(&app, "POST", "/chats", Some(r#"{"id":"adv1"}"#)).await;
    send(&app, "POST", "/chats/adv1/task", Some(r#"{"prompt":"go"}"#)).await;
    let (_, merge) = send(&app, "GET", "/chats/adv1/merge", None).await;
    assert!(
        merge.contains("Advanced"),
        "covered turn auto-advanced: {merge}"
    );
    let (_, body) = send(&app, "GET", "/tasks", None).await;
    assert!(
        !body.contains(r#""kind":"review""#),
        "no review queued: {body}"
    );
    // The rationale is AUDIT-plane evidence (ADR 0082 posture): the
    // conversation says only that the merge happened; the rule citation
    // lives on the audit trail, never in the transcript.
    let (_, audit) = send(&app, "GET", "/chats/adv1/audit", None).await;
    assert!(
        audit.contains("writes-within(*.txt)"),
        "the audit trail cites the rule: {audit}"
    );
    let (_, transcript) = send(&app, "GET", "/chats/adv1/transcript", None).await;
    assert!(
        !transcript.contains("writes-within"),
        "policy rationale never leaks into the conversation: {transcript}"
    );

    // Narrow the scope so the same write is no longer covered → holds.
    let rules = serde_json::json!({
        "value": r#"{"version":1,"rules":[{"advance":"writes-within","paths":["docs/**"]}]}"#
    })
    .to_string();
    send(
        &app,
        "PUT",
        "/account/settings/advancement.rules",
        Some(&rules),
    )
    .await;
    send(&app, "POST", "/chats", Some(r#"{"id":"adv2"}"#)).await;
    send(&app, "POST", "/chats/adv2/task", Some(r#"{"prompt":"go"}"#)).await;
    let (_, merge) = send(&app, "GET", "/chats/adv2/merge", None).await;
    assert!(
        merge.contains("Clean") && !merge.contains("Advanced"),
        "uncovered turn holds for review: {merge}"
    );
    let (_, body) = send(&app, "GET", "/tasks", None).await;
    assert!(
        body.contains(r#""id":"adv2""#) && body.contains(r#""kind":"review""#),
        "held turn queues review: {body}"
    );
}

#[tokio::test]
async fn admitted_run_events_reach_the_live_stream() {
    let (_d, wb) = workbench();
    // Subscribe before driving commands (as an SSE client would).
    let mut rx = wb.lock_unpoisoned().sender("eng-stream").subscribe();
    let app = open_control_plane(wb);

    for cmd in ["\"RequestRun\"", "\"AdmitRun\"", "\"StartRun\""] {
        let (s, _) = send(&app, "POST", "/scopes/eng-stream/run/command", Some(cmd)).await;
        assert_eq!(s, StatusCode::OK);
    }

    // Each admitted command published an `admitted` event in order.
    let mut phases = Vec::new();
    for _ in 0..3 {
        match rx.recv().await.unwrap() {
            ServerEvent::Admitted { text, .. } => phases.push(text),
            other => panic!("expected admitted, got {other:?}"),
        }
    }
    assert!(phases[0].contains("Requested"));
    assert!(phases[1].contains("Admitted"));
    assert!(phases[2].contains("Running"));
}

/// The onboarding checklist (ADR 0075 Phase 2/3) is seeded on a fresh workbench,
/// surfaces as `issue` tasks in the unified `/tasks` projection, and advances
/// when the matching app event fires — here, connecting an LLM credential closes
/// the "credential" step end-to-end through the HTTP surface.
#[tokio::test]
async fn onboarding_checklist_appears_and_advances_on_credential() {
    // Onboarding is gated off under the fake agent; pin the real runtime (and
    // serialize against fake-agent tests) so the checklist actually seeds.
    let _real = crate::test_support::real_agent_env();
    let dir = tempfile::tempdir().unwrap();
    let wb = crate::workbench_state::build_workbench(dir.path()).unwrap();
    let app = open_control_plane(Arc::new(Mutex::new(wb)));

    // The seeded onboarding steps show up as `issue` tasks, each with an assignee.
    let (status, body) = send(&app, "GET", "/tasks", None).await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let tasks = v["tasks"].as_array().unwrap();
    let issue_titles = |v: &serde_json::Value| -> Vec<String> {
        v["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["kind"] == "issue")
            .filter_map(|t| t["title"].as_str().map(str::to_owned))
            .collect()
    };
    let titles = issue_titles(&v);
    assert!(
        titles.iter().any(|t| t == "Connect a model"),
        "expected the credential onboarding step, got {titles:?}"
    );
    assert!(titles.iter().any(|t| t == "Create a project"));
    assert!(
        tasks.iter().all(|t| t["assignee"].is_string()),
        "every task carries an assignee authority (ADR 0075 §4)"
    );

    // Connecting a credential fires app.credential_connected, which closes the step.
    let (status, _) = send(
        &app,
        "POST",
        "/account/credentials",
        Some(r#"{"provider":"anthropic","token":"sk-test-xyz"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(&app, "GET", "/tasks", None).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let titles = issue_titles(&v);
    assert!(
        !titles.iter().any(|t| t == "Connect a model"),
        "the credential step should be closed after linking, got {titles:?}"
    );
    assert!(
        titles.iter().any(|t| t == "Create a project"),
        "unrelated onboarding steps stay open"
    );
}

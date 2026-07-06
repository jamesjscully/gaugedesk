//! Organization admin surface, end to end (`ORG-1`, B10/B11). Drives the mounted
//! `control_plane`: set org settings, invite members, change a role, deactivate, and
//! assert the directory reads back — plus the structural guard that an org always
//! keeps a break-glass owner.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use gaugewright_app::Workbench;
use gaugewright_ee::org_routes::enterprise_control_plane;
use gaugewright_store::Store;
use gaugewright_workspace::Instance;

fn workbench() -> (tempfile::TempDir, Router) {
    let dir = tempfile::tempdir().unwrap();
    let instance = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
    let store = Store::open_in_memory().unwrap();
    let wb = Workbench::with_instance("inst-test", instance, store);
    (dir, enterprise_control_plane(Arc::new(Mutex::new(wb))))
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
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

/// Like `send`, but stamps the `X-Gaugewright-Tenant` header (DEPLOY-6) so the request
/// resolves to a named tenant's scope.
async fn send_tenant(
    app: &Router,
    method: &str,
    uri: &str,
    tenant: &str,
    body: Option<&str>,
) -> (StatusCode, Value) {
    let b = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-gaugewright-tenant", tenant);
    let req = match body {
        Some(j) => b
            .header("content-type", "application/json")
            .body(Body::from(j.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
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
async fn multi_tenant_admin_surfaces_are_scope_isolated() {
    // DEPLOY-6 end-to-end: two named tenants configure the same admin surfaces over the
    // same control plane and never see each other; the default (header-less) tenant is a
    // third, independent org.
    let (_dir, app) = workbench();
    assert_eq!(
        send_tenant(
            &app,
            "POST",
            "/admin/org",
            "acme",
            Some(r#"{"display_name":"Acme"}"#)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send_tenant(
            &app,
            "POST",
            "/admin/org",
            "globex",
            Some(r#"{"display_name":"Globex"}"#)
        )
        .await
        .0,
        StatusCode::OK
    );
    // a member added under acme must not appear for globex (the isolation contract).
    send_tenant(
        &app,
        "POST",
        "/admin/members",
        "acme",
        Some(r#"{"authority":"u1","email":"u1@acme.com","role":"member"}"#),
    )
    .await;

    let (_, a) = send_tenant(&app, "GET", "/admin/org", "acme", None).await;
    assert_eq!(a["org"]["display_name"], "Acme");
    let (_, g) = send_tenant(&app, "GET", "/admin/org", "globex", None).await;
    assert_eq!(g["org"]["display_name"], "Globex"); // not Acme — scope-isolated
    let (_, am) = send_tenant(&app, "GET", "/admin/members", "acme", None).await;
    let (_, gm) = send_tenant(&app, "GET", "/admin/members", "globex", None).await;
    assert!(am["members"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["authority"] == "u1"));
    assert!(
        gm["members"].as_array().unwrap().is_empty(),
        "globex has no acme member: {gm}"
    );
    // the default tenant (no header) is independent — untouched by either named tenant.
    let (_, d) = send(&app, "GET", "/admin/org", None).await;
    assert!(d["org"].is_null(), "default tenant unaffected: {d}");
}

#[tokio::test]
async fn org_settings_round_trip() {
    let (_dir, app) = workbench();
    let (status, _) = send(
        &app,
        "POST",
        "/admin/org",
        Some(r#"{"display_name":"Acme","verified_domains":["acme.com"],"default_region":"eu"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(&app, "GET", "/admin/org", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["org"]["display_name"], "Acme");
    assert_eq!(body["org"]["verified_domains"][0], "acme.com");
    assert_eq!(body["org"]["default_region"], "eu");
}

#[tokio::test]
async fn invite_list_and_change_role() {
    let (_dir, app) = workbench();

    // Seed an owner so role changes below have a standing owner to protect.
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","email":"o@acme.com","role":"owner","status":"active"}"#),
    )
    .await;

    let (status, body) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"alice-auth","email":"alice@acme.com","role":"member"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["member"]["status"], "invited"); // default
    assert_eq!(body["member"]["id"], "alice-auth"); // defaults to authority

    let (status, body) = send(&app, "GET", "/admin/members", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["members"].as_array().unwrap().len(), 2);

    // Promote alice to admin.
    let (status, body) = send(
        &app,
        "POST",
        "/admin/members/alice-auth/role",
        Some(r#"{"role":"admin"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["member"]["role"], "admin");
}

#[tokio::test]
async fn unknown_role_is_rejected() {
    let (_dir, app) = workbench();
    let (status, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"x","role":"superuser"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn deactivate_marks_deprovisioned() {
    let (_dir, app) = workbench();
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
    )
    .await;
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"bob-auth","role":"member","status":"active"}"#),
    )
    .await;

    let (status, body) = send(&app, "POST", "/admin/members/bob-auth/deactivate", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["member"]["status"], "deprovisioned");
}

#[tokio::test]
async fn cannot_deactivate_or_demote_the_last_owner() {
    let (_dir, app) = workbench();
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
    )
    .await;

    // Demote the only owner → refused.
    let (status, _) = send(
        &app,
        "POST",
        "/admin/members/owner-auth/role",
        Some(r#"{"role":"admin"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Deactivate the only owner → refused.
    let (status, _) = send(&app, "POST", "/admin/members/owner-auth/deactivate", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn billing_round_trips_and_is_not_authority() {
    let (_dir, app) = workbench();
    // An active member exists.
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"alice","role":"member","status":"active"}"#),
    )
    .await;

    // Set a plan with seats; seats_used reflects active membership.
    let (status, body) = send(
        &app,
        "POST",
        "/admin/billing",
        Some(r#"{"plan":"business","seats":10}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["billing"]["seats"], 10);

    let (status, body) = send(&app, "GET", "/admin/billing", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["seats_used"], 1);

    // BILL-3: lapse the plan to zero seats — it confers/revokes no authority.
    send(
        &app,
        "POST",
        "/admin/billing",
        Some(r#"{"plan":"free","seats":0}"#),
    )
    .await;
    // The active member's role/status is untouched by billing.
    let (_s, body) = send(&app, "GET", "/admin/members", None).await;
    let alice = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["authority"] == "alice")
        .unwrap()
        .clone();
    assert_eq!(alice["status"], "active");
    assert_eq!(alice["role"], "member");
}

#[tokio::test]
async fn security_policy_round_trips() {
    let (_dir, app) = workbench();
    let (status, body) = send(
        &app,
        "POST",
        "/admin/security",
        Some(r#"{"require_mfa":true,"session_lifetime_secs":3600,"idle_timeout_secs":900,"residency_region":"eu"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["security"]["require_mfa"], true);

    let (status, body) = send(&app, "GET", "/admin/security", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["security"]["session_lifetime_secs"], 3600);
    assert_eq!(body["security"]["residency_region"], "eu");
}

#[tokio::test]
async fn audit_retention_min_guarantee_defaults_to_a_year_and_is_configurable() {
    // AUD-3: the minimum-retention guarantee is published on the audit timeline. The log is
    // append-only/forever (INV-6); this is the contractual floor, default one year.
    let (_dir, app) = workbench();
    let (status, body) = send(&app, "GET", "/admin/audit", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["retention_min_days"], 365,
        "default guarantee is one year"
    );

    // A buyer can configure a longer guarantee; it round-trips and the timeline publishes it.
    let (status, _) = send(
        &app,
        "POST",
        "/admin/security",
        Some(r#"{"audit_retention_min_days":2555}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(&app, "GET", "/admin/audit", None).await;
    assert_eq!(body["retention_min_days"], 2555);
    let (_, sec) = send(&app, "GET", "/admin/security", None).await;
    assert_eq!(sec["security"]["audit_retention_min_days"], 2555);
}

#[tokio::test]
async fn org_kind_defaults_to_client_and_accepts_consultant() {
    let (_dir, app) = workbench();
    // Default kind is client (the existing single-org path is unchanged).
    let (status, body) = send(
        &app,
        "POST",
        "/admin/org",
        Some(r#"{"display_name":"Acme"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["org"]["kind"], "client");

    // A consultant org is the same primitive, different party (DEPLOY-6, ADR 0061).
    let (status, body) = send(
        &app,
        "POST",
        "/admin/org",
        Some(r#"{"display_name":"Expert LLC","kind":"consultant"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["org"]["kind"], "consultant");
}

#[tokio::test]
async fn placement_policy_round_trips() {
    let (_dir, app) = workbench();
    // Default (no record): the open policy — admits everything.
    let (status, body) = send(&app, "GET", "/admin/placement-policy", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["placement_policy"]["require_attested"], false);

    // Tighten: require attested, restrict to counterparty-hosted.
    let (status, body) = send(
        &app,
        "POST",
        "/admin/placement-policy",
        Some(r#"{"require_attested":true,"allowed_operators":["counterparty"]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["placement_policy"]["require_attested"], true);

    let (status, body) = send(&app, "GET", "/admin/placement-policy", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["placement_policy"]["require_attested"], true);
    assert_eq!(
        body["placement_policy"]["allowed_operators"][0],
        "counterparty"
    );
}

#[tokio::test]
async fn sso_connection_round_trips() {
    let (_dir, app) = workbench();
    let (status, body) = send(
        &app,
        "POST",
        "/admin/sso",
        Some(r#"{"protocol":"oidc","issuer":"https://idp.example.com","audiences":["client-1"],"enforce_sso":true}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sso"]["protocol"], "oidc");
    assert_eq!(body["sso"]["enforce_sso"], true);

    let (status, body) = send(&app, "GET", "/admin/sso", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sso"]["issuer"], "https://idp.example.com");
    assert_eq!(body["sso"]["audiences"][0], "client-1");
}

#[tokio::test]
async fn domain_capture_auto_join() {
    let (_dir, app) = workbench();
    send(
        &app,
        "POST",
        "/admin/org",
        Some(r#"{"display_name":"Acme","verified_domains":["acme.com"]}"#),
    )
    .await;

    // A verified-domain email auto-joins as an active member.
    let (status, body) = send(
        &app,
        "POST",
        "/admin/members/auto-join",
        Some(r#"{"authority":"alice-auth","email":"alice@acme.com"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["member"]["role"], "member");
    assert_eq!(body["member"]["status"], "active");

    // An unverified domain is refused.
    let (status, _) = send(
        &app,
        "POST",
        "/admin/members/auto-join",
        Some(r#"{"authority":"eve","email":"eve@evil.com"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn audit_timeline_records_governance_actions() {
    let (_dir, app) = workbench();
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
    )
    .await;
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"alice","role":"member"}"#),
    )
    .await;
    send(
        &app,
        "POST",
        "/admin/members/alice/role",
        Some(r#"{"role":"admin"}"#),
    )
    .await;

    // Full timeline: two invites + one role change.
    let (status, body) = send(&app, "GET", "/admin/audit", None).await;
    assert_eq!(status, StatusCode::OK);
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries
        .iter()
        .any(|e| e["action"] == "member.role" && e["target"] == "alice"));

    // Filter by action.
    let (status, body) = send(&app, "GET", "/admin/audit?action=member.role", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["entries"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn role_change_on_missing_member_is_404() {
    let (_dir, app) = workbench();
    let (status, _) = send(
        &app,
        "POST",
        "/admin/members/ghost/role",
        Some(r#"{"role":"admin"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn configuring_oidc_sso_with_an_unreachable_issuer_surfaces_an_error_and_does_not_lock_out() {
    // Enterprise-mode activation (`ID-3`): POST /admin/sso (re)builds wb.idp from the
    // connection. A bogus/unreachable issuer must NOT clobber the existing verifier
    // (here: none) — a bad runtime edit can't lock admins out — and the activation
    // error is surfaced so the operator sees it.
    let (_dir, app) = workbench();
    let (status, body) = send(
        &app,
        "POST",
        "/admin/sso",
        // Port 9 (discard) → connection refused → discovery fails fast.
        Some(r#"{"protocol":"oidc","issuer":"http://127.0.0.1:9/realms/x","audiences":["client-1"]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the connection is still saved");
    assert_eq!(
        body["oidc_active"],
        serde_json::json!(false),
        "discovery failed → not activated"
    );
    assert!(
        body["activation_error"].is_string(),
        "the operator sees why activation failed: {body:?}"
    );

    // The verifier was left untouched (none) → admin stays ungated, not bricked: a
    // read without any bearer still succeeds.
    let (status, _) = send(&app, "GET", "/admin/org", None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "admin is not locked out by a bad SSO edit"
    );
}

#[tokio::test]
async fn integration_details_expose_the_sp_side_values() {
    // ONB-1: the admin reads our SP/SCIM values to paste into their IdP.
    let (_dir, app) = workbench();
    let (status, body) = send(&app, "GET", "/admin/integration", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["oidc"]["redirect_uri"]
        .as_str()
        .unwrap()
        .ends_with("/auth/callback"));
    assert!(body["saml"]["metadata_url"]
        .as_str()
        .unwrap()
        .ends_with("/saml/metadata"));
    assert!(body["scim"]["base_url"]
        .as_str()
        .unwrap()
        .ends_with("/scim/v2"));
}

#[tokio::test]
async fn sso_test_connection_reports_unreachable_issuer() {
    // ONB-3: a real OIDC discovery probe; an unreachable issuer → ok:false (operational,
    // never stored). Port 9 (discard) → connection refused fast.
    let (_dir, app) = workbench();
    let (status, body) = send(
        &app,
        "POST",
        "/admin/sso/test",
        Some(r#"{"protocol":"oidc","issuer":"http://127.0.0.1:9/realms/x","audiences":["client-1"]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], serde_json::json!(false));
    assert!(body["detail"].is_string());
}

#[tokio::test]
async fn domain_verify_token_returns_the_txt_record() {
    // ONB-5: the deterministic TXT record an admin publishes (domain lowercased).
    let (_dir, app) = workbench();
    let (status, body) = send(
        &app,
        "POST",
        "/admin/domains/verify-token",
        Some(r#"{"domain":"Acme.com"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["record_name"],
        serde_json::json!("_gaugewright-challenge.acme.com")
    );
    assert!(body["value"]
        .as_str()
        .unwrap()
        .starts_with("gaugewright-domain-verification="));
}

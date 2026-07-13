//! Live-route RBAC enforcement (M3 `RBAC-5`). With an `IdentityProvider` wired
//! (enterprise mode) and the org directory provisioned, the `/admin/*` routes are
//! gated by the actor's directory role: an `owner`/`admin` token is admitted, a
//! `member` token is forbidden, an unauthenticated/garbage token is unauthorized.
//! Single-user local mode (no IdP) stays ungated — covered by `org_admin.rs`.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use gaugewright_app::identity::LoopbackIdentityProvider;
use gaugewright_app::library::{
    Admission, ChatRecord, InstanceKind, InstanceRecord, ProjectRecord, RecordOp, LIBRARY_SCOPE,
};
use gaugewright_app::Workbench;
use gaugewright_core::abac::AuthorityAttributes;
use gaugewright_core::ids::AuthorityId;
use gaugewright_ee::org_routes::enterprise_control_plane;
use gaugewright_store::Store;
use gaugewright_workspace::Instance;

fn workbench_with_idp() -> (tempfile::TempDir, Router) {
    let dir = tempfile::tempdir().unwrap();
    let instance = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
    let store = Store::open_in_memory().unwrap();
    // The IdP only authenticates a token → authority; the *role* is read from the
    // directory (Org::role_of), so default attributes are fine here.
    let idp = LoopbackIdentityProvider::new()
        .enroll(
            "owner-token",
            AuthorityId::new("owner-auth"),
            AuthorityAttributes::default(),
        )
        .enroll(
            "member-token",
            AuthorityId::new("member-auth"),
            AuthorityAttributes::default(),
        )
        .enroll(
            "viewer-token",
            AuthorityId::new("viewer-auth"),
            AuthorityAttributes::default(),
        )
        .enroll(
            "admin-a-token",
            AuthorityId::new("admin-a"),
            AuthorityAttributes::default(),
        );
    let wb = Workbench::with_instance("inst-test", instance, store)
        .with_identity_provider(Arc::new(idp));
    (dir, enterprise_control_plane(Arc::new(Mutex::new(wb))))
}

/// A workbench (enterprise mode) whose library already holds a project `proj-acme` with a
/// placement `i-acme` and a chat `chat-acme` on it — so the ENTSEC-2 path resolver can map a
/// `/chats/chat-acme/*` or `/projects/proj-acme/*` request to its project. Enrolls an owner plus
/// two plain consultants (`consultant-a`, `consultant-b`) in the IdP.
fn workbench_with_scoped_project() -> (tempfile::TempDir, Router) {
    workbench_with_scoped_project_cfg(false)
}

/// As [`workbench_with_scoped_project`], with `audit_reads` controlling SECAUD-4
/// sensitive-read auditing (off in the default harness).
fn workbench_with_scoped_project_cfg(audit_reads: bool) -> (tempfile::TempDir, Router) {
    let dir = tempfile::tempdir().unwrap();
    let instance = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    // Seed the library: a project, a using-instance bound into it, and a chat on that instance.
    let project = ProjectRecord {
        id: "proj-acme".into(),
        op: RecordOp::Upsert,
        name: "Acme".into(),
        is_default: false,
        network_isolated: false,
        run_purpose: None,
        deployment_mode: None,
    };
    let placement = InstanceRecord {
        id: "i-acme".into(),
        op: RecordOp::Upsert,
        kind: InstanceKind::Using,
        agent_id: "a1".into(),
        project_id: Some("proj-acme".into()),
        version: 1,
        admission: Admission::Active,
    };
    let chat = ChatRecord {
        id: "chat-acme".into(),
        op: RecordOp::Upsert,
        instance_id: "i-acme".into(),
        title: "Acme work".into(),
        created_position: 1,
        forked_from: None,
    };
    store
        .append_record(
            LIBRARY_SCOPE,
            "project",
            &serde_json::to_string(&project).unwrap(),
        )
        .unwrap();
    store
        .append_record(
            LIBRARY_SCOPE,
            "instance",
            &serde_json::to_string(&placement).unwrap(),
        )
        .unwrap();
    store
        .append_record(
            LIBRARY_SCOPE,
            "chat",
            &serde_json::to_string(&chat).unwrap(),
        )
        .unwrap();
    let idp = LoopbackIdentityProvider::new()
        .enroll(
            "owner-token",
            AuthorityId::new("owner-auth"),
            AuthorityAttributes::default(),
        )
        .enroll(
            "a-token",
            AuthorityId::new("consultant-a"),
            AuthorityAttributes::default(),
        )
        .enroll(
            "b-token",
            AuthorityId::new("consultant-b"),
            AuthorityAttributes::default(),
        );
    let mut wb = Workbench::with_instance("inst-test", instance, store)
        .with_identity_provider(Arc::new(idp))
        .with_audit_reads(audit_reads);
    wb.rebuild_library(); // fold the seeded library records into the projection
    (dir, enterprise_control_plane(Arc::new(Mutex::new(wb))))
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<&str>,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let req = match body {
        Some(body) => b
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
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
async fn enterprise_mode_gates_admin_routes_by_role() {
    let (_dir, app) = workbench_with_idp();

    // Bootstrap: an empty directory is seedable without a token (there is nobody to
    // authorize against yet) — seed the first active owner.
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "bootstrap seeding the first owner");

    // Now provisioned. The owner token may add a member.
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"member-auth","role":"member","status":"active"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "owner manages members");

    // A member token lacks ManageMembers → 403.
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"q","role":"member"}"#),
        Some("member-token"),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "member cannot manage members");

    // No credential → 401.
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"q","role":"member"}"#),
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "anonymous is unauthorized once provisioned"
    );

    // Unrecognized credential → 401 (authentication fails).
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"q","role":"member"}"#),
        Some("bogus-token"),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "garbage token is unauthorized");

    // Reads need console access: a member sees no console (403), the owner reads (200).
    let (s, _) = send(&app, "GET", "/admin/members", None, Some("member-token")).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "member has no console");
    let (s, body) = send(&app, "GET", "/admin/members", None, Some("owner-token")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["members"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn team_scoped_admin_cannot_administer_another_team() {
    let (_dir, app) = workbench_with_idp();
    // Bootstrap an owner, then (as owner) an admin scoped to team A and two members.
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
        None,
    )
    .await;
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"admin-a","role":"admin","status":"active","team":"A"}"#),
        Some("owner-token"),
    )
    .await;
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"id":"alice","authority":"alice","role":"member","status":"active","team":"A"}"#),
        Some("owner-token"),
    )
    .await;
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"id":"bob","authority":"bob","role":"member","status":"active","team":"B"}"#),
        Some("owner-token"),
    )
    .await;

    // The team-A admin may change a team-A member's role…
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members/alice/role",
        Some(r#"{"role":"viewer"}"#),
        Some("admin-a-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "admin administers own team");

    // …but not a team-B member's (outside scope → 403).
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members/bob/role",
        Some(r#"{"role":"viewer"}"#),
        Some("admin-a-token"),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "admin cannot cross teams");

    // The owner is org-wide — may administer team B.
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members/bob/role",
        Some(r#"{"role":"viewer"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "owner is org-wide");
}

#[tokio::test]
async fn export_is_gated_by_role_policy() {
    // RBAC-6 / RBAC-5 export half: the org policy's `viewer ⇒ no export` rule is
    // enforced at the live export route.
    let (_dir, app) = workbench_with_idp();
    // bootstrap owner, then add an active viewer.
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
        None,
    )
    .await;
    send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"viewer-auth","role":"viewer","status":"active"}"#),
        Some("owner-token"),
    )
    .await;

    let body = r#"{"ProposeExport":{"source_required":[]}}"#;

    // A viewer is denied export by the policy → 403 (the gate fires before admit).
    let (s, _) = send(
        &app,
        "POST",
        "/scopes/eng-1/export/command",
        Some(body),
        Some("viewer-token"),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "viewer is denied export");

    // The owner passes the export gate (the admit then proceeds; not a 403).
    let (s, _) = send(
        &app,
        "POST",
        "/scopes/eng-1/export/command",
        Some(body),
        Some("owner-token"),
    )
    .await;
    assert_ne!(s, StatusCode::FORBIDDEN, "owner passes the export gate");
}

#[tokio::test]
async fn enterprise_mode_gates_data_routes_for_active_members() {
    // ENTSEC-1 (ADR 0065): in enterprise mode the DATA routes — not just /admin/* — require an
    // authenticated active member; solo mode (no IdP) stays open (covered by the other suites,
    // which never attach an IdP). /health is exempt.
    let (_dir, app) = workbench_with_idp();

    // /health is exempt — open even before provisioning, without a token.
    let (s, _) = send(&app, "GET", "/health", None, None).await;
    assert_eq!(s, StatusCode::OK, "health is always open");

    // Bootstrap-seed the first owner, then add a plain member.
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"member-auth","role":"member","status":"active"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Now provisioned: GET /workspace (a data route) is gated.
    let (s, _) = send(&app, "GET", "/workspace", None, None).await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "anonymous cannot read the workspace once provisioned"
    );

    let (s, _) = send(&app, "GET", "/workspace", None, Some("bogus-token")).await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "a garbage token is unauthorized"
    );

    // An authenticated authority that is NOT an active member → 403 (enrolled in the IdP but
    // never provisioned into the directory).
    let (s, _) = send(&app, "GET", "/workspace", None, Some("viewer-token")).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "an authenticated non-member is forbidden"
    );

    // A plain member (no console capability) CAN read the data routes — unlike /admin/*.
    let (s, _) = send(&app, "GET", "/workspace", None, Some("member-token")).await;
    assert_eq!(s, StatusCode::OK, "an active member reads the workspace");

    // /health stays exempt even when provisioned.
    let (s, _) = send(&app, "GET", "/health", None, None).await;
    assert_eq!(s, StatusCode::OK, "health stays exempt");

    // The /admin capability gate is unchanged: a member still has no console access.
    let (s, _) = send(&app, "GET", "/admin/members", None, Some("member-token")).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "member has no console access (admin gate intact)"
    );
}

#[tokio::test]
async fn entsec2_scopes_data_routes_to_granted_projects() {
    // ENTSEC-2 (ADR 0065): a plain member sees only the projects granted to them; owner/admin
    // bypass; a non-granted member is forbidden the project's data routes, fail-closed.
    let (_dir, app) = workbench_with_scoped_project();

    // Bootstrap an owner, then two plain members (consultants).
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    for who in ["consultant-a", "consultant-b"] {
        let (s, _) = send(
            &app,
            "POST",
            "/admin/members",
            Some(&format!(
                r#"{{"authority":"{who}","role":"member","status":"active"}}"#
            )),
            Some("owner-token"),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
    }

    // Grant consultant-a access to proj-acme (owner administers grants).
    let (s, _) = send(
        &app,
        "POST",
        "/admin/grants",
        Some(r#"{"authority":"consultant-a","project_id":"proj-acme"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "owner grants a member a project");

    // The grant shows up in the admin list.
    let (s, grants) = send(&app, "GET", "/admin/grants", None, Some("owner-token")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(grants["grants"].as_array().unwrap().len(), 1);

    // A non-granted member is forbidden the project's data routes (scope gate, fail-closed).
    let (s, _) = send(
        &app,
        "GET",
        "/projects/proj-acme/home",
        None,
        Some("b-token"),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "ungranted member is out of scope for the project"
    );
    let (s, _) = send(
        &app,
        "GET",
        "/chats/chat-acme/transcript",
        None,
        Some("b-token"),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "ungranted member cannot read the project's chat"
    );

    // The granted member reaches the project's data routes (200 on the project-home rollup).
    let (s, _) = send(
        &app,
        "GET",
        "/projects/proj-acme/home",
        None,
        Some("a-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "granted member reads their project");
    let (s, _) = send(
        &app,
        "GET",
        "/chats/chat-acme/transcript",
        None,
        Some("a-token"),
    )
    .await;
    assert_ne!(
        s,
        StatusCode::FORBIDDEN,
        "granted member passes the scope gate for the chat"
    );

    // The owner bypasses scoping — sees the project with no grant of its own.
    let (s, _) = send(
        &app,
        "GET",
        "/projects/proj-acme/home",
        None,
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "owner bypasses project scoping");

    // The workspace nav is membership-gated (200 for any active member), but its *content*
    // is now visibility-scoped (ENTSEC-2, see `entsec2_scopes_the_workspace_nav_content`).
    let (s, _) = send(&app, "GET", "/workspace", None, Some("b-token")).await;
    assert_eq!(
        s,
        StatusCode::OK,
        "the workspace nav is reachable by any member"
    );

    // Revoke consultant-a's grant → access is withdrawn (INV-18 future-only revocation).
    let (s, _) = send(
        &app,
        "DELETE",
        "/admin/grants",
        Some(r#"{"authority":"consultant-a","project_id":"proj-acme"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(
        &app,
        "GET",
        "/projects/proj-acme/home",
        None,
        Some("a-token"),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a revoked grant withdraws access");
}

#[tokio::test]
async fn secaud4_audits_sensitive_reads_when_enabled() {
    // SECAUD-4 (CC7.2): with read-auditing on, a granted member's GET of project-scoped
    // data is recorded in the org audit trail ("who read this client's data"); the
    // workspace nav (not project-scoped) is not. The default harness (off) records no read.
    let (_dir, app) = workbench_with_scoped_project_cfg(true);
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"consultant-a","role":"member","status":"active"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(
        &app,
        "POST",
        "/admin/grants",
        Some(r#"{"authority":"consultant-a","project_id":"proj-acme"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // The member reads the project's chat transcript (a sensitive GET).
    let (s, _) = send(
        &app,
        "GET",
        "/chats/chat-acme/transcript",
        None,
        Some("a-token"),
    )
    .await;
    assert_ne!(s, StatusCode::FORBIDDEN);
    // ...and reads the non-scoped workspace nav (must NOT be audited — high-volume, not data).
    let (_s, _) = send(&app, "GET", "/workspace", None, Some("a-token")).await;

    let (s, audit) = send(
        &app,
        "GET",
        "/admin/audit?actor=consultant-a",
        None,
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let entries = audit["entries"].as_array().expect("audit entries");
    assert!(
        entries.iter().any(|e| e["action"]
            .as_str()
            .unwrap_or("")
            .contains("GET /chats/chat-acme/transcript")),
        "the sensitive read is audited: {audit}"
    );
    assert!(
        !entries
            .iter()
            .any(|e| e["action"].as_str().unwrap_or("").contains("/workspace")),
        "the non-scoped nav read is not audited: {audit}"
    );
}

#[tokio::test]
async fn secaud4_reads_are_not_audited_by_default() {
    // SECAUD-4: with read-auditing OFF (the default), a member's sensitive GET leaves no
    // audit entry — the opt-in is genuinely off unless the deployment enables it.
    let (_dir, app) = workbench_with_scoped_project(); // audit_reads = false
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"consultant-a","role":"member","status":"active"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(
        &app,
        "POST",
        "/admin/grants",
        Some(r#"{"authority":"consultant-a","project_id":"proj-acme"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_s, _) = send(
        &app,
        "GET",
        "/chats/chat-acme/transcript",
        None,
        Some("a-token"),
    )
    .await;

    let (s, audit) = send(
        &app,
        "GET",
        "/admin/audit?actor=consultant-a",
        None,
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let entries = audit["entries"].as_array().expect("audit entries");
    assert!(
        !entries
            .iter()
            .any(|e| e["action"].as_str().unwrap_or("").starts_with("GET ")),
        "no GET read is audited by default: {audit}"
    );
}

#[tokio::test]
async fn entsec_blocks_export_to_disk_and_audits_member_actions() {
    // ENTSEC-5 + ENTSEC-4 (ADR 0065).
    let (_dir, app) = workbench_with_idp();
    // Provision an owner (bootstrap, no token) + a plain member.
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"member-auth","role":"member","status":"active"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // ENTSEC-5: export-to-disk would write client data to the consultant's endpoint — refused in
    // enterprise mode (the guard fires before any resource resolution).
    let (s, _) = send(
        &app,
        "POST",
        "/chats/eng-1/resources/r1/export-to-disk",
        Some(r#"{"dest":"/tmp/x"}"#),
        Some("member-token"),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "export-to-disk disabled in enterprise mode"
    );

    // ENTSEC-4: a member's mutating data-route action is recorded in the org audit trail.
    let (s, _) = send(
        &app,
        "POST",
        "/projects",
        Some(r#"{"name":"Acme"}"#),
        Some("member-token"),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CREATED,
        "an active member may create a project"
    );
    let (s, audit) = send(
        &app,
        "GET",
        "/admin/audit?actor=member-auth",
        None,
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let entries = audit["entries"].as_array().expect("audit entries");
    assert!(
        entries.iter().any(|e| e["action"]
            .as_str()
            .unwrap_or("")
            .contains("POST /projects")
            && e["actor"] == "member-auth"),
        "the member's POST /projects is in the audit trail: {audit}"
    );
}

/// ENTSEC-2 (ADR 0065): the workspace **nav content** is scoped to what the caller may see,
/// not just the per-route access gate. A scoped member sees only their granted projects (and
/// only chats within them) in `GET /workspace`; the owner sees everything; an ungranted
/// member sees no client projects at all — so project/chat *existence* never leaks.
#[tokio::test]
async fn entsec2_scopes_the_workspace_nav_content() {
    let (_dir, app) = workbench_with_scoped_project();

    // Bootstrap an owner, then two plain members; grant only consultant-a → proj-acme.
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    for who in ["consultant-a", "consultant-b"] {
        let (s, _) = send(
            &app,
            "POST",
            "/admin/members",
            Some(&format!(
                r#"{{"authority":"{who}","role":"member","status":"active"}}"#
            )),
            Some("owner-token"),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
    }
    let (s, _) = send(
        &app,
        "POST",
        "/admin/grants",
        Some(r#"{"authority":"consultant-a","project_id":"proj-acme"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Helpers over GET /workspace: does the nav (for this token) surface the project / chat?
    async fn nav(app: &Router, token: &str) -> Value {
        let (s, body) = send(app, "GET", "/workspace", None, Some(token)).await;
        assert_eq!(s, StatusCode::OK, "nav reachable: {body}");
        body
    }
    let has_project = |v: &Value, id: &str| {
        v["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == id)
    };
    let has_recent_chat = |v: &Value, id: &str| {
        v["recent"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == id)
    };

    // Owner: bypasses scoping — sees the project and its chat.
    let owner = nav(&app, "owner-token").await;
    assert!(has_project(&owner, "proj-acme"), "owner sees the project");
    assert!(has_recent_chat(&owner, "chat-acme"), "owner sees the chat");

    // Granted member: sees exactly their project and its chat.
    let a = nav(&app, "a-token").await;
    assert!(
        has_project(&a, "proj-acme"),
        "granted member sees their project"
    );
    assert!(
        has_recent_chat(&a, "chat-acme"),
        "granted member sees the chat"
    );

    // Ungranted member: the project and its chat are absent — existence does not leak.
    let b = nav(&app, "b-token").await;
    assert!(
        !has_project(&b, "proj-acme"),
        "ungranted member sees no project: {b}"
    );
    assert!(
        !has_recent_chat(&b, "chat-acme"),
        "ungranted member sees no chat: {b}"
    );

    // The list endpoint (GET /chats) applies the same visibility filter for the ungranted
    // member — it never returns chat-acme as an engagement id.
    let (s, list) = send(&app, "GET", "/chats", None, Some("b-token")).await;
    assert_eq!(s, StatusCode::OK, "chats list reachable: {list}");
    assert!(
        !list["engagements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == "chat-acme"),
        "ungranted member's chats list excludes the project chat: {list}"
    );
}

/// SEC-2: the org **session idle-timeout** is enforced on data routes. With an
/// `idle_timeout_secs` policy set, an authenticated member's session goes stale after the
/// idle window and the data route refuses it `401` (re-authentication required); the pure
/// timeout logic (lifetime + idle, keying, unset-is-noop) is covered by the
/// `session_activity` unit tests — this proves the live wiring end to end.
#[tokio::test]
async fn sec2_idle_timeout_expires_a_session_on_data_routes() {
    let (_dir, app) = workbench_with_idp();

    // Bootstrap an owner (provisions the directory).
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Set a 1-second idle timeout (owner has ConfigureSecurity).
    let (s, _) = send(
        &app,
        "POST",
        "/admin/security",
        Some(r#"{"idle_timeout_secs":1}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "owner sets the session policy");

    // First data-route request starts the session — allowed.
    let (s, _) = send(&app, "GET", "/workspace", None, Some("owner-token")).await;
    assert_eq!(s, StatusCode::OK, "the session is fresh");

    // Idle past the timeout → the same token is now refused (re-auth required).
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let (s, _) = send(&app, "GET", "/workspace", None, Some("owner-token")).await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "an idle-timed-out session is refused on the data route"
    );
}

/// ENTSEC-5: in enterprise mode the server-local *path* context ingest is disabled (a remote
/// client must not drive the server to read its filesystem) — POST /chats/:id/context is 403
/// for an authenticated member; the client uploads instead. The guard fires before any
/// engagement lookup, so it holds regardless of the chat.
#[tokio::test]
async fn entsec5_path_context_ingest_disabled_in_enterprise() {
    let (_dir, app) = workbench_with_scoped_project();
    // Bootstrap an owner so the directory is provisioned and the owner token authenticates.
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, _) = send(
        &app,
        "POST",
        "/chats/chat-acme/context",
        Some(r#"{"path":"/etc"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "server-path context ingest is refused in enterprise mode"
    );
}

/// ITGOV-2: the IT session roster (GET /admin/sessions) lists members active on the data
/// routes — populated by the admission shell, console-read gated, and it never exposes a
/// bearer (only the authority).
#[tokio::test]
async fn itgov2_session_roster_lists_active_members() {
    let (_dir, app) = workbench_with_idp();
    // Bootstrap an owner + a plain member.
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"owner-auth","role":"owner","status":"active"}"#),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(
        &app,
        "POST",
        "/admin/members",
        Some(r#"{"authority":"member-auth","role":"member","status":"active"}"#),
        Some("owner-token"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // The member is not on the roster yet (no data-route activity from them).
    let (s, roster) = send(&app, "GET", "/admin/sessions", None, Some("owner-token")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        !roster["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["authority"] == "member-auth"),
        "member absent before their first request: {roster}"
    );

    // The member makes an authenticated data request → they appear in the roster.
    let (s, _) = send(&app, "GET", "/workspace", None, Some("member-token")).await;
    assert_eq!(s, StatusCode::OK);
    let (s, roster) = send(&app, "GET", "/admin/sessions", None, Some("owner-token")).await;
    assert_eq!(s, StatusCode::OK, "{roster}");
    let sessions = roster["sessions"].as_array().unwrap();
    assert!(
        sessions.iter().any(|r| r["authority"] == "member-auth"),
        "the active member is on the roster: {roster}"
    );
    // The roster never carries a bearer/token field.
    assert!(
        sessions
            .iter()
            .all(|r| r.get("bearer").is_none() && r.get("token").is_none()),
        "no bearer leaks in the roster: {roster}"
    );

    // A plain member has no console access → the roster read is forbidden.
    let (s, _) = send(&app, "GET", "/admin/sessions", None, Some("member-token")).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "member cannot read the IT console"
    );
}

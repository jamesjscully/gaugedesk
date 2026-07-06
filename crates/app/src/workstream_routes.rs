//! The workstream surface (`WS-E`): create / list / join / leave / archive / promote
//! the shared auto-sync lines within a placement.
//!
//! A workstream's *existence* is a durable library [`WorkstreamRecord`] (name + which
//! placement); its authoritative status + membership live in the per-workstream
//! [`WorkstreamState`] reducer (scope = the workstream id), folded on demand. Joining
//! re-homes a chat's worktree onto the stream main (`workstream/<id>/main`) so its turns
//! greedily auto-sync there (the engine hook, `WS-D`); leaving / archiving re-homes back
//! to the placement mainline. Promotion runs the boundary-gated `advanced → integrated`
//! merge hop into the mainline (`MAINLINE_INTEGRATION_REQUIRES_BOUNDARY`).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::library::{gen_id, RecordOp, WorkstreamRecord};
use crate::{LockUnpoisoned, SharedWorkbench, Workbench};
use gaugewright_core::merge::{MergeCommand, MergeState};
use gaugewright_core::workstream::{WorkstreamCommand, WorkstreamPhase, WorkstreamState};
use gaugewright_store::AdmitError;
use gaugewright_workspace::MergeOutcome;

#[derive(Deserialize)]
pub struct CreateWorkstreamBody {
    pub name: String,
}

#[derive(Deserialize)]
pub struct MemberBody {
    pub chat: String,
}

impl Workbench {
    /// Re-home every workstream member's engagement onto its stream main from the folded
    /// reducer membership (`WS-E`) — run after engagements reconcile from git on startup,
    /// since git only knows the `main`-based worktree, not which workstream a chat joined.
    pub fn restore_workstream_homing(&mut self) {
        let ws_ids = self.library_workstream_ids();
        for ws_id in ws_ids {
            let members: Vec<String> = self
                .store_ref()
                .fold::<WorkstreamState>(&ws_id)
                .map(|s| s.members.into_iter().collect())
                .unwrap_or_default();
            for chat in members {
                // The chat's owning placement mints the stream ref token — no
                // ref-format knowledge outside the workspace seam (W7).
                let Some(target) = self
                    .live_engagement_instance_id(&chat)
                    .and_then(|iid| self.workstream_target(iid, &ws_id))
                else {
                    continue;
                };
                self.set_engagement_target(&chat, target);
            }
        }
    }

    /// The stream ref token for `ws_id`, minted by an open placement's
    /// workspace impl (`None` when the placement is not open).
    fn workstream_target(&self, instance_id: &str, ws_id: &str) -> Option<String> {
        self.instances
            .get(instance_id)
            .map(|inst| inst.workstream_ref(ws_id))
    }

    /// The mainline token of the placement owning `chat`'s live engagement.
    fn chat_mainline(&self, chat: &str) -> Option<String> {
        self.live_engagement_instance_id(chat)
            .and_then(|iid| self.instances.get(iid))
            .map(|inst| inst.mainline().to_string())
    }

    /// Re-home a chat's engagement onto a shared ref — joining a workstream
    /// (`workstream/<id>/main`) or leaving it back to `main` (`WS-E`). The membership
    /// authority is the [`WorkstreamState`] reducer; this updates the in-memory
    /// worktree target the auto-sync hook and the merge surface read. A no-op if the
    /// chat has no live engagement.
    pub fn set_engagement_target(&mut self, chat_id: &str, target: impl Into<String>) {
        self.set_live_engagement_target(chat_id, target);
    }

    /// Create the git-side workstream reference under an open placement repo.
    pub fn create_workstream_ref(
        &self,
        instance_id: &str,
        workstream_id: &str,
    ) -> std::io::Result<()> {
        self.create_instance_workstream_ref(instance_id, workstream_id)
    }

    /// Append a workstream declaration to the library log, apply it to the in-memory
    /// projection, and publish the workspace-change reference (`INV-10`).
    pub fn write_workstream(&mut self, record: WorkstreamRecord) -> i64 {
        self.write_workstream_record(record)
    }

    /// Re-stamp the just-written workstream record with its durable library
    /// position so navigation ordering is stable.
    pub fn restamp_workstream_position(&mut self, workstream_id: &str, position: i64) {
        self.library_restamp_workstream_position(workstream_id, position);
    }

    /// Live workstream records for a placement.
    pub fn workstreams_in(&self, instance_id: &str) -> Vec<&WorkstreamRecord> {
        self.library_workstreams_in(instance_id)
    }

    /// A cloned workstream declaration, if this workbench knows it.
    pub fn workstream(&self, workstream_id: &str) -> Option<WorkstreamRecord> {
        self.library_workstream(workstream_id)
    }

    /// Whether this workbench knows a workstream declaration.
    pub fn has_workstream(&self, workstream_id: &str) -> bool {
        self.library_has_workstream(workstream_id)
    }

    /// Fold one workstream's state from the durable log.
    pub fn workstream_state(&self, workstream_id: &str) -> Option<WorkstreamState> {
        self.store_ref().fold::<WorkstreamState>(workstream_id).ok()
    }

    /// Fold one workstream's member chat ids.
    pub fn workstream_members(&self, workstream_id: &str) -> Vec<String> {
        self.workstream_state(workstream_id)
            .map(|state| state.members.into_iter().collect())
            .unwrap_or_default()
    }

    /// Admit a workstream reducer command.
    pub fn admit_workstream(
        &mut self,
        workstream_id: &str,
        command: WorkstreamCommand,
    ) -> Result<(), AdmitError> {
        self.store_mut()
            .admit::<WorkstreamState>(workstream_id, command)
            .map(|_| ())
    }

    /// The placement id that owns a chat's live engagement.
    pub fn engagement_instance_id(&self, chat_id: &str) -> Option<&str> {
        self.live_engagement_instance_id(chat_id)
    }

    /// Promote a workstream ref into its placement mainline.
    pub fn promote_workstream_ref_to_main(
        &self,
        workstream_id: &str,
        instance_id: &str,
    ) -> std::io::Result<MergeOutcome> {
        self.promote_instance_workstream_ref_to_main(workstream_id, instance_id)
    }

    /// Record the verified reducer path for a successful workstream promotion.
    pub fn admit_workstream_promotion(&mut self, workstream_id: &str) -> Result<(), AdmitError> {
        let scope = format!("ws-merge-{workstream_id}");
        for command in [
            MergeCommand::StartMerge,
            MergeCommand::GitClean,
            MergeCommand::PolicyAdmit,
            MergeCommand::AdvanceStandingRef,
            MergeCommand::AdmitBoundaryIntegration,
            MergeCommand::IntegrateToMainline,
        ] {
            self.store_mut().admit::<MergeState>(&scope, command)?;
        }
        Ok(())
    }
}

// ---- POST /placements/:iid/workstreams -----------------------------------

/// Create a named workstream in a placement (a user **or** an agent may call this).
/// Branches `workstream/<id>/main` off the placement mainline, admits `CreateWorkstream`
/// on the new workstream scope, and records the nav declaration.
pub async fn create_workstream(
    State(wb): State<SharedWorkbench>,
    Path(iid): Path<String>,
    Json(body): Json<CreateWorkstreamBody>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    let ws_id = gen_id("ws");
    if let Err(e) = wb.create_workstream_ref(&iid, &ws_id) {
        let status = if e.kind() == std::io::ErrorKind::NotFound {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        return (status, format!("{e}")).into_response();
    }
    let home = wb.authority().as_str().to_string();
    if let Err(e) = wb.admit_workstream(
        &ws_id,
        WorkstreamCommand::CreateWorkstream {
            name: body.name.clone(),
            home: home.clone(),
            creator: home,
        },
    ) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response();
    }
    let pos = wb.write_workstream(WorkstreamRecord {
        id: ws_id.clone(),
        op: RecordOp::Upsert,
        instance_id: iid,
        name: body.name.clone(),
        created_position: 0,
    });
    // Re-stamp the record's position so nav ordering is stable (mirrors create_chat_in).
    wb.restamp_workstream_position(&ws_id, pos);
    (
        StatusCode::CREATED,
        Json(json!({ "id": ws_id, "name": body.name })),
    )
        .into_response()
}

// ---- GET /placements/:iid/workstreams ------------------------------------

/// List a placement's workstreams with folded status + members.
pub async fn list_workstreams(
    State(wb): State<SharedWorkbench>,
    Path(iid): Path<String>,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    let out: Vec<_> = wb
        .workstreams_in(&iid)
        .iter()
        .map(|w| workstream_json(&wb, w))
        .collect();
    Json(json!({ "workstreams": out })).into_response()
}

/// One workstream's projection: name + status + member chat ids (folded from the
/// reducer). Shared by the list route and the workspace tree.
pub fn workstream_json(wb: &Workbench, rec: &WorkstreamRecord) -> serde_json::Value {
    let state = wb.workstream_state(&rec.id);
    let status = match state.as_ref().map(|s| s.phase) {
        Some(WorkstreamPhase::Archived) => "archived",
        Some(WorkstreamPhase::Active) => "active",
        _ => "active",
    };
    let members: Vec<String> = state
        .map(|s| s.members.into_iter().collect())
        .unwrap_or_default();
    json!({
        "id": rec.id,
        "name": rec.name,
        "placement_id": rec.instance_id,
        "status": status,
        "members": members,
    })
}

// ---- POST /workstreams/:id/join ------------------------------------------

/// Re-home a chat onto a workstream's main — it now greedily auto-syncs there.
pub async fn join_workstream(
    State(wb): State<SharedWorkbench>,
    Path(ws_id): Path<String>,
    Json(body): Json<MemberBody>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    let Some(rec) = wb.workstream(&ws_id) else {
        return (StatusCode::NOT_FOUND, "no such workstream").into_response();
    };
    // Within-a-placement only: a chat may join only a workstream in its own placement.
    if wb.engagement_instance_id(&body.chat) != Some(rec.instance_id.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            "chat is not in this workstream's placement",
        )
            .into_response();
    }
    if let Err(e) = wb.admit_workstream(
        &ws_id,
        WorkstreamCommand::JoinWorkstream {
            chat: body.chat.clone(),
        },
    ) {
        return (StatusCode::CONFLICT, format!("{e:?}")).into_response();
    }
    // The workstream's placement mints the ref token; the placement is open
    // whenever the member chat's engagement is live (checked above).
    if let Some(target) = wb.workstream_target(&rec.instance_id, &ws_id) {
        wb.set_engagement_target(&body.chat, target);
    }
    wb.notify_library_changed("workstream", &ws_id, "upsert");
    (StatusCode::OK, Json(json!({ "joined": body.chat }))).into_response()
}

// ---- POST /workstreams/:id/leave -----------------------------------------

/// Re-home a chat back to the placement mainline (leave the workstream).
pub async fn leave_workstream(
    State(wb): State<SharedWorkbench>,
    Path(ws_id): Path<String>,
    Json(body): Json<MemberBody>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if !wb.has_workstream(&ws_id) {
        return (StatusCode::NOT_FOUND, "no such workstream").into_response();
    }
    if let Err(e) = wb.admit_workstream(
        &ws_id,
        WorkstreamCommand::LeaveWorkstream {
            chat: body.chat.clone(),
        },
    ) {
        return (StatusCode::CONFLICT, format!("{e:?}")).into_response();
    }
    if let Some(target) = wb.chat_mainline(&body.chat) {
        wb.set_engagement_target(&body.chat, target);
    }
    wb.notify_library_changed("workstream", &ws_id, "upsert");
    (StatusCode::OK, Json(json!({ "left": body.chat }))).into_response()
}

// ---- POST /workstreams/:id/archive ---------------------------------------

/// Archive a workstream: re-home every member back to the placement mainline, then
/// drive the reducer's terminal `archive` (which empties membership). The bounded
/// escape (`INV-23`) — no chat is left auto-syncing into a dead ref.
pub async fn archive_workstream(
    State(wb): State<SharedWorkbench>,
    Path(ws_id): Path<String>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    if !wb.has_workstream(&ws_id) {
        return (StatusCode::NOT_FOUND, "no such workstream").into_response();
    }
    // The members to re-home, read from the reducer before it empties them.
    let members = wb.workstream_members(&ws_id);
    if let Err(e) = wb.admit_workstream(&ws_id, WorkstreamCommand::ArchiveWorkstream) {
        return (StatusCode::CONFLICT, format!("{e:?}")).into_response();
    }
    for chat in members {
        if let Some(target) = wb.chat_mainline(&chat) {
            wb.set_engagement_target(&chat, target);
        }
    }
    wb.notify_library_changed("workstream", &ws_id, "upsert");
    (StatusCode::OK, Json(json!({ "archived": ws_id }))).into_response()
}

// ---- POST /workstreams/:id/promote ---------------------------------------

/// Promote a workstream's main into the placement mainline — the explicit,
/// boundary-gated `advanced → integrated` hop. Performs the real git merge, then
/// records it through the verified merge reducer (the boundary command gates the
/// integrate, `MAINLINE_INTEGRATION_REQUIRES_BOUNDARY`). A git conflict leaves the
/// mainline untouched (`PARTIAL_MERGE_NOT_STANDING`) for repair.
pub async fn promote_workstream(
    State(wb): State<SharedWorkbench>,
    Path(ws_id): Path<String>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    let Some(rec) = wb.workstream(&ws_id) else {
        return (StatusCode::NOT_FOUND, "no such workstream").into_response();
    };
    // The real merge: workstream/<id>/main -> main, in the placement repo.
    let outcome = match wb.promote_workstream_ref_to_main(&ws_id, &rec.instance_id) {
        Ok(outcome) => outcome,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    };
    if outcome == MergeOutcome::Conflict {
        return (
            StatusCode::CONFLICT,
            "workstream conflicts with the mainline — resolve before promoting",
        )
            .into_response();
    }
    // Record the integration through the verified reducer on the workstream's own merge
    // scope: clean -> advanced, then the boundary-gated advanced -> integrated.
    if let Err(e) = wb.admit_workstream_promotion(&ws_id) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response();
    }
    wb.notify_library_changed("workstream", &ws_id, "upsert");
    (
        StatusCode::OK,
        Json(json!({ "promoted": ws_id, "phase": "Integrated" })),
    )
        .into_response()
}

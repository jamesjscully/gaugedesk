//! Local chat/engagement route handlers.
//!
//! This is the open workbench compatibility surface for the original `/chats/*`
//! APIs: worktree reads/writes, transcript/events, merge/revert/sync, task
//! turns, and e2e reset hooks.

use std::convert::Infallible;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, Sse},
    response::IntoResponse,
    Json,
};
use futures::Stream;
use gaugewright_boundary::definition::CONFIG_PATH;
use gaugewright_core::merge::{MergeCommand, MergeState};
use gaugewright_store::Store;
use gaugewright_workspace::{ChatWorkspace, FileEntry, MergeOutcome, WorkspaceError};
use serde::Deserialize;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::{
    build_workbench, engine, err_response,
    library::{ChatMode, ChatRecord, RecordOp, LIBRARY_SCOPE},
    LockUnpoisoned, ServerEvent, SharedWorkbench, Workbench,
};

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub(crate) enum EngagementMergeAction {
    Admit,
    Reject,
    Repair,
    Retry,
    Integrate,
}

pub(crate) enum EngagementCreateError {
    Exists,
    NoDefaultInstance,
    Git(String),
}

pub(crate) struct CreatedEngagement {
    pub id: String,
    pub branch: String,
    pub path: String,
}

pub(crate) struct EngagementTaskContext {
    pub worktree: std::path::PathBuf,
    pub sender: broadcast::Sender<ServerEvent>,
    pub mode: ChatMode,
}

impl Workbench {
    // `pub` for the hosted embed plane (`cloud/embed-host`): activating a public
    // session materializes its same-id engagement under the served placement.
    pub fn materialize_engagement_in_instance(
        &mut self,
        chat_id: &str,
        instance_id: &str,
        config_json: &str,
    ) -> Result<std::path::PathBuf, String> {
        if let Some(eng) = self.engagements.get(chat_id) {
            return Ok(eng.path().to_path_buf());
        }
        let inst = self
            .instances
            .get(instance_id)
            .ok_or_else(|| format!("deployment instance '{instance_id}' not open"))?;
        let eng = inst.create_engagement(chat_id).map_err(|e| e.to_string())?;
        let worktree = eng.path().to_path_buf();
        let _ = eng.write_file(CONFIG_PATH, config_json);
        self.register_engagement(chat_id, instance_id, eng);
        Ok(worktree)
    }

    pub(crate) fn create_default_engagement(
        &mut self,
        id: String,
        title: String,
    ) -> Result<CreatedEngagement, EngagementCreateError> {
        if self.engagements.contains_key(&id) {
            return Err(EngagementCreateError::Exists);
        }
        let inst_id = self.default_instance.clone();
        let Some(inst) = self.instances.get(&inst_id) else {
            return Err(EngagementCreateError::NoDefaultInstance);
        };
        let eng = inst
            .create_engagement(&id)
            .map_err(|e| EngagementCreateError::Git(e.to_string()))?;
        let branch = eng.branch().to_string();
        let path = eng.path().to_string_lossy().to_string();
        let rec = ChatRecord {
            id: id.clone(),
            op: RecordOp::Upsert,
            instance_id: inst_id.clone(),
            title,
            created_position: 0,
            forked_from: None,
        };
        self.write_created_chat_record(rec);
        self.register_engagement(id.clone(), inst_id, eng);
        Ok(CreatedEngagement { id, branch, path })
    }

    /// Register a live engagement handle under its owning instance.
    pub fn register_engagement(
        &mut self,
        chat_id: impl Into<String>,
        inst_id: impl Into<String>,
        eng: Box<dyn ChatWorkspace>,
    ) {
        let chat_id = chat_id.into();
        self.engagement_index
            .insert(chat_id.clone(), inst_id.into());
        self.engagements.insert(chat_id, eng);
    }

    /// Whether a live engagement handle is registered under this chat id.
    pub fn has_engagement(&self, chat_id: &str) -> bool {
        self.engagements.contains_key(chat_id)
    }

    pub(crate) fn set_live_engagement_target(&mut self, chat_id: &str, target: impl Into<String>) {
        if let Some(eng) = self.engagements.get_mut(chat_id) {
            // The git impl's re-home cannot fail (AM-4: the Result channel exists
            // for impls that must re-home fail-closed).
            let _ = eng.set_target(&target.into());
        }
    }

    pub(crate) fn live_engagement_instance_id(&self, chat_id: &str) -> Option<&str> {
        self.engagement_index.get(chat_id).map(String::as_str)
    }

    pub(crate) fn engagement_ids(&self) -> Vec<String> {
        self.engagements.keys().cloned().collect()
    }

    pub(crate) fn engagement_diff(&self, id: &str) -> Option<Result<String, WorkspaceError>> {
        self.engagements.get(id).map(|eng| eng.diff_against_main())
    }

    pub(crate) fn engagement_config_json(&self, id: &str) -> Option<String> {
        let eng = self.engagements.get(id)?;
        Some(
            eng.read_file(CONFIG_PATH)
                .unwrap_or_else(|_| "{}".to_string()),
        )
    }

    pub(crate) fn write_engagement_config(
        &mut self,
        id: &str,
        body: &str,
    ) -> Option<Result<(), WorkspaceError>> {
        let eng = self.engagements.get(id)?;
        // The facet's io failure carries the bare io message, so the 500 body
        // stays the raw io text it has always been.
        let written = eng.write_file(CONFIG_PATH, body);
        Some(written.map(|()| {
            self.publish(
                id,
                ServerEvent::Admitted {
                    kind: "authoring".into(),
                    text: "agent config updated".into(),
                },
            );
        }))
    }

    pub(crate) fn engagement_transcript_json(
        &self,
        id: &str,
    ) -> Result<String, gaugewright_store::AdmitError> {
        self.store_ref()
            .records(id, "transcript")
            .map(|rows| format!("[{}]", rows.join(",")))
    }

    /// Ingest context bytes into a live engagement and commit the worktree.
    pub fn ingest_context_into_engagement(
        &mut self,
        chat_id: &str,
        path: &std::path::Path,
    ) -> Option<Result<(usize, String), WorkspaceError>> {
        let eng = self.engagements.get(chat_id)?;
        let n = match eng.ingest(path) {
            Ok(n) => n,
            Err(e) => return Some(Err(e)),
        };
        let commit = match eng.commit_turn(&format!("ingest context: {}", path.display())) {
            Ok(commit) => commit.map(|c| c.0).unwrap_or_default(),
            Err(e) => return Some(Err(e)),
        };
        Some(Ok((n, commit)))
    }

    /// Ingest **uploaded** context bytes into a live engagement and commit (`ENTSEC-5`): the
    /// upload counterpart of [`ingest_context_into_engagement`](Self::ingest_context_into_engagement)
    /// for the enterprise thin-client, where the client's files are sent as an upload rather
    /// than a server-local path. `None` if the engagement is unknown.
    pub fn ingest_upload_into_engagement(
        &mut self,
        chat_id: &str,
        files: &[(String, String)],
    ) -> Option<Result<(usize, String), WorkspaceError>> {
        let eng = self.engagements.get(chat_id)?;
        let n = match eng.ingest_upload(files) {
            Ok(n) => n,
            Err(e) => return Some(Err(e)),
        };
        let commit = match eng.commit_turn(&format!("ingest uploaded context: {n} file(s)")) {
            Ok(commit) => commit.map(|c| c.0).unwrap_or_default(),
            Err(e) => return Some(Err(e)),
        };
        Some(Ok((n, commit)))
    }

    /// The current file manifest for a live engagement.
    pub fn engagement_tree(&self, chat_id: &str) -> Option<Result<Vec<FileEntry>, WorkspaceError>> {
        self.engagements.get(chat_id).map(|eng| eng.tree())
    }

    /// Read one file from a live engagement worktree.
    pub fn read_engagement_file(
        &self,
        chat_id: &str,
        path: &str,
    ) -> Option<Result<String, WorkspaceError>> {
        self.engagements.get(chat_id).map(|eng| eng.read_file(path))
    }

    pub(crate) fn write_engagement_file(
        &mut self,
        chat_id: &str,
        path: &str,
        body: &str,
    ) -> Option<Result<(), WorkspaceError>> {
        let eng = self.engagements.get(chat_id)?;
        let result = eng
            .write_file(path, body)
            .and_then(|_| eng.commit_turn(&format!("edit {path}")).map(|_| ()));
        if result.is_ok() {
            let ev = ServerEvent::Admitted {
                kind: "edit".into(),
                text: format!("edited {path}"),
            };
            let _ = self
                .store_mut()
                .append_record(chat_id, "transcript", &ev.to_json());
            self.publish(chat_id, ev);
        }
        Some(result)
    }

    pub(crate) fn engagement_merge_state(
        &self,
        id: &str,
    ) -> Result<MergeState, gaugewright_store::AdmitError> {
        self.store_ref().fold::<MergeState>(id)
    }

    pub(crate) fn revert_engagement(&mut self, id: &str) -> Option<Result<(), WorkspaceError>> {
        let eng = self.engagements.get(id)?;
        let result = eng.revert_to_main();
        if result.is_ok() {
            self.publish(
                id,
                ServerEvent::Admitted {
                    kind: "revert".into(),
                    text: "reverted to main — engagement work discarded".into(),
                },
            );
        }
        Some(result)
    }

    fn admit_merge_command(
        &mut self,
        id: &str,
        command: MergeCommand,
    ) -> Result<MergeState, String> {
        self.store_mut()
            .admit::<MergeState>(id, command)
            .map_err(|e| format!("{e:?}"))
    }

    pub(crate) fn apply_engagement_merge_action(
        &mut self,
        id: &str,
        action: EngagementMergeAction,
    ) -> Option<Result<MergeState, String>> {
        if !self.engagements.contains_key(id) {
            return None;
        }
        let result = match action {
            EngagementMergeAction::Reject => {
                self.admit_merge_command(id, MergeCommand::PolicyReject)
            }
            EngagementMergeAction::Repair => {
                self.admit_merge_command(id, MergeCommand::SubmitRepair)
            }
            EngagementMergeAction::Admit => self
                .admit_merge_command(id, MergeCommand::PolicyAdmit)
                .and_then(
                    |_| match self.engagements.get(id).unwrap().merge_into_main() {
                        Ok(MergeOutcome::Clean) => {
                            self.admit_merge_command(id, MergeCommand::AdvanceStandingRef)
                        }
                        Ok(MergeOutcome::Conflict) => {
                            Err("main changed since review — re-review the diff".into())
                        }
                        Err(e) => Err(e.to_string()),
                    },
                ),
            EngagementMergeAction::Integrate => self
                .admit_merge_command(id, MergeCommand::AdmitBoundaryIntegration)
                .and_then(|_| self.admit_merge_command(id, MergeCommand::IntegrateToMainline)),
            EngagementMergeAction::Retry => {
                match self.engagements.get(id).unwrap().merge_into_main() {
                    Ok(MergeOutcome::Clean) => {
                        let n = self
                            .store_ref()
                            .fold::<MergeState>(id)
                            .map(|s| s.retry_keys_used.len())
                            .unwrap_or(0);
                        self.admit_merge_command(
                            id,
                            MergeCommand::RetryRepair(format!("retry-{n}")),
                        )
                    }
                    Ok(MergeOutcome::Conflict) => {
                        Err("still conflicting — resolve in the editor".into())
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
        };
        if let Ok(state) = &result {
            let line = format!("merge → {:?}", state.phase);
            let event = ServerEvent::Admitted {
                kind: "merge".into(),
                text: line,
            };
            let _ = self
                .store_mut()
                .append_record(id, "transcript", &event.to_json());
            self.publish(id, event);
        }
        Some(result)
    }

    pub(crate) fn engagement_task_context(&mut self, id: &str) -> Option<EngagementTaskContext> {
        let eng = self.engagements.get(id)?;
        let worktree = eng.path().to_path_buf();
        let mode = self.library_chat_mode(id);
        let sender = self.sender(id);
        Some(EngagementTaskContext {
            worktree,
            sender,
            mode,
        })
    }

    pub(crate) fn sync_engagement_from_main(
        &mut self,
        id: &str,
    ) -> Option<Result<MergeOutcome, WorkspaceError>> {
        let eng = self.engagements.get(id)?;
        let result = eng.sync_from_main();
        if matches!(result, Ok(MergeOutcome::Clean)) {
            let ev = ServerEvent::Admitted {
                kind: "sync".into(),
                text: "synced from main".into(),
            };
            let _ = self
                .store_mut()
                .append_record(id, "transcript", &ev.to_json());
            self.publish(id, ev);
        }
        Some(result)
    }

    pub(crate) fn workspace_sender(&mut self) -> broadcast::Sender<ServerEvent> {
        self.sender(LIBRARY_SCOPE)
    }
}

#[derive(Deserialize)]
pub(crate) struct CreateEngagement {
    /// Optional. When absent, the server mints one (`gen_id("chat")`) — the path the
    /// All-chats "+ new chat" quick-start uses, since the UI never mints ids. Tests
    /// and scripts may still pin an explicit id (back-compat).
    #[serde(default)]
    id: Option<String>,
}

/// Open a new engagement on the **default** instance — the hidden Personal default
/// placement (ADR 0036), so this is a **work** chat (ADR 0035). The All-chats
/// "+ new chat" affordance and back-compat tests/scripts both land here. A worktree
/// off that instance's `main`, recorded as a chat so it shows in `/workspace` and
/// survives a restart.
pub(crate) async fn create_engagement(
    State(wb): State<SharedWorkbench>,
    Json(body): Json<CreateEngagement>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    // An explicit id keeps its raw value as the title (back-compat); a minted id gets
    // the "new chat" placeholder so the nav renders it as "Untitled" until the first
    // message auto-titles it (state/chat-title) — never the raw `chat-…` token.
    let (id, title) = match body.id {
        Some(id) => (id.clone(), id),
        None => (crate::library::gen_id("chat"), "new chat".to_string()),
    };
    match wb.create_default_engagement(id, title) {
        Ok(created) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": created.id,
                "branch": created.branch,
                "path": created.path,
            })),
        )
            .into_response(),
        Err(EngagementCreateError::Exists) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "engagement exists" })),
        )
            .into_response(),
        Err(EngagementCreateError::NoDefaultInstance) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "no default instance" })),
        )
            .into_response(),
        Err(EngagementCreateError::Git(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// List open engagement ids (a projection).
pub(crate) async fn list_engagements(
    State(wb): State<SharedWorkbench>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    // ENTSEC-2: a scoped member sees only chats in their granted projects (a no-op for
    // solo/owner); a chat outside a visible project is dropped, not just access-denied.
    let vis = wb.project_visibility(crate::net_http::bearer(&headers));
    let ids: Vec<_> = wb
        .engagement_ids()
        .into_iter()
        .filter(|id| wb.chat_visible(id, &vis))
        .collect();
    Json(serde_json::json!({ "engagements": ids })).into_response()
}

/// The reviewer's diff: the engagement branch against `main`.
pub(crate) async fn engagement_diff(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    let Some(diff) = wb.engagement_diff(&id) else {
        return (StatusCode::NOT_FOUND, "no such engagement").into_response();
    };
    match diff {
        Ok(diff) => (StatusCode::OK, Json(serde_json::json!({ "diff": diff }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// Agent authoring (edit mode): read the engagement's `.agent-config.json`
/// (the agent's policy + model). Returns `{}` if none is set yet.
pub(crate) async fn get_config(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    let Some(body) = wb.engagement_config_json(&id) else {
        return (StatusCode::NOT_FOUND, "no such engagement").into_response();
    };
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

/// Agent authoring: write the engagement's `.agent-config.json`. The body is
/// **parsed** into the policy model first (parse-don't-validate at the boundary);
/// a malformed config is rejected rather than persisted.
pub(crate) async fn put_config(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
    body: String,
) -> impl IntoResponse {
    // Validate by parsing into the boundary's policy model before writing.
    if let Err(e) = gaugewright_boundary::AgentConfig::from_json(&body) {
        return (
            StatusCode::BAD_REQUEST,
            format!("invalid agent config: {e}"),
        )
            .into_response();
    }
    let mut wb = wb.lock_unpoisoned();
    let Some(result) = wb.write_engagement_config(&id, &body) else {
        return (StatusCode::NOT_FOUND, "no such engagement").into_response();
    };
    match result {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "saved": true }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// The durable transcript snapshot (`app-stack.md`: the transcript is a client
/// reduction of the server stream, **repairable from a snapshot**). Returns the
/// engagement's admitted transcript records in order — the client reduces these,
/// then subscribes to live SSE for the in-progress turn.
pub(crate) async fn get_transcript(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    match wb.engagement_transcript_json(&id) {
        Ok(body) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        Err(e) => err_response(e),
    }
}

/// The worktree file tree (the WORKSPACE panel, `navigation.md`).
pub(crate) async fn get_tree(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    let Some(tree) = wb.engagement_tree(&id) else {
        return (StatusCode::NOT_FOUND, "no such engagement").into_response();
    };
    match tree {
        Ok(entries) => {
            let files: Vec<_> = entries
                .into_iter()
                .map(|e| serde_json::json!({ "path": e.path, "is_dir": e.is_dir }))
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "files": files }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    path: String,
}

/// Read a worktree file (the content viewer's View mode).
pub(crate) async fn get_file(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
    Query(q): Query<FileQuery>,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    let Some(content) = wb.read_engagement_file(&id, &q.path) else {
        return (StatusCode::NOT_FOUND, "no such engagement").into_response();
    };
    match content {
        Ok(content) => (StatusCode::OK, content).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e}")).into_response(),
    }
}

/// Save a worktree file (the editor's Edit mode) and commit it — the human's edit
/// is a contribution to the engagement thread that rides the merge. Each save is a
/// commit on the engagement branch, so git is the file's durable version history
/// (surfaced via the Diff / promote-to-main surface), not a parallel store.
pub(crate) async fn put_file(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
    Query(q): Query<FileQuery>,
    body: String,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    let Some(result) = wb.write_engagement_file(&id, &q.path, &body) else {
        return (StatusCode::NOT_FOUND, "no such engagement").into_response();
    };
    match result {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "saved": true }))).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e}")).into_response(),
    }
}

/// Merge state (the review surface): the turn's branch-vs-`main` merge lifecycle.
pub(crate) async fn get_merge(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let wb = wb.lock_unpoisoned();
    match wb.engagement_merge_state(&id) {
        Ok(state) => (StatusCode::OK, Json(state)).into_response(),
        Err(e) => err_response(e),
    }
}

/// Discard an engagement's work, restoring its worktree to `main` — the user-facing
/// **revert** (UX-5). `main` is untouched; the dropped work is recoverable only by redoing
/// it. Fail-closed: an unknown engagement 404s.
pub(crate) async fn post_revert(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    let Some(result) = wb.revert_engagement(&id) else {
        return (StatusCode::NOT_FOUND, "no such engagement").into_response();
    };
    if let Err(e) = result {
        return (StatusCode::BAD_REQUEST, format!("{e}")).into_response();
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "reverted": true })),
    )
        .into_response()
}

pub(crate) async fn post_merge_command(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
    Json(action): Json<EngagementMergeAction>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    let Some(result) = wb.apply_engagement_merge_action(&id, action) else {
        return (StatusCode::NOT_FOUND, "no such engagement").into_response();
    };

    match result {
        Ok(state) => (StatusCode::OK, Json(state)).into_response(),
        Err(e) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "rejected": e })),
        )
            .into_response(),
    }
}

/// Live event stream (SSE): the engagement's operational + admitted events as
/// they happen. The client reduces this into its transcript.
pub(crate) async fn engagement_events(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = wb.lock_unpoisoned().sender(&id).subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| {
        msg.ok()
            .map(|ev: ServerEvent| Ok(Event::default().data(ev.to_json())))
    });
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

/// Live **workspace** event stream (SSE): a "changed" ping whenever the library
/// mutates (a chat/project/archetype/placement created, renamed, or removed — on
/// THIS client or any other, e.g. a paired device). The client re-reads `/workspace`
/// on each ping, so every nav mirrors the node live (the push the system is built
/// on, not a poll). Subscribes to the reserved `library` stream key.
pub(crate) async fn workspace_events(
    State(wb): State<SharedWorkbench>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = wb.lock_unpoisoned().workspace_sender().subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| {
        msg.ok()
            .map(|ev: ServerEvent| Ok(Event::default().data(ev.to_json())))
    });
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

#[derive(Deserialize)]
pub(crate) struct TaskBody {
    prompt: String,
    /// Native image content blocks attached to this message (UX-14). Sent to Pi as
    /// model input; never recorded in the durable transcript. Absent ⇒ a text turn.
    #[serde(default)]
    images: Vec<gaugewright_harness::ImageContent>,
}

/// Task an engagement: drive one real Pi turn in its worktree, streaming the
/// operational events live (SSE) and returning the diff + output. Effects route
/// through the membrane (`.agent-config.json` policy + the gaugewright Pi plugin).
pub(crate) async fn post_task(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
    Json(body): Json<TaskBody>,
) -> impl IntoResponse {
    // Brief lock: confirm the engagement and grab its worktree, live sender, mode.
    let (worktree, sender, mode) = {
        let mut g = wb.lock_unpoisoned();
        let Some(context) = g.engagement_task_context(&id) else {
            return (StatusCode::NOT_FOUND, "no such engagement").into_response();
        };
        (context.worktree, context.sender, context.mode)
    };

    let wb2 = wb.clone();
    let task = body.prompt;
    let images = body.images;
    let id2 = id.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        engine::run_engagement_turn(&wb2, &id2, &worktree, &sender, &task, &images, mode)
    })
    .await;

    match outcome {
        Ok(Ok(result)) => (StatusCode::OK, Json(result)).into_response(),
        Ok(Err(e)) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "task panicked").into_response(),
    }
}

/// Sync settled `main` into this engagement (WC-1): pick up work other engagements
/// in the workstream promoted. Returns the outcome; a conflict leaves the worktree
/// for repair (the merge review surface).
pub(crate) async fn post_sync(
    State(wb): State<SharedWorkbench>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut wb = wb.lock_unpoisoned();
    let Some(result) = wb.sync_engagement_from_main(&id) else {
        return (StatusCode::NOT_FOUND, "no such engagement").into_response();
    };
    match result {
        Ok(MergeOutcome::Clean) => (
            StatusCode::OK,
            Json(serde_json::json!({ "synced": true, "conflict": false })),
        )
            .into_response(),
        Ok(MergeOutcome::Conflict) => (
            StatusCode::OK,
            Json(serde_json::json!({ "synced": false, "conflict": true })),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// Stop a running turn (`run-chat.md`: Stop = abort the run). Fires the turn's
/// out-of-band interrupt handle so its blocking `recv` returns and the run fails;
/// the session is retired and the next turn respawns. A no-op if nothing is
/// running (or in fake-agent mode, where turns are instant).
pub(crate) async fn post_stop(
    State(_wb): State<SharedWorkbench>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match engine::running_turn_interrupt(&id) {
        Some(interrupt) => {
            // The handle was captured at turn start (the default is `kill -KILL`
            // on the runtime's pid — SIGKILL is reliable, Pi may ignore TERM
            // mid-stream). The on-disk session persists, so only the in-memory
            // turn is aborted.
            interrupt();
            (StatusCode::OK, Json(serde_json::json!({ "stopped": true }))).into_response()
        }
        None => (
            StatusCode::OK,
            Json(serde_json::json!({ "stopped": false, "reason": "nothing running" })),
        )
            .into_response(),
    }
}

/// **Test-only** — reset the control plane to a freshly-seeded state. Gated behind
/// `GAUGEWRIGHT_TEST_RESET` (set by the e2e launcher), so it is inert in a normal run.
///
/// The e2e suite shares one control plane across all scenarios, serially; with no
/// reset the append-only store accumulates every scenario's projects, archetypes
/// and chats, and later scenarios collide with the pile (stale `.first()` matches,
/// off-screen menus on a tall tree). This hands each scenario a clean slate: stop
/// every live agent process, wipe the on-disk state, and rebuild the seeded
/// workbench in place behind the shared mutex.
pub(crate) async fn post_test_reset(State(wb): State<SharedWorkbench>) -> impl IntoResponse {
    if std::env::var("GAUGEWRIGHT_TEST_RESET").is_err() {
        return (StatusCode::FORBIDDEN, "reset is disabled").into_response();
    }
    let mut guard = wb.lock_unpoisoned();
    let root = guard.root_path();
    if root.as_os_str().is_empty() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "no state root to reset").into_response();
    }
    guard.shutdown_sessions_for_reset();
    engine::clear_running_turns();
    // Drop the old workbench — closing the sqlite store and releasing the instance
    // worktrees — by swapping in a throwaway in-memory one, so the files unlink.
    match Store::open_in_memory() {
        Ok(scratch) => drop(std::mem::replace(&mut *guard, Workbench::new(scratch))),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("reset scratch store: {e}"),
            )
                .into_response()
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    // Clear any armed test-only conflict injection (UX-7) so it can't leak across scenarios.
    engine::set_force_merge_conflict(false);
    match build_workbench(&root) {
        Ok(fresh) => {
            *guard = fresh;
            (StatusCode::OK, Json(serde_json::json!({ "reset": true }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("rebuild: {e}")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct ForceConflictBody {
    #[serde(default)]
    on: bool,
}

/// Test-only (`UX-7`): arm/disarm merge-conflict injection so a browser BDD can drive the
/// `INV-24` conflict-repair path. Inert unless `GAUGEWRIGHT_TEST_RESET` is set, like
/// [`post_test_reset`]; `POST /test/reset` also clears it.
pub(crate) async fn post_test_force_conflict(
    Json(body): Json<ForceConflictBody>,
) -> impl IntoResponse {
    if std::env::var("GAUGEWRIGHT_TEST_RESET").is_err() {
        return (StatusCode::FORBIDDEN, "conflict injection is disabled").into_response();
    }
    engine::set_force_merge_conflict(body.on);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "force_conflict": body.on })),
    )
        .into_response()
}

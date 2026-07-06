//! The canonical agent loop: task an agent against a folder and let it work —
//! headlessly, end-to-end through the verified spine.
//!
//! This is the orchestrator the Phase-2 gate names: it creates an engagement
//! worktree off the instance's `main`, admits the [[run]] lifecycle into the
//! durable store, drives one [[runtime-session]] turn over Pi through the egress
//! membrane, auto-commits the worktree, and surfaces the diff + output. The
//! durable truth (run events) lives in the store; the worktree holds the work;
//! the membrane is the chokepoint for every effect.
//!
//! Each collaborator is the verified piece built in its own crate — this module
//! only sequences them; it owns no protection logic of its own.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Test-only **conflict injection** (`UX-7`): when set, a completing turn's merge probe is
/// forced to `Conflict`, driving the engagement into the isolated/repair-context path
/// (`INV-24`) so a browser BDD can exercise conflict-repair without staging a real adversarial
/// git conflict. Toggled by the `POST /test/force-conflict` route (gated by
/// `GAUGEWRIGHT_TEST_RESET`); cleared by `POST /test/reset`. Inert in a normal run.
static FORCE_MERGE_CONFLICT: AtomicBool = AtomicBool::new(false);

/// Registry of in-flight turns to their [`InterruptHandle`], kept outside the
/// workbench mutex so a Stop request can terminate a running turn's runtime
/// without blocking on the lock the turn itself holds.
static RUNNING_TURNS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, InterruptHandle>>,
> = std::sync::OnceLock::new();

fn running_turns() -> &'static std::sync::Mutex<std::collections::HashMap<String, InterruptHandle>>
{
    RUNNING_TURNS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Record a turn's interrupt handle for out-of-band Stop handling.
pub(crate) fn register_running_turn(id: &str, interrupt: InterruptHandle) {
    running_turns()
        .lock_unpoisoned()
        .insert(id.to_string(), interrupt);
}

/// Clear a turn's interrupt handle after the turn exits.
pub(crate) fn clear_running_turn(id: &str) {
    running_turns().lock_unpoisoned().remove(id);
}

/// The interrupt handle for a running turn, if any.
pub(crate) fn running_turn_interrupt(id: &str) -> Option<InterruptHandle> {
    running_turns().lock_unpoisoned().get(id).cloned()
}

/// Clear all running-turn interrupt handles, used by the test reset route.
pub(crate) fn clear_running_turns() {
    running_turns().lock_unpoisoned().clear();
}

/// Set the test-only merge-conflict injection flag (`UX-7`).
pub fn set_force_merge_conflict(on: bool) {
    FORCE_MERGE_CONFLICT.store(on, Ordering::Relaxed);
}

/// Whether merge-conflict injection is armed (`UX-7`).
pub fn force_merge_conflict() -> bool {
    FORCE_MERGE_CONFLICT.load(Ordering::Relaxed)
}

use gaugewright_boundary::{definition, AgentConfig, AuthoringMode, Decision, Effect, Membrane};
use tokio::sync::broadcast;

use crate::harness_select::ScriptedFakeFactory;
use crate::library::ChatMode;
use crate::stream::ServerEvent;
use crate::{LockUnpoisoned, SharedWorkbench, Workbench};

impl Workbench {
    /// Place a chat's runtime in a *different* trust authority: register its
    /// **remote** harness alongside the local sessions (`WORKBENCH-REMOTE-1`). A
    /// chat is local or remote, never both, so an existing local session under the
    /// same id is retired (shut down) first — the workbench holds one runtime per
    /// chat, just at one of two placements.
    pub fn register_remote_session(
        &mut self,
        chat_id: impl Into<String>,
        harness: Box<dyn gaugewright_harness::RemoteHarness>,
    ) {
        let chat_id = chat_id.into();
        if let Some(local) = self.sessions.remove(&chat_id) {
            let _ = local.shutdown();
        }
        self.remote_sessions.insert(chat_id, harness);
    }

    /// Whether a chat is placed remotely (has a registered remote harness). The
    /// engine consults this to route a turn down the local or the remote path.
    pub fn is_remote(&self, chat_id: &str) -> bool {
        self.remote_sessions.contains_key(chat_id)
    }

    #[cfg(test)]
    pub(crate) fn seed_local_session_for_test(
        &mut self,
        chat_id: impl Into<String>,
        harness: Box<dyn gaugewright_harness::Harness>,
    ) {
        self.sessions.insert(chat_id.into(), harness);
    }

    #[cfg(test)]
    pub(crate) fn has_local_session_for_test(&self, chat_id: &str) -> bool {
        self.sessions.contains_key(chat_id)
    }

    /// The peer endpoint a remotely-placed chat is reached at, if any — the relay
    /// resolves it (ADR 0020); the workbench only records *which* placement holds.
    pub fn remote_address(&self, chat_id: &str) -> Option<&str> {
        self.remote_sessions.get(chat_id).map(|h| h.address())
    }

    /// The network egress posture for a chat, resolved through its project
    /// (see [`crate::library::Library::chat_network_isolated`]). Open by default;
    /// an explicit per-project opt-in isolates. Read by the engine when building
    /// Pi's sandbox.
    pub fn chat_network_isolated(&self, chat_id: &str) -> bool {
        self.library_chat_network_isolated(chat_id)
    }

    /// Stop and forget all in-memory local/remote agent sessions before the
    /// test-only reset swaps the durable workbench state.
    pub(crate) fn shutdown_sessions_for_reset(&mut self) {
        for (_, session) in std::mem::take(&mut self.sessions) {
            let _ = session.shutdown();
        }
        self.remote_sessions.clear();
    }
}

/// The editor persona used in **edit mode**: the agent you edit *with* (ADR
/// 0027). It works on the *current* agent's definition — prefixed to the prompt
/// so the model edits the agent rather than doing end-user work.
pub const EDITOR_FRAMING: &str =
    "You are the editor: you improve THIS agent's own definition in the current workspace. \
Its model and security policy live in `.agent-config.json`; its instructions live in `AGENTS.md`. \
Edit those files to satisfy the request, then briefly explain what you changed. \
Do not perform end-user tasks in edit mode — refine the agent itself.";

/// Append a durable transcript record (admitted run evidence) to the engagement's
/// log — the snapshot the client reduces on load (`app-stack.md`: repairable).
fn record_transcript(store: &mut Store, scope: &str, event: &ServerEvent) {
    let _ = store.append_record(scope, "transcript", &event.to_json());
}
use gaugewright_core::merge::{MergeCommand, MergePhase, MergeState};
use gaugewright_core::run::{RunCommand, RunPhase, RunState};
use gaugewright_harness::{
    CredentialProbe, EgressGate, GateDecision, Harness, HarnessFactory, HarnessSpec, ImageContent,
    InterruptHandle, Observation, TurnOutcome,
};
use gaugewright_store::{AdmitError, Store};
use gaugewright_workspace::{ChatWorkspace, MergeOutcome};

/// A membrane-backed egress gate: maps a Pi tool name to an [`Effect`] and asks
/// the [`Membrane`] to rule. Tools known to leave the workspace (network) are
/// classified as external; everything else as an in-workspace effect.
pub struct MembraneGate {
    membrane: Membrane,
    external_tools: BTreeSet<String>,
}

impl MembraneGate {
    pub fn new(config: &AgentConfig, external_tools: BTreeSet<String>) -> Self {
        Self {
            membrane: Membrane::new(config.policy.clone()),
            external_tools,
        }
    }

    /// Bind the engagement's chat mode so the membrane can enforce the
    /// method-definition write-gate (`INV-24`): edit may edit the agent's own
    /// definition, use is read-only to it.
    pub fn with_mode(mut self, mode: ChatMode) -> Self {
        let authoring = match mode {
            ChatMode::Edit => AuthoringMode::Edit,
            ChatMode::Use => AuthoringMode::Use,
        };
        self.membrane = self.membrane.with_mode(authoring);
        self
    }
}

impl EgressGate for MembraneGate {
    fn classify_tool(&self, tool: &str, target: Option<&str>) -> GateDecision {
        let effect = if self.external_tools.contains(tool) {
            Effect::external(tool)
        } else {
            Effect::in_workspace(tool)
        }
        .with_target(target.map(|s| s.to_string()));
        match self.membrane.classify(&effect) {
            Decision::Allow => GateDecision::Allow,
            Decision::Block(r) => GateDecision::Block(r.to_string()),
            Decision::Stage(r) => GateDecision::Stage(r.to_string()),
        }
    }
}

/// Tools known to leave the workspace (network). The membrane treats everything
/// else as an in-workspace effect.
fn default_external_tools() -> BTreeSet<String> {
    ["fetch", "web", "curl", "http", "download"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// The method-definition surface paths to make read-only for a turn (ADR 0030):
/// in use mode, the surface files that exist in the worktree; in edit mode none
/// (the editor edits the definition). These become sandbox `read_only_roots`.
/// The egress hosts the model endpoint needs, by provider (RF-B3). This is the
/// single deliberate network grant the bridge declares so a deny-by-default
/// sandbox can still reach the model — every *other* destination (a `curl` to an
/// attacker host) is outside this declared set. The host list is intentionally
/// conservative per provider; it is the allowlist the per-host egress proxy will
/// enforce once that routing lands (until then it records intent and flips the
/// posture to allow). An unknown provider falls back to the OpenAI/codex set.
/// Resolve a turn's provider: a non-empty **host override** (`GAUGEWRIGHT_MODEL_PROVIDER`, set
/// only by a SERVE-2 deployment image to force its egress membrane) wins over the chat's
/// `.agent-config.json` provider, which wins over the codex OAuth default. Pure (the env is read
/// at the call site) so the precedence is unit-testable.
fn resolve_turn_provider(host_override: Option<String>, config_provider: Option<String>) -> String {
    host_override
        .filter(|s| !s.is_empty())
        .or(config_provider.filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "openai-codex".to_string())
}

/// Resolve a turn's model: a non-empty host override (`GAUGEWRIGHT_MODEL`) wins over the chat's
/// configured model; `None` leaves Pi's per-provider default. Paired with
/// [`resolve_turn_provider`] so a host that forces the provider can pin a compatible model.
fn resolve_turn_model(
    host_override: Option<String>,
    config_model: Option<String>,
) -> Option<String> {
    host_override
        .filter(|s| !s.is_empty())
        .or(config_model.filter(|s| !s.is_empty()))
}

fn model_endpoint_hosts(provider: Option<&str>) -> Vec<String> {
    let hosts: &[&str] = match provider.unwrap_or("openai-codex") {
        // Host-managed providers (SERVE-2 hosted embed): the sandbox egresses only to
        // the host-managed gateway endpoint; provider-token details live in the private
        // managed-service host, not in the open engine.
        p if p.contains("cloudflare") => &["gateway.ai.cloudflare.com", "api.cloudflare.com"],
        p if p.starts_with("openai") || p.contains("codex") => {
            &["api.openai.com", "chatgpt.com", "auth.openai.com"]
        }
        p if p.contains("anthropic") => &["api.anthropic.com"],
        p if p.contains("azure") => &["openai.azure.com"],
        // Unknown provider: default to the codex/OpenAI endpoints rather than
        // opening the network wide — a misconfigured provider fails closed-ish.
        _ => &["api.openai.com", "chatgpt.com", "auth.openai.com"],
    };
    hosts.iter().map(|s| s.to_string()).collect()
}

fn method_surface_readonly_roots(worktree: &Path, mode: ChatMode) -> Vec<std::path::PathBuf> {
    if !matches!(mode, ChatMode::Use) {
        return Vec::new();
    }
    definition::READONLY_ROOTS
        .iter()
        .map(|s| worktree.join(s))
        .filter(|p| p.exists())
        .collect()
}

/// The result of one tasked turn.
#[derive(Debug, serde::Serialize)]
pub struct TaskResult {
    pub run_phase: RunPhase,
    pub assistant_text: String,
    /// The engagement-branch-vs-`main` diff a reviewer sees.
    pub diff: String,
    /// The turn's new revision id as its bare string (git: the auto-commit's
    /// short hash), if the turn changed anything.
    pub commit: Option<String>,
    /// The merge lifecycle phase after the turn: `Clean` (awaiting the human's
    /// admit/reject of the diff) or `Rejected` (a git conflict → isolated).
    pub merge_phase: MergePhase,
    pub mediated_tool_calls: Vec<String>,
    /// Effects the membrane blocked (the out-of-policy path).
    pub blocked_effects: Vec<String>,
    pub pending_approvals: Vec<String>,
    /// The runtime/model error that failed this turn, if any — lets the client show
    /// an honest status immediately (the same text is also a durable transcript line).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug)]
pub enum EngineError {
    Admit(AdmitError),
    Workspace(gaugewright_workspace::WorkspaceError),
    Harness(std::io::Error),
}
impl From<AdmitError> for EngineError {
    fn from(e: AdmitError) -> Self {
        EngineError::Admit(e)
    }
}
impl From<gaugewright_workspace::WorkspaceError> for EngineError {
    fn from(e: gaugewright_workspace::WorkspaceError) -> Self {
        EngineError::Workspace(e)
    }
}
/// Human-readable turn-failure text — what the turn routes surface as the HTTP
/// error body (`post_task` 502). The workspace/harness legs carry impl-minted
/// messages; an admission error has no Display and keeps its Debug rendering.
impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Admit(e) => write!(f, "{e:?}"),
            EngineError::Workspace(e) => write!(f, "{e}"),
            EngineError::Harness(e) => write!(f, "{e}"),
        }
    }
}

/// Run one task turn against an existing engagement worktree.
///
/// `transport` drives the engagement's Pi subprocess (real [`PiProcess`] in
/// production, a scripted transport in tests); `gate` is the membrane. The run
/// lifecycle is admitted into `store` under `scope`; the runtime-session is
/// seeded to `executing` and advanced by the turn.
///
/// [`PiProcess`]: gaugewright_pi_bridge::PiProcess
pub fn run_task<G: EgressGate>(
    store: &mut Store,
    scope: &str,
    engagement: &dyn ChatWorkspace,
    harness: &mut dyn Harness,
    gate: &G,
    task: &str,
    images: &[ImageContent],
) -> Result<TaskResult, EngineError> {
    run_task_streaming(
        store,
        scope,
        engagement,
        harness,
        gate,
        task,
        images,
        &mut |_| {},
    )
}

/// As [`run_task`], but `sink` receives each operational [`Observation`] as the
/// turn produces it — the control plane forwards these onto the live SSE stream.
/// `images` are native image content blocks sent to the harness as model input
/// for this turn; they are **never** recorded in the durable transcript.
#[allow(clippy::too_many_arguments)]
pub fn run_task_streaming<G: EgressGate>(
    store: &mut Store,
    scope: &str,
    engagement: &dyn ChatWorkspace,
    harness: &mut dyn Harness,
    gate: &G,
    task: &str,
    images: &[ImageContent],
    sink: &mut dyn FnMut(&Observation),
) -> Result<TaskResult, EngineError> {
    // Observability span (RF-A8): scope + task size only — never the task text or
    // any content (those are protected; the span is operational metadata). The
    // span covers the whole turn; a completion event records the outcome below.
    let _span = tracing::info_span!("engine.turn", scope, task_len = task.len()).entered();
    // 1. Admit the run into durable truth. Each task is a fresh run (ADR 0026):
    //    a fresh engagement begins from Init (requestRun); a subsequent turn
    //    re-enters from the prior run's terminal state (retryRun). Either way the
    //    run must be re-admitted before it can start (INV-11).
    let begin = match store.fold::<RunState>(scope)?.phase {
        RunPhase::Init => RunCommand::RequestRun,
        _ => RunCommand::RetryRun, // a terminal prior run → next turn
    };
    store.admit::<RunState>(scope, begin)?;
    store.admit::<RunState>(scope, RunCommand::AdmitRun)?;
    store.admit::<RunState>(scope, RunCommand::StartRun)?;

    // Admit the user message as durable transcript evidence (turn-boundary). The
    // transcript records the **raw** task; mode framing is invisible context the
    // model receives, not something the user typed.
    record_transcript(
        store,
        scope,
        &ServerEvent::User {
            text: task.to_string(),
        },
    );

    // 2. Drive one turn through the **harness** (ADR 0031) over the membrane. The
    //    harness owns its protocol + session; the prompt is the raw task (the agent's
    //    persona is its own `.pi/SYSTEM.md` in use mode, or the editor persona via
    //    Pi's `--system-prompt` in edit mode — see `run_engagement_turn`, ADR 0029).
    let outcome: TurnOutcome = harness
        .run_turn(gate, task, images, sink)
        .map_err(EngineError::Harness)?;

    // 3a. Admit the runtime's execution evidence into the run (INV-4): each tool
    //     decision the membrane ruled on is an observation that becomes standing
    //     run state only by this admission, while the run is still `running`.
    for _ in &outcome.observations {
        store.admit::<RunState>(scope, RunCommand::RecordObservation)?;
    }

    // 3b. Auto-commit the worktree (per-turn), then capture the reviewer's diff.
    let commit = engagement.commit_turn(task)?;
    let diff = engagement.diff_against_main()?;

    // 4. Map the turn outcome onto the run lifecycle: clean turn → completed,
    //    a Pi/stream error → failed. Either way the events are durable facts.
    let run_phase = if outcome.error.is_none() {
        store.admit::<RunState>(scope, RunCommand::CompleteRun)?;
        RunPhase::Completed
    } else {
        store.admit::<RunState>(scope, RunCommand::FailRun)?;
        RunPhase::Failed
    };

    // 5. Drive the merge lifecycle's start: re-enter + probe the branch-vs-`main`
    //    merge (no mutation). The human gates the advance later via the merge API.
    store.admit::<MergeState>(scope, MergeCommand::StartMerge)?;
    let probe = engagement.merge_probe()?;
    // UX-7: a test-only injection forces the conflict path (INV-24 isolate + repair context)
    // so a browser BDD can drive conflict-repair without staging a real git conflict.
    let merge_cmd = if force_merge_conflict() {
        MergeCommand::GitConflict
    } else {
        match probe {
            MergeOutcome::Clean => MergeCommand::GitClean,
            MergeOutcome::Conflict => MergeCommand::GitConflict,
        }
    };
    let merge = store.admit::<MergeState>(scope, merge_cmd)?;

    // 6. Record this turn's reads (every granted context resource) into the durable
    //    engagement read-set, then mint/refresh the derived output resource from it.
    //    Taint is engagement-scoped (ADR 0026): the output's stakeholders are the
    //    owners of everything the engagement has read across turns — sound even after
    //    a read context is later revoked or tombstoned — so a later export/review
    //    gates on persisted handles, not a loose stakeholder set.
    if let Ok(reads) = crate::resource_store::granted_context(store, scope) {
        let _ = crate::resource_store::record_reads(store, scope, &reads);
    }
    // The output is owned by the scope's authenticated owning authority
    // (`determine_scope_authority`, the SCOPE-AUTH-1 seam), not the hardcoded
    // local constant (MINT-1). In the single-user collapse a bare engagement
    // scope resolves to itself; under federation a `scope:<authority>:<rest>`
    // scope resolves to the authority the server authenticated for the call, so
    // a minted output is owned by — and governed by — the right keyset (D-REMOTE).
    let owner = gaugewright_core::determine_scope_authority(scope);
    let _ = crate::resource_store::mint_output(
        store,
        scope,
        owner.as_str(),
        commit.as_ref().map(|c| c.0.as_str()).unwrap_or_default(),
    );

    let blocked_effects: Vec<String> = outcome
        .observations
        .iter()
        .filter(|o| o.kind == "egress_blocked")
        .map(|o| o.detail.clone())
        .collect();

    // Admit the rest of the turn as durable transcript evidence, in order: each
    // boundary decision (tool line, its result, blocks) exactly as it streamed,
    // then the agent's final message, then the run outcome. Replaying the same
    // observations the live stream carried means a reloaded transcript keeps each
    // tool line's target/args/result — click-to-open survives the turn ending
    // (run-chat.md "live vs truth": the durable layer is the same reduction).
    for obs in &outcome.observations {
        match obs.kind {
            "egress" | "egress_staged" | "tool_result" | "egress_blocked" => {
                record_transcript(store, scope, &ServerEvent::from_observation(obs));
            }
            _ => {} // streamed text is operational-only; not durable evidence
        }
    }
    record_transcript(
        store,
        scope,
        &ServerEvent::Assistant {
            text: outcome.assistant_text.clone(),
        },
    );
    // A failed turn records *why* as durable evidence, so the user sees the reason
    // (e.g. a model rejecting an image) on the next snapshot — not just a generic
    // "didn't finish". The reason is diagnostic text, never protected content.
    if let Some(reason) = &outcome.error {
        record_transcript(
            store,
            scope,
            &ServerEvent::Error {
                reason: reason.clone(),
                code: None,
            },
        );
    }
    record_transcript(
        store,
        scope,
        &ServerEvent::Admitted {
            kind: "run".into(),
            text: format!("run → {run_phase:?}"),
        },
    );

    // Turn outcome as operational metadata only (counts + phases, no content).
    tracing::info!(
        ?run_phase,
        merge_phase = ?merge.phase,
        observations = outcome.observations.len(),
        mediated_tool_calls = outcome.mediated_tool_calls.len(),
        blocked_effects = blocked_effects.len(),
        pending_approvals = outcome.pending_approvals.len(),
        "engine.turn complete"
    );
    Ok(TaskResult {
        run_phase,
        assistant_text: outcome.assistant_text,
        diff,
        commit: commit.map(|c| c.0),
        merge_phase: merge.phase,
        mediated_tool_calls: outcome.mediated_tool_calls,
        blocked_effects,
        pending_approvals: outcome.pending_approvals,
        error: outcome.error,
    })
}

/// The result of one **remote-placed** turn (`ENGINE-REMOTE-1`). A turn that runs
/// in a *different* trust authority has no local worktree, so there is no local
/// commit / diff / merge to surface — the orchestrator's truth is the federated
/// observation count (each crossed the owner's bridge and was owner-admitted,
/// `INV-4`) and the minted output handle (owned by the scope's authority, MINT-1).
#[derive(Debug, serde::Serialize)]
pub struct RemoteTaskResult {
    pub run_phase: RunPhase,
    /// The peer endpoint the turn ran at (the relay resolves it, ADR 0020).
    pub remote_address: String,
    /// How many remote observations crossed the bridge and were owner-admitted.
    pub federated_observations: u32,
    /// The owning authority the derived output was minted under (MINT-1).
    pub output_owner: String,
}

/// Drive one task turn against a **remote-placed** runtime, wiring remote-harness
/// support into the engine orchestrator (`ENGINE-REMOTE-1`).
///
/// This is the remote sibling of [`run_task`]: instead of driving a local harness
/// and committing a worktree, it admits the run lifecycle, runs the turn on a
/// [`RemoteHarness`] in its own authority, and returns each observation **through
/// federation** ([`remote_runtime::federate_remote_turn`], `OBSERVATION-FEDERATION-1`)
/// so a relayed outcome becomes run truth only via the owner's admission (`INV-4`).
/// The derived output is minted under the scope's owning authority
/// ([`determine_scope_authority`](gaugewright_core::determine_scope_authority), MINT-1),
/// not the hardcoded local constant.
///
/// The single-process loopback shape ([`RemoteLoopbackHarness`]) and a real
/// cross-machine relay attach behind the same seam with no rearchitecture
/// (`RENDEZVOUS-STUB-1`).
///
/// [`RemoteLoopbackHarness`]: gaugewright_pi_bridge::RemoteLoopbackHarness
pub fn run_task_remote(
    store: &mut Store,
    scope: &str,
    harness: &mut dyn gaugewright_harness::RemoteHarness,
    gate: &dyn EgressGate,
    task: &str,
) -> Result<RemoteTaskResult, EngineError> {
    // 1. Admit the run into durable truth (same precondition as the local path):
    //    a fresh engagement begins from Init, a subsequent turn re-enters from the
    //    prior terminal state. Either way the run must be re-admitted (INV-11).
    let begin = match store.fold::<RunState>(scope)?.phase {
        RunPhase::Init => RunCommand::RequestRun,
        _ => RunCommand::RetryRun,
    };
    store.admit::<RunState>(scope, begin)?;
    store.admit::<RunState>(scope, RunCommand::AdmitRun)?;
    store.admit::<RunState>(scope, RunCommand::StartRun)?;
    record_transcript(
        store,
        scope,
        &ServerEvent::User {
            text: task.to_string(),
        },
    );

    let remote_address = harness.address().to_string();

    // 2. Run the turn in the remote authority and federate its observations back:
    //    each crosses the owner's bridge as a signed message over the relay seam
    //    and becomes standing run evidence only via the OWNER's admission (INV-4).
    //    A relay/transport failure fails the run; otherwise it completes.
    let federated_observations =
        match crate::remote_runtime::federate_remote_turn(store, scope, harness, gate, task) {
            Ok(count) => {
                store.admit::<RunState>(scope, RunCommand::CompleteRun)?;
                count
            }
            Err(crate::remote_runtime::RemoteRuntimeError::Admit(e)) => {
                return Err(EngineError::Admit(e))
            }
            Err(crate::remote_runtime::RemoteRuntimeError::Turn(e)) => {
                store.admit::<RunState>(scope, RunCommand::FailRun)?;
                return Err(EngineError::Harness(e));
            }
        };

    // 3. Record this turn's reads, then mint/refresh the derived output under the
    //    scope's owning authority (MINT-1) — the work is owned by, and governed by,
    //    the right keyset even though it ran in a different authority. There is no
    //    local commit, so the output's locator carries no commit hash.
    if let Ok(reads) = crate::resource_store::granted_context(store, scope) {
        let _ = crate::resource_store::record_reads(store, scope, &reads);
    }
    let owner = gaugewright_core::determine_scope_authority(scope);
    let _ = crate::resource_store::mint_output(store, scope, owner.as_str(), "");

    let run_phase = RunPhase::Completed;
    record_transcript(
        store,
        scope,
        &ServerEvent::Admitted {
            kind: "run".into(),
            text: format!("run → {run_phase:?}"),
        },
    );

    Ok(RemoteTaskResult {
        run_phase,
        remote_address,
        federated_observations,
        output_owner: owner.as_str().to_string(),
    })
}

/// Fail-closed model-credential check (LLM-1, [ADR 0062]): does a usable credential
/// resolve for `provider`? A **BYOK** provider needs its key linked (present in
/// `credential_envs`, resolved from the account's `SEC-4`-sealed store); an **OAuth**
/// provider (`openai-codex`, …) authenticates via the runtime adapter's own store,
/// which the turn's `factory` answers for ([`HarnessFactory::credential_status`]).
/// The refusal POLICY — whether a turn runs — stays here; the adapter only reports
/// its own state. Returns an **actionable** error when nothing resolves, so a real
/// run refuses up front instead of letting the runtime fail opaquely on a missing key.
fn llm_credential_status(
    provider: &str,
    credential_envs: &[(String, String)],
    factory: &dyn HarnessFactory,
) -> Result<(), String> {
    // BYOK: a provider with a linked-key env mapping — the one provider → env map,
    // [`crate::account::provider_env_var`] — needs its key resolved into the turn env.
    if let Some(var) = crate::account::provider_env_var(provider) {
        return if credential_envs.iter().any(|(k, _)| k == var) {
            Ok(())
        } else {
            Err(format!(
                "No {provider} key is linked, so this model can't run. Link an \
                 {provider} key in Account settings, or pick a different model."
            ))
        };
    }
    match provider {
        // Host-managed providers (SERVE-2 hosted embed, ADR 0064): the concrete gateway
        // secret names and provider routing live in the private managed-service host. The
        // open engine only requires a neutral readiness signal from that host.
        "cloudflare-ai-gateway" | "cloudflare-workers-ai" => {
            let get = |k: &str| -> Option<String> {
                credential_envs
                    .iter()
                    .find(|(kk, _)| kk == k)
                    .map(|(_, v)| v.clone())
                    .or_else(|| std::env::var(k).ok())
            };
            host_managed_model_status(provider, &get)
        }
        // OAuth providers authenticate via the adapter's own auth store.
        _ => match factory.credential_status(provider, credential_envs) {
            CredentialProbe::Ready => Ok(()),
            CredentialProbe::Missing(reason) => Err(reason),
        },
    }
}

/// Fail-closed check for host-managed model providers (SERVE-2 hosted embed, ADR
/// 0064): the private host validates and injects provider-specific config, then
/// reports a generic readiness flag to the open engine. Pure (takes a `get`
/// resolver) so it is unit-testable without process env or private secret names.
fn host_managed_model_status(
    provider: &str,
    get: &dyn Fn(&str) -> Option<String>,
) -> Result<(), String> {
    let ready = get("GAUGEWRIGHT_HOST_MODEL_READY")
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"));
    if ready {
        Ok(())
    } else {
        Err(format!(
            "The {provider} model can't run: the managed host has not reported model \
             readiness. Configure the private managed runtime, then set \
             GAUGEWRIGHT_HOST_MODEL_READY=1."
        ))
    }
}

/// Blocking: holds the workbench lock for the turn (local single-user MVP). SSE
/// subscribers already hold their receivers, so the live stream is unaffected.
#[allow(clippy::too_many_arguments)]
/// Record a fail-closed pre-flight refusal (LLM-1: no model credential resolves for
/// the turn) as a durable failure turn on `scope`: the user's message, then the reason
/// as a coded [`ServerEvent::Error`] line, with the run admitted through to `Failed`.
/// This mirrors the durable shape of a harness-level failure (see [`run_task_streaming`])
/// so the client's existing failed-turn handling surfaces it in the chat log uniformly
/// — and the `code` lets the client render an "open settings" action instead of plain
/// text. Returns the same `TaskResult { Failed, error }` an in-turn failure returns.
fn record_precheck_failure(
    store: &mut Store,
    scope: &str,
    task: &str,
    reason: String,
    code: &str,
) -> Result<TaskResult, String> {
    // The run starts then immediately fails on the gate — the same lifecycle a turn
    // that reaches the harness and errors admits (RequestRun→AdmitRun→StartRun→FailRun),
    // minus the observations no turn produced.
    let begin = match store
        .fold::<RunState>(scope)
        .map_err(|e| format!("{e:?}"))?
        .phase
    {
        RunPhase::Init => RunCommand::RequestRun,
        _ => RunCommand::RetryRun,
    };
    for cmd in [
        begin,
        RunCommand::AdmitRun,
        RunCommand::StartRun,
        RunCommand::FailRun,
    ] {
        store
            .admit::<RunState>(scope, cmd)
            .map_err(|e| format!("{e:?}"))?;
    }
    record_transcript(
        store,
        scope,
        &ServerEvent::User {
            text: task.to_string(),
        },
    );
    record_transcript(
        store,
        scope,
        &ServerEvent::Error {
            reason: reason.clone(),
            code: Some(code.to_string()),
        },
    );
    Ok(TaskResult {
        run_phase: RunPhase::Failed,
        assistant_text: String::new(),
        diff: String::new(),
        commit: None,
        merge_phase: MergePhase::Clean,
        mediated_tool_calls: Vec::new(),
        blocked_effects: Vec::new(),
        pending_approvals: Vec::new(),
        error: Some(reason),
    })
}

/// Drive one turn for an engagement, streaming observations live to its
/// broadcast `sender`. The engine resolves the turn's *policy* (mode framing,
/// credentials, provider/model, fail-closed precheck, base sandbox) into a
/// [`HarnessSpec`]; the runtime itself is constructed by the factory the
/// per-turn selector picks ([`crate::harness_select::factory_for_turn`] — the
/// real Pi adapter, or the scripted fake under `GAUGEWRIGHT_FAKE_AGENT`).
/// Returns the turn result, or a human-readable error (the model endpoint may
/// be unauthenticated/offline).
///
/// Blocking: holds the workbench lock for the turn (local single-user MVP). SSE
/// subscribers already hold their receivers, so the live stream is unaffected.
pub fn run_engagement_turn(
    wb: &SharedWorkbench,
    id: &str,
    worktree: &Path,
    sender: &broadcast::Sender<ServerEvent>,
    task: &str,
    images: &[ImageContent],
    mode: ChatMode,
) -> Result<TaskResult, String> {
    let config =
        AgentConfig::from_file(&worktree.join(definition::CONFIG_PATH)).unwrap_or_default();
    let gate = MembraneGate::new(&config, default_external_tools()).with_mode(mode);

    // A linked provider account (ACCT-1) is injected into the runtime so it is actually
    // used: resolve the operator's sealed credentials to provider env vars now (no lock
    // is held here), and add them to the Pi env below. Empty when nothing is linked.
    // Nearest-scope-wins (LLM-2, ADR 0062): a credential pinned in *this chat's project*
    // overrides the account default per provider.
    let credential_envs: Vec<(String, String)> = {
        let g = wb.lock_unpoisoned();
        g.resolved_credential_envs_for_chat(id)
    };

    // The agent's persona is a Pi-native method resource (ADR 0029), not something
    // gaugewright prepends to the prompt:
    //   - use mode: no override — Pi discovers the agent's own `.pi/SYSTEM.md` +
    //     `AGENTS.md` from the worktree;
    //   - edit mode: the editor persona is passed as Pi's `--system-prompt`, so it
    //     *replaces* the agent's own SYSTEM.md for the authoring turn.
    let system_prompt: Option<String> = match mode {
        ChatMode::Edit => Some(EDITOR_FRAMING.to_string()),
        ChatMode::Use => None,
    };

    // The one harness decision point (SUB-0): which adapter drives this turn.
    // Consulted per turn — tests flip `GAUGEWRIGHT_FAKE_AGENT` against a live
    // workbench, so the selection must never be cached at startup.
    let factory = crate::harness_select::factory_for_turn();

    // Mock-LLM mode: no Pi spawn, no model call. The scripted fake drives the
    // exact same turn loop (membrane + reducers unchanged); its pre-turn side
    // effects — the `[slow]` hold and the note append — run here, in the
    // blocking pool BEFORE any lock is taken (see `ScriptedFakeFactory::pre_turn`).
    let result = if factory.kind() == ScriptedFakeFactory::KIND {
        ScriptedFakeFactory::pre_turn(worktree, task)?;
        // The fake ignores the runtime config; the spec carries the shell's
        // minimal base policy for the seam's sake. Provider resolution and the
        // fail-closed credential precheck are real-run policy, skipped here as
        // before.
        let spec = HarnessSpec {
            chat_id: id.to_string(),
            worktree: worktree.to_path_buf(),
            mode,
            provider: None,
            model: None,
            thinking: None,
            system_prompt,
            credentials: credential_envs,
            sandbox: gaugewright_harness::sandbox::SandboxPolicy::new(vec![worktree.to_path_buf()]),
        };
        drive_persistent_turn(wb, id, &gate, task, images, sender, factory.as_ref(), &spec)?
    } else {
        // Resolve the turn's provider/model. A **deployment host** (SERVE-2 hosted embed)
        // forces every turn through its egress membrane by setting `GAUGEWRIGHT_MODEL_PROVIDER`
        // (+ `GAUGEWRIGHT_MODEL`) in the image — that OVERRIDES the archetype's authored
        // provider so a method published with `openai-codex` still routes via the Cloudflare AI
        // Gateway in the sandbox (ADR 0064 membrane; provider-token details remain in the
        // private managed-service host). Unset on the desktop, so the chat's
        // `.agent-config.json` provider (or the codex default)
        // wins there. Pure resolvers so the precedence is unit-testable.
        let provider = resolve_turn_provider(
            std::env::var("GAUGEWRIGHT_MODEL_PROVIDER").ok(),
            config.provider.clone(),
        );
        let model = resolve_turn_model(
            std::env::var("GAUGEWRIGHT_MODEL").ok(),
            config.model.clone(),
        );
        // Fail closed (LLM-1, ADR 0062): refuse a real run when no model credential resolves for
        // the resolved provider. Record it as a durable, coded failure turn so the chat log
        // shows *why* with an actionable "open settings" affordance — not just a status line —
        // then return the same Failed shape an in-turn failure returns (never let Pi fail opaquely).
        if let Err(reason) = llm_credential_status(&provider, &credential_envs, factory.as_ref()) {
            let _ = sender.send(ServerEvent::Error {
                reason: reason.clone(),
                code: Some("no_credential".into()),
            });
            let mut g = wb.lock_unpoisoned();
            return record_precheck_failure(&mut g.store, id, task, reason, "no_credential");
        }

        // The OS sandbox the turn runs under (ADR 0030): the worktree is writable;
        // in use mode the definition surface is re-imposed read-only, so a write by
        // ANY tool — edit, write, or bash — fails at the kernel. This is the
        // load-bearing INV-24 enforcement. It is the SHELL's base policy only: the
        // adapter extends it with its private needs (Pi adds its session dir +
        // `~/.pi` as writable roots — see the Pi factory's `pi_config_for`).
        let sandbox_policy = {
            let writable = vec![worktree.to_path_buf()];
            // Network egress posture (RF-B3) is a **per-project** choice, open by
            // default: the operator opts *into* isolation per project, so a chat can
            // reach the model out of the box. The core `SandboxPolicy` still defaults
            // deny (the library invariant is unchanged); we flip the app default here
            // by acknowledging unfiltered egress unless this chat's project isolates.
            // The one egress the bridge legitimately needs is the model endpoint, so
            // we name it explicitly — recorded and auditable, not ambient. Because the
            // per-host egress proxy is not yet built (deferred infra), the kernel can
            // only deny *all* or allow *all*, so "open" is UNFILTERED egress (M-1).
            // `GAUGEWRIGHT_ALLOW_UNFILTERED_EGRESS=1` still force-opens regardless of the
            // project setting, mirroring the conscious `GAUGEWRIGHT_SANDBOX=0` opt-out.
            let egress_hosts = model_endpoint_hosts(Some(&provider));
            let project_isolated = wb.lock_unpoisoned().chat_network_isolated(id);
            let forced_open =
                std::env::var("GAUGEWRIGHT_ALLOW_UNFILTERED_EGRESS").as_deref() == Ok("1");
            let unfiltered_egress_ack = forced_open || !project_isolated;
            if !unfiltered_egress_ack {
                eprintln!(
                    "[gaugewright] NOTE: this project isolates Pi's network (fail-closed); \
                     the model endpoint ({}) is unreachable. Turn off isolation for \
                     the project to let the agent reach the model.",
                    egress_hosts.join(", ")
                );
            } else if !project_isolated {
                eprintln!(
                    "[gaugewright] NOTE: Pi runs with UNFILTERED network egress (project \
                     network is open; the per-host egress proxy is not yet built, so \
                     the agent can reach ANY host — not just the model endpoint {}). \
                     Isolate the project to fail closed.",
                    egress_hosts.join(", ")
                );
            }
            gaugewright_harness::sandbox::SandboxPolicy::new(writable)
                .read_only(method_surface_readonly_roots(worktree, mode))
                .allow_hosts(egress_hosts)
                .allow_unfiltered_egress(unfiltered_egress_ack)
        };
        let spec = HarnessSpec {
            chat_id: id.to_string(),
            worktree: worktree.to_path_buf(),
            mode,
            // Pin the codex endpoint by default (the authed OAuth provider) so a bare
            // model name can't silently resolve to an unauthenticated provider. Resolved
            // once above for the fail-closed credential check.
            provider: Some(provider),
            model,
            // Per-chat reasoning effort (LLM-1, ADR 0062): unset → Pi's per-model default.
            thinking: config.thinking.clone(),
            // Edit mode replaces the agent's own SYSTEM.md with the editor persona;
            // use mode leaves it unset so the adapter discovers the agent's definition.
            system_prompt,
            // A linked provider account (ACCT-1), if any — resolved above,
            // nearest-scope-wins (LLM-2, ADR 0062).
            credentials: credential_envs,
            sandbox: sandbox_policy,
        };
        drive_persistent_turn(wb, id, &gate, task, images, sender, factory.as_ref(), &spec)?
    };

    // WS-D: if this chat is homed to a workstream, greedily auto-sync its clean turn
    // into the stream main and let siblings pick it up — the low-friction collaboration
    // hop. A non-member chat (target `main`) is untouched: its merge stays Clean for the
    // human's review, exactly as before.
    greedy_autosync(wb, id, sender);

    let _ = sender.send(ServerEvent::Admitted {
        kind: "run".into(),
        text: format!("run → {:?}", result.run_phase),
    });
    Ok(result)
}

/// Drive one **visitor turn** for a public session (SERVE-1 hosted embed). This is the embed
/// plane's turn entrypoint: it bridges the public-session lifecycle to the same turn engine the
/// desktop uses, with two embed-specific rules.
///
/// 1. **Fail-closed on lifecycle phase** (`INV-20`): a turn runs ONLY when the session is
///    `Active`. A not-yet-activated, expiring, or torn-down session is refused — never run.
/// 2. **Always Use mode** — the audience *uses* the deployed archetype and never edits the
///    method (embed spec: end-users drive work through chat, not file edits; the method surface
///    stays read-only, `INV-24`).
///
/// The session's turns run in a Workbench engagement of the **same id** (`session_id ==
/// engagement id`) in the deployment's workspace, so `run_engagement_turn` finds the worktree
/// and folds the run into that scope. The model provider is the host-forced one
/// ([`resolve_turn_provider`] in a SERVE-2 image), with provider-token details
/// owned by the private managed-service host (ADR 0064 membrane).
#[allow(clippy::too_many_arguments)]
pub fn run_public_turn(
    wb: &SharedWorkbench,
    session_phase: gaugewright_core::public_session::Phase,
    session_id: &str,
    worktree: &Path,
    sender: &broadcast::Sender<ServerEvent>,
    task: &str,
    images: &[ImageContent],
) -> Result<TaskResult, String> {
    public_turn_allowed(session_phase)?;
    run_engagement_turn(
        wb,
        session_id,
        worktree,
        sender,
        task,
        images,
        ChatMode::Use,
    )
}

/// Fail-closed gate for a public-session turn (`INV-20`): only an `Active` session may run a
/// turn. Pure, so the policy is unit-testable independently of the engine. Returns an actionable
/// reason naming the phase so the embed surface can show "temporarily unavailable" rather than a
/// crash.
fn public_turn_allowed(phase: gaugewright_core::public_session::Phase) -> Result<(), String> {
    use gaugewright_core::public_session::Phase;
    if phase == Phase::Active {
        Ok(())
    } else {
        Err(format!(
            "public session is not active (phase {phase:?}); refusing the turn (fail-closed)"
        ))
    }
}

/// The greedy auto-sync hop (`WS-D`). When the just-finished turn's chat is a member of
/// a workstream (its worktree targets `workstream/<id>/main`, not `main`) **and** its
/// merge probe came back Clean, this:
///   1. admits the membership-gated `Contribute` on the workstream scope (attribution +
///      the gate: a non-member or archived stream is rejected, and we bail);
///   2. auto-admits the clean merge into the stream main (PolicyAdmit → real merge →
///      AdvanceStandingRef) — the auto-admit-in-stream policy that makes it feel
///      automatic while every advance stays an admitted event (`INV-2`/`INV-4`);
///   3. has every sibling member of the same stream `sync_from_main`, picking the work up.
///
/// A conflict at any step leaves that contribution isolated for the existing merge repair
/// flow — the shared ref only ever advances on a clean merge.
fn greedy_autosync(wb: &SharedWorkbench, id: &str, sender: &broadcast::Sender<ServerEvent>) {
    let mut g = wb.lock_unpoisoned();
    g.greedy_autosync(id, sender);
}

impl Workbench {
    // Membership is encoded in the worktree target: `main` ⇒ not a member, nothing to do.
    fn greedy_autosync(&mut self, id: &str, sender: &broadcast::Sender<ServerEvent>) {
        use gaugewright_core::workstream::{WorkstreamCommand, WorkstreamState};
        let Some(target) = self.engagements.get(id).map(|e| e.target().to_string()) else {
            return;
        };
        // The owning workspace impl parses the ref token — the engine holds no
        // ref-format knowledge (W7).
        let Some(ws_id) = self
            .engagement_index
            .get(id)
            .and_then(|iid| self.instances.get(iid))
            .and_then(|inst| inst.workstream_id_of(&target))
        else {
            return;
        };
        let store = &mut self.store;
        let engagements = &mut self.engagements;

        // Only a clean turn advances the stream; a conflict stays isolated (the merge
        // reducer already moved it to Rejected/Repairing) for repair.
        if store.fold::<MergeState>(id).map(|m| m.phase).ok() != Some(MergePhase::Clean) {
            return;
        }
        // The membership gate + attribution. A rejection (non-member / archived) means this
        // chat may not advance the stream — leave the merge Clean for the human instead. The
        // contribution is attributed to the scope's authority (WS-G): the local hub authority
        // for a local turn, the crossing authority for a remote-driven one.
        let by = gaugewright_core::determine_scope_authority(id)
            .as_str()
            .to_string();
        if store
            .admit::<WorkstreamState>(
                &ws_id,
                WorkstreamCommand::Contribute {
                    chat: id.to_string(),
                    by,
                },
            )
            .is_err()
        {
            return;
        }
        // Auto-admit the clean merge into the stream main.
        if store
            .admit::<MergeState>(id, MergeCommand::PolicyAdmit)
            .is_err()
        {
            return;
        }
        match engagements.get(id).map(|e| e.merge_into_main()) {
            Some(Ok(MergeOutcome::Clean)) => {
                let _ = store.admit::<MergeState>(id, MergeCommand::AdvanceStandingRef);
            }
            // Raced with another writer — leave it for the review surface rather than forcing.
            _ => return,
        }
        record_transcript(
            store,
            id,
            &ServerEvent::Admitted {
                kind: "merge".into(),
                text: "synced into the workstream".into(),
            },
        );
        let _ = sender.send(ServerEvent::Admitted {
            kind: "merge".into(),
            text: "synced into the workstream".into(),
        });

        // Sibling auto-pull: every other member of the same stream picks the work up. A
        // sibling conflict aborts cleanly (its worktree is unchanged) and surfaces on its
        // next interaction — the shared ref is unaffected.
        let siblings: Vec<String> = engagements
            .iter()
            .filter(|(cid, e)| cid.as_str() != id && e.target() == target)
            .map(|(cid, _)| cid.clone())
            .collect();
        for sib in siblings {
            if let Some(se) = engagements.get(&sib) {
                let _ = se.sync_from_main();
            }
        }
    }
}

/// The sink that fans a turn's observations onto the live `sender` (skipping
/// internal lifecycle progress).
fn live_sink(sender: &broadcast::Sender<ServerEvent>) -> impl FnMut(&Observation) + '_ {
    move |obs: &Observation| {
        if obs.kind == "progress" {
            return;
        }
        let _ = sender.send(ServerEvent::from_observation(obs));
    }
}

/// Drive one turn over the engagement's session, constructed by `factory` from
/// `spec`. A caching adapter's harness ([`HarnessFactory::reuse_across_turns`],
/// Pi) is **persistent** — created on the first turn and reused thereafter, so
/// the conversation thread carries context across turns (`pi-rpc.md`); a turn
/// that errors retires the (likely dead) harness so the next turn recreates a
/// fresh thread. A non-caching adapter (the scripted fake) gets a fresh harness
/// every turn.
#[allow(clippy::too_many_arguments)]
fn drive_persistent_turn(
    wb: &SharedWorkbench,
    id: &str,
    gate: &MembraneGate,
    task: &str,
    images: &[ImageContent],
    sender: &broadcast::Sender<ServerEvent>,
    factory: &dyn HarnessFactory,
    spec: &HarnessSpec,
) -> Result<TaskResult, String> {
    let mut g = wb.lock_unpoisoned();
    g.drive_persistent_local_turn(id, gate, task, images, sender, factory, spec)
}

impl Workbench {
    #[allow(clippy::too_many_arguments)]
    fn drive_persistent_local_turn(
        &mut self,
        id: &str,
        gate: &MembraneGate,
        task: &str,
        images: &[ImageContent],
        sender: &broadcast::Sender<ServerEvent>,
        factory: &dyn HarnessFactory,
        spec: &HarnessSpec,
    ) -> Result<TaskResult, String> {
        let store = &mut self.store;
        let engagements = &self.engagements;
        let sessions = &mut self.sessions;
        let eng = engagements
            .get(id)
            .ok_or_else(|| "engagement gone".to_string())?;

        // A non-caching adapter never enters the session map: a fresh harness
        // per turn (the scripted fake's one-shot transport — caching it would
        // fail turn 2 with "stream ended"), dropped when the turn ends.
        if !factory.reuse_across_turns() {
            let mut harness = factory
                .create(spec)
                .map_err(|e| format!("spawn {}: {e}", factory.kind()))?;
            let mut sink = live_sink(sender);
            return run_task_streaming(
                store,
                id,
                eng.as_ref(),
                harness.as_mut(),
                gate,
                task,
                images,
                &mut sink,
            )
            .map_err(|e| e.to_string());
        }

        if !sessions.contains_key(id) {
            let harness = factory
                .create(spec)
                .map_err(|e| format!("spawn {}: {e}", factory.kind()))?;
            sessions.insert(id.to_string(), harness);
        }
        let harness: &mut dyn Harness =
            sessions.get_mut(id).expect("session just ensured").as_mut();
        // Publish this turn's interrupt handle so a concurrent Stop can terminate it
        // out-of-band (unblocking `recv`) — the registry is outside the workbench lock
        // this turn holds. A harness with nothing to interrupt registers nothing.
        if let Some(interrupt) = harness.interrupt_handle() {
            register_running_turn(id, interrupt);
        }

        let mut sink = live_sink(sender);
        let result = run_task_streaming(
            store,
            id,
            eng.as_ref(),
            harness,
            gate,
            task,
            images,
            &mut sink,
        )
        .map_err(|e| e.to_string());
        clear_running_turn(id);

        // A turn that errored (or was Stop-killed: its `recv` hit EOF and reported a
        // stream error) retires the now-dead process so the next turn respawns. A
        // Stop-killed turn reports `outcome.error`, so retire that too.
        let stream_died = result
            .as_ref()
            .map(|r| r.run_phase == RunPhase::Failed)
            .unwrap_or(true);
        if stream_died {
            if let Some(dead) = sessions.remove(id) {
                let _ = dead.shutdown();
            }
        }
        result
    }
}

/// Drive one turn over an engagement's **remote** session — a runtime placed in a
/// different trust authority and held in the workbench's `remote_sessions` map
/// alongside the local ones (`WORKBENCH-REMOTE-1`). This is the workbench-level
/// sibling of [`drive_persistent_turn`]: it pulls the registered
/// [`RemoteHarness`](gaugewright_harness::RemoteHarness) for `id` and routes the turn
/// through [`run_task_remote`] (`ENGINE-REMOTE-1`), so the remote outcome becomes
/// run truth only via the owner's federated admission (`INV-4`). The remote path
/// has no local worktree, so there is no commit/diff/merge to surface.
pub fn drive_remote_turn(
    wb: &SharedWorkbench,
    id: &str,
    gate: &dyn EgressGate,
    task: &str,
) -> Result<RemoteTaskResult, String> {
    let mut g = wb.lock_unpoisoned();
    g.drive_registered_remote_turn(id, gate, task)
}

impl Workbench {
    fn drive_registered_remote_turn(
        &mut self,
        id: &str,
        gate: &dyn EgressGate,
        task: &str,
    ) -> Result<RemoteTaskResult, String> {
        let store = &mut self.store;
        let remote_sessions = &mut self.remote_sessions;
        let harness = remote_sessions
            .get_mut(id)
            .ok_or_else(|| format!("no remote session for {id}"))?;
        run_task_remote(store, id, harness.as_mut(), gate, task).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fake_agent_env;
    use gaugewright_pi_bridge::{run_rpc_turn, RpcTransport, ScriptedTransport};
    use gaugewright_workspace::Instance;
    use std::collections::VecDeque;
    use std::io;

    // Fail-closed credential check (LLM-1, ADR 0062): a BYOK provider needs its key
    // present in the resolved env; absent ⇒ an actionable refusal, never a silent run.
    // The BYOK leg is shell policy — the factory is never consulted for it.
    #[test]
    fn byok_provider_requires_its_linked_key() {
        let pi = gaugewright_pi_bridge::PiHarnessFactory;
        let with_openai = vec![("OPENAI_API_KEY".to_string(), "sk-x".to_string())];
        assert!(llm_credential_status("openai", &with_openai, &pi).is_ok());
        // anthropic needs ITS key, not openai's
        assert!(llm_credential_status("anthropic", &with_openai, &pi).is_err());
        // nothing linked ⇒ refused with an actionable message
        let err = llm_credential_status("anthropic", &[], &pi).unwrap_err();
        assert!(err.contains("anthropic"), "names the provider: {err}");
        assert!(
            err.to_lowercase().contains("account settings"),
            "points to the fix: {err}"
        );
    }

    // Host-managed provider (SERVE-2 hosted embed, ADR 0064): the fail-closed
    // check trusts only a neutral readiness signal from the private host, keeping
    // provider-specific secret names out of the open engine.
    #[test]
    fn host_managed_provider_requires_host_readiness() {
        use std::collections::HashMap;
        let env = |pairs: &[(&str, &str)]| {
            let m: HashMap<String, String> = pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            move |k: &str| m.get(k).cloned()
        };

        let ready = env(&[("GAUGEWRIGHT_HOST_MODEL_READY", "1")]);
        assert!(host_managed_model_status("cloudflare-ai-gateway", &ready).is_ok());

        let not_ready = env(&[]);
        let err = host_managed_model_status("cloudflare-ai-gateway", &not_ready).unwrap_err();
        assert!(
            err.contains("GAUGEWRIGHT_HOST_MODEL_READY"),
            "names the readiness flag: {err}"
        );

        let false_value = env(&[("GAUGEWRIGHT_HOST_MODEL_READY", "0")]);
        assert!(host_managed_model_status("cloudflare-workers-ai", &false_value).is_err());
    }

    // A SERVE-2 deployment host forces every turn's provider/model (so a method authored with
    // `openai-codex` still egresses via the gateway); absent the override the chat's config wins.
    #[test]
    fn host_override_wins_then_config_then_default() {
        // Host override beats everything (the SERVE-2 membrane).
        assert_eq!(
            resolve_turn_provider(
                Some("cloudflare-ai-gateway".into()),
                Some("anthropic".into())
            ),
            "cloudflare-ai-gateway"
        );
        // No override ⇒ the chat's configured provider.
        assert_eq!(
            resolve_turn_provider(None, Some("anthropic".into())),
            "anthropic"
        );
        // Neither ⇒ the codex OAuth default. An empty override/config is ignored, not honored.
        assert_eq!(
            resolve_turn_provider(Some(String::new()), None),
            "openai-codex"
        );
        // Model: override wins, else config, else None (Pi's per-provider default).
        assert_eq!(
            resolve_turn_model(Some("claude-3.5-sonnet".into()), Some("gpt-x".into())).as_deref(),
            Some("claude-3.5-sonnet")
        );
        assert_eq!(
            resolve_turn_model(None, Some("gpt-x".into())).as_deref(),
            Some("gpt-x")
        );
        assert_eq!(resolve_turn_model(Some(String::new()), None), None);
    }

    // A public-session turn is fail-closed on the lifecycle phase (INV-20): only Active runs.
    #[test]
    fn public_turn_only_runs_when_active() {
        use gaugewright_core::public_session::Phase;
        assert!(public_turn_allowed(Phase::Active).is_ok());
        for p in [Phase::Init, Phase::Opened, Phase::Expiring, Phase::TornDown] {
            let err = public_turn_allowed(p).unwrap_err();
            assert!(
                err.contains("not active"),
                "actionable refusal for {p:?}: {err}"
            );
        }
    }

    // The hosted turn path end-to-end (fake agent): an Active public session drives a real turn
    // in its same-id engagement; a non-active session is refused before any work runs.
    #[test]
    fn run_public_turn_drives_an_active_session_and_refuses_otherwise() {
        use gaugewright_core::public_session::Phase;
        use std::sync::{Arc, Mutex};
        use tokio::sync::broadcast;

        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
        let eng = inst.create_engagement("sess-1").unwrap();
        let worktree = eng.path().to_path_buf();
        let store = Store::open_in_memory().unwrap();
        let wb = Arc::new(Mutex::new(crate::Workbench::with_instance(
            "inst-test",
            inst,
            store,
        )));
        wb.lock()
            .unwrap()
            .register_engagement("sess-1", "inst-test", Box::new(eng));
        let (tx, _rx) = broadcast::channel(16);

        // Not active ⇒ refused, no turn, no worktree mutation.
        let refused = run_public_turn(&wb, Phase::Opened, "sess-1", &worktree, &tx, "hi", &[]);
        assert!(refused.unwrap_err().contains("not active"));
        assert!(
            !worktree.join("agent-note.txt").exists(),
            "no turn ran for an inactive session"
        );

        // Active ⇒ a real turn runs (Use mode), producing a diff.
        let _fake_agent = fake_agent_env();
        let ok =
            run_public_turn(&wb, Phase::Active, "sess-1", &worktree, &tx, "do it", &[]).unwrap();
        assert_eq!(ok.run_phase, RunPhase::Completed);
        assert!(ok.diff.contains("agent-note.txt"), "real diff: {}", ok.diff);
    }

    // The egress allowlist routes Cloudflare providers to Cloudflare's hosts only — the upstream
    // model key never reaches the sandbox (ADR 0064 membrane).
    #[test]
    fn model_endpoint_hosts_allows_cloudflare_gateway() {
        let hosts = model_endpoint_hosts(Some("cloudflare-ai-gateway"));
        assert!(
            hosts.iter().any(|h| h == "gateway.ai.cloudflare.com"),
            "gateway host: {hosts:?}"
        );
        // The gateway proxies upstream server-side, so the upstream endpoints are NOT opened.
        assert!(
            !hosts.iter().any(|h| h == "api.anthropic.com"),
            "no upstream egress: {hosts:?}"
        );
        // Workers AI direct resolves to the Cloudflare API host.
        assert!(model_endpoint_hosts(Some("cloudflare-workers-ai"))
            .iter()
            .any(|h| h == "api.cloudflare.com"));
    }

    /// A scripted Pi transport: canned stdout lines in, sent commands recorded.
    struct Scripted {
        out: VecDeque<String>,
        sent: Vec<String>,
    }
    impl Scripted {
        fn new(lines: &[&str]) -> Self {
            Self {
                out: lines.iter().map(|s| s.to_string()).collect(),
                sent: Vec::new(),
            }
        }
    }
    impl RpcTransport for Scripted {
        fn send(&mut self, line: &str) -> io::Result<()> {
            self.sent.push(line.to_string());
            Ok(())
        }
        fn recv(&mut self) -> io::Result<Option<String>> {
            Ok(self.out.pop_front())
        }
    }
    // The scripted transport is also a Harness (ADR 0031) so tests drive the engine
    // through the same harness-agnostic seam the real Pi adapter uses.
    impl Harness for Scripted {
        fn run_turn(
            &mut self,
            gate: &dyn EgressGate,
            prompt: &str,
            images: &[ImageContent],
            sink: &mut dyn FnMut(&Observation),
        ) -> io::Result<TurnOutcome> {
            run_rpc_turn(self, gate, prompt, images, sink)
        }
    }

    /// INV-24 through the app-side adapter: `MembraneGate.with_mode` maps the chat
    /// mode to the boundary write-gate, and the tool target is threaded in.
    #[test]
    fn membrane_gate_enforces_the_edit_use_write_gate() {
        use gaugewright_harness::GateDecision;
        let cfg = AgentConfig::default();
        let use_gate = MembraneGate::new(&cfg, default_external_tools()).with_mode(ChatMode::Use);
        // use mode: writing the definition surface is blocked…
        assert!(matches!(
            use_gate.classify_tool("edit", Some(".pi/SYSTEM.md")),
            GateDecision::Block(_)
        ));
        assert!(matches!(
            use_gate.classify_tool("write", Some("AGENTS.md")),
            GateDecision::Block(_)
        ));
        assert!(matches!(
            use_gate.classify_tool("edit", Some(".agent-config.json")),
            GateDecision::Block(_)
        ));
        // …but ordinary work is allowed, and reading its own definition is allowed.
        assert!(matches!(
            use_gate.classify_tool("edit", Some("src/main.rs")),
            GateDecision::Allow
        ));
        assert!(matches!(
            use_gate.classify_tool("read", Some(".pi/SYSTEM.md")),
            GateDecision::Allow
        ));

        // edit mode: the editor may write the definition surface.
        let edit_gate = MembraneGate::new(&cfg, default_external_tools()).with_mode(ChatMode::Edit);
        assert!(matches!(
            edit_gate.classify_tool("edit", Some(".pi/SYSTEM.md")),
            GateDecision::Allow
        ));
    }

    /// ADR 0030: the sandbox read-only roots = the existing definition surface in
    /// use mode, and nothing in edit mode (the editor edits the definition).
    #[test]
    fn method_surface_readonly_roots_use_vs_edit() {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();
        std::fs::create_dir_all(wt.join(".pi")).unwrap();
        std::fs::write(wt.join(".pi/SYSTEM.md"), "x").unwrap();
        std::fs::write(wt.join("AGENTS.md"), "y").unwrap();
        // .agent-config.json absent → not included (a bind needs an existing source)

        let ro = method_surface_readonly_roots(wt, ChatMode::Use);
        assert!(
            ro.contains(&wt.join(".pi")),
            "use mode protects .pi: {ro:?}"
        );
        assert!(ro.contains(&wt.join("AGENTS.md")));
        assert!(
            !ro.iter().any(|p| p.ends_with(".agent-config.json")),
            "absent file skipped"
        );

        assert!(
            method_surface_readonly_roots(wt, ChatMode::Edit).is_empty(),
            "edit mode leaves the definition writable"
        );
    }

    /// The Phase-2 gate, end-to-end and headless: a default agent works in a
    /// worktree via (scripted) Pi, auto-commits, produces a diff + output — and
    /// the membrane blocks an out-of-policy effect.
    #[test]
    fn canonical_loop_works_in_worktree_and_blocks_out_of_policy_effect() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
        let eng = inst.create_engagement("e1").unwrap();

        // default agent: trust-by-default in-workspace, but `bash` is blocked.
        let config = AgentConfig::from_json(
            r#"{ "model": "gpt-5.5", "policy": { "block_tools": ["bash"] } }"#,
        )
        .unwrap();
        let gate = MembraneGate::new(&config, BTreeSet::new());

        // Pi: edits a file (in-policy), then attempts bash (blocked), then ends.
        // The edit's effect on the worktree is simulated by the test writing the
        // file — the bridge mediates the *decision*, the plugin does the write.
        std::fs::write(eng.path().join("answer.txt"), "42\n").unwrap();
        let mut transport = Scripted::new(&[
            r#"{"type":"agent_start"}"#,
            r#"{"type":"text_delta","delta":"Writing the answer."}"#,
            r#"{"type":"tool_execution_start","toolCallId":"t1","toolName":"write","args":{}}"#,
            r#"{"type":"tool_execution_end","toolCallId":"t1"}"#,
            r#"{"type":"tool_execution_start","toolCallId":"t2","toolName":"bash","args":{}}"#,
            r#"{"type":"agent_end","messages":[]}"#,
            r#"{"type":"response","command":"get_last_assistant_text","success":true,"data":{"text":"Done. The answer is 42."}}"#,
        ]);

        let mut store = Store::open_in_memory().unwrap();
        let result = run_task(
            &mut store,
            "eng-1",
            &eng,
            &mut transport,
            &gate,
            "write the answer",
            &[],
        )
        .unwrap();

        // the run completed and is durable in the log
        assert_eq!(result.run_phase, RunPhase::Completed);
        assert_eq!(
            store.fold::<RunState>("eng-1").unwrap().phase,
            RunPhase::Completed
        );

        // it produced output + a diff, auto-committed in the worktree
        assert_eq!(result.assistant_text, "Done. The answer is 42.");
        assert!(result.commit.is_some(), "the turn auto-committed");
        assert!(result.diff.contains("answer.txt") && result.diff.contains("42"));

        // the in-policy write was mediated; the out-of-policy bash was blocked
        assert_eq!(result.mediated_tool_calls, vec!["write".to_string()]);
        assert!(
            result.blocked_effects.iter().any(|b| b.contains("bash")),
            "the membrane blocked the out-of-policy effect: {:?}",
            result.blocked_effects
        );

        // keeping the work merges it into main
        eng.merge_into_main().unwrap();
        assert!(inst.repo().join("answer.txt").exists());
    }

    /// The durable transcript keeps each tool line's target/args/result, so a
    /// reloaded chat stays clickable (run-chat.md click-to-open survives the turn).
    #[test]
    fn durable_transcript_keeps_tool_target_and_result() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
        let eng = inst.create_engagement("e1").unwrap();
        let gate = MembraneGate::new(&AgentConfig::default(), default_external_tools());
        let mut transport = Scripted::new(&[
            r#"{"type":"tool_execution_start","toolCallId":"t1","toolName":"write","args":{"path":"answer.txt"}}"#,
            r#"{"type":"tool_execution_end","toolCallId":"t1","result":"wrote 1 file","isError":false}"#,
            r#"{"type":"agent_end","messages":[]}"#,
            r#"{"type":"response","command":"get_last_assistant_text","success":true,"data":{"text":"ok"}}"#,
        ]);
        let mut store = Store::open_in_memory().unwrap();
        run_task(&mut store, "eng-1", &eng, &mut transport, &gate, "go", &[]).unwrap();

        let rows = store.records("eng-1", "transcript").unwrap();
        let joined = rows.join("\n");
        assert!(
            joined.contains(r#""type":"tool""#),
            "a durable tool line: {joined}"
        );
        assert!(
            joined.contains(r#""target":"answer.txt""#),
            "tool target survives: {joined}"
        );
        assert!(
            joined.contains(r#""type":"toolresult""#),
            "the result is durable: {joined}"
        );
        assert!(
            joined.contains("wrote 1 file"),
            "the result body survives: {joined}"
        );
    }

    /// A failed turn surfaces *why*: the runtime error becomes `TaskResult.error`
    /// AND a durable `error` transcript record — so the user sees the reason (e.g. a
    /// model rejecting an image), not just a generic "didn't finish" (UX-14).
    #[test]
    fn a_failed_turn_records_its_error_reason() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
        let eng = inst.create_engagement("e1").unwrap();
        let gate = MembraneGate::new(&AgentConfig::default(), default_external_tools());
        // Pi reports a model-level error (e.g. an image to a non-vision model), then ends.
        let mut transport = Scripted::new(&[
            r#"{"type":"agent_start"}"#,
            r#"{"type":"error","error":"model gpt-x does not support image input"}"#,
            r#"{"type":"agent_end","messages":[]}"#,
            r#"{"type":"response","command":"get_last_assistant_text","success":true,"data":{"text":""}}"#,
        ]);
        let mut store = Store::open_in_memory().unwrap();
        let result = run_task(
            &mut store,
            "eng-1",
            &eng,
            &mut transport,
            &gate,
            "describe the image",
            &[],
        )
        .unwrap();

        assert_eq!(result.run_phase, RunPhase::Failed);
        assert_eq!(
            result.error.as_deref(),
            Some("model gpt-x does not support image input")
        );

        // …and it's durable: a reloaded transcript shows the reason as an error line.
        let joined = store.records("eng-1", "transcript").unwrap().join("\n");
        assert!(
            joined.contains(r#""type":"error""#) && joined.contains("does not support image input"),
            "the failure reason is a durable transcript line: {joined}"
        );
    }

    /// A fail-closed pre-flight refusal (no model credential) is a durable, coded
    /// failure turn — the user message plus a `code:"no_credential"` error line — so the
    /// chat log shows it and the client can render an "open settings" action (LLM-1).
    #[test]
    fn precheck_failure_is_a_durable_coded_error_turn() {
        let mut store = Store::open_in_memory().unwrap();
        let result = record_precheck_failure(
            &mut store,
            "eng-nc",
            "summarize the deck",
            "No model sign-in found. Link a key in Account settings.".to_string(),
            "no_credential",
        )
        .unwrap();

        assert_eq!(result.run_phase, RunPhase::Failed);
        assert_eq!(
            result.error.as_deref(),
            Some("No model sign-in found. Link a key in Account settings.")
        );
        assert_eq!(
            store.fold::<RunState>("eng-nc").unwrap().phase,
            RunPhase::Failed
        );

        // Durable: the user's message and a machine-readable error line both persist.
        let joined = store.records("eng-nc", "transcript").unwrap().join("\n");
        assert!(
            joined.contains(r#""type":"user""#) && joined.contains("summarize the deck"),
            "the user message is durable: {joined}"
        );
        assert!(
            joined.contains(r#""type":"error""#) && joined.contains(r#""code":"no_credential""#),
            "the error line carries the machine-readable code: {joined}"
        );
    }

    /// The streaming sink receives each observation live (the SSE seam).
    #[test]
    fn streaming_sink_receives_observations_live() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
        let eng = inst.create_engagement("e1").unwrap();
        let gate = MembraneGate::new(&AgentConfig::default(), default_external_tools());

        let mut transport = Scripted::new(&[
            r#"{"type":"text_delta","delta":"hi"}"#,
            r#"{"type":"tool_execution_start","toolCallId":"t1","toolName":"read","args":{}}"#,
            r#"{"type":"agent_end","messages":[]}"#,
            r#"{"type":"response","command":"get_last_assistant_text","success":true,"data":{"text":"hi"}}"#,
        ]);
        let mut store = Store::open_in_memory().unwrap();

        let mut streamed: Vec<String> = Vec::new();
        let mut sink = |obs: &Observation| streamed.push(obs.kind.to_string());
        run_task_streaming(
            &mut store,
            "e1",
            &eng,
            &mut transport,
            &gate,
            "go",
            &[],
            &mut sink,
        )
        .unwrap();

        // the text delta and the mediated tool both reached the sink live
        assert!(streamed.contains(&"text".to_string()));
        assert!(streamed.contains(&"egress".to_string()));
    }

    /// The prompt sent to the model is the **raw task** — no framing prefix
    /// (ADR 0029). The agent's persona is a Pi-native method resource
    /// (`.pi/SYSTEM.md` in use mode, or `--system-prompt` in edit mode), never
    /// prepended to the user message; the transcript records the raw task too.
    #[test]
    fn the_prompt_sent_is_the_raw_task_no_framing_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
        let eng = inst.create_engagement("e1").unwrap();
        let gate = MembraneGate::new(&AgentConfig::default(), default_external_tools());
        let mut transport = Scripted::new(&[
            r#"{"type":"agent_end","messages":[]}"#,
            r#"{"type":"response","command":"get_last_assistant_text","success":true,"data":{"text":"ok"}}"#,
        ]);
        let mut store = Store::open_in_memory().unwrap();
        let mut sink = |_: &Observation| {};
        run_task_streaming(
            &mut store,
            "e1",
            &eng,
            &mut transport,
            &gate,
            "tighten the policy",
            &[],
            &mut sink,
        )
        .unwrap();

        // the model receives the raw task, not a persona prefix.
        assert!(transport.sent[0].contains("tighten the policy"));
        assert!(
            !transport.sent[0].contains("You are the editor"),
            "no framing prefix: {}",
            transport.sent[0]
        );
        // the durable transcript shows the raw task the user typed.
        let user = store
            .records("e1", "transcript")
            .unwrap()
            .into_iter()
            .find(|r| r.contains("\"user\""))
            .unwrap();
        assert!(
            user.contains("tighten the policy"),
            "raw transcript: {user}"
        );
    }

    /// Mock-LLM mode: `run_engagement_turn` completes a turn deterministically
    /// (no Pi spawn) with a real worktree diff — the same path the E2E suite uses.
    #[test]
    fn fake_agent_mode_completes_a_turn_with_a_real_diff() {
        use std::sync::{Arc, Mutex};
        use tokio::sync::broadcast;

        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
        let eng = inst.create_engagement("e1").unwrap();
        let worktree = eng.path().to_path_buf();
        let store = Store::open_in_memory().unwrap();
        let wb = Arc::new(Mutex::new(crate::Workbench::with_instance(
            "inst-test",
            inst,
            store,
        )));
        wb.lock()
            .unwrap()
            .register_engagement("e1", "inst-test", Box::new(eng));

        let _fake_agent = fake_agent_env();
        let (tx, _rx) = broadcast::channel(16);
        let result = run_engagement_turn(
            &wb,
            "e1",
            &worktree,
            &tx,
            "do the thing",
            &[],
            ChatMode::Use,
        )
        .unwrap();

        assert_eq!(result.run_phase, RunPhase::Completed);
        assert!(result.commit.is_some(), "the fake turn auto-committed");
        assert!(
            result.diff.contains("agent-note.txt"),
            "real diff: {}",
            result.diff
        );
        // default policy (trust-by-default) mediates both in-workspace tools
        assert_eq!(
            result.mediated_tool_calls,
            vec!["write".to_string(), "bash".to_string()]
        );
        // INV-4: the turn's execution evidence was admitted into the run.
        let obs = wb.lock().unwrap().run_state("e1").unwrap().observations;
        assert!(obs > 0, "run recorded admitted observations: {obs}");
    }

    /// With a policy that blocks `bash`, the membrane stops it (the safety claim),
    /// while the in-policy `write` still goes through.
    #[test]
    fn fake_agent_membrane_blocks_out_of_policy_tool() {
        use std::sync::{Arc, Mutex};
        use tokio::sync::broadcast;

        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
        let eng = inst.create_engagement("e1").unwrap();
        let worktree = eng.path().to_path_buf();
        // policy blocks bash
        std::fs::write(
            worktree.join(".agent-config.json"),
            r#"{"policy":{"block_tools":["bash"]}}"#,
        )
        .unwrap();
        let store = Store::open_in_memory().unwrap();
        let wb = Arc::new(Mutex::new(crate::Workbench::with_instance(
            "inst-test",
            inst,
            store,
        )));
        wb.lock()
            .unwrap()
            .register_engagement("e1", "inst-test", Box::new(eng));

        let _fake_agent = fake_agent_env();
        let (tx, _rx) = broadcast::channel(16);
        let result =
            run_engagement_turn(&wb, "e1", &worktree, &tx, "go", &[], ChatMode::Use).unwrap();

        assert_eq!(
            result.mediated_tool_calls,
            vec!["write".to_string()],
            "bash not mediated"
        );
        assert!(
            result.blocked_effects.iter().any(|b| b.contains("bash")),
            "the membrane blocked bash: {:?}",
            result.blocked_effects
        );
    }

    /// MINT-1: a turn's derived output is minted under the scope's owning
    /// authority (`determine_scope_authority`), not the hardcoded local constant.
    /// A federated `scope:<authority>:<rest>` scope resolves to that authority, so
    /// the minted output is owned by — and governed by — the right keyset.
    #[test]
    fn output_is_minted_under_the_scopes_owning_authority() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
        let eng = inst.create_engagement("e1").unwrap();
        let gate = MembraneGate::new(&AgentConfig::default(), default_external_tools());
        let mut transport = Scripted::new(&[
            r#"{"type":"agent_end","messages":[]}"#,
            r#"{"type":"response","command":"get_last_assistant_text","success":true,"data":{"text":"ok"}}"#,
        ]);
        let mut store = Store::open_in_memory().unwrap();

        // A federated scope owned by `acme` (the second `:`-segment).
        let scope = "scope:acme:run-1";
        run_task(&mut store, scope, &eng, &mut transport, &gate, "go", &[]).unwrap();

        // The derived output exists and is owned by `acme`, not `local-user`.
        let out_id = crate::resource_store::output_id(scope);
        let rec = crate::resource_store::get(&store, scope, &out_id)
            .unwrap()
            .expect("a derived output was minted");
        assert_eq!(
            rec.resource.owner.as_str(),
            "acme",
            "owned by the scope's authority"
        );
        assert_ne!(
            rec.resource.owner.as_str(),
            crate::LOCAL_AUTHORITY,
            "not the hardcoded local owner"
        );
    }

    /// ENGINE-REMOTE-1: the engine orchestrator drives a turn against a
    /// **remote-placed** runtime (`RemoteLoopbackHarness`, REMOTE-RPC-1). The run
    /// lifecycle is admitted, the remote turn's observations come back *through
    /// federation* (OBSERVATION-FEDERATION-1) and become run truth only via the
    /// owner's admission (INV-4), the run completes, and the derived output is
    /// minted under the scope's owning authority (MINT-1) — no local worktree.
    #[test]
    fn engine_drives_a_remote_placed_turn_and_federates_its_observations() {
        use gaugewright_pi_bridge::RemoteLoopbackHarness;

        let mut store = Store::open_in_memory().unwrap();
        // A federated scope owned by `acme` (the second `:`-segment), so the minted
        // output is governed by that authority, not the hardcoded local owner.
        let scope = "scope:acme:remote-run";
        let gate = MembraneGate::new(&AgentConfig::default(), default_external_tools());

        // The remote peer streams two text tokens, so two observations cross back.
        let mut harness = RemoteLoopbackHarness::new(
            "127.0.0.1:7788",
            [
                r#"{"type":"agent_start"}"#,
                r#"{"type":"text_delta","delta":"remote "}"#,
                r#"{"type":"text_delta","delta":"work"}"#,
                r#"{"type":"agent_end","messages":[]}"#,
                r#"{"type":"response","command":"get_last_assistant_text","success":true,"data":{"text":"remote work"}}"#,
            ],
        );

        let result =
            run_task_remote(&mut store, scope, &mut harness, &gate, "do it remotely").unwrap();

        // The run completed and is durable in the log.
        assert_eq!(result.run_phase, RunPhase::Completed);
        assert_eq!(
            store.fold::<RunState>(scope).unwrap().phase,
            RunPhase::Completed
        );
        assert_eq!(
            result.remote_address, "127.0.0.1:7788",
            "the peer endpoint the turn ran at"
        );

        // INV-4: each remote observation crossed the bridge and was owner-admitted;
        // the run's admitted-observation count matches what federated across.
        assert!(
            result.federated_observations >= 2,
            "the two text tokens federated back"
        );
        assert_eq!(
            store.fold::<RunState>(scope).unwrap().observations,
            result.federated_observations,
            "the owner admitted exactly the federated observations into run truth",
        );
        let crossed = crate::federation_relay::admitted(&store, scope).unwrap();
        assert_eq!(crossed.len() as u32, result.federated_observations);
        for fact in &crossed {
            let handle = fact["payload_handle"].as_str().unwrap();
            assert!(
                handle.starts_with("obs::"),
                "a handle crossed, never the body (INV-10)"
            );
        }

        // MINT-1: the derived output is owned by the scope's authority (`acme`).
        assert_eq!(result.output_owner, "acme");
        let out_id = crate::resource_store::output_id(scope);
        let rec = crate::resource_store::get(&store, scope, &out_id)
            .unwrap()
            .expect("a derived output was minted");
        assert_eq!(
            rec.resource.owner.as_str(),
            "acme",
            "owned by the scope's authority"
        );
        assert_ne!(
            rec.resource.owner.as_str(),
            crate::LOCAL_AUTHORITY,
            "not the hardcoded local owner"
        );
    }

    /// WORKBENCH-REMOTE-1: the workbench holds a chat's **remote** harness session
    /// alongside the local ones, and [`drive_remote_turn`] routes a turn against it
    /// through the same `run_task_remote` orchestrator (ENGINE-REMOTE-1) — the
    /// observations federate back and become run truth via the owner's admission
    /// (INV-4), with no local worktree.
    #[test]
    fn workbench_holds_a_remote_session_and_drives_a_turn_against_it() {
        use gaugewright_pi_bridge::RemoteLoopbackHarness;
        use gaugewright_workspace::Instance;
        use std::sync::{Arc, Mutex};

        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
        let store = Store::open_in_memory().unwrap();
        let wb = Arc::new(Mutex::new(crate::Workbench::with_instance(
            "inst-test",
            inst,
            store,
        )));

        // Place this chat's runtime in a different authority (`acme`): register its
        // remote harness on the workbench, where it lives beside any local session.
        let scope = "scope:acme:wb-remote";
        wb.lock_unpoisoned().register_remote_session(
            scope,
            Box::new(RemoteLoopbackHarness::new(
                "127.0.0.1:7799",
                [
                    r#"{"type":"agent_start"}"#,
                    r#"{"type":"text_delta","delta":"remote "}"#,
                    r#"{"type":"text_delta","delta":"work"}"#,
                    r#"{"type":"agent_end","messages":[]}"#,
                    r#"{"type":"response","command":"get_last_assistant_text","success":true,"data":{"text":"remote work"}}"#,
                ],
            )),
        );

        // The workbench reports the chat as remotely placed, at the peer endpoint.
        assert!(
            wb.lock_unpoisoned().is_remote(scope),
            "the chat is placed remotely"
        );
        assert_eq!(
            wb.lock_unpoisoned().remote_address(scope),
            Some("127.0.0.1:7799")
        );

        let gate = MembraneGate::new(&AgentConfig::default(), default_external_tools());
        let result = drive_remote_turn(&wb, scope, &gate, "do it remotely").unwrap();

        // The run completed via the remote orchestrator; its observations federated
        // back and were owner-admitted into run truth (INV-4).
        assert_eq!(result.run_phase, RunPhase::Completed);
        assert_eq!(result.remote_address, "127.0.0.1:7799");
        assert!(
            result.federated_observations >= 2,
            "the text tokens federated back"
        );
        assert_eq!(
            wb.lock_unpoisoned().run_state(scope).unwrap().observations,
            result.federated_observations,
            "the owner admitted exactly the federated observations",
        );
        // MINT-1: the output is minted under the scope's authority (`acme`).
        assert_eq!(result.output_owner, "acme");
    }

    /// E2E-TEST-1: the whole D-REMOTE two-authority loopback story in one turn —
    /// an owner drives a turn whose runtime is *placed in another authority*
    /// (`scope:acme:…`, `RemoteLoopbackHarness`), the remote observations cross the
    /// owner's bridge **through federation** as signed handle-only messages and
    /// become run truth only via the owner's admission (INV-4 / INV-10), and the
    /// derived output is minted under the scope's authority (MINT-1). The crossing's
    /// security teeth (INV-21) are asserted on the same relay the turn rides: a
    /// genuine signed envelope admits, while a forged signature, a mismatched bridge
    /// grant, and a replayed nonce each deny target admission.
    ///
    /// Marked `#[ignore]` (run via `-- --ignored`): the heavier end-to-end
    /// composition over the loopback substrate, distinct from the focused
    /// orchestrator/workbench units above. A real cross-machine relay attaches
    /// behind the same seam with no rearchitecture (ADR 0020).
    #[test]
    #[ignore = "E2E-TEST-1: end-to-end two-authority loopback; run with --ignored"]
    fn e2e_two_authority_loopback_federation_with_signatures() {
        use gaugewright_core::federated_delivery::{
            Authority, DeliveryCommand, DeliveryEnvelope, DeliveryPhase, DeliveryState,
        };
        use gaugewright_core::ids::{BridgeGrantId, Nonce, PublicKey};
        use gaugewright_core::signature::Signature;
        use gaugewright_pi_bridge::RemoteLoopbackHarness;
        use gaugewright_store::AdmitError;

        let mut store = Store::open_in_memory().unwrap();
        // The owner federates work to a runtime placed in the `acme` authority.
        let scope = "scope:acme:e2e-run";
        let gate = MembraneGate::new(&AgentConfig::default(), default_external_tools());

        // --- 1. A remote-placed turn whose observations federate back ------------
        // Two streamed text tokens cross the owner's bridge as handle-only facts.
        let mut harness = RemoteLoopbackHarness::new(
            "127.0.0.1:7900",
            [
                r#"{"type":"agent_start"}"#,
                r#"{"type":"text_delta","delta":"remote "}"#,
                r#"{"type":"text_delta","delta":"work"}"#,
                r#"{"type":"agent_end","messages":[]}"#,
                r#"{"type":"response","command":"get_last_assistant_text","success":true,"data":{"text":"remote work"}}"#,
            ],
        );

        let result =
            run_task_remote(&mut store, scope, &mut harness, &gate, "do it remotely").unwrap();

        // The run completed and is durable; the turn ran at the peer endpoint.
        assert_eq!(result.run_phase, RunPhase::Completed);
        assert_eq!(
            store.fold::<RunState>(scope).unwrap().phase,
            RunPhase::Completed
        );
        assert_eq!(result.remote_address, "127.0.0.1:7900");

        // INV-4: each remote observation crossed the bridge and was owner-admitted;
        // the run's admitted-observation count matches what federated across.
        assert!(
            result.federated_observations >= 2,
            "the two text tokens federated back"
        );
        assert_eq!(
            store.fold::<RunState>(scope).unwrap().observations,
            result.federated_observations,
            "the owner admitted exactly the federated observations into run truth",
        );
        // INV-10: only handles crossed the bridge — never the observation body.
        let crossed = crate::federation_relay::admitted(&store, scope).unwrap();
        assert_eq!(crossed.len() as u32, result.federated_observations);
        for fact in &crossed {
            let handle = fact["payload_handle"].as_str().unwrap();
            assert!(
                handle.starts_with("obs::"),
                "a handle crossed, never the body (INV-10)"
            );
        }

        // MINT-1: the derived output is owned by the scope's authority (`acme`),
        // governed by the right keyset though it ran in a different authority.
        assert_eq!(result.output_owner, "acme");
        let out_id = crate::resource_store::output_id(scope);
        let rec = crate::resource_store::get(&store, scope, &out_id)
            .unwrap()
            .expect("a derived output was minted");
        assert_eq!(
            rec.resource.owner.as_str(),
            "acme",
            "owned by the scope's authority"
        );
        assert_ne!(
            rec.resource.owner.as_str(),
            crate::LOCAL_AUTHORITY,
            "not the hardcoded local owner"
        );

        // --- 2. The crossing's security teeth on the same delivery shell (INV-21) -
        // A genuine signed envelope under the bound grant admits at the target.
        let signed = |correlation: &str| DeliveryEnvelope {
            signed_bytes: correlation.as_bytes().to_vec(),
            signature: Signature::new(vec![0u8; 64]),
            source_pubkey: PublicKey::new("04loopback-source"),
            nonce: Nonce::new(format!("nonce::{correlation}")),
            bridge_grant_id: BridgeGrantId::new("bridge-grant-7"),
            device_key: PublicKey::new("04dev1ce0ke7"),
            device_active: true,
        };
        let cross = |store: &mut Store, correlation: &str, envelope: DeliveryEnvelope| -> bool {
            let ds = crate::federation_relay::delivery_scope(correlation);
            store
                .admit::<DeliveryState>(&ds, DeliveryCommand::AuthorizeFederatedMessage)
                .unwrap();
            store
                .admit::<DeliveryState>(&ds, DeliveryCommand::EnqueueFederatedMessage)
                .unwrap();
            store
                .admit::<DeliveryState>(&ds, DeliveryCommand::RecordRelayDelivery)
                .unwrap();
            match store
                .admit::<DeliveryState>(&ds, DeliveryCommand::AdmitTargetReceipt { envelope })
            {
                Ok(s) => s.phase == DeliveryPhase::TargetAdmitted,
                Err(AdmitError::Rejected(_)) => false,
                Err(e) => panic!("unexpected delivery error: {e:?}"),
            }
        };

        // A genuine crossing admits: target authority + verified signature, relay-blind.
        assert!(
            cross(&mut store, "e2e-ok", signed("e2e-ok")),
            "a signed envelope admits"
        );
        let s = store
            .fold::<DeliveryState>(&crate::federation_relay::delivery_scope("e2e-ok"))
            .unwrap();
        assert_eq!(
            s.target_admitted_by,
            Authority::Target,
            "INV-13: only the target admits"
        );
        assert!(
            s.signature_verified,
            "INV-21: the source signature was verified before admission"
        );
        assert!(
            !s.relay_has_payload_access,
            "INV-10: the relay gained no payload read"
        );
        assert_ne!(
            s.payload_authority,
            Authority::Relay,
            "INV-14: the relay is never payload authority"
        );

        // A forged (malformed) signature is denied (fails closed).
        let mut forged = signed("e2e-forged");
        forged.signature = Signature::new(vec![0u8; 8]);
        assert!(
            !cross(&mut store, "e2e-forged", forged),
            "INV-21: an unverifiable signature denies admission"
        );

        // A mismatched bridge grant is denied.
        let mut wrong_grant = signed("e2e-wrong-grant");
        wrong_grant.bridge_grant_id = BridgeGrantId::new("bridge-grant-OTHER");
        assert!(
            !cross(&mut store, "e2e-wrong-grant", wrong_grant),
            "INV-21: a mismatched grant denies admission"
        );

        // Anti-replay: re-presenting an admitted envelope spends no second nonce.
        let env = signed("e2e-replay");
        assert!(
            cross(&mut store, "e2e-replay", env.clone()),
            "first crossing admits"
        );
        let ds = crate::federation_relay::delivery_scope("e2e-replay");
        match store
            .admit::<DeliveryState>(&ds, DeliveryCommand::AdmitTargetReceipt { envelope: env })
        {
            Err(AdmitError::Rejected(_)) => {}
            other => {
                panic!("INV-21: re-presenting an admitted envelope must be denied, got {other:?}")
            }
        }
        let s = store.fold::<DeliveryState>(&ds).unwrap();
        assert_eq!(
            s.seen_nonces.len(),
            1,
            "INV-21: the replay spent no further nonce"
        );
    }

    /// WORKBENCH-REMOTE-1: a chat is local *or* remote, never both. Placing a remote
    /// session retires any local one under the same id, so the two maps stay disjoint.
    #[test]
    fn registering_a_remote_session_retires_a_local_one() {
        use gaugewright_pi_bridge::RemoteLoopbackHarness;
        use gaugewright_workspace::Instance;
        use std::sync::{Arc, Mutex};

        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
        let store = Store::open_in_memory().unwrap();
        let mut wb = crate::Workbench::with_instance("inst-test", inst, store);

        // Seed a local session under the chat id, then place it remotely.
        wb.seed_local_session_for_test(
            "c1",
            Box::new(ScriptedTransport::new(Vec::<String>::new())),
        );
        assert!(!wb.is_remote("c1"));

        wb.register_remote_session(
            "c1",
            Box::new(RemoteLoopbackHarness::new(
                "127.0.0.1:7800",
                Vec::<String>::new(),
            )),
        );
        assert!(wb.is_remote("c1"), "now placed remotely");
        assert!(
            !wb.has_local_session_for_test("c1"),
            "the local session was retired"
        );

        let _ = Arc::new(Mutex::new(wb)); // exercises the SharedWorkbench shape
    }
}

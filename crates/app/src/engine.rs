//! The canonical agent loop: task an agent against a folder and let it work —
//! headlessly, end-to-end through the verified spine.
//!
//! This is the orchestrator the Phase-2 gate names: it creates an engagement
//! worktree off the instance's `main`, admits the [[run]] lifecycle into the
//! durable store, drives one [[runtime-session]] turn through the selected harness and egress
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
/// workspace conflict. Toggled by the `POST /test/force-conflict` route (gated by
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
use crate::policy_compiler::PolicyCompilationInput;
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
    /// the selected harness's egress policy.
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
Its authored WhippleScript package lives in `.whipple/draft`; frozen package versions are read-only. \
Edit the draft persona, workflow, and capability registry to satisfy the request, then briefly explain what you changed. \
Do not perform end-user tasks in edit mode — refine the agent itself.";

/// Append a durable transcript record (admitted run evidence) to the engagement's
/// log — the snapshot the client reduces on load (`app-stack.md`: repairable).
fn record_transcript(store: &mut Store, scope: &str, event: &ServerEvent) {
    let _ = store.append_record(scope, "transcript", &event.to_json());
}

const RUNTIME_EVIDENCE_POINTER_KIND: &str = "runtime_evidence_pointer";

#[derive(serde::Serialize)]
struct RuntimeEvidenceCrossing<'a> {
    runtime: &'static str,
    pointer: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_cut_ref: Option<WorkspaceCutRef<'a>>,
}

#[derive(serde::Serialize)]
struct WorkspaceCutRef<'a> {
    substrate: &'static str,
    revision: &'a str,
}

/// Admit body-free WhippleScript pointers at most once. The record's assigned
/// GaugeDesk scope position and the WhippleScript position inside `pointer`
/// form the cross-store cut. The workspace revision is a WhippleScript-native
/// manifest cut, not a commit in a second authority store.
fn admit_runtime_evidence_pointers(
    store: &mut Store,
    scope: &str,
    pointers: &[String],
    workspace_revision: Option<&str>,
) -> Result<Vec<i64>, AdmitError> {
    use sha2::{Digest, Sha256};

    let mut positions = Vec::with_capacity(pointers.len());
    for pointer in pointers {
        let key = format!(
            "whip:pointer:{}",
            hex::encode(Sha256::digest(pointer.as_bytes()))
        );
        let crossing = RuntimeEvidenceCrossing {
            runtime: "whipplescript",
            pointer,
            workspace_cut_ref: workspace_revision.map(|revision| WorkspaceCutRef {
                substrate: "whipplescript",
                revision,
            }),
        };
        let payload = serde_json::to_string(&crossing)?;
        let (position, _) =
            store.append_record_with_key(scope, &key, RUNTIME_EVIDENCE_POINTER_KIND, &payload)?;
        positions.push(position);
    }
    Ok(positions)
}
use gaugewright_core::merge::{MergeCommand, MergePhase, MergeState};
use gaugewright_core::run::{RunCommand, RunPhase, RunState};
use gaugewright_harness::{
    CredentialProbe, EgressGate, GateDecision, Harness, HarnessFactory, HarnessSpec, HumanPrompt,
    ImageContent, InterruptHandle, Observation, TurnOutcome,
};
use gaugewright_store::{AdmitError, Store};
use gaugewright_workspace::{ChatWorkspace, MergeOutcome};

/// A membrane-backed egress gate: maps a harness tool name to an [`Effect`] and asks
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

/// Defense-in-depth package/control roots: work chats protect all package bytes;
/// edit chats protect frozen versions while leaving only the draft writable;
/// GaugeDesk runtime selection is always protected.
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
pub(crate) fn resolve_turn_provider(
    host_override: Option<String>,
    config_provider: Option<String>,
) -> String {
    host_override
        .filter(|s| !s.is_empty())
        .or(config_provider.filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "openai-codex".to_string())
}

/// Resolve a turn's model: a non-empty host override (`GAUGEWRIGHT_MODEL`) wins over the chat's
/// configured model; `None` leaves the selected provider's default. Paired with
/// [`resolve_turn_provider`] so a host that forces the provider can pin a compatible model.
pub(crate) fn resolve_turn_model(
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

/// The network egress posture a turn runs under (RF-B3, CORE-5). Pure so the
/// precedence is unit-testable:
///
/// - operator forced unfiltered egress (`GAUGEWRIGHT_ALLOW_UNFILTERED_EGRESS=1`) ⇒
///   [`Network::Allow`] — the conscious unfiltered opt-in wins over everything;
/// - the project isolates its network ⇒ [`Network::Deny`];
/// - a non-isolated project ⇒ [`Network::Filtered`], admitting only the resolved
///   model endpoint.
///
/// WhippleScript owns the provider client, fixes its request URL from the admitted
/// binding, and refuses redirects. It can therefore enforce the model-endpoint
/// filter directly without depending on the legacy Pi subprocess/netns routing
/// capability. Isolation (`Deny`) and the conscious unfiltered opt-in (`Allow`)
/// remain GaugeDesk product-policy decisions.
fn egress_posture(
    project_isolated: bool,
    forced_unfiltered: bool,
) -> gaugewright_harness::sandbox::Network {
    use gaugewright_harness::sandbox::Network;
    if forced_unfiltered {
        Network::Allow
    } else if project_isolated {
        Network::Deny
    } else {
        Network::Filtered
    }
}

fn method_surface_readonly_roots(worktree: &Path, mode: ChatMode) -> Vec<std::path::PathBuf> {
    let package_roots = match mode {
        ChatMode::Use => definition::READONLY_ROOTS,
        ChatMode::Edit => definition::EDIT_READONLY_ROOTS,
    };
    package_roots
        .iter()
        .chain(definition::CONTROL_READONLY_ROOTS.iter())
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
    /// The turn's opaque WhippleScript cut id, if the turn changed anything.
    pub commit: Option<String>,
    /// The merge lifecycle phase after the turn: `Clean` (awaiting the human's
    /// admit/reject of the diff) or `Rejected` (a workspace conflict → isolated).
    pub merge_phase: MergePhase,
    pub mediated_tool_calls: Vec<String>,
    /// Effects the membrane blocked (the out-of-policy path).
    pub blocked_effects: Vec<String>,
    pub pending_approvals: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_human: Option<HumanPrompt>,
    /// The runtime/model error that failed this turn, if any — lets the client show
    /// an honest status immediately (the same text is also a durable transcript line).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The turn's certified dynamic guarantee outcomes (DR-0036 §2), matched by
    /// name at settle by the advancement policy (ADR 0082 §5). Empty when the
    /// runtime published no report — the local-truth path decides.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub guarantee_outcomes: Vec<gaugewright_harness::GuaranteeOutcome>,
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
/// `harness` drives the engagement's selected runtime; `gate` is the membrane. The run
/// lifecycle is admitted into `store` under `scope`; the runtime-session is
/// seeded to `executing` and advanced by the turn.
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
    let initial_phase = store.fold::<RunState>(scope)?.phase;
    let resuming_human = initial_phase == RunPhase::AwaitingHuman;
    match initial_phase {
        RunPhase::Init => {
            store.admit::<RunState>(scope, RunCommand::RequestRun)?;
            store.admit::<RunState>(scope, RunCommand::AdmitRun)?;
            store.admit::<RunState>(scope, RunCommand::StartRun)?;
        }
        RunPhase::Requested => {
            store.admit::<RunState>(scope, RunCommand::AdmitRun)?;
            store.admit::<RunState>(scope, RunCommand::StartRun)?;
        }
        RunPhase::Admitted => {
            store.admit::<RunState>(scope, RunCommand::StartRun)?;
        }
        RunPhase::Running => {}
        // Delay the product transition until WhippleScript has admitted the
        // correlated answer. A malformed/refused answer leaves the run waiting.
        RunPhase::AwaitingHuman => {}
        RunPhase::Completed | RunPhase::Failed | RunPhase::Canceled => {
            store.admit::<RunState>(scope, RunCommand::RetryRun)?;
            store.admit::<RunState>(scope, RunCommand::AdmitRun)?;
            store.admit::<RunState>(scope, RunCommand::StartRun)?;
        }
    }

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
    //    harness owns its protocol + session; the prompt is the raw task. Persona
    //    comes from the selected authored package or separate editor package.
    let outcome: TurnOutcome = harness
        .run_turn(gate, task, images, sink)
        .map_err(EngineError::Harness)?;

    if resuming_human {
        store.admit::<RunState>(scope, RunCommand::ResumeRun)?;
    }

    // 3a. Admit the runtime's execution evidence into the run (INV-4): each tool
    //     decision the membrane ruled on is an observation that becomes standing
    //     run state only by this admission, while the run is still `running`.
    for _ in &outcome.observations {
        store.admit::<RunState>(scope, RunCommand::RecordObservation)?;
    }

    if outcome.pending_human.is_some() {
        admit_runtime_evidence_pointers(store, scope, &outcome.runtime_evidence_pointers, None)?;
        store.admit::<RunState>(scope, RunCommand::AwaitHuman)?;
        for obs in &outcome.observations {
            if matches!(
                obs.kind,
                "egress" | "egress_staged" | "tool_result" | "egress_blocked"
            ) {
                record_transcript(store, scope, &ServerEvent::from_observation(obs));
            }
        }
        record_transcript(
            store,
            scope,
            &ServerEvent::Assistant {
                text: outcome.assistant_text.clone(),
            },
        );
        record_transcript(
            store,
            scope,
            &ServerEvent::Admitted {
                kind: "run".into(),
                text: "run → AwaitingHuman".into(),
            },
        );
        let merge_phase = store.fold::<MergeState>(scope)?.phase;
        tracing::info!(
            run_phase = ?RunPhase::AwaitingHuman,
            ?merge_phase,
            observations = outcome.observations.len(),
            "engine.turn awaiting authenticated human"
        );
        return Ok(TaskResult {
            run_phase: RunPhase::AwaitingHuman,
            assistant_text: outcome.assistant_text,
            diff: engagement.diff_against_main()?,
            commit: None,
            merge_phase,
            mediated_tool_calls: outcome.mediated_tool_calls,
            blocked_effects: outcome
                .observations
                .iter()
                .filter(|observation| observation.kind == "egress_blocked")
                .map(|observation| observation.detail.clone())
                .collect(),
            pending_approvals: outcome.pending_approvals,
            pending_human: outcome.pending_human,
            error: None,
            // A suspended turn has no terminal receipt yet, hence no report.
            guarantee_outcomes: Vec::new(),
        });
    }

    // 3b. Auto-commit the worktree (per-turn), then capture the reviewer's diff.
    let commit = engagement.commit_turn(task)?;
    let diff = engagement.diff_against_main()?;
    admit_runtime_evidence_pointers(
        store,
        scope,
        &outcome.runtime_evidence_pointers,
        commit.as_ref().map(|commit| commit.0.as_str()),
    )?;

    // 4. Map the turn outcome onto the run lifecycle: clean turn → completed,
    //    a runtime/stream error → failed. Either way the events are durable facts.
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
    // so a browser BDD can drive conflict-repair without staging a real workspace conflict.
    let merge_cmd = if force_merge_conflict() {
        MergeCommand::WorkspaceConflict
    } else {
        match probe {
            MergeOutcome::Clean => MergeCommand::WorkspaceClean,
            MergeOutcome::Conflict => MergeCommand::WorkspaceConflict,
        }
    };
    let merge = store.admit::<MergeState>(scope, merge_cmd)?;

    // 6. Record this turn's reads (every granted context resource) into the durable
    //    engagement read-set, then mint/refresh the derived output resource from it.
    //    Taint is engagement-scoped (ADR 0026): the output's stakeholders are the
    //    owners of everything the engagement has read across turns — sound even after
    //    a read context is later revoked or tombstoned — so a later export/review
    //    gates on persisted handles, not a loose stakeholder set.
    let output_reads = if outcome.output_flow_signature.is_empty() {
        crate::resource_store::granted_context(store, scope)
    } else {
        crate::resource_store::certified_output_reads(store, scope, &outcome.output_flow_signature)
    };
    if let Ok(reads) = output_reads {
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
        guarantee_outcomes: outcome.guarantee_outcomes,
        commit: commit.map(|c| c.0),
        merge_phase: merge.phase,
        mediated_tool_calls: outcome.mediated_tool_calls,
        blocked_effects,
        pending_approvals: outcome.pending_approvals,
        pending_human: None,
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
/// The test-only single-process loopback harness and a real cross-machine relay
/// attach behind the same neutral seam with no rearchitecture
/// (`RENDEZVOUS-STUB-1`).
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
/// resolve for `provider`? A **BYOK** provider needs its exact-reference
/// capability resolved from the account's `SEC-4`-sealed store; an **OAuth**
/// provider (`openai-codex`, …) authenticates via the runtime adapter's own store,
/// which the turn's `factory` answers for ([`HarnessFactory::credential_status`]).
/// The refusal POLICY — whether a turn runs — stays here; the adapter only reports
/// its own state. Returns an **actionable** error when nothing resolves, so a real
/// run refuses up front instead of letting the runtime fail opaquely on a missing key.
fn llm_credential_status(
    provider: &str,
    credential_capability: Option<&dyn gaugewright_harness::CredentialCapability>,
    factory: &dyn HarnessFactory,
) -> Result<(), String> {
    // BYOK providers require an exact-reference GaugeDesk capability. Secret
    // bytes remain sealed until WhippleScript admits that reference.
    if crate::account::provider_env_var(provider).is_some() {
        return if credential_capability.is_some() {
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
            let get = |k: &str| std::env::var(k).ok();
            host_managed_model_status(provider, &get)
        }
        // OAuth providers authenticate via the adapter's own auth store.
        _ => match factory.credential_status(provider, credential_capability) {
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
    let current = store
        .fold::<RunState>(scope)
        .map_err(|e| format!("{e:?}"))?
        .phase;
    if current == RunPhase::AwaitingHuman {
        // Credential availability is product policy, not an answer to the
        // suspended ask. Keep the exact epoch live so the authenticated user can
        // retry after fixing credentials; do not convert suspension into failure.
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
        return Ok(TaskResult {
            run_phase: RunPhase::AwaitingHuman,
            assistant_text: String::new(),
            diff: String::new(),
            commit: None,
            merge_phase: store
                .fold::<MergeState>(scope)
                .map_err(|e| format!("{e:?}"))?
                .phase,
            mediated_tool_calls: Vec::new(),
            blocked_effects: Vec::new(),
            pending_approvals: Vec::new(),
            pending_human: None,
            error: Some(reason),
            guarantee_outcomes: Vec::new(),
        });
    }
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
        pending_human: None,
        error: Some(reason),
        guarantee_outcomes: Vec::new(),
    })
}

/// Drive one turn for an engagement, streaming observations live to its
/// broadcast `sender`. The engine resolves the turn's *policy* (mode framing,
/// credentials, provider/model, fail-closed precheck, base sandbox) into a
/// [`HarnessSpec`]; the runtime itself is constructed by the factory the
/// per-turn selector picks ([`crate::harness_select::factory_for_turn`] — the
/// real WhippleScript adapter, or the scripted fake under `GAUGEWRIGHT_FAKE_AGENT`).
/// Returns the turn result, or a human-readable error (the model endpoint may
/// be unauthenticated/offline).
///
/// Blocking: holds the workbench lock for the turn (local single-user MVP). SSE
/// subscribers already hold their receivers, so the live stream is unaffected.
pub struct EngagementTurnInput<'a> {
    pub task: &'a str,
    pub images: &'a [ImageContent],
    pub mode: ChatMode,
    pub authenticated_actor: Option<&'a gaugewright_core::ids::AuthorityId>,
}

pub fn run_engagement_turn(
    wb: &SharedWorkbench,
    id: &str,
    worktree: &Path,
    sender: &broadcast::Sender<ServerEvent>,
    input: EngagementTurnInput<'_>,
) -> Result<TaskResult, String> {
    let EngagementTurnInput {
        task,
        images,
        mode,
        authenticated_actor,
    } = input;
    let config =
        AgentConfig::from_file(&worktree.join(definition::CONFIG_PATH)).unwrap_or_default();
    let gate = MembraneGate::new(&config, default_external_tools()).with_mode(mode);

    // GaugeDesk keeps credential custody. The selected provider material is
    // resolved later into one exact-reference in-memory capability; the turn no
    // longer receives an ambient environment-shaped secret vector.
    let (whip_factory, actor, package_selection) = {
        let g = wb.lock_unpoisoned();
        let factory = g
            .whip_harness_factory()
            .map_err(|error| error.to_string())?;
        (
            factory,
            authenticated_actor
                .cloned()
                .unwrap_or_else(|| g.authority().clone()),
            g.package_selection_for_chat(id),
        )
    };

    let (package_root, package_version_ref) = match mode {
        ChatMode::Edit => (None, None),
        ChatMode::Use => package_selection
            .map(|(version, package_ref)| {
                (
                    Some(worktree.join(gaugewright_boundary::definition::version_root(version))),
                    Some(package_ref),
                )
            })
            .unwrap_or((None, None)),
    };

    // Persona is package content (ADR 0081), never host runtime configuration:
    // work chats select the placement's exact authored package; edit chats
    // select GaugeDesk's separate editor package.
    let system_prompt: Option<String> = match mode {
        ChatMode::Edit => Some(EDITOR_FRAMING.to_string()),
        ChatMode::Use => None,
    };

    // The one harness decision point (SUB-0): which adapter drives this turn.
    // Consulted per turn — tests flip `GAUGEWRIGHT_FAKE_AGENT` against a live
    // workbench, so the selection must never be cached at startup.
    let factory = crate::harness_select::factory_for_turn(whip_factory);

    // Mock-LLM mode: no WhippleScript runtime, no model call. The scripted fake drives the
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
            package_root: package_root.clone(),
            package_version_ref: package_version_ref.clone(),
            policy_epoch: None,
            signed_policy_envelope: None,
            provider_binding_ref: None,
            credential_ref: None,
            placement_ceiling_ref: None,
            provider: None,
            model: None,
            thinking: None,
            system_prompt,
            credential_capability: None,
            credentials: Vec::new(),
            sandbox: gaugewright_harness::sandbox::SandboxPolicy::new(vec![worktree.to_path_buf()]),
        };
        drive_persistent_turn(
            wb,
            id,
            &gate,
            task,
            images,
            sender,
            factory.as_ref(),
            &spec,
            actor.as_str(),
        )?
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
        let credential_ref = wb.lock_unpoisoned().credential_ref_for_chat(id, &provider);
        let credential_capability = if provider == "openai-codex" {
            match crate::codex_oauth::resolve_runtime_credential(wb) {
                Ok(Some(credential)) => Some(crate::account::resolved_credential_capability(
                    credential_ref.clone(),
                    credential.access,
                    Some(credential.account_id),
                )),
                Ok(None) => None,
                Err(reason) => {
                    let _ = sender.send(ServerEvent::Error {
                        reason: reason.clone(),
                        code: Some("credential_refresh_failed".into()),
                    });
                    let mut workbench = wb.lock_unpoisoned();
                    return record_precheck_failure(
                        &mut workbench.store,
                        id,
                        task,
                        reason,
                        "credential_refresh_failed",
                    );
                }
            }
        } else {
            wb.lock_unpoisoned()
                .credential_capability_for_chat(id, &provider)
        };
        let model = resolve_turn_model(
            std::env::var("GAUGEWRIGHT_MODEL").ok(),
            config.model.clone(),
        );
        let provider_descriptor =
            gaugewright_whip_runtime::native_provider_descriptor(&provider, model.as_deref())
                .map_err(|error| error.to_string())?;
        // Fail closed (LLM-1, ADR 0062): refuse a real run when no model credential resolves for
        // the resolved provider. Record it as a durable, coded failure turn so the chat log
        // shows *why* with an actionable "open settings" affordance — not just a status line —
        // then return the same Failed shape an in-turn failure returns (never let the runtime fail opaquely).
        if let Err(reason) = llm_credential_status(
            &provider,
            credential_capability.as_deref(),
            factory.as_ref(),
        ) {
            let _ = sender.send(ServerEvent::Error {
                reason: reason.clone(),
                code: Some("no_credential".into()),
            });
            let mut g = wb.lock_unpoisoned();
            return record_precheck_failure(&mut g.store, id, task, reason, "no_credential");
        }

        // GaugeDesk's workspace and egress policy for this turn (ADR 0030): the
        // worktree is writable, while use mode marks the method definition
        // read-only. WhippleScript resolves that policy into confined native
        // workspace capabilities, so writes outside the grant or into protected
        // subtrees fail before filesystem execution (INV-24).
        let sandbox_policy = {
            use gaugewright_harness::sandbox::Network;
            let writable = vec![worktree.to_path_buf()];
            // Network egress posture (RF-B3, CORE-5) is a **per-project** choice. A
            // non-isolated project reaches ONLY the model endpoints (Filtered, enforced
            // by the host-filtering egress proxy) **where the host can enforce that**;
            // where it can't, it keeps the accepted open-by-default posture (unfiltered
            // with a disclosed lower ceiling — the 2026-06-17 product decision) rather
            // than breaking model access. The model endpoint is named explicitly
            // (recorded + auditable; load-bearing under Filtered).
            // `GAUGEWRIGHT_ALLOW_UNFILTERED_EGRESS=1` force-opens to UNFILTERED egress
            // regardless (the conscious opt-in, mirroring `GAUGEWRIGHT_SANDBOX=0`); an
            // isolated project denies network entirely. A `Filtered` request the host
            // can't enforce is failed closed to `Deny` by the harness — never silently
            // to `Allow` — which is exactly why the engine only requests it when enforceable.
            let egress_hosts = model_endpoint_hosts(Some(&provider));
            let project_isolated = wb.lock_unpoisoned().chat_network_isolated(id);
            let forced_unfiltered =
                std::env::var("GAUGEWRIGHT_ALLOW_UNFILTERED_EGRESS").as_deref() == Ok("1");
            let posture = egress_posture(project_isolated, forced_unfiltered);
            match posture {
                Network::Deny => eprintln!(
                    "[gaugewright] NOTE: this project denies WhippleScript provider egress; \
                     the model endpoint ({}) is unreachable. Turn off isolation for \
                     the project to let the agent reach the model.",
                    egress_hosts.join(", ")
                ),
                Network::Filtered => eprintln!(
                    "[gaugewright] NOTE: WhippleScript provider egress is restricted to \
                     the admitted model endpoint ({}) and redirects fail closed.",
                    egress_hosts.join(", ")
                ),
                Network::Allow => eprintln!(
                    "[gaugewright] NOTE: project policy allows unfiltered egress \
                     (GAUGEWRIGHT_ALLOW_UNFILTERED_EGRESS=1); the current WhippleScript \
                     package still exposes only its governed provider endpoint ({}).",
                    egress_hosts.join(", ")
                ),
            }
            let base = gaugewright_harness::sandbox::SandboxPolicy::new(writable)
                .read_only(method_surface_readonly_roots(worktree, mode));
            match posture {
                // Filtered: the allowlist is load-bearing (enforced by the proxy).
                Network::Filtered => base.filter_egress(egress_hosts),
                // Unfiltered opt-in: record the intended targets, then open wide.
                Network::Allow => base.allow_hosts(egress_hosts).allow_unfiltered_egress(true),
                // Isolated: record intent for audit; posture stays Deny.
                Network::Deny => base.allow_hosts(egress_hosts),
            }
        };
        let package_capabilities: BTreeSet<String> = match mode {
            ChatMode::Use => {
                let root = package_root.as_deref().ok_or_else(|| {
                    "a work chat has no selected WhippleScript package root".to_owned()
                })?;
                gaugewright_whip_runtime::AuthoredAgentPackage::load(root)
                    .map_err(|error| error.to_string())?
                    .capabilities()
                    .iter()
                    .cloned()
                    .collect()
            }
            ChatMode::Edit => gaugewright_whip_runtime::editor_package_capabilities()
                .map_err(|error| error.to_string())?,
        };
        let policy_epoch = {
            let mut g = wb.lock_unpoisoned();
            let project_id = g.library_project_of_chat(id);
            let turn_purpose = g.library_chat_run_purpose(id);
            let granted = crate::resource_store::granted_context(&g.store, id)
                .map_err(|error| format!("{error:?}"))?
                .into_iter()
                .collect::<BTreeSet<_>>();
            let mut resources =
                crate::resource_store::list(&g.store, id).map_err(|error| format!("{error:?}"))?;
            resources.retain(|record| granted.contains(&record.resource.id));
            let org =
                crate::org::Org::rebuild(g.store_ref()).map_err(|error| format!("{error:?}"))?;
            // The operator's auto-keep scopes (ATTN-3) become an envelope
            // guarantee declaration the runtime evaluates per turn (ADR 0082
            // §5). A scope change re-canonicalizes the policy → new epoch.
            // (Read before the mutable compile call below.)
            let advancement_scopes = crate::advancement::AdvancementRules::parse(
                g.account_settings()
                    .ok()
                    .and_then(|s| {
                        s.get(crate::advancement::ADVANCEMENT_RULES_SETTING)
                            .cloned()
                    })
                    .as_deref(),
            )
            .declared_scopes();
            let actor_attributes = g.idp.as_ref().map_or_else(
                || gaugewright_core::abac::AuthorityAttributes {
                    clearance: gaugewright_core::abac::Clearance(3),
                    roles: BTreeSet::from([gaugewright_core::abac::Role::owner()]),
                    region: org
                        .security
                        .as_ref()
                        .and_then(|security| security.residency_region.as_deref())
                        .or_else(|| {
                            org.org
                                .as_ref()
                                .and_then(|record| record.default_region.as_deref())
                        })
                        .map(gaugewright_core::abac::Region::new),
                    ..gaugewright_core::abac::AuthorityAttributes::default()
                },
                |idp| idp.claims(&actor),
            );
            g.compile_whipple_policy(PolicyCompilationInput {
                chat_id: id.to_owned(),
                project_id,
                actor: actor.as_str().to_owned(),
                actor_attributes,
                org_policy: org.policy(),
                turn_purpose,
                package_capabilities,
                provider: provider.clone(),
                model: provider_descriptor.model.clone(),
                base_url: provider_descriptor.base_url.clone(),
                credential_ref,
                placement_kind: "local".to_owned(),
                command_network: sandbox_policy.network
                    != gaugewright_harness::sandbox::Network::Deny,
                resources,
                advancement_scopes,
            })?
        };
        let spec = HarnessSpec {
            chat_id: id.to_string(),
            worktree: worktree.to_path_buf(),
            mode,
            package_root,
            package_version_ref,
            policy_epoch: Some(policy_epoch.epoch),
            signed_policy_envelope: Some(policy_epoch.signed_envelope),
            provider_binding_ref: Some(policy_epoch.provider_binding_ref),
            credential_ref: Some(policy_epoch.credential_ref),
            placement_ceiling_ref: Some(policy_epoch.placement_ceiling_ref),
            // Pin the codex endpoint by default (the authed OAuth provider) so a bare
            // model name can't silently resolve to an unauthenticated provider. Resolved
            // once above for the fail-closed credential check.
            provider: Some(provider),
            model: Some(provider_descriptor.model),
            // Per-chat reasoning effort (LLM-1, ADR 0062): unset → the provider default.
            thinking: config.thinking.clone(),
            // Only the editor package receives host-supplied editor framing.
            // Work-chat persona is immutable authored package content.
            system_prompt,
            credential_capability,
            // A linked provider account (ACCT-1), if any — resolved above,
            // nearest-scope-wins (LLM-2, ADR 0062).
            credentials: Vec::new(),
            sandbox: sandbox_policy,
        };
        drive_persistent_turn(
            wb,
            id,
            &gate,
            task,
            images,
            sender,
            factory.as_ref(),
            &spec,
            actor.as_str(),
        )?
    };

    // WS-D: if this chat is homed to a workstream, greedily auto-sync its clean turn
    // into the stream main and let siblings pick it up — the low-friction collaboration
    // hop. A non-member chat (target `main`) is untouched: its merge stays Clean for the
    // human's review, exactly as before.
    greedy_autosync(wb, id, sender);

    // ADR 0082 §4: the shipped no-op rule (ATTN-1 — a turn that changed nothing
    // has no review surface) and the operator's fail-closed advancement rules
    // (ATTN-3) both evaluate here; anything not explicitly advanced holds.
    // The turn's certified guarantee outcomes (when the runtime published a
    // report) take precedence over local workspace truth (ADR 0082 §5).
    auto_advance_turn(wb, id, sender, &result.guarantee_outcomes);

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
        EngagementTurnInput {
            task,
            images,
            mode: ChatMode::Use,
            authenticated_actor: None,
        },
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

/// The settle-time auto-advance (ADR 0082 §4): a settled turn on a **mainline**
/// chat auto-admits and advances its Clean merge when either
///
/// 1. **the shipped no-op rule** (ATTN-1) applies — the diff names no file at
///    all, so the keep would gate nothing (strictly empty only: an
///    internal-only dotfile diff still holds, `.agent-config.json` is where a
///    policy loosening lives); or
/// 2. **an operator advancement rule** (ATTN-3, `advancement.rs`) covers it —
///    fail-closed, with unwaivable config-touch and external-read guards.
///
/// Every advance stays admitted events (`INV-2`/`INV-4`) plus a transcript
/// citation saying *why* no human gated it. Mainline chats only: a workstream
/// member's clean turn is `greedy_autosync`'s job; advancing it here would
/// bypass the membership `Contribute` gate (WS-G).
fn auto_advance_turn(
    wb: &SharedWorkbench,
    id: &str,
    sender: &broadcast::Sender<ServerEvent>,
    guarantee_outcomes: &[gaugewright_harness::GuaranteeOutcome],
) {
    let mut g = wb.lock_unpoisoned();
    g.auto_advance_turn(id, sender, guarantee_outcomes);
}

/// Whether a unified diff names no file — mirrors the web client's `diffHasFiles`
/// (changed-files.ts), so "nothing to review" means the same thing on both sides.
fn diff_names_no_files(diff: &str) -> bool {
    !diff.lines().any(|line| line.starts_with("diff --git "))
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

    // See [`auto_advance_turn`]. Split from `greedy_autosync` because the two
    // advance different refs under different gates: the stream hop is membership-
    // gated (Contribute), the mainline hop is emptiness/rule-gated.
    fn auto_advance_turn(
        &mut self,
        id: &str,
        sender: &broadcast::Sender<ServerEvent>,
        guarantee_outcomes: &[gaugewright_harness::GuaranteeOutcome],
    ) {
        let Some(target) = self.engagements.get(id).map(|e| e.target().to_string()) else {
            return;
        };
        // A workstream member is greedy_autosync's job — never advanced from here.
        let is_member = self
            .engagement_index
            .get(id)
            .and_then(|iid| self.instances.get(iid))
            .and_then(|inst| inst.workstream_id_of(&target))
            .is_some();
        if is_member {
            return;
        }
        if self.store.fold::<MergeState>(id).map(|m| m.phase).ok() != Some(MergePhase::Clean) {
            return;
        }
        let Some(diff) = self
            .engagements
            .get(id)
            .and_then(|e| e.diff_against_main().ok())
        else {
            return; // unreadable diff → hold (fail-closed)
        };
        let (citation, noop) = if diff_names_no_files(&diff) {
            // ATTN-1, the shipped no-op rule: any named file — internal
            // dotfiles included — falls through to the operator rules below.
            (
                "the turn changed no files (no-op rule, ADR 0082)".to_string(),
                true,
            )
        } else {
            // ATTN-3, the operator's advancement rules: fail-closed. Facts are
            // GaugeDesk-owned workspace truth (write side) + the engagement's
            // certified read-set stakeholders (read side); a fact that can't
            // be resolved holds rather than advances.
            let rules = crate::advancement::AdvancementRules::parse(
                self.account_settings()
                    .ok()
                    .and_then(|s| {
                        s.get(crate::advancement::ADVANCEMENT_RULES_SETTING)
                            .cloned()
                    })
                    .as_deref(),
            );
            if rules.is_empty() {
                return;
            }
            let owner = gaugewright_core::determine_scope_authority(id);
            let external =
                crate::resource_store::external_read_stakeholders(&self.store, id, owner.as_str())
                    .unwrap_or_else(|_| vec!["<unresolved>".to_string()]);
            let facts = crate::advancement::TurnFacts {
                changed_paths: crate::advancement::TurnFacts::changed_paths_of(&diff),
                external_read_stakeholders: external,
            };
            // The unwaivable guards apply before EITHER decision path — a
            // certified write guarantee does not certify these axes.
            if facts.violates_safety().is_some() {
                return;
            }
            // Certified-first (ADR 0082 §5): a held operator guarantee advances
            // on the runtime's certificate; a certified violation holds hard,
            // never consulting local truth against it; unwitnessed falls back
            // to the local-truth coverage check.
            let citation = match rules.decide_from_guarantees(guarantee_outcomes) {
                crate::advancement::GuaranteeVerdict::AdvanceHeld(citation) => {
                    format!("{citation} (ADR 0082)")
                }
                crate::advancement::GuaranteeVerdict::HoldViolated(_) => return,
                crate::advancement::GuaranteeVerdict::Unwitnessed => match rules.decide(&facts) {
                    Some(citation) => format!("{citation} (ADR 0082)"),
                    None => return,
                },
            };
            (citation, false)
        };
        let store = &mut self.store;
        let engagements = &self.engagements;
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
            // Raced with another writer — leave it Clean for the review surface.
            _ => return,
        }
        // ADR 0082 §4: every auto-advance is admitted as durable evidence
        // citing the rule it matched. That WHY is governance audit, not
        // conversation — it lands on the engagement's audit record
        // (`GET /chats/:id/audit`), never in the user's transcript. The user
        // surface stays silent for a no-op turn (nothing they can see moved)
        // and says it in plain words when real changes advanced.
        let _ = store.append_record(
            id,
            "audit",
            &serde_json::json!({ "kind": "auto_advance", "citation": citation }).to_string(),
        );
        if !noop {
            let advanced = ServerEvent::Admitted {
                kind: "merge".into(),
                text: "merged to main automatically".to_string(),
            };
            record_transcript(store, id, &advanced);
            let _ = sender.send(advanced);
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
/// `spec`. A caching adapter's harness ([`HarnessFactory::reuse_across_turns`])
/// is **persistent** — created on the first turn and reused thereafter, so
/// the conversation thread carries context across turns; a turn
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
    actor_ref: &str,
) -> Result<TaskResult, String> {
    let mut g = wb.lock_unpoisoned();
    let result =
        g.drive_persistent_local_turn(id, gate, task, images, sender, factory, spec, actor_ref);
    // Advance the onboarding checklist on a completed turn (ADR 0075 Phase 2).
    // Idempotent: once the "first_turn" item is closed, later turns match nothing.
    // Best-effort, under the lock we already hold; never affects the turn result.
    if matches!(&result, Ok(r) if r.run_phase == RunPhase::Completed) {
        g.advance_onboarding("first_turn", &serde_json::json!({ "chat": id }).to_string());
    }
    result
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
        actor_ref: &str,
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
            harness.bind_authenticated_actor(actor_ref);
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
        // Refresh on every request: a persistent chat may be answered by a
        // different authenticated member than the one who created its harness.
        harness.bind_authenticated_actor(actor_ref);
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

    #[derive(Debug)]
    struct PresentCredential;

    impl gaugewright_harness::CredentialCapability for PresentCredential {
        fn credential_ref(&self) -> &str {
            "credential:test"
        }

        fn resolve(
            &self,
            credential_ref: &str,
        ) -> io::Result<gaugewright_harness::CredentialMaterial> {
            if credential_ref != self.credential_ref() {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "wrong ref"));
            }
            Ok(gaugewright_harness::CredentialMaterial::new("secret", None))
        }
    }

    #[test]
    fn runtime_evidence_crossing_is_pointer_only_position_paired_and_idempotent() {
        let mut store = Store::open_in_memory().unwrap();
        let pointer = r#"{"pointer_kind":"event","pointer":{"position":{"instance_ref":"whip:1","sequence":7},"evidence_ref":"whip:evidence:7"}}"#.to_owned();
        let first = admit_runtime_evidence_pointers(
            &mut store,
            "chat-1",
            std::slice::from_ref(&pointer),
            Some("whipple-cut-1"),
        )
        .unwrap();
        let replay = admit_runtime_evidence_pointers(
            &mut store,
            "chat-1",
            std::slice::from_ref(&pointer),
            Some("whipple-cut-1"),
        )
        .unwrap();
        assert_eq!(first, replay);
        let rows = store
            .records("chat-1", RUNTIME_EVIDENCE_POINTER_KIND)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("whip:evidence:7"));
        assert!(rows[0].contains("whipple-cut-1"));
        assert!(!rows[0].contains("evidence_body"));
    }

    // Fail-closed credential check (LLM-1, ADR 0062): a BYOK provider needs its
    // reference-bound capability; absent ⇒ an actionable refusal, never a silent run.
    // The BYOK leg is shell policy — the factory is never consulted for it.
    #[test]
    fn byok_provider_requires_its_linked_key() {
        let pi = gaugewright_pi_bridge::PiHarnessFactory;
        let capability = PresentCredential;
        assert!(llm_credential_status("openai", Some(&capability), &pi).is_ok());
        // nothing linked ⇒ refused with an actionable message
        let err = llm_credential_status("anthropic", None, &pi).unwrap_err();
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
        // Model: override wins, else config, else None (provider default).
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

    // CORE-5: GaugeDesk decides the per-turn egress posture; WhippleScript enforces
    // the admitted provider endpoint without relying on Pi's netns capability.
    #[test]
    fn egress_posture_filters_provider_calls_unless_policy_overrides() {
        use gaugewright_harness::sandbox::Network;
        assert_eq!(egress_posture(false, false), Network::Filtered);
        assert_eq!(egress_posture(true, false), Network::Deny);
        // The explicit operator escape hatch preserves its existing precedence.
        assert_eq!(egress_posture(false, true), Network::Allow);
        assert_eq!(egress_posture(true, true), Network::Allow);
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

    /// The compatibility membrane mirrors native package/control ownership for
    /// fake and retired adapters. WhippleScript is the production authority.
    #[test]
    fn membrane_gate_enforces_the_edit_use_write_gate() {
        use gaugewright_harness::GateDecision;
        let cfg = AgentConfig::default();
        let use_gate = MembraneGate::new(&cfg, default_external_tools()).with_mode(ChatMode::Use);
        // use mode: writing the definition surface is blocked…
        assert!(matches!(
            use_gate.classify_tool("edit", Some(".whipple/draft/persona.md")),
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
            use_gate.classify_tool("read", Some(".whipple/versions/1/persona.md")),
            GateDecision::Allow
        ));
        assert!(matches!(
            use_gate.classify_tool("edit", Some("AGENTS.md")),
            GateDecision::Allow
        ));

        // edit mode: the editor may write the definition surface.
        let edit_gate = MembraneGate::new(&cfg, default_external_tools()).with_mode(ChatMode::Edit);
        assert!(matches!(
            edit_gate.classify_tool("edit", Some(".whipple/draft/persona.md")),
            GateDecision::Allow
        ));
        assert!(matches!(
            edit_gate.classify_tool("edit", Some(".agent-config.json")),
            GateDecision::Block(_)
        ));
    }

    /// Package selection is load-bearing; the OS roots are defense in depth.
    #[test]
    fn method_surface_readonly_roots_use_vs_edit() {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();
        std::fs::create_dir_all(wt.join(".whipple/versions/1")).unwrap();
        std::fs::create_dir_all(wt.join(".whipple/draft")).unwrap();
        std::fs::write(wt.join(".agent-config.json"), "{}").unwrap();

        let ro = method_surface_readonly_roots(wt, ChatMode::Use);
        assert!(ro.contains(&wt.join(".whipple")));
        assert!(ro.contains(&wt.join(".agent-config.json")));

        let edit_ro = method_surface_readonly_roots(wt, ChatMode::Edit);
        assert!(edit_ro.contains(&wt.join(".whipple/versions")));
        assert!(edit_ro.contains(&wt.join(".agent-config.json")));
        assert!(!edit_ro.contains(&wt.join(".whipple/draft")));
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

    #[test]
    fn precheck_failure_does_not_terminalize_a_suspended_epoch() {
        let mut store = Store::open_in_memory().unwrap();
        for command in [
            RunCommand::RequestRun,
            RunCommand::AdmitRun,
            RunCommand::StartRun,
            RunCommand::AwaitHuman,
        ] {
            store.admit::<RunState>("eng-wait", command).unwrap();
        }

        let result = record_precheck_failure(
            &mut store,
            "eng-wait",
            "blue",
            "Reconnect the model credential.".to_owned(),
            "no_credential",
        )
        .unwrap();

        assert_eq!(result.run_phase, RunPhase::AwaitingHuman);
        assert_eq!(
            store.fold::<RunState>("eng-wait").unwrap().phase,
            RunPhase::AwaitingHuman
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

    #[test]
    fn human_question_suspends_without_commit_then_resumes_same_run() {
        use gaugewright_harness::testing::ScriptedHarness;

        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("wt")).unwrap();
        let eng = inst.create_engagement("e1").unwrap();
        let gate = MembraneGate::new(&AgentConfig::default(), default_external_tools());
        let ask = HumanPrompt {
            ask_ref: "ask-1".to_owned(),
            question: "Which color?".to_owned(),
            choices: vec!["blue".to_owned(), "green".to_owned()],
            freeform_allowed: false,
            label_ref: "label-1".to_owned(),
            evidence_ref: "evidence-1".to_owned(),
        };
        let mut harness = ScriptedHarness::new(vec![
            TurnOutcome {
                assistant_text: ask.question.clone(),
                observations: vec![Observation {
                    kind: "human_ask",
                    detail: ask.question.clone(),
                    tool: None,
                }],
                pending_human: Some(ask.clone()),
                ..TurnOutcome::default()
            },
            TurnOutcome {
                assistant_text: "Blue it is.".to_owned(),
                ..TurnOutcome::default()
            },
        ]);
        let mut store = Store::open_in_memory().unwrap();
        let mut sink = |_: &Observation| {};

        let suspended = run_task_streaming(
            &mut store,
            "e1",
            &eng,
            &mut harness,
            &gate,
            "ask if needed",
            &[],
            &mut sink,
        )
        .unwrap();
        assert_eq!(suspended.run_phase, RunPhase::AwaitingHuman);
        assert_eq!(suspended.pending_human, Some(ask));
        assert!(suspended.pending_approvals.is_empty());
        assert!(suspended.commit.is_none());
        assert_eq!(
            store.fold::<RunState>("e1").unwrap().phase,
            RunPhase::AwaitingHuman
        );

        let completed = run_task_streaming(
            &mut store,
            "e1",
            &eng,
            &mut harness,
            &gate,
            "blue",
            &[],
            &mut sink,
        )
        .unwrap();
        assert_eq!(completed.run_phase, RunPhase::Completed);
        assert!(completed.pending_human.is_none());
        assert_eq!(
            store.fold::<RunState>("e1").unwrap().phase,
            RunPhase::Completed
        );
    }

    /// The prompt sent to the model is the **raw task** — no framing prefix.
    /// Persona belongs to the selected authored package (or editor package),
    /// while the transcript records only the raw user task.
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
    /// (no runtime/model call) with a real worktree diff — the E2E path.
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
            EngagementTurnInput {
                task: "do the thing",
                images: &[],
                mode: ChatMode::Use,
                authenticated_actor: None,
            },
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
        let result = run_engagement_turn(
            &wb,
            "e1",
            &worktree,
            &tx,
            EngagementTurnInput {
                task: "go",
                images: &[],
                mode: ChatMode::Use,
                authenticated_actor: None,
            },
        )
        .unwrap();

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

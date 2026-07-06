//! The one harness decision point (SUB-0, ADR 0071 §3): which
//! [`HarnessFactory`] drives a turn. Everything else in the engine is
//! adapter-blind — it resolves the turn's *policy* into a
//! [`HarnessSpec`](gaugewright_harness::HarnessSpec) and lets the selected
//! factory construct the runtime.

use std::io;
use std::path::Path;
use std::sync::Arc;

use gaugewright_harness::{CredentialProbe, Harness, HarnessFactory, HarnessSpec};
use gaugewright_pi_bridge::{PiHarnessFactory, ScriptedTransport};

/// Select the factory for ONE turn. Consulted per turn, never cached at
/// startup: tests flip `GAUGEWRIGHT_FAKE_AGENT` against a live workbench (the
/// `fake_agent_env` guard), so the selection must track the env var turn by
/// turn. `GAUGEWRIGHT_FAKE_AGENT` selects the scripted fake; otherwise the real
/// Pi adapter runs. (A richer harness-kind selection policy is parked to
/// SUB-1.)
pub fn factory_for_turn() -> Arc<dyn HarnessFactory> {
    if std::env::var("GAUGEWRIGHT_FAKE_AGENT").is_ok() {
        Arc::new(ScriptedFakeFactory)
    } else {
        Arc::new(PiHarnessFactory)
    }
}

/// The mock-LLM adapter (`GAUGEWRIGHT_FAKE_AGENT`): no Pi spawn, no model call.
/// A fresh [`ScriptedTransport`] replays the canned Pi wire lines through the
/// real streaming pipeline (membrane + reducers unchanged) every turn. The
/// fixture stays deliberately Pi-shaped — swapping in a neutral scripted
/// harness would risk transcript drift across the e2e suite, so that swap is
/// SUB-1.
pub struct ScriptedFakeFactory;

impl ScriptedFakeFactory {
    /// The stable adapter id the engine branches on for the fake's shell-side
    /// differences: skip provider resolution and the fail-closed credential
    /// precheck, and run [`Self::pre_turn`] first.
    pub const KIND: &'static str = "scripted-fake";

    /// The fake's pre-turn side effects, verbatim from the pre-seam engine: a
    /// `[slow]` task holds the turn open (so the client's busy state — and the
    /// send queue stacked on top of the composer — is observable to the e2e
    /// driver), and the agent appends a line to a note file so the diff/keep
    /// flow — and multi-turn accumulation — is real and deterministic.
    ///
    /// MUST run before the workbench lock is taken (the engine calls it from
    /// the blocking pool, pre-lock): the e2e suite opens chats and queues
    /// messages DURING the `[slow]` window, so holding the workbench mutex
    /// through the sleep would serialize what the tests observe as concurrent.
    pub fn pre_turn(worktree: &Path, task: &str) -> Result<(), String> {
        use std::io::Write;
        if task.contains("[slow]") {
            std::thread::sleep(std::time::Duration::from_millis(3500));
        }
        let note = worktree.join("agent-note.txt");
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&note)
            .map_err(|e| format!("fake agent open: {e}"))?;
        writeln!(f, "agent-note for task: {task}").map_err(|e| format!("fake agent write: {e}"))?;
        Ok(())
    }
}

impl HarnessFactory for ScriptedFakeFactory {
    fn kind(&self) -> &'static str {
        Self::KIND
    }

    /// A fresh transport over the canned wire lines; the spec is ignored (the
    /// fake needs no runtime config — its worktree side effects run in
    /// [`Self::pre_turn`], outside the workbench lock).
    fn create(&self, _spec: &HarnessSpec) -> io::Result<Box<dyn Harness>> {
        Ok(Box::new(ScriptedTransport::new(fake_turn_lines())))
    }

    /// Never cached: a scripted transport is one-shot, so caching it across
    /// turns would fail turn 2 with "stream ended". This preserves the
    /// fresh-transport-per-turn behavior exactly.
    fn reuse_across_turns(&self) -> bool {
        false
    }

    /// The fake needs no credentials. (The engine's fail-closed precheck skips
    /// the fake branch anyway — shell policy, unchanged from the pre-seam
    /// engine.)
    fn credential_status(
        &self,
        _provider: &str,
        _resolved_envs: &[(String, String)],
    ) -> CredentialProbe {
        CredentialProbe::Ready
    }
}

/// The canned Pi event stream for the mock-LLM turn: a streamed text token, a
/// `write` tool call (in-workspace), a `bash` tool call (so the membrane has an
/// effect to rule on — mediated by default, blocked when policy blocks it),
/// end-of-turn, and the final assistant text.
fn fake_turn_lines() -> Vec<String> {
    vec![
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Wrote agent-note.txt."}}"#.into(),
        r#"{"type":"tool_execution_start","toolCallId":"t1","toolName":"write","args":{"path":"agent-note.txt"}}"#.into(),
        r#"{"type":"tool_execution_end","toolCallId":"t1","result":"wrote 1 file","isError":false}"#.into(),
        r#"{"type":"tool_execution_start","toolCallId":"t2","toolName":"bash","args":{"command":"echo hi"}}"#.into(),
        r#"{"type":"tool_execution_end","toolCallId":"t2","isError":false}"#.into(),
        r#"{"type":"agent_end","messages":[]}"#.into(),
        r#"{"type":"response","command":"get_last_assistant_text","success":true,"data":{"text":"Wrote agent-note.txt."}}"#.into(),
    ]
}

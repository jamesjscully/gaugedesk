//! Neutral test doubles for the harness seam.
//!
//! Nothing here is adapter-shaped: the doubles fabricate the seam's own types
//! directly, proving the [`Harness`] contract needs no runtime wire behind it.
//! (The app's `GAUGEWRIGHT_FAKE_AGENT` fake deliberately keeps replaying Pi
//! wire lines through the Pi adapter's transport — swapping it for this
//! neutral double is SUB-1's transcript-drift decision, not SUB-0's.)

use std::collections::VecDeque;
use std::io;

use crate::{EgressGate, Harness, ImageContent, Observation, TurnOutcome};

/// A scripted [`Harness`] that fabricates its turn evidence directly: each
/// queued [`TurnOutcome`] serves exactly one turn, its `observations` streamed
/// to the sink before the outcome is returned (the order every real adapter
/// honors). No Pi wire, no subprocess, no runtime — the template for a future
/// adapter conformance run (SUB-1).
///
/// Running past the script is an error, matching a one-shot transport; a
/// factory over this double should report `reuse_across_turns() == false`.
pub struct ScriptedHarness {
    turns: VecDeque<TurnOutcome>,
}

impl ScriptedHarness {
    pub fn new(turns: Vec<TurnOutcome>) -> Self {
        Self {
            turns: turns.into(),
        }
    }
}

impl Harness for ScriptedHarness {
    fn run_turn(
        &mut self,
        _gate: &dyn EgressGate,
        _prompt: &str,
        _images: &[ImageContent],
        sink: &mut dyn FnMut(&Observation),
    ) -> io::Result<TurnOutcome> {
        let outcome = self
            .turns
            .pop_front()
            .ok_or_else(|| io::Error::other("scripted harness: no turn scripted"))?;
        for obs in &outcome.observations {
            sink(obs);
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AllowAllGate;

    /// The double streams a turn's observations before returning its outcome,
    /// serves one outcome per turn, and fails cleanly past the script.
    #[test]
    fn scripted_harness_streams_then_returns_each_scripted_turn() {
        let mut harness = ScriptedHarness::new(vec![TurnOutcome {
            assistant_text: "done".into(),
            observations: vec![Observation {
                kind: "text",
                detail: "working".into(),
                tool: None,
            }],
            ..TurnOutcome::default()
        }]);

        let mut streamed = Vec::new();
        let outcome = harness
            .run_turn(&AllowAllGate, "go", &[], &mut |o| {
                streamed.push(o.detail.clone())
            })
            .unwrap();
        assert_eq!(streamed, vec!["working".to_string()]);
        assert_eq!(outcome.assistant_text, "done");

        let past_script = harness.run_turn(&AllowAllGate, "again", &[], &mut |_| {});
        assert!(past_script.is_err(), "running past the script is an error");
    }
}

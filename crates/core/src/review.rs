//! Output review / release lifecycle — the `(decide, evolve)` reducer, ported
//! from `specs/models/review-lifecycle.qnt`.
//!
//! Discharges `INV-16` / `SAFE_RELEASE`: an output is `Released` only after
//! **every required stakeholder consented** (conjunctive consent). The first
//! multi-stakeholder reducer; required consenters are `stakeholders \ {recipient}`.

use std::collections::BTreeSet;

use crate::boundary::Authority;
use crate::Rejection;

/// Legal states. `Released | Withheld` are terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReviewPhase {
    Init,
    Proposed,
    Cleared,
    Released,
    Withheld,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ReviewState {
    pub phase: ReviewPhase,
    pub required: BTreeSet<Authority>,
    pub consented: BTreeSet<Authority>,
}

impl Default for ReviewState {
    fn default() -> Self {
        Self {
            phase: ReviewPhase::Init,
            required: BTreeSet::new(),
            consented: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReviewCommand {
    Propose {
        required: BTreeSet<Authority>,
    },
    Consent(Authority),
    Reject(Authority),
    Revoke(Authority),
    Release,
    /// Withdraw/expire a pending proposal (placement policy or the proposer) →
    /// `withheld(canceled)`. Terminal, like a reject.
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReviewEvent {
    Proposed {
        required: BTreeSet<Authority>,
    },
    Consented(Authority),
    Cleared,
    Rejected(Authority),
    ConsentRevoked(Authority),
    Released,
    /// The proposal was withheld for a reason (e.g. `canceled`/`expired`).
    Withheld(String),
}

fn reject(reason: &'static str) -> Result<Vec<ReviewEvent>, Rejection> {
    Err(Rejection { reason })
}

/// `decide` — **pure**. Reads only state + command.
pub fn decide(state: &ReviewState, command: ReviewCommand) -> Result<Vec<ReviewEvent>, Rejection> {
    use ReviewPhase::*;
    match command {
        ReviewCommand::Propose { required } => match state.phase {
            Init => Ok(vec![ReviewEvent::Proposed { required }]),
            _ => reject("propose: an instance already exists"),
        },
        // A required stakeholder consents; clears when the set is complete.
        ReviewCommand::Consent(s) => {
            let ok = state.phase == Proposed
                && state.required.contains(&s)
                && !state.consented.contains(&s);
            if !ok {
                return reject("consent: not a pending required stakeholder");
            }
            let mut events = vec![ReviewEvent::Consented(s.clone())];
            let mut after = state.consented.clone();
            after.insert(s);
            if after == state.required {
                events.push(ReviewEvent::Cleared);
            }
            Ok(events)
        }
        ReviewCommand::Reject(s) => {
            if matches!(state.phase, Proposed | Cleared) && state.required.contains(&s) {
                Ok(vec![ReviewEvent::Rejected(s)])
            } else {
                reject("reject: not a required stakeholder in a reviewable phase")
            }
        }
        ReviewCommand::Revoke(s) => {
            if matches!(state.phase, Proposed | Cleared) && state.consented.contains(&s) {
                Ok(vec![ReviewEvent::ConsentRevoked(s)])
            } else {
                reject("revoke: nothing to revoke (or already released — INV-18)")
            }
        }
        ReviewCommand::Release => match state.phase {
            Cleared => Ok(vec![ReviewEvent::Released]),
            _ => reject("release: not cleared"),
        },
        // cancel → withheld(canceled); only a non-terminal proposal can be withdrawn.
        ReviewCommand::Cancel => match state.phase {
            Proposed | Cleared => Ok(vec![ReviewEvent::Withheld("canceled".into())]),
            _ => reject("cancel: no pending proposal"),
        },
    }
}

/// `evolve` — **pure** fold.
pub fn evolve(state: &ReviewState, event: ReviewEvent) -> ReviewState {
    use ReviewPhase::*;
    let mut s = state.clone();
    match event {
        ReviewEvent::Proposed { required } => {
            s.phase = Proposed;
            s.required = required;
            s.consented.clear();
        }
        ReviewEvent::Consented(a) => {
            s.consented.insert(a);
        }
        ReviewEvent::Cleared => s.phase = Cleared,
        ReviewEvent::Rejected(_) => s.phase = Withheld,
        ReviewEvent::ConsentRevoked(a) => {
            s.consented.remove(&a);
            s.phase = Proposed; // no longer all-consented
        }
        ReviewEvent::Released => s.phase = Released,
        ReviewEvent::Withheld(_) => s.phase = Withheld,
    }
    s
}

impl crate::Lifecycle for ReviewState {
    type State = ReviewState;
    type Command = ReviewCommand;
    type Event = ReviewEvent;
    const KIND: &'static str = "review";
    fn decide(state: &ReviewState, command: ReviewCommand) -> Result<Vec<ReviewEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &ReviewState, event: ReviewEvent) -> ReviewState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn req(names: &[&str]) -> BTreeSet<Authority> {
        names.iter().map(|s| Authority::from(*s)).collect()
    }

    fn apply(state: &ReviewState, command: ReviewCommand) -> ReviewState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }

    #[test]
    fn happy_path_releases_only_after_all_consent() {
        let s = ReviewState::default();
        let s = apply(
            &s,
            ReviewCommand::Propose {
                required: req(&["A", "B"]),
            },
        );
        let s = apply(&s, ReviewCommand::Consent("A".into()));
        assert_eq!(s.phase, ReviewPhase::Proposed, "one consent is not enough");
        let s = apply(&s, ReviewCommand::Consent("B".into()));
        assert_eq!(s.phase, ReviewPhase::Cleared);
        let s = apply(&s, ReviewCommand::Release);
        assert_eq!(s.phase, ReviewPhase::Released);
    }

    #[test]
    fn reject_withholds() {
        let s = ReviewState::default();
        let s = apply(
            &s,
            ReviewCommand::Propose {
                required: req(&["A", "B"]),
            },
        );
        let s = apply(&s, ReviewCommand::Consent("A".into()));
        let s = apply(&s, ReviewCommand::Reject("B".into()));
        assert_eq!(s.phase, ReviewPhase::Withheld);
    }

    #[test]
    fn cancel_withholds_a_pending_proposal() {
        let s = ReviewState::default();
        let s = apply(
            &s,
            ReviewCommand::Propose {
                required: req(&["A", "B"]),
            },
        );
        let s = apply(&s, ReviewCommand::Consent("A".into()));
        let s = apply(&s, ReviewCommand::Cancel);
        assert_eq!(s.phase, ReviewPhase::Withheld, "canceled → withheld");
        // and a released disclosure can't be canceled (terminal, INV-18).
        let r = ReviewState::default();
        let r = apply(
            &r,
            ReviewCommand::Propose {
                required: req(&["A"]),
            },
        );
        let r = apply(&r, ReviewCommand::Consent("A".into()));
        let r = apply(&r, ReviewCommand::Release);
        let r = apply(&r, ReviewCommand::Cancel);
        assert_eq!(
            r.phase,
            ReviewPhase::Released,
            "cancel after release is a no-op"
        );
    }

    #[test]
    fn revoke_before_clear_drops_back() {
        let s = ReviewState::default();
        let s = apply(
            &s,
            ReviewCommand::Propose {
                required: req(&["A", "B"]),
            },
        );
        let s = apply(&s, ReviewCommand::Consent("A".into()));
        let s = apply(&s, ReviewCommand::Consent("B".into()));
        assert_eq!(s.phase, ReviewPhase::Cleared);
        let s = apply(&s, ReviewCommand::Revoke("A".into()));
        assert_eq!(s.phase, ReviewPhase::Proposed, "no longer releasable");
    }

    fn arb_command() -> impl Strategy<Value = ReviewCommand> {
        let who = prop_oneof![
            Just(Authority::from("A")),
            Just(Authority::from("B")),
            Just(Authority::from("X"))
        ];
        prop_oneof![
            Just(ReviewCommand::Propose {
                required: req(&["A", "B"])
            }),
            who.clone().prop_map(ReviewCommand::Consent),
            who.clone().prop_map(ReviewCommand::Reject),
            who.prop_map(ReviewCommand::Revoke),
            Just(ReviewCommand::Release),
            Just(ReviewCommand::Cancel),
        ]
    }

    proptest! {
        /// INV-16 / SAFE_RELEASE: Released ⇒ every required stakeholder consented.
        #[test]
        fn safe_release(commands in prop::collection::vec(arb_command(), 0..40)) {
            let mut s = ReviewState::default();
            for c in commands {
                s = apply(&s, c);
                if s.phase == ReviewPhase::Released {
                    prop_assert!(
                        s.required.is_subset(&s.consented),
                        "released without full consent (INV-16 violated)"
                    );
                }
            }
        }
    }
}

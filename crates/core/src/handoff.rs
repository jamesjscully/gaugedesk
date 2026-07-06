//! Project handoff / authority relocation — the `(decide, evolve)` reducer that
//! relocates a project's single home authority from an **origin** (e.g. a consultant
//! who creates the project) to a **target** (e.g. the client who will host the data
//! and runs). It is the Rust mirror of [`handoff.qnt`](../../../specs/models/handoff.qnt)
//! and implements [`lifecycles/handoff.md`](../../../specs/lifecycles/handoff.md)
//! (`FED-6`).
//!
//! Relocation is a two-phase commit over an unreliable link: ship the relocation
//! **state**, then flip home. The relocation state is the full append-only **log**
//! *and* the project's **content bytes** (its instances' git object graphs — the bytes
//! behind every relocated handle). The reducer encodes the safe ordering so that, in
//! every reachable state:
//! - `EXACTLY_ONE_HOME` — exactly one authority is home; never two (split-brain),
//!   never zero (stranded) (`INV-1`/`INV-7`).
//! - `STATE_BEFORE_HOME` — the target holds the full state (log **and** content bytes)
//!   before it becomes home (`INV-6`/`INV-8`): relocation never loses or forks the log,
//!   and never leaves the new home authoritative over handles whose bytes never arrived.
//! - `COMMIT_TERMINAL` — a committed handoff does not silently reverse; home moves
//!   again only via a fresh handoff.
//! - `ABORT_KEEPS_ORIGIN_HOME` — an interrupted/declined handoff rolls back to the
//!   origin (`INV-23` bounded escape); the project is never stranded.
//!
//! The origin stays home through `offer` and `syncLog`; the single `HandoffCommitted`
//! event is the only transition that flips home. The carriage ([`federation`]) ships the
//! log scopes and the content bundles together and applies both before issuing `SyncLog`,
//! so `LogSynced` records that the **whole** state landed; the commit gate then refuses
//! to flip home unless both are present. Cross-machine identity/admission of the offer
//! rides [`federation`](crate::federation) (`INV-13`); this reducer owns the relocation
//! state machine only.

use crate::Rejection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HandoffPhase {
    /// The project's home is the origin; no handoff in flight.
    Draft,
    /// The origin has offered the handoff and begun shipping the log; origin is
    /// still home (an offer is not a transfer — `INV-13`).
    Offered,
    /// The target holds the full log but is not yet home; origin remains home.
    LogSynced,
    /// The single relocation fact is admitted: home is the target. Terminal.
    Committed,
    /// The handoff was abandoned (cancel/decline/transport failure/timeout); home
    /// remains the origin. Terminal.
    Aborted,
}

/// Which authority currently holds the project's single home. A closed two-state
/// choice — *not* a pair of `home_origin` / `home_target` bools — so the
/// `EXACTLY_ONE_HOME` invariant (never split-brain, never stranded) is a **type
/// guarantee** here, not a checked condition.
///
/// The mirror [`handoff.qnt`](../../../specs/models/handoff.qnt) deliberately keeps
/// the two-bool form: the *model* needs the illegal both/neither states
/// representable so its teeth (`PREMATURE_DEMOTE` / `EAGER_PROMOTE` /
/// `ABORT_ROLLS_FORWARD`) can produce them and prove the invariant catches them.
/// The Rust encoding cannot reach those states at all — a strictly more precise,
/// still-faithful mirror of the same lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Home {
    Origin,
    Target,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandoffState {
    pub phase: HandoffPhase,
    /// Which authority is the project's home — exactly one, always (`INV-1`/`INV-7`).
    pub home: Home,
    /// The target has received the full append-only log.
    pub target_has_log: bool,
    /// The target has received the project's content bytes (its instances' git object
    /// graphs — the bytes behind every relocated handle).
    pub target_has_content: bool,
    /// True once committed — used to assert commit terminality.
    pub sealed: bool,
}

impl Default for HandoffState {
    fn default() -> Self {
        Self {
            phase: HandoffPhase::Draft,
            home: Home::Origin,
            target_has_log: false,
            target_has_content: false,
            sealed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffCommand {
    /// Origin offers the handoff and begins shipping the log.
    OfferHandoff,
    /// Target acknowledges it holds the full log.
    SyncLog,
    /// Commit the relocation — the single fact that moves home to the target.
    CommitHandoff,
    /// Abandon the in-flight handoff; home rolls back to the origin.
    AbortHandoff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HandoffEvent {
    HandoffOffered,
    LogSynced,
    HandoffCommitted,
    HandoffAborted,
}

fn reject(reason: &'static str) -> Result<Vec<HandoffEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(
    state: &HandoffState,
    command: HandoffCommand,
) -> Result<Vec<HandoffEvent>, Rejection> {
    use HandoffPhase::*;
    match command {
        HandoffCommand::OfferHandoff => match state.phase {
            Draft => Ok(vec![HandoffEvent::HandoffOffered]),
            _ => reject("offerHandoff: a handoff is already in flight or terminal"),
        },
        HandoffCommand::SyncLog => match state.phase {
            Offered => Ok(vec![HandoffEvent::LogSynced]),
            _ => reject("syncLog: no offered handoff to sync against"),
        },
        // STATE_BEFORE_HOME: home flips only from logSynced, with the full relocation
        // state present — the log *and* the content bytes.
        HandoffCommand::CommitHandoff => {
            if state.phase == LogSynced && state.target_has_log && state.target_has_content {
                Ok(vec![HandoffEvent::HandoffCommitted])
            } else {
                reject(
                    "commitHandoff: target must hold the full state — log and content — first \
                     (STATE_BEFORE_HOME)",
                )
            }
        }
        // The INV-23 escape; admissible only while in flight, and it rolls back.
        HandoffCommand::AbortHandoff => match state.phase {
            Offered | LogSynced => Ok(vec![HandoffEvent::HandoffAborted]),
            _ => reject("abortHandoff: not in flight"),
        },
    }
}

pub fn evolve(state: &HandoffState, event: HandoffEvent) -> HandoffState {
    use HandoffPhase::*;
    let mut s = *state;
    match event {
        // An offer is not a transfer — the origin stays home (INV-13).
        HandoffEvent::HandoffOffered => s.phase = Offered,
        // The carriage applies both the log scopes and the content bundles before
        // issuing SyncLog, so this records that the whole relocation state landed.
        HandoffEvent::LogSynced => {
            s.phase = LogSynced;
            s.target_has_log = true;
            s.target_has_content = true;
        }
        // The single relocation fact — the only transition that moves home.
        HandoffEvent::HandoffCommitted => {
            s.phase = Committed;
            s.home = Home::Target;
            s.sealed = true;
        }
        // Rollback: home stays with the origin (ABORT_KEEPS_ORIGIN_HOME).
        HandoffEvent::HandoffAborted => {
            s.phase = Aborted;
            s.home = Home::Origin;
        }
    }
    s
}

impl crate::Lifecycle for HandoffState {
    type State = HandoffState;
    type Command = HandoffCommand;
    type Event = HandoffEvent;
    const KIND: &'static str = "handoff";
    fn decide(
        state: &HandoffState,
        command: HandoffCommand,
    ) -> Result<Vec<HandoffEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &HandoffState, event: HandoffEvent) -> HandoffState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn apply(state: &HandoffState, command: HandoffCommand) -> HandoffState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(*state, |s, e| evolve(&s, e)),
            Err(_) => *state,
        }
    }

    #[test]
    fn happy_handoff_relocates_home_to_target() {
        use HandoffCommand::*;
        let s = HandoffState::default();
        assert_eq!(s.home, Home::Origin, "origin starts home");
        let s = apply(&s, OfferHandoff);
        assert_eq!(s.home, Home::Origin, "offer is not a transfer");
        let s = apply(&s, SyncLog);
        assert!(
            s.home == Home::Origin && s.target_has_log && s.target_has_content,
            "state synced (log + content), origin still home"
        );
        let s = apply(&s, CommitHandoff);
        assert_eq!(s.phase, HandoffPhase::Committed);
        assert_eq!(s.home, Home::Target, "home relocated to target");
        assert!(s.sealed);
    }

    #[test]
    fn commit_requires_log_synced() {
        // STATE_BEFORE_HOME: cannot flip home straight from offered.
        let s = apply(&HandoffState::default(), HandoffCommand::OfferHandoff);
        assert!(decide(&s, HandoffCommand::CommitHandoff).is_err());
    }

    #[test]
    fn commit_requires_content_not_just_log() {
        // STATE_BEFORE_HOME: a target that holds the log but not the content bytes
        // (a content-blind would-be home) must not be able to commit. The carriage
        // never produces this state, but the gate is what makes that a *contract*.
        let s = HandoffState {
            phase: HandoffPhase::LogSynced,
            home: Home::Origin,
            target_has_log: true,
            target_has_content: false,
            sealed: false,
        };
        assert!(
            decide(&s, HandoffCommand::CommitHandoff).is_err(),
            "commit must be refused while the content bytes are absent"
        );
    }

    #[test]
    fn abort_rolls_back_to_origin() {
        use HandoffCommand::*;
        let s = apply(&HandoffState::default(), OfferHandoff);
        let s = apply(&s, AbortHandoff);
        assert_eq!(s.phase, HandoffPhase::Aborted);
        assert_eq!(s.home, Home::Origin, "abort keeps origin home");
    }

    #[test]
    fn terminal_is_final() {
        use HandoffCommand::*;
        // committed: no further transition.
        let s = apply(&HandoffState::default(), OfferHandoff);
        let s = apply(&s, SyncLog);
        let committed = apply(&s, CommitHandoff);
        for c in [OfferHandoff, SyncLog, CommitHandoff, AbortHandoff] {
            assert!(decide(&committed, c).is_err(), "committed is terminal");
        }
        // aborted: no further transition.
        let aborted = apply(&apply(&HandoffState::default(), OfferHandoff), AbortHandoff);
        for c in [OfferHandoff, SyncLog, CommitHandoff, AbortHandoff] {
            assert!(decide(&aborted, c).is_err(), "aborted is terminal");
        }
    }

    fn arb_command() -> impl Strategy<Value = HandoffCommand> {
        use HandoffCommand::*;
        prop_oneof![
            Just(OfferHandoff),
            Just(SyncLog),
            Just(CommitHandoff),
            Just(AbortHandoff),
        ]
    }

    proptest! {
        /// The handoff invariants — mirrors handoff.qnt — hold over every reachable trace.
        #[test]
        fn handoff_invariants(commands in prop::collection::vec(arb_command(), 0..50)) {
            let mut s = HandoffState::default();
            for c in commands {
                s = apply(&s, c);
                // EXACTLY_ONE_HOME is now a *type* guarantee — `Home` is a single
                // two-state choice, so split-brain/stranded are unrepresentable. The
                // model (`handoff.qnt`) is where the two-bool form + teeth prove the
                // lifecycle logic can never *reach* an illegal home.
                // STATE_BEFORE_HOME: the target is home only if it holds the full
                // relocation state — the log AND the content bytes.
                if s.home == Home::Target {
                    prop_assert!(s.target_has_log, "target became home without the full log");
                    prop_assert!(
                        s.target_has_content,
                        "target became home without the content bytes"
                    );
                }
                // COMMIT_TERMINAL: once sealed, home stays with the target.
                if s.sealed {
                    prop_assert_eq!(s.home, Home::Target, "committed home reversed");
                }
                // ABORT_KEEPS_ORIGIN_HOME.
                if s.phase == HandoffPhase::Aborted {
                    prop_assert_eq!(s.home, Home::Origin, "abort did not keep origin home");
                }
            }
        }
    }
}

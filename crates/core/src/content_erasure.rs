//! Content erasure / tombstone lifecycle — ported from
//! `specs/models/content-erasure.qnt`.
//!
//! Erasure tombstones a payload (makes it unresolvable for future use) **without**
//! rewriting history. Discharges:
//! - `TOMBSTONE_REQUIRES_APPROVAL` — a tombstone needs the erasure authority's
//!   approval (no ghost approver counts).
//! - `TOMBSTONE_BLOCKS_FUTURE_RESOLUTION` — a tombstoned payload never resolves
//!   again for new use.
//! - `HISTORY_PRESERVED` / `METADATA_PRESERVED` — historical handle/event
//!   references and audit metadata survive erasure (`INV-6`/`INV-18`).
//! - `EXPORTED_COPY_NOT_RECALLED` — a copy already past the boundary edge is not
//!   recalled by erasure (`INV-18`).

use std::collections::BTreeSet;

use crate::boundary::Authority;
use crate::Rejection;

/// The single authorized erasure authority (the owner of the content). Named once
/// here; comparisons brand it into an [`Authority`] rather than matching bare `&str`.
pub const OWNER: &str = "owner";

/// The owner principal as a typed [`Authority`] — the privileged erasure approver.
fn owner() -> Authority {
    Authority::new(OWNER)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ErasurePhase {
    Init,
    Requested,
    Approved,
    Denied,
    Tombstoned,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErasureState {
    pub phase: ErasurePhase,
    pub approved_by: BTreeSet<Authority>,
    pub tombstoned: bool,
    pub payload_resolvable: bool,
    pub historical_handle_present: bool,
    pub metadata_present: bool,
    pub ever_exported: bool,
    pub exported_copy_held: bool,
}

impl Default for ErasureState {
    fn default() -> Self {
        Self {
            phase: ErasurePhase::Init,
            approved_by: BTreeSet::new(),
            tombstoned: false,
            payload_resolvable: true,
            historical_handle_present: true,
            metadata_present: true,
            ever_exported: false,
            exported_copy_held: false,
        }
    }
}

impl ErasureState {
    /// The erasure is authorized iff the owner has approved (`approved` in Quint).
    fn approved(&self) -> bool {
        self.approved_by.contains(&owner())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErasureCommand {
    RequestErasure,
    Approve(Authority),
    Reject,
    /// Commit the tombstone (requires owner approval — `TOMBSTONE_REQUIRES_APPROVAL`).
    Tombstone,
    TombstoneFailure,
    RetryTombstone,
    /// Withdraw a pending erasure request → `denied`.
    Cancel,
    /// Placement policy expires a pending erasure request → `denied`.
    Expire,
    /// A copy crosses the boundary edge before any tombstone.
    ExportBeforeTombstone,
    /// Resolve the payload for use; blocked once tombstoned.
    ResolvePayload,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ErasureEvent {
    ErasureRequested,
    Approved(Authority),
    Rejected,
    Tombstoned,
    TombstoneFailed,
    TombstoneRetried,
    ErasureCanceled,
    ErasureExpired,
    Exported,
    PayloadResolved,
}

fn reject(reason: &'static str) -> Result<Vec<ErasureEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(
    state: &ErasureState,
    command: ErasureCommand,
) -> Result<Vec<ErasureEvent>, Rejection> {
    use ErasurePhase::*;
    match command {
        ErasureCommand::RequestErasure => match state.phase {
            Init => Ok(vec![ErasureEvent::ErasureRequested]),
            _ => reject("requestErasure: not in init"),
        },
        // NO_GHOST_APPROVAL: only the owner is an authorized approver.
        ErasureCommand::Approve(a) => {
            if state.phase == Requested && a.as_str() == OWNER {
                Ok(vec![ErasureEvent::Approved(a)])
            } else {
                reject("approve: not a pending owner approval")
            }
        }
        ErasureCommand::Reject => match state.phase {
            Requested => Ok(vec![ErasureEvent::Rejected]),
            _ => reject("reject: not requested"),
        },
        // Negative paths on a pending request → denied (the tombstone never runs).
        ErasureCommand::Cancel => match state.phase {
            Requested => Ok(vec![ErasureEvent::ErasureCanceled]),
            _ => reject("cancel: no pending erasure request"),
        },
        ErasureCommand::Expire => match state.phase {
            Requested | Approved => Ok(vec![ErasureEvent::ErasureExpired]),
            _ => reject("expireErasure: no pending erasure"),
        },
        // TOMBSTONE_REQUIRES_APPROVAL: never tombstone without owner approval.
        ErasureCommand::Tombstone => {
            if state.approved() {
                Ok(vec![ErasureEvent::Tombstoned])
            } else {
                reject("tombstone: requires owner approval (TOMBSTONE_REQUIRES_APPROVAL)")
            }
        }
        ErasureCommand::TombstoneFailure => match state.phase {
            Approved => Ok(vec![ErasureEvent::TombstoneFailed]),
            _ => reject("tombstoneFailure: not approved"),
        },
        ErasureCommand::RetryTombstone => match state.phase {
            Failed => Ok(vec![ErasureEvent::TombstoneRetried]),
            _ => reject("retryTombstone: not failed"),
        },
        ErasureCommand::ExportBeforeTombstone => {
            if !state.tombstoned {
                Ok(vec![ErasureEvent::Exported])
            } else {
                reject("exportBeforeTombstone: already tombstoned")
            }
        }
        // TOMBSTONE_BLOCKS_FUTURE_RESOLUTION: no resolve once tombstoned.
        ErasureCommand::ResolvePayload => {
            if state.payload_resolvable {
                Ok(vec![ErasureEvent::PayloadResolved])
            } else {
                reject("resolvePayload: tombstoned, not resolvable (TOMBSTONE_BLOCKS_FUTURE_RESOLUTION)")
            }
        }
    }
}

pub fn evolve(state: &ErasureState, event: ErasureEvent) -> ErasureState {
    use ErasurePhase::*;
    let mut s = state.clone();
    match event {
        ErasureEvent::ErasureRequested => s.phase = Requested,
        ErasureEvent::Approved(a) => {
            s.phase = Approved;
            s.approved_by.insert(a);
        }
        ErasureEvent::Rejected | ErasureEvent::ErasureCanceled | ErasureEvent::ErasureExpired => {
            s.phase = Denied;
        }
        ErasureEvent::Tombstoned => {
            s.phase = Tombstoned;
            s.tombstoned = true;
            s.payload_resolvable = false;
            // history, metadata, and any exported copy are preserved — by construction.
        }
        ErasureEvent::TombstoneFailed => s.phase = Failed,
        ErasureEvent::TombstoneRetried => s.phase = Approved,
        ErasureEvent::Exported => {
            s.ever_exported = true;
            s.exported_copy_held = true;
        }
        ErasureEvent::PayloadResolved => {} // evidence of use; no state change
    }
    s
}

impl crate::Lifecycle for ErasureState {
    type State = ErasureState;
    type Command = ErasureCommand;
    type Event = ErasureEvent;
    const KIND: &'static str = "content_erasure";
    fn decide(
        state: &ErasureState,
        command: ErasureCommand,
    ) -> Result<Vec<ErasureEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &ErasureState, event: ErasureEvent) -> ErasureState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn apply(state: &ErasureState, command: ErasureCommand) -> ErasureState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }

    #[test]
    fn cancel_and_expire_deny_a_pending_request() {
        let c = apply(&ErasureState::default(), ErasureCommand::RequestErasure);
        let c = apply(&c, ErasureCommand::Cancel);
        assert_eq!(c.phase, ErasurePhase::Denied);
        // a denied request cannot then be tombstoned (TOMBSTONE_REQUIRES_APPROVAL).
        assert!(decide(&c, ErasureCommand::Tombstone).is_err());
        let e = apply(&ErasureState::default(), ErasureCommand::RequestErasure);
        let e = apply(&e, ErasureCommand::Expire);
        assert_eq!(e.phase, ErasurePhase::Denied);
    }

    #[test]
    fn approve_then_tombstone_preserves_history() {
        let s = ErasureState::default();
        let s = apply(&s, ErasureCommand::RequestErasure);
        let s = apply(&s, ErasureCommand::Approve(OWNER.into()));
        let s = apply(&s, ErasureCommand::Tombstone);
        assert!(s.tombstoned && !s.payload_resolvable);
        assert!(
            s.historical_handle_present && s.metadata_present,
            "history/metadata preserved"
        );
    }

    #[test]
    fn export_then_tombstone_does_not_recall() {
        let s = ErasureState::default();
        let s = apply(&s, ErasureCommand::ExportBeforeTombstone);
        let s = apply(&s, ErasureCommand::RequestErasure);
        let s = apply(&s, ErasureCommand::Approve(OWNER.into()));
        let s = apply(&s, ErasureCommand::Tombstone);
        assert!(s.exported_copy_held, "EXPORTED_COPY_NOT_RECALLED");
        assert!(!s.payload_resolvable);
    }

    #[test]
    fn tombstone_without_approval_rejected() {
        let s = ErasureState::default();
        let s = apply(&s, ErasureCommand::RequestErasure);
        assert!(
            decide(&s, ErasureCommand::Tombstone).is_err(),
            "TOMBSTONE_REQUIRES_APPROVAL"
        );
    }

    fn arb_command() -> impl Strategy<Value = ErasureCommand> {
        prop_oneof![
            Just(ErasureCommand::RequestErasure),
            prop_oneof![Just(owner()), Just(Authority::from("ghost"))]
                .prop_map(ErasureCommand::Approve),
            Just(ErasureCommand::Reject),
            Just(ErasureCommand::Cancel),
            Just(ErasureCommand::Expire),
            Just(ErasureCommand::Tombstone),
            Just(ErasureCommand::TombstoneFailure),
            Just(ErasureCommand::RetryTombstone),
            Just(ErasureCommand::ExportBeforeTombstone),
            Just(ErasureCommand::ResolvePayload),
        ]
    }

    proptest! {
        /// The erasure invariants hold over every reachable trace.
        #[test]
        fn erasure_invariants(commands in prop::collection::vec(arb_command(), 0..50)) {
            let mut s = ErasureState::default();
            let mut resolved_after_tombstone = false;
            for c in commands {
                let resolving = c == ErasureCommand::ResolvePayload
                    && decide(&s, ErasureCommand::ResolvePayload).is_ok();
                if resolving && s.tombstoned {
                    resolved_after_tombstone = true;
                }
                s = apply(&s, c);

                // TOMBSTONE_REQUIRES_APPROVAL: a tombstone implies owner approval.
                if s.tombstoned {
                    prop_assert!(s.approved_by.contains(&owner()));
                }
                // NO_GHOST_APPROVAL: only the owner is ever recorded as approver.
                prop_assert!(!s.approved_by.contains(&Authority::from("ghost")));
                // TOMBSTONE_BLOCKS_FUTURE_RESOLUTION.
                prop_assert!(!resolved_after_tombstone);
                // HISTORY_PRESERVED / METADATA_PRESERVED.
                prop_assert!(s.historical_handle_present && s.metadata_present);
                // EXPORTED_COPY_NOT_RECALLED.
                prop_assert!(!s.ever_exported || s.exported_copy_held);
            }
        }
    }
}

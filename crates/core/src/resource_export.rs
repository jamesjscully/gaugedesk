//! Resource export lifecycle — the general egress lifecycle, ported from
//! `specs/models/resource-export.qnt`.
//!
//! Discharges `EXPORT_REQUIRES_SOURCE_AND_TARGET`: a resource is `Exported` only
//! after **every required source stakeholder consented AND the target admitted**
//! (`INV-13`). Export is the irreversible boundary-edge crossing; revocation
//! before export blocks it and is future-only (`INV-18`).

use std::collections::BTreeSet;

use crate::boundary::Authority;
use crate::Rejection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExportPhase {
    Init,
    Requested,
    Cleared,
    Exported,
    /// Terminal: a source rejected, or the proposal was canceled/expired.
    Denied,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ExportState {
    pub phase: ExportPhase,
    pub source_required: BTreeSet<Authority>,
    pub source_consented: BTreeSet<Authority>,
    pub target_admitted: bool,
}

impl Default for ExportState {
    fn default() -> Self {
        Self {
            phase: ExportPhase::Init,
            source_required: BTreeSet::new(),
            source_consented: BTreeSet::new(),
            target_admitted: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExportCommand {
    ProposeExport {
        source_required: BTreeSet<Authority>,
    },
    SourceConsent(Authority),
    TargetAdmit,
    Revoke(Authority),
    Export,
    /// A source stakeholder rejects the export → `denied` (terminal).
    Reject(Authority),
    /// Withdraw a pending proposal → `denied`.
    Cancel,
    /// Placement policy expires a pending proposal → `denied`.
    Expire,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExportEvent {
    ExportProposed {
        source_required: BTreeSet<Authority>,
    },
    SourceConsented(Authority),
    TargetAdmitted,
    Cleared,
    ConsentRevoked(Authority),
    Exported,
    ExportRejected(Authority),
    ExportCanceled,
    ExportExpired,
}

fn reject(reason: &'static str) -> Result<Vec<ExportEvent>, Rejection> {
    Err(Rejection { reason })
}

fn ready(required: &BTreeSet<Authority>, consented: &BTreeSet<Authority>, target: bool) -> bool {
    required.is_subset(consented) && target
}

pub fn decide(state: &ExportState, command: ExportCommand) -> Result<Vec<ExportEvent>, Rejection> {
    use ExportPhase::*;
    match command {
        ExportCommand::ProposeExport { source_required } => match state.phase {
            Init => Ok(vec![ExportEvent::ExportProposed { source_required }]),
            _ => reject("proposeExport: already exists"),
        },
        ExportCommand::SourceConsent(a) => {
            let ok = state.phase == Requested
                && state.source_required.contains(&a)
                && !state.source_consented.contains(&a);
            if !ok {
                return reject("sourceConsent: not a pending source stakeholder");
            }
            let mut events = vec![ExportEvent::SourceConsented(a.clone())];
            let mut after = state.source_consented.clone();
            after.insert(a);
            if ready(&state.source_required, &after, state.target_admitted) {
                events.push(ExportEvent::Cleared);
            }
            Ok(events)
        }
        ExportCommand::TargetAdmit => {
            if state.phase != Requested || state.target_admitted {
                return reject("targetAdmit: not applicable");
            }
            let mut events = vec![ExportEvent::TargetAdmitted];
            if ready(&state.source_required, &state.source_consented, true) {
                events.push(ExportEvent::Cleared);
            }
            Ok(events)
        }
        ExportCommand::Revoke(a) => {
            if matches!(state.phase, Requested | Cleared) && state.source_consented.contains(&a) {
                Ok(vec![ExportEvent::ConsentRevoked(a)])
            } else {
                reject("revoke: nothing to revoke (or already exported — INV-18)")
            }
        }
        ExportCommand::Export => match state.phase {
            Cleared => Ok(vec![ExportEvent::Exported]),
            _ => reject("export: not cleared (EXPORT_REQUIRES_SOURCE_AND_TARGET)"),
        },
        // Negative paths → denied (terminal). A source may reject; the proposer
        // may cancel; placement policy may expire — never after `exported` (INV-18).
        ExportCommand::Reject(a) => {
            if matches!(state.phase, Requested | Cleared) && state.source_required.contains(&a) {
                Ok(vec![ExportEvent::ExportRejected(a)])
            } else {
                reject("rejectExport: not a source stakeholder in a reviewable phase")
            }
        }
        ExportCommand::Cancel => match state.phase {
            Requested => Ok(vec![ExportEvent::ExportCanceled]),
            _ => reject("cancel: no pending export request"),
        },
        ExportCommand::Expire => match state.phase {
            Requested | Cleared => Ok(vec![ExportEvent::ExportExpired]),
            _ => reject("expireExport: no pending proposal"),
        },
    }
}

pub fn evolve(state: &ExportState, event: ExportEvent) -> ExportState {
    use ExportPhase::*;
    let mut s = state.clone();
    match event {
        ExportEvent::ExportProposed { source_required } => {
            s.phase = Requested;
            s.source_required = source_required;
            s.source_consented.clear();
            s.target_admitted = false;
        }
        ExportEvent::SourceConsented(a) => {
            s.source_consented.insert(a);
        }
        ExportEvent::TargetAdmitted => s.target_admitted = true,
        ExportEvent::Cleared => s.phase = Cleared,
        ExportEvent::ConsentRevoked(a) => {
            s.source_consented.remove(&a);
            s.phase = Requested; // no longer cleared
        }
        ExportEvent::Exported => s.phase = Exported,
        ExportEvent::ExportRejected(_)
        | ExportEvent::ExportCanceled
        | ExportEvent::ExportExpired => {
            s.phase = Denied;
        }
    }
    s
}

impl crate::Lifecycle for ExportState {
    type State = ExportState;
    type Command = ExportCommand;
    type Event = ExportEvent;
    const KIND: &'static str = "resource_export";
    fn decide(state: &ExportState, command: ExportCommand) -> Result<Vec<ExportEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &ExportState, event: ExportEvent) -> ExportState {
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
    fn apply(state: &ExportState, command: ExportCommand) -> ExportState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }

    #[test]
    fn export_needs_source_and_target() {
        let s = ExportState::default();
        let s = apply(
            &s,
            ExportCommand::ProposeExport {
                source_required: req(&["A"]),
            },
        );
        let s = apply(&s, ExportCommand::SourceConsent("A".into()));
        assert_eq!(
            s.phase,
            ExportPhase::Requested,
            "source consent alone is not enough"
        );
        let s = apply(&s, ExportCommand::TargetAdmit);
        assert_eq!(s.phase, ExportPhase::Cleared);
        let s = apply(&s, ExportCommand::Export);
        assert_eq!(s.phase, ExportPhase::Exported);
    }

    #[test]
    fn revoke_blocks_export() {
        let s = ExportState::default();
        let s = apply(
            &s,
            ExportCommand::ProposeExport {
                source_required: req(&["A"]),
            },
        );
        let s = apply(&s, ExportCommand::SourceConsent("A".into()));
        let s = apply(&s, ExportCommand::TargetAdmit);
        assert_eq!(s.phase, ExportPhase::Cleared);
        let s = apply(&s, ExportCommand::Revoke("A".into()));
        assert_eq!(s.phase, ExportPhase::Requested);
        assert!(
            decide(&s, ExportCommand::Export).is_err(),
            "REVOCATION_BLOCKS_EXPORT"
        );
    }

    #[test]
    fn reject_cancel_expire_deny_the_export() {
        // a source reject denies a pending export.
        let s = ExportState::default();
        let s = apply(
            &s,
            ExportCommand::ProposeExport {
                source_required: req(&["A", "B"]),
            },
        );
        let s = apply(&s, ExportCommand::Reject("B".into()));
        assert_eq!(s.phase, ExportPhase::Denied);
        // cancel denies a requested proposal; not after exported (terminal).
        let c = ExportState::default();
        let c = apply(
            &c,
            ExportCommand::ProposeExport {
                source_required: req(&["A"]),
            },
        );
        let c = apply(&c, ExportCommand::Cancel);
        assert_eq!(c.phase, ExportPhase::Denied);
        let e = ExportState::default();
        let e = apply(
            &e,
            ExportCommand::ProposeExport {
                source_required: req(&["A"]),
            },
        );
        let e = apply(&e, ExportCommand::SourceConsent("A".into()));
        let e = apply(&e, ExportCommand::TargetAdmit);
        let e = apply(&e, ExportCommand::Export);
        let e = apply(&e, ExportCommand::Cancel); // no-op after exported
        assert_eq!(e.phase, ExportPhase::Exported);
    }

    fn arb_command() -> impl Strategy<Value = ExportCommand> {
        let who = prop_oneof![
            Just(Authority::from("A")),
            Just(Authority::from("B")),
            Just(Authority::from("X"))
        ];
        prop_oneof![
            Just(ExportCommand::ProposeExport {
                source_required: req(&["A", "B"])
            }),
            who.clone().prop_map(ExportCommand::SourceConsent),
            Just(ExportCommand::TargetAdmit),
            who.clone().prop_map(ExportCommand::Revoke),
            who.prop_map(ExportCommand::Reject),
            Just(ExportCommand::Export),
            Just(ExportCommand::Cancel),
            Just(ExportCommand::Expire),
        ]
    }

    proptest! {
        /// EXPORT_REQUIRES_SOURCE_AND_TARGET (INV-13): Exported ⇒ all required
        /// source stakeholders consented AND the target admitted.
        #[test]
        fn export_requires_source_and_target(commands in prop::collection::vec(arb_command(), 0..40)) {
            let mut s = ExportState::default();
            for c in commands {
                s = apply(&s, c);
                if s.phase == ExportPhase::Exported {
                    prop_assert!(
                        s.source_required.is_subset(&s.source_consented) && s.target_admitted,
                        "exported without full source consent + target admission"
                    );
                }
            }
        }
    }
}

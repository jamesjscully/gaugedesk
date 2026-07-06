//! Deployment entitlement lifecycle — the `(decide, evolve)` reducer, ported from
//! `specs/models/deployment-entitlement.qnt` (ADR 0021). M2.
//!
//! Gates **governed** package use: one installed package × one deployment context.
//! An active entitlement says the context is *currently allowed* to use the install
//! for governed runs. Discharges:
//! - `ENTITLEMENT_REQUIRES_AUTHORITY` — an active entitlement requires the
//!   entitlement authority's basis. Billing/support receipts are **evidence, not
//!   authority** — they never activate.
//! - `RUN_ELIGIBILITY_REQUIRES_ACTIVE_ENTITLEMENT` — governed run eligibility
//!   requires an active entitlement.
//! - `SUSPENSION_BLOCKS_FUTURE_RUNS` — suspended/closed blocks future governed runs.
//! - `HISTORY_PRESERVED_ON_CLOSE` — closeout is future-only; it never erases run or
//!   receipt history (`INV-18`).

use crate::Rejection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EntitlementPhase {
    None,
    Requested,
    Active,
    Suspended,
    Closed,
    Denied,
    /// The package install backing the entitlement was withdrawn — terminal. The
    /// entitlement is void; future runs / key release are blocked.
    Withdrawn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntitlementState {
    pub phase: EntitlementPhase,
    pub package_installed: bool,
    pub package_eligible: bool,
    /// The entitlement authority admitted it (the load-bearing basis).
    pub activated_by_authority: bool,
    pub billing_receipt: bool,
    pub support_receipt: bool,
    pub run_eligible: bool,
    pub ever_run: bool,
    pub history_present: bool,
}

impl Default for EntitlementState {
    fn default() -> Self {
        Self {
            phase: EntitlementPhase::None,
            package_installed: false,
            package_eligible: false,
            activated_by_authority: false,
            billing_receipt: false,
            support_receipt: false,
            run_eligible: false,
            ever_run: false,
            history_present: false,
        }
    }
}

impl EntitlementState {
    /// Whether the entitlement is in its active phase. **Derived from `phase`** — the
    /// lifecycle keeps `phase` authoritative (withdraw → `Withdrawn`, suspend →
    /// `Suspended`, close → `Closed` all move it out of `Active`), so this can never
    /// disagree with the phase the way the old `entitlement_active` bool could.
    pub fn entitlement_active(&self) -> bool {
        matches!(self.phase, EntitlementPhase::Active)
    }
    /// Whether the entitlement is currently suspended (derived from `phase`).
    pub fn suspended(&self) -> bool {
        matches!(self.phase, EntitlementPhase::Suspended)
    }
    /// Whether the entitlement has been closed out (derived from `phase`).
    pub fn closed(&self) -> bool {
        matches!(self.phase, EntitlementPhase::Closed)
    }
    /// A governed run is eligible only under a live, authority-admitted entitlement:
    /// the install is present and eligible, the entitlement is in its `Active` phase,
    /// and the authority admitted it. Eligibility gates (sealed-key release, billing)
    /// consult this. Now that `phase` is authoritative it is true exactly when
    /// `phase == Active` (the install/authority facts always hold there).
    pub fn active_entitlement(&self) -> bool {
        self.package_installed
            && self.package_eligible
            && self.entitlement_active()
            && self.activated_by_authority
            && !self.suspended()
            && !self.closed()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntitlementCommand {
    AdmitPackageInstall,
    RequestEntitlement,
    ActivateEntitlement,
    RecordBillingReceipt,
    RecordSupportReceipt,
    MarkRunEligible,
    StartGovernedRun,
    SuspendEntitlement,
    ResumeEntitlement,
    CloseEntitlement,
    DenyOrExpire,
    WithdrawPackageInstall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EntitlementEvent {
    PackageInstallAdmitted,
    EntitlementRequested,
    EntitlementActivated,
    BillingReceiptRecorded,
    SupportReceiptRecorded,
    RunMarkedEligible,
    GovernedRunStarted,
    EntitlementSuspended,
    EntitlementResumed,
    EntitlementClosed,
    EntitlementDenied,
    PackageInstallWithdrawn,
}

fn reject(reason: &'static str) -> Result<Vec<EntitlementEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(
    state: &EntitlementState,
    command: EntitlementCommand,
) -> Result<Vec<EntitlementEvent>, Rejection> {
    use EntitlementPhase::*;
    match command {
        EntitlementCommand::AdmitPackageInstall => match state.phase {
            None => Ok(vec![EntitlementEvent::PackageInstallAdmitted]),
            _ => reject("admitPackageInstall: entitlement already started"),
        },
        EntitlementCommand::RequestEntitlement => {
            if state.phase == None && state.package_installed {
                Ok(vec![EntitlementEvent::EntitlementRequested])
            } else {
                reject("requestEntitlement: needs an installed package")
            }
        }
        // ENTITLEMENT_REQUIRES_AUTHORITY: activation is the authority's act.
        EntitlementCommand::ActivateEntitlement => {
            if state.phase == Requested && state.package_installed && state.package_eligible {
                Ok(vec![EntitlementEvent::EntitlementActivated])
            } else {
                reject("activateEntitlement: needs a request on an eligible install")
            }
        }
        // Receipts are evidence — recordable across live phases, never activating.
        EntitlementCommand::RecordBillingReceipt => match state.phase {
            Requested | Active | Suspended | Closed => {
                Ok(vec![EntitlementEvent::BillingReceiptRecorded])
            }
            _ => reject("recordBillingReceipt: no entitlement to attach to"),
        },
        EntitlementCommand::RecordSupportReceipt => match state.phase {
            Requested | Active | Suspended | Closed => {
                Ok(vec![EntitlementEvent::SupportReceiptRecorded])
            }
            _ => reject("recordSupportReceipt: no entitlement to attach to"),
        },
        // RUN_ELIGIBILITY_REQUIRES_ACTIVE_ENTITLEMENT / SUSPENSION_BLOCKS_FUTURE_RUNS.
        EntitlementCommand::MarkRunEligible => {
            if state.active_entitlement() {
                Ok(vec![EntitlementEvent::RunMarkedEligible])
            } else {
                reject("markRunEligible: requires an active entitlement")
            }
        }
        EntitlementCommand::StartGovernedRun => {
            if state.run_eligible {
                Ok(vec![EntitlementEvent::GovernedRunStarted])
            } else {
                reject("startGovernedRun: not run-eligible")
            }
        }
        EntitlementCommand::SuspendEntitlement => match state.phase {
            Active => Ok(vec![EntitlementEvent::EntitlementSuspended]),
            _ => reject("suspendEntitlement: not active"),
        },
        EntitlementCommand::ResumeEntitlement => {
            if state.phase == Suspended && state.package_installed && state.package_eligible {
                Ok(vec![EntitlementEvent::EntitlementResumed])
            } else {
                reject("resumeEntitlement: needs a suspended, still-eligible entitlement")
            }
        }
        EntitlementCommand::CloseEntitlement => match state.phase {
            Active | Suspended => Ok(vec![EntitlementEvent::EntitlementClosed]),
            _ => reject("closeEntitlement: not active/suspended"),
        },
        EntitlementCommand::DenyOrExpire => match state.phase {
            Requested => Ok(vec![EntitlementEvent::EntitlementDenied]),
            _ => reject("denyOrExpire: no pending request"),
        },
        EntitlementCommand::WithdrawPackageInstall => {
            if state.package_installed {
                Ok(vec![EntitlementEvent::PackageInstallWithdrawn])
            } else {
                reject("withdrawPackageInstall: nothing installed")
            }
        }
    }
}

pub fn evolve(state: &EntitlementState, event: EntitlementEvent) -> EntitlementState {
    use EntitlementPhase::*;
    let mut s = *state;
    match event {
        EntitlementEvent::PackageInstallAdmitted => {
            s.package_installed = true;
            s.package_eligible = true;
            s.history_present = true;
        }
        EntitlementEvent::EntitlementRequested => s.phase = Requested,
        // `phase` is now authoritative for active/suspended/closed — those are
        // derived from it, so the transitions set only `phase` plus the genuinely
        // orthogonal facts (authority basis, run-eligibility, history).
        EntitlementEvent::EntitlementActivated => {
            s.phase = Active;
            s.activated_by_authority = true;
        }
        EntitlementEvent::BillingReceiptRecorded => {
            s.billing_receipt = true;
            s.history_present = true;
        }
        EntitlementEvent::SupportReceiptRecorded => {
            s.support_receipt = true;
            s.history_present = true;
        }
        EntitlementEvent::RunMarkedEligible => s.run_eligible = true,
        EntitlementEvent::GovernedRunStarted => {
            s.ever_run = true;
            s.history_present = true;
        }
        EntitlementEvent::EntitlementSuspended => {
            s.phase = Suspended;
            s.run_eligible = false;
        }
        EntitlementEvent::EntitlementResumed => {
            s.phase = Active;
            s.activated_by_authority = true;
            s.run_eligible = false;
        }
        // HISTORY_PRESERVED_ON_CLOSE: close blocks future runs, never erases history.
        EntitlementEvent::EntitlementClosed => {
            s.phase = Closed;
            s.run_eligible = false;
        }
        EntitlementEvent::EntitlementDenied => {
            s.phase = Denied;
            s.activated_by_authority = false;
        }
        // Withdrawing the install voids the entitlement: `phase` moves to the terminal
        // `Withdrawn` so it can never read `Active` while inactive (the desync the old
        // `entitlement_active` bool allowed).
        EntitlementEvent::PackageInstallWithdrawn => {
            s.phase = Withdrawn;
            s.package_eligible = false;
            s.run_eligible = false;
        }
    }
    s
}

impl crate::Lifecycle for EntitlementState {
    type State = EntitlementState;
    type Command = EntitlementCommand;
    type Event = EntitlementEvent;
    const KIND: &'static str = "deployment_entitlement";
    fn decide(
        state: &EntitlementState,
        command: EntitlementCommand,
    ) -> Result<Vec<EntitlementEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &EntitlementState, event: EntitlementEvent) -> EntitlementState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn apply(state: &EntitlementState, command: EntitlementCommand) -> EntitlementState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(*state, |s, e| evolve(&s, e)),
            Err(_) => *state,
        }
    }
    fn active() -> EntitlementState {
        let s = EntitlementState::default();
        let s = apply(&s, EntitlementCommand::AdmitPackageInstall);
        let s = apply(&s, EntitlementCommand::RequestEntitlement);
        apply(&s, EntitlementCommand::ActivateEntitlement)
    }

    #[test]
    fn activate_then_run_then_suspend_blocks_resume_allows() {
        let s = apply(&active(), EntitlementCommand::MarkRunEligible);
        let s = apply(&s, EntitlementCommand::StartGovernedRun);
        assert!(s.ever_run);
        // suspend blocks future runs (RUN_WHILE_SUSPENDED teeth)…
        let s = apply(&s, EntitlementCommand::SuspendEntitlement);
        assert!(!s.run_eligible);
        assert!(
            decide(&s, EntitlementCommand::MarkRunEligible).is_err(),
            "no eligibility while suspended"
        );
        // …resume restores it.
        let s = apply(&s, EntitlementCommand::ResumeEntitlement);
        let s = apply(&s, EntitlementCommand::MarkRunEligible);
        assert!(s.run_eligible);
    }

    #[test]
    fn receipts_do_not_activate_entitlement() {
        // BILLING/SUPPORT_GRANTS_ENTITLEMENT teeth: a receipt is evidence, not authority.
        let s = EntitlementState::default();
        let s = apply(&s, EntitlementCommand::AdmitPackageInstall);
        let s = apply(&s, EntitlementCommand::RequestEntitlement);
        let s = apply(&s, EntitlementCommand::RecordBillingReceipt);
        let s = apply(&s, EntitlementCommand::RecordSupportReceipt);
        assert!(s.billing_receipt && s.support_receipt);
        assert!(
            !s.active_entitlement(),
            "receipts must not activate the entitlement"
        );
        // RUN_WITHOUT_ENTITLEMENT teeth: no run eligibility without activation.
        assert!(decide(&s, EntitlementCommand::MarkRunEligible).is_err());
    }

    #[test]
    fn close_preserves_history() {
        let s = apply(&active(), EntitlementCommand::MarkRunEligible);
        let s = apply(&s, EntitlementCommand::StartGovernedRun);
        let s = apply(&s, EntitlementCommand::RecordBillingReceipt);
        let s = apply(&s, EntitlementCommand::CloseEntitlement);
        // CLOSE_ERASES_HISTORY teeth: closeout preserves run + receipt evidence.
        assert_eq!(s.phase, EntitlementPhase::Closed);
        assert!(s.ever_run && s.billing_receipt && s.history_present);
    }

    fn arb_command() -> impl Strategy<Value = EntitlementCommand> {
        use EntitlementCommand::*;
        prop_oneof![
            Just(AdmitPackageInstall),
            Just(RequestEntitlement),
            Just(ActivateEntitlement),
            Just(RecordBillingReceipt),
            Just(RecordSupportReceipt),
            Just(MarkRunEligible),
            Just(StartGovernedRun),
            Just(SuspendEntitlement),
            Just(ResumeEntitlement),
            Just(CloseEntitlement),
            Just(DenyOrExpire),
            Just(WithdrawPackageInstall),
        ]
    }

    proptest! {
        #[test]
        fn deployment_entitlement_invariants(commands in prop::collection::vec(arb_command(), 0..60)) {
            let mut s = EntitlementState::default();
            for c in commands {
                // RUN_ELIGIBILITY_REQUIRES_ACTIVE_ENTITLEMENT (+ SUSPENSION_BLOCKS_FUTURE_RUNS):
                // marking eligible succeeds only under an active entitlement.
                if c == EntitlementCommand::MarkRunEligible && decide(&s, c).is_ok() {
                    prop_assert!(s.active_entitlement(), "run-eligible without an active entitlement");
                }
                s = apply(&s, c);
                // ENTITLEMENT_REQUIRES_AUTHORITY.
                if s.entitlement_active() {
                    prop_assert!(s.activated_by_authority, "active without authority basis");
                }
                // PHASE_IS_AUTHORITATIVE: `phase == Active` now *implies* the entitlement
                // is genuinely active — the desync the old `entitlement_active` bool
                // allowed (withdraw clearing it while phase stayed Active) is gone.
                if matches!(s.phase, EntitlementPhase::Active) {
                    prop_assert!(s.active_entitlement(), "phase Active but entitlement not active");
                }
                // SUSPENSION_BLOCKS_FUTURE_RUNS: blocked ⇒ not run-eligible.
                if s.suspended() || s.closed() {
                    prop_assert!(!s.run_eligible, "run-eligible while blocked");
                }
                // HISTORY_PRESERVED_ON_CLOSE.
                if s.closed() && s.ever_run {
                    prop_assert!(s.history_present, "close erased run history");
                }
            }
        }
    }
}

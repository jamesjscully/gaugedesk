//! Package distribution lifecycle — the `(decide, evolve)` reducer, ported from
//! `specs/models/package-distribution.qnt` (ADR 0016). M2.
//!
//! One package version × one target install track: source publication (in the
//! source scope) and target install (in the target scope), correlated by
//! federation. Discharges:
//! - `INSTALL_REQUIRES_PUBLISHED_TARGET_ADMISSION` — install needs source
//!   publication **and** target admission (`INV-13`).
//! - `INSTALL_DOES_NOT_GRANT_PAYLOAD_OR_RUN` — install is target admission of a
//!   manifest; it grants **no** payload access and **no** run authority
//!   (`INV-10/11/12`). Structurally: this reducer has no grant event — payload
//!   access is [[resource-access]], run authority is [[run]].
//! - `WITHDRAWAL_BLOCKS_FUTURE` — after withdrawal, no new install and no new run.
//! - `HISTORY_PRESERVED_ON_WITHDRAWAL` — withdrawal is future-only; it never erases
//!   prior install evidence (`INV-18`).
//! - `UPGRADE_REQUIRES_REPLACEMENT_ADMISSION` — supersession needs the replacement
//!   published **and** target-admitted.

use crate::Rejection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DistPhase {
    Draft,
    Published,
    InstallRequested,
    Installed,
    Disabled,
    Withdrawn,
    Removed,
    Superseded,
    Denied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DistState {
    pub phase: DistPhase,
    pub published: bool,
    pub source_basis: bool,
    pub target_admitted: bool,
    pub installed: bool,
    pub ever_installed: bool,
    pub withdrawn: bool,
    /// A governed run was noted as eligible against this install (the model's
    /// `futureRunStarted`) — used to prove withdrawal blocks future use.
    pub run_noted: bool,
    pub replacement_published: bool,
    pub replacement_target_admitted: bool,
    pub superseded: bool,
}

impl Default for DistState {
    fn default() -> Self {
        Self {
            phase: DistPhase::Draft,
            published: false,
            source_basis: false,
            target_admitted: false,
            installed: false,
            ever_installed: false,
            withdrawn: false,
            run_noted: false,
            replacement_published: false,
            replacement_target_admitted: false,
            superseded: false,
        }
    }
}

impl DistState {
    /// Install is admissible only with source publication + target admission and no
    /// withdrawal (`INSTALL_REQUIRES_PUBLISHED_TARGET_ADMISSION`).
    fn install_ready(&self) -> bool {
        self.published && self.source_basis && self.target_admitted && !self.withdrawn
    }
    fn replacement_ready(&self) -> bool {
        self.replacement_published && self.replacement_target_admitted
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistCommand {
    PublishPackage,
    RequestInstall,
    TargetAdmitInstall,
    InstallPackage,
    WithdrawPackage,
    /// Note a governed run is eligible against the install (downstream run truth
    /// lives in [[run]]; here only to prove withdrawal blocks future use).
    NoteRunEligible,
    DisableInstall,
    RemoveInstall,
    PublishReplacement,
    TargetAdmitReplacement,
    AdmitUpgrade,
    RejectOrExpire,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DistEvent {
    PackagePublished,
    InstallRequested,
    InstallTargetAdmitted,
    PackageInstalled,
    PackageWithdrawn,
    RunNoted,
    InstallDisabled,
    InstallRemoved,
    ReplacementPublished,
    ReplacementTargetAdmitted,
    PackageSuperseded,
    InstallDenied,
}

fn reject(reason: &'static str) -> Result<Vec<DistEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(state: &DistState, command: DistCommand) -> Result<Vec<DistEvent>, Rejection> {
    use DistPhase::*;
    match command {
        DistCommand::PublishPackage => match state.phase {
            Draft => Ok(vec![DistEvent::PackagePublished]),
            _ => reject("publishPackage: not a draft"),
        },
        DistCommand::RequestInstall => match state.phase {
            Published => Ok(vec![DistEvent::InstallRequested]),
            _ => reject("requestInstall: requires a published package"),
        },
        DistCommand::TargetAdmitInstall => match state.phase {
            InstallRequested => Ok(vec![DistEvent::InstallTargetAdmitted]),
            _ => reject("targetAdmitInstall: no pending install request"),
        },
        // INSTALL_REQUIRES_PUBLISHED_TARGET_ADMISSION.
        DistCommand::InstallPackage => {
            if state.phase == InstallRequested && state.install_ready() {
                Ok(vec![DistEvent::PackageInstalled])
            } else {
                reject("installPackage: requires published + target admission (INV-13)")
            }
        }
        DistCommand::WithdrawPackage => match state.phase {
            Published | InstallRequested | Installed | Disabled | Superseded => {
                Ok(vec![DistEvent::PackageWithdrawn])
            }
            _ => reject("withdrawPackage: nothing withdrawable"),
        },
        // WITHDRAWAL_BLOCKS_FUTURE: a run is eligible only on a live, non-withdrawn install.
        DistCommand::NoteRunEligible => {
            if state.installed && !state.withdrawn {
                Ok(vec![DistEvent::RunNoted])
            } else {
                reject("noteRunEligible: needs a live, non-withdrawn install")
            }
        }
        DistCommand::DisableInstall => match state.phase {
            Installed => Ok(vec![DistEvent::InstallDisabled]),
            _ => reject("disableInstall: not installed"),
        },
        DistCommand::RemoveInstall => match state.phase {
            Installed | Disabled => Ok(vec![DistEvent::InstallRemoved]),
            _ => reject("removeInstall: nothing installed"),
        },
        DistCommand::PublishReplacement => match state.phase {
            Installed | Disabled => Ok(vec![DistEvent::ReplacementPublished]),
            _ => reject("publishReplacement: needs an install to upgrade"),
        },
        DistCommand::TargetAdmitReplacement => {
            if state.replacement_published {
                Ok(vec![DistEvent::ReplacementTargetAdmitted])
            } else {
                reject("targetAdmitReplacement: no replacement published")
            }
        }
        // UPGRADE_REQUIRES_REPLACEMENT_ADMISSION.
        DistCommand::AdmitUpgrade => {
            if state.replacement_ready() {
                Ok(vec![DistEvent::PackageSuperseded])
            } else {
                reject("admitUpgrade: replacement must be published + target-admitted")
            }
        }
        DistCommand::RejectOrExpire => match state.phase {
            Published | InstallRequested => Ok(vec![DistEvent::InstallDenied]),
            _ => reject("rejectOrExpire: nothing to deny"),
        },
    }
}

pub fn evolve(state: &DistState, event: DistEvent) -> DistState {
    use DistPhase::*;
    let mut s = *state;
    match event {
        DistEvent::PackagePublished => {
            s.phase = Published;
            s.published = true;
            s.source_basis = true;
        }
        DistEvent::InstallRequested => s.phase = InstallRequested,
        DistEvent::InstallTargetAdmitted => s.target_admitted = true,
        DistEvent::PackageInstalled => {
            s.phase = Installed;
            s.installed = true;
            s.ever_installed = true;
        }
        // HISTORY_PRESERVED_ON_WITHDRAWAL: withdrawal never unsets `installed`.
        DistEvent::PackageWithdrawn => {
            s.phase = Withdrawn;
            s.withdrawn = true;
        }
        DistEvent::RunNoted => s.run_noted = true,
        DistEvent::InstallDisabled => s.phase = Disabled,
        DistEvent::InstallRemoved => {
            s.phase = Removed;
            s.installed = false;
        }
        DistEvent::ReplacementPublished => s.replacement_published = true,
        DistEvent::ReplacementTargetAdmitted => s.replacement_target_admitted = true,
        DistEvent::PackageSuperseded => {
            s.phase = Superseded;
            s.installed = false;
            s.superseded = true;
        }
        DistEvent::InstallDenied => s.phase = Denied,
    }
    s
}

impl crate::Lifecycle for DistState {
    type State = DistState;
    type Command = DistCommand;
    type Event = DistEvent;
    const KIND: &'static str = "package_distribution";
    fn decide(state: &DistState, command: DistCommand) -> Result<Vec<DistEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &DistState, event: DistEvent) -> DistState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn apply(state: &DistState, command: DistCommand) -> DistState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(*state, |s, e| evolve(&s, e)),
            Err(_) => *state,
        }
    }
    fn installed() -> DistState {
        let s = DistState::default();
        let s = apply(&s, DistCommand::PublishPackage);
        let s = apply(&s, DistCommand::RequestInstall);
        let s = apply(&s, DistCommand::TargetAdmitInstall);
        apply(&s, DistCommand::InstallPackage)
    }

    #[test]
    fn publish_install_withdraw_preserves_history() {
        let s = installed();
        assert_eq!(s.phase, DistPhase::Installed);
        let s = apply(&s, DistCommand::WithdrawPackage);
        assert_eq!(s.phase, DistPhase::Withdrawn);
        // WITHDRAW_ERASES_HISTORY teeth: install evidence survives withdrawal.
        assert!(
            s.installed && s.ever_installed,
            "withdrawal preserved the install record"
        );
    }

    #[test]
    fn upgrade_requires_replacement_admission() {
        let s = installed();
        let s = apply(&s, DistCommand::PublishReplacement);
        // UPGRADE_WITHOUT_ADMISSION teeth: upgrade refused until the replacement is admitted.
        assert!(
            decide(&s, DistCommand::AdmitUpgrade).is_err(),
            "no upgrade without admission"
        );
        let s = apply(&s, DistCommand::TargetAdmitReplacement);
        let s = apply(&s, DistCommand::AdmitUpgrade);
        assert_eq!(s.phase, DistPhase::Superseded);
    }

    #[test]
    fn install_unpublished_and_after_withdrawal_are_rejected() {
        // INSTALL_UNPUBLISHED teeth: can't install a draft.
        let s = DistState::default();
        assert!(decide(&s, DistCommand::InstallPackage).is_err());
        // INSTALL_AFTER_WITHDRAWAL teeth: no install/run after withdrawal.
        let s = apply(&installed(), DistCommand::WithdrawPackage);
        assert!(
            decide(&s, DistCommand::NoteRunEligible).is_err(),
            "no run after withdrawal"
        );
    }

    fn arb_command() -> impl Strategy<Value = DistCommand> {
        use DistCommand::*;
        prop_oneof![
            Just(PublishPackage),
            Just(RequestInstall),
            Just(TargetAdmitInstall),
            Just(InstallPackage),
            Just(WithdrawPackage),
            Just(NoteRunEligible),
            Just(DisableInstall),
            Just(RemoveInstall),
            Just(PublishReplacement),
            Just(TargetAdmitReplacement),
            Just(AdmitUpgrade),
            Just(RejectOrExpire),
        ]
    }

    proptest! {
        /// Every invariant holds over every reachable trace.
        #[test]
        fn package_distribution_invariants(commands in prop::collection::vec(arb_command(), 0..60)) {
            let mut s = DistState::default();
            for c in commands {
                // INSTALL_REQUIRES_PUBLISHED_TARGET_ADMISSION: a successful install was ready.
                if c == DistCommand::InstallPackage && decide(&s, c).is_ok() {
                    prop_assert!(s.install_ready(), "installed without published+target admission");
                }
                // UPGRADE_REQUIRES_REPLACEMENT_ADMISSION.
                if c == DistCommand::AdmitUpgrade && decide(&s, c).is_ok() {
                    prop_assert!(s.replacement_ready(), "upgraded without replacement admission");
                }
                // WITHDRAWAL_BLOCKS_FUTURE: once withdrawn, no new install or run.
                if s.withdrawn {
                    prop_assert!(decide(&s, DistCommand::InstallPackage).is_err());
                    prop_assert!(decide(&s, DistCommand::NoteRunEligible).is_err());
                }
                s = apply(&s, c);
                // HISTORY_PRESERVED_ON_WITHDRAWAL: ever-installed stays evidenced.
                if s.ever_installed {
                    prop_assert!(
                        s.installed || s.phase == DistPhase::Removed || s.superseded,
                        "install history was erased"
                    );
                }
            }
        }
    }
}

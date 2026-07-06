//! Agent version lifecycle — the `(decide, evolve)` reducer, ported from
//! `specs/models/agent-version.qnt` (ADR 0019). M2.
//!
//! Freezes a mutable agent **draft** into an immutable, packageable **snapshot**.
//! Discharges:
//! - `PACKAGE_REQUIRES_FROZEN_VERSION` — a package may reference only a frozen
//!   snapshot, never a mutable draft.
//! - `FROZEN_VERSION_IMMUTABLE` — the method/config/posture snapshot cannot change
//!   after freeze (the frozen revision is fixed; no revise once frozen).
//! - `RETIREMENT_BLOCKS_FUTURE_PACKAGING` — a retired version cannot be newly
//!   packaged.
//! - `PAST_PACKAGES_PRESERVED` — retirement is future-only; it never erases a
//!   version's existing package references (`INV-18`).

use crate::Rejection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VersionPhase {
    Init,
    Draft,
    /// Immutable snapshot — packageable.
    Frozen,
    /// At least one package references the frozen snapshot.
    Packaged,
    /// Future packaging blocked; historical package refs preserved.
    Retired,
    /// Terminal: a draft canceled before freezing.
    Abandoned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionState {
    pub phase: VersionPhase,
    /// Bumped by each draft revision; frozen at freeze time.
    pub draft_revision: u32,
    /// The snapshot revision, fixed at freeze (`FROZEN_VERSION_IMMUTABLE`).
    pub frozen_revision: u32,
    /// Has at least one package reference — **preserved** through retirement.
    pub packaged: bool,
}

impl Default for VersionState {
    fn default() -> Self {
        Self {
            phase: VersionPhase::Init,
            draft_revision: 0,
            frozen_revision: 0,
            packaged: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionCommand {
    CreateDraft,
    ReviseDraft,
    FreezeVersion,
    RecordPackageReference,
    RetireVersion,
    AbandonDraft,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VersionEvent {
    DraftCreated,
    DraftRevised,
    VersionFrozen,
    PackageReferenced,
    VersionRetired,
    DraftAbandoned,
}

fn reject(reason: &'static str) -> Result<Vec<VersionEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(
    state: &VersionState,
    command: VersionCommand,
) -> Result<Vec<VersionEvent>, Rejection> {
    use VersionPhase::*;
    match command {
        VersionCommand::CreateDraft => match state.phase {
            Init => Ok(vec![VersionEvent::DraftCreated]),
            _ => reject("createDraft: a version already exists"),
        },
        // FROZEN_VERSION_IMMUTABLE: only a draft is editable.
        VersionCommand::ReviseDraft => match state.phase {
            Draft => Ok(vec![VersionEvent::DraftRevised]),
            _ => reject("reviseDraft: not a draft (FROZEN_VERSION_IMMUTABLE)"),
        },
        VersionCommand::FreezeVersion => match state.phase {
            Draft => Ok(vec![VersionEvent::VersionFrozen]),
            _ => reject("freezeVersion: only a draft can be frozen"),
        },
        // PACKAGE_REQUIRES_FROZEN_VERSION + RETIREMENT_BLOCKS_FUTURE_PACKAGING:
        // a package may reference only a frozen, non-retired version.
        VersionCommand::RecordPackageReference => match state.phase {
            Frozen | Packaged => Ok(vec![VersionEvent::PackageReferenced]),
            _ => reject("recordPackageReference: requires a frozen, non-retired version"),
        },
        VersionCommand::RetireVersion => match state.phase {
            Frozen | Packaged => Ok(vec![VersionEvent::VersionRetired]),
            _ => reject("retireVersion: only a frozen/packaged version retires"),
        },
        VersionCommand::AbandonDraft => match state.phase {
            Draft => Ok(vec![VersionEvent::DraftAbandoned]),
            _ => reject("abandonDraft: not a draft"),
        },
    }
}

pub fn evolve(state: &VersionState, event: VersionEvent) -> VersionState {
    use VersionPhase::*;
    let mut s = *state;
    match event {
        VersionEvent::DraftCreated => {
            s.phase = Draft;
            s.draft_revision = 1;
        }
        VersionEvent::DraftRevised => s.draft_revision += 1,
        VersionEvent::VersionFrozen => {
            s.phase = Frozen;
            s.frozen_revision = s.draft_revision; // the snapshot revision is now fixed
        }
        VersionEvent::PackageReferenced => {
            s.phase = Packaged;
            s.packaged = true;
        }
        // PAST_PACKAGES_PRESERVED: retirement never unsets `packaged`.
        VersionEvent::VersionRetired => s.phase = Retired,
        VersionEvent::DraftAbandoned => s.phase = Abandoned,
    }
    s
}

impl crate::Lifecycle for VersionState {
    type State = VersionState;
    type Command = VersionCommand;
    type Event = VersionEvent;
    const KIND: &'static str = "agent_version";
    fn decide(
        state: &VersionState,
        command: VersionCommand,
    ) -> Result<Vec<VersionEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &VersionState, event: VersionEvent) -> VersionState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn apply(state: &VersionState, command: VersionCommand) -> VersionState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(*state, |s, e| evolve(&s, e)),
            Err(_) => *state,
        }
    }

    #[test]
    fn freeze_then_package_then_retire_preserves_the_package() {
        let s = VersionState::default();
        let s = apply(&s, VersionCommand::CreateDraft);
        let s = apply(&s, VersionCommand::ReviseDraft);
        let s = apply(&s, VersionCommand::FreezeVersion);
        assert_eq!(s.phase, VersionPhase::Frozen);
        assert_eq!(
            s.frozen_revision, s.draft_revision,
            "snapshot revision fixed at freeze"
        );
        let s = apply(&s, VersionCommand::RecordPackageReference);
        assert_eq!(s.phase, VersionPhase::Packaged);
        // retire is future-only — the package reference survives (PAST_PACKAGES_PRESERVED).
        let s = apply(&s, VersionCommand::RetireVersion);
        assert_eq!(s.phase, VersionPhase::Retired);
        assert!(s.packaged, "retirement preserved the existing package");
    }

    // --- teeth: the reducer refuses each move the Quint teeth flags would allow ---

    #[test]
    fn package_draft_is_rejected() {
        // PACKAGE_DRAFT: packaging a non-frozen draft would violate
        // PACKAGE_REQUIRES_FROZEN_VERSION — the reducer refuses it.
        let s = VersionState::default();
        let s = apply(&s, VersionCommand::CreateDraft);
        assert!(decide(&s, VersionCommand::RecordPackageReference).is_err());
    }

    #[test]
    fn mutate_frozen_is_rejected() {
        // MUTATE_FROZEN: revising after freeze would violate FROZEN_VERSION_IMMUTABLE.
        let s = VersionState::default();
        let s = apply(&s, VersionCommand::CreateDraft);
        let s = apply(&s, VersionCommand::FreezeVersion);
        assert!(
            decide(&s, VersionCommand::ReviseDraft).is_err(),
            "no revise after freeze"
        );
    }

    #[test]
    fn package_after_retire_is_rejected() {
        // PACKAGE_AFTER_RETIRE: packaging a retired version is blocked.
        let s = VersionState::default();
        let s = apply(&s, VersionCommand::CreateDraft);
        let s = apply(&s, VersionCommand::FreezeVersion);
        let s = apply(&s, VersionCommand::RetireVersion);
        assert!(decide(&s, VersionCommand::RecordPackageReference).is_err());
    }

    fn arb_command() -> impl Strategy<Value = VersionCommand> {
        prop_oneof![
            Just(VersionCommand::CreateDraft),
            Just(VersionCommand::ReviseDraft),
            Just(VersionCommand::FreezeVersion),
            Just(VersionCommand::RecordPackageReference),
            Just(VersionCommand::RetireVersion),
            Just(VersionCommand::AbandonDraft),
        ]
    }

    proptest! {
        /// All four invariants hold over every reachable trace.
        #[test]
        fn agent_version_invariants(commands in prop::collection::vec(arb_command(), 0..40)) {
            let mut s = VersionState::default();
            let mut ever_packaged = false;
            for c in commands {
                // RETIREMENT_BLOCKS_FUTURE_PACKAGING: a retired version refuses new packaging.
                if s.phase == VersionPhase::Retired {
                    prop_assert!(decide(&s, VersionCommand::RecordPackageReference).is_err());
                }
                s = apply(&s, c);
                if s.packaged { ever_packaged = true; }
                // PACKAGE_REQUIRES_FROZEN_VERSION: anything packaged went through freeze.
                if s.packaged {
                    prop_assert!(s.frozen_revision >= 1, "packaged a non-frozen version");
                }
                // FROZEN_VERSION_IMMUTABLE: past freeze, the snapshot revision is fixed.
                if matches!(s.phase, VersionPhase::Frozen | VersionPhase::Packaged | VersionPhase::Retired) {
                    prop_assert_eq!(s.draft_revision, s.frozen_revision, "frozen snapshot mutated");
                }
                // PAST_PACKAGES_PRESERVED: ever-packaged ⇒ still packaged.
                if ever_packaged {
                    prop_assert!(s.packaged, "a package reference was erased");
                }
            }
        }
    }
}

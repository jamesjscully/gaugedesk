//! Instance / deployment lifecycle (M1, ADR 0027). Ported from
//! `specs/models/instance-lifecycle.qnt`.
//!
//! An [[instance]] is a `(bound agent, workspace)` binding owning settled `main`.
//! M0 treats it as a bare record; M1 promotes its lifecycle so a deployment can be
//! version-pinned, suspended, torn down, and entered for support — with evidence
//! preserved across teardown.
//!
//! The reducer is pure: the shell admits `PinVersion` / `Suspend` / `Resume` /
//! `TearDown` / `EnterSupport`. Discharges: PIN_IMMUTABLE · RUNNABLE_REQUIRES_ACTIVE
//! · SUSPEND_BLOCKS_RUN · TORNDOWN_TERMINAL · SUPPORT_ENTRY_IMMUTABLE ·
//! EVIDENCE_PRESERVED.

use crate::Rejection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstancePhase {
    #[default]
    Created,
    Active,
    Suspended,
    TornDown,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, serde::Serialize)]
pub struct InstanceState {
    pub phase: InstancePhase,
    /// The pinned agent-version, set once and never changed (`None` until pinned).
    pub pinned_version: Option<String>,
    /// The authority that entered support mode, recorded once (`None` until entered).
    pub support_entry: Option<String>,
    /// Derived: an instance accepts new runs only while `Active`.
    pub runnable: bool,
    /// Placement-scoped **local config** — a `.agent-config.json` overlay merged over the
    /// archetype's config for new chats here (config-only customization, `placement.md`).
    /// `None` until set; **editable** (re-set overwrites). Never edits the shared method.
    pub local_config: Option<String>,
    /// Placement-scoped **notes** — project context fed to the method alongside the
    /// archetype's instructions. `None` until set; editable. Config-only, like above.
    pub notes: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InstanceCommand {
    /// Pin the agent-version; the only path out of `Created`. At most once.
    PinVersion(String),
    /// Active → Suspended: block new runs, preserve evidence.
    Suspend,
    /// Suspended → Active.
    Resume,
    /// Active|Suspended → TornDown (terminal); evidence preserved.
    TearDown,
    /// Record the support-mode entry point (the entering authority). At most once.
    EnterSupport(String),
    /// Set this placement's local config overlay + notes (config-only customization,
    /// `placement.md`). Repeatable while the instance is live — re-set overwrites.
    SetLocalConfig { config: String, notes: String },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InstanceEvent {
    VersionPinned(String),
    Suspended,
    Resumed,
    TornDown,
    SupportEntered(String),
    LocalConfigSet { config: String, notes: String },
}

fn reject(reason: &'static str) -> Result<Vec<InstanceEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(
    state: &InstanceState,
    command: InstanceCommand,
) -> Result<Vec<InstanceEvent>, Rejection> {
    use InstancePhase::*;
    match command {
        // PIN_IMMUTABLE: only from Created, and only once.
        InstanceCommand::PinVersion(v) => {
            if state.phase == Created && state.pinned_version.is_none() {
                Ok(vec![InstanceEvent::VersionPinned(v)])
            } else {
                reject("pinVersion: already pinned (a version is pinned at most once)")
            }
        }
        InstanceCommand::Suspend => match state.phase {
            Active => Ok(vec![InstanceEvent::Suspended]),
            _ => reject("suspend: instance is not active"),
        },
        InstanceCommand::Resume => match state.phase {
            Suspended => Ok(vec![InstanceEvent::Resumed]),
            _ => reject("resume: instance is not suspended"),
        },
        // TORNDOWN_TERMINAL: cannot tear down a torn-down (or un-pinned) instance.
        InstanceCommand::TearDown => match state.phase {
            Active | Suspended => Ok(vec![InstanceEvent::TornDown]),
            _ => reject("tearDown: instance is not active or suspended"),
        },
        // SUPPORT_ENTRY_IMMUTABLE: recorded once, while not torn down.
        InstanceCommand::EnterSupport(by) => {
            let live = matches!(state.phase, Active | Suspended);
            if live && state.support_entry.is_none() {
                Ok(vec![InstanceEvent::SupportEntered(by)])
            } else {
                reject("enterSupport: already entered, or instance not live")
            }
        }
        // Config-only customization: settable/editable any time the instance is not torn
        // down. Unlike a pin, it is mutable — re-set overwrites the overlay + notes.
        InstanceCommand::SetLocalConfig { config, notes } => {
            if state.phase == TornDown {
                reject("setLocalConfig: instance is torn down")
            } else {
                Ok(vec![InstanceEvent::LocalConfigSet { config, notes }])
            }
        }
    }
}

pub fn evolve(state: &InstanceState, event: InstanceEvent) -> InstanceState {
    let mut s = state.clone();
    match event {
        InstanceEvent::VersionPinned(v) => {
            s.phase = InstancePhase::Active;
            s.pinned_version = Some(v);
            s.runnable = true;
        }
        InstanceEvent::Suspended => {
            s.phase = InstancePhase::Suspended;
            s.runnable = false;
        }
        InstanceEvent::Resumed => {
            s.phase = InstancePhase::Active;
            s.runnable = true;
        }
        // EVIDENCE_PRESERVED: keep pinned_version + support_entry across teardown.
        InstanceEvent::TornDown => {
            s.phase = InstancePhase::TornDown;
            s.runnable = false;
        }
        InstanceEvent::SupportEntered(by) => {
            s.support_entry = Some(by);
        }
        InstanceEvent::LocalConfigSet { config, notes } => {
            s.local_config = Some(config);
            s.notes = Some(notes);
        }
    }
    s
}

impl crate::Lifecycle for InstanceState {
    type State = InstanceState;
    type Command = InstanceCommand;
    type Event = InstanceEvent;
    const KIND: &'static str = "instance";
    fn decide(
        state: &InstanceState,
        command: InstanceCommand,
    ) -> Result<Vec<InstanceEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &InstanceState, event: InstanceEvent) -> InstanceState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn apply(state: &InstanceState, command: InstanceCommand) -> InstanceState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }

    #[test]
    fn pin_then_suspend_resume_teardown() {
        use InstanceCommand::*;
        let s = InstanceState::default();
        assert_eq!(s.phase, InstancePhase::Created);
        let s = apply(&s, PinVersion("v1".into()));
        assert_eq!(s.phase, InstancePhase::Active);
        assert_eq!(s.pinned_version.as_deref(), Some("v1"));
        assert!(s.runnable);
        let s = apply(&s, Suspend);
        assert_eq!(s.phase, InstancePhase::Suspended);
        assert!(!s.runnable);
        let s = apply(&s, Resume);
        assert!(s.runnable);
        let s = apply(&s, EnterSupport("expert-A".into()));
        assert_eq!(s.support_entry.as_deref(), Some("expert-A"));
        let s = apply(&s, TearDown);
        assert_eq!(s.phase, InstancePhase::TornDown);
        // EVIDENCE_PRESERVED: version + support entry survive teardown.
        assert_eq!(s.pinned_version.as_deref(), Some("v1"));
        assert_eq!(s.support_entry.as_deref(), Some("expert-A"));
    }

    #[test]
    fn cannot_repin_or_act_after_teardown() {
        use InstanceCommand::*;
        let s = InstanceState::default();
        let s = apply(&s, PinVersion("v1".into()));
        // re-pin is rejected (immutable)
        let s = apply(&s, PinVersion("v2".into()));
        assert_eq!(s.pinned_version.as_deref(), Some("v1"));
        let s = apply(&s, TearDown);
        // no transitions leave torn_down
        let s = apply(&s, Resume);
        let s = apply(&s, Suspend);
        let s = apply(&s, EnterSupport("late".into()));
        assert_eq!(s.phase, InstancePhase::TornDown);
        assert!(s.support_entry.is_none());
    }

    #[test]
    fn local_config_is_editable_then_frozen_by_teardown() {
        use InstanceCommand::*;
        let s = InstanceState::default();
        let s = apply(&s, PinVersion("v1".into()));
        // settable while live, and re-set overwrites (unlike a pin)
        let s = apply(
            &s,
            SetLocalConfig {
                config: "{\"a\":1}".into(),
                notes: "first".into(),
            },
        );
        assert_eq!(s.local_config.as_deref(), Some("{\"a\":1}"));
        assert_eq!(s.notes.as_deref(), Some("first"));
        let s = apply(
            &s,
            SetLocalConfig {
                config: "{\"a\":2}".into(),
                notes: "second".into(),
            },
        );
        assert_eq!(s.local_config.as_deref(), Some("{\"a\":2}"));
        assert_eq!(s.notes.as_deref(), Some("second"));
        // torn down → no more config changes, but the last value is preserved
        let s = apply(&s, TearDown);
        let s = apply(
            &s,
            SetLocalConfig {
                config: "{\"a\":3}".into(),
                notes: "third".into(),
            },
        );
        assert_eq!(s.local_config.as_deref(), Some("{\"a\":2}"));
    }

    fn arb_command() -> impl Strategy<Value = InstanceCommand> {
        use InstanceCommand::*;
        prop_oneof![
            prop_oneof![Just("v1".to_string()), Just("v2".to_string())].prop_map(PinVersion),
            Just(Suspend),
            Just(Resume),
            Just(TearDown),
            prop_oneof![Just("e1".to_string()), Just("e2".to_string())].prop_map(EnterSupport),
            prop_oneof![
                Just("{}".to_string()),
                Just("{\"model\":\"x\"}".to_string())
            ]
            .prop_map(|config| SetLocalConfig {
                config,
                notes: "n".to_string()
            }),
        ]
    }

    proptest! {
        /// The instance-lifecycle invariants hold over every reachable trace.
        #[test]
        fn instance_invariants(commands in prop::collection::vec(arb_command(), 0..50)) {
            let mut s = InstanceState::default();
            let mut pinned_once: Option<String> = None;
            let mut support_once: Option<String> = None;
            for c in commands {
                s = apply(&s, c);
                // RUNNABLE_REQUIRES_ACTIVE
                if s.runnable {
                    prop_assert_eq!(s.phase, InstancePhase::Active);
                    // PIN_IMMUTABLE: runnable ⟹ a version is pinned
                    prop_assert!(s.pinned_version.is_some());
                }
                // SUSPEND_BLOCKS_RUN
                if s.phase == InstancePhase::Suspended {
                    prop_assert!(!s.runnable);
                }
                // TORNDOWN_TERMINAL
                if s.phase == InstancePhase::TornDown {
                    prop_assert!(!s.runnable);
                }
                // PIN_IMMUTABLE: once pinned, the value never changes
                if let Some(v) = &s.pinned_version {
                    match &pinned_once {
                        Some(seen) => prop_assert_eq!(seen, v),
                        None => pinned_once = Some(v.clone()),
                    }
                }
                // SUPPORT_ENTRY_IMMUTABLE: once recorded, never changes
                if let Some(e) = &s.support_entry {
                    match &support_once {
                        Some(seen) => prop_assert_eq!(seen, e),
                        None => support_once = Some(e.clone()),
                    }
                }
            }
        }
    }
}

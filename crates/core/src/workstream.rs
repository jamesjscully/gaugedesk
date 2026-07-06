//! Workstream — the `(decide, evolve)` reducer for a shared, explicitly-created
//! auto-sync line that member chats greedily sync into. It is the Rust mirror of
//! [`workstream.qnt`](../../../specs/models/workstream.qnt) and implements
//! [`lifecycles/workstream.md`](../../../specs/lifecycles/workstream.md) (`WS-B`).
//!
//! This reducer owns only the part the per-contribution [`merge`](crate::merge) does not
//! know about: the stream as a shared object, and its **membership** (which chats target
//! it). The gated ref-advance itself is delegated to `merge.rs` (a clean-git + admitted-
//! policy advance, auto-admitted in-stream by the shell); the boundary-gated promotion to
//! the placement mainline is `merge.rs`'s `advanced → integrated` hop; and the single-home
//! guarantee is inherited from [`handoff`](crate::handoff) (`EXACTLY_ONE_HOME`).
//!
//! What this reducer enforces, in every reachable state:
//! - `MEMBERSHIP_GATES_AUTOSYNC` — a contribution advances the stream main **only** for a
//!   member of an active stream; a non-member never advances it (fail-closed, `INV-20`).
//! - `ARCHIVE_REHOMES_MEMBERS` — archiving empties membership (the shell re-homes those
//!   chats to the placement mainline) so no chat is left homed to a dead ref (`INV-23`).
//! - `ARCHIVED_ACCEPTS_NO_ADVANCE` — a corollary of the two above: an archived stream's
//!   ref never advances again (no member exists to contribute).
//!
//! `Contribute` is the admitted spine for the greedy auto-sync advance: the shell asks
//! this reducer "may chat C advance stream S's main?" before doing the git push, so the
//! decision to advance a standing ref is an admitted event (`INV-2`/`INV-4`), and the
//! emitted `ContributionAdmitted { chat, by }` carries the driving authority — the
//! attribution that makes a federated workstream's history legible (`WS-G`).

use crate::Rejection;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WorkstreamPhase {
    /// No workstream in this scope yet (the fold default, before `CreateWorkstream`).
    Absent,
    /// The stream exists; member chats greedily auto-sync into its main.
    Active,
    /// The stream is closed; its ref never advances again and membership is empty
    /// (members re-homed to the placement mainline). Terminal.
    Archived,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkstreamState {
    pub phase: WorkstreamPhase,
    /// The chats homed to this stream's main. A chat targets exactly one shared ref;
    /// being absent here means it targets the placement mainline (the default).
    pub members: BTreeSet<String>,
    /// The stream's name (set at creation; empty before).
    pub name: String,
    /// The single home authority that owns this stream's ref + membership log
    /// (`EXACTLY_ONE_HOME`, inherited from handoff). Empty before creation.
    pub home: String,
}

impl Default for WorkstreamState {
    fn default() -> Self {
        Self {
            phase: WorkstreamPhase::Absent,
            members: BTreeSet::new(),
            name: String::new(),
            home: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkstreamCommand {
    /// Create the shared line (by a user or an agent). Admitted only from `Absent`.
    CreateWorkstream {
        name: String,
        home: String,
        creator: String,
    },
    /// Re-home a chat onto this stream's main. Admitted only while `Active`.
    JoinWorkstream { chat: String },
    /// Re-home a member chat back to the placement mainline. Admitted only while `Active`.
    LeaveWorkstream { chat: String },
    /// Close the stream; re-homes remaining members to mainline. Admitted only from `Active`.
    ArchiveWorkstream,
    /// A member chat's turn-finalize advances the stream main — the admitted spine for the
    /// greedy auto-sync push. Admitted only for a member of an `Active` stream. `by` is the
    /// authority that drove the turn (the contribution's attribution, `WS-G`): the local
    /// hub authority for a local turn, or the crossing authority for a remote-driven one.
    Contribute { chat: String, by: String },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WorkstreamEvent {
    WorkstreamCreated {
        name: String,
        home: String,
        creator: String,
    },
    ChatJoined {
        chat: String,
    },
    ChatLeft {
        chat: String,
    },
    /// Records which members were re-homed off the dead stream (the bounded escape).
    WorkstreamArchived {
        rehomed: Vec<String>,
    },
    /// A member's contribution was admitted to advance the stream main. Carries the
    /// contributing chat **and** the authority that drove it (`WS-G` attribution), so a
    /// federated workstream's history is legible: which actor advanced the shared line.
    ContributionAdmitted {
        chat: String,
        by: String,
    },
}

fn reject(reason: &'static str) -> Result<Vec<WorkstreamEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(
    state: &WorkstreamState,
    command: WorkstreamCommand,
) -> Result<Vec<WorkstreamEvent>, Rejection> {
    use WorkstreamPhase::*;
    match command {
        WorkstreamCommand::CreateWorkstream {
            name,
            home,
            creator,
        } => match state.phase {
            Absent => Ok(vec![WorkstreamEvent::WorkstreamCreated {
                name,
                home,
                creator,
            }]),
            _ => reject("createWorkstream: a workstream already exists in this scope"),
        },
        WorkstreamCommand::JoinWorkstream { chat } => match state.phase {
            Active => Ok(vec![WorkstreamEvent::ChatJoined { chat }]),
            _ => reject("joinWorkstream: the workstream is not active"),
        },
        WorkstreamCommand::LeaveWorkstream { chat } => {
            if state.phase == Active && state.members.contains(&chat) {
                Ok(vec![WorkstreamEvent::ChatLeft { chat }])
            } else {
                reject("leaveWorkstream: not an active stream or chat is not a member")
            }
        }
        WorkstreamCommand::ArchiveWorkstream => match state.phase {
            Active => Ok(vec![WorkstreamEvent::WorkstreamArchived {
                rehomed: state.members.iter().cloned().collect(),
            }]),
            _ => reject("archiveWorkstream: only an active workstream can be archived"),
        },
        // MEMBERSHIP_GATES_AUTOSYNC: the stream main advances only for a member of an
        // active stream. A non-member (or any chat once archived) is rejected.
        WorkstreamCommand::Contribute { chat, by } => {
            if state.phase == Active && state.members.contains(&chat) {
                Ok(vec![WorkstreamEvent::ContributionAdmitted { chat, by }])
            } else {
                reject("contribute: chat is not a member of an active workstream")
            }
        }
    }
}

pub fn evolve(state: &WorkstreamState, event: WorkstreamEvent) -> WorkstreamState {
    use WorkstreamPhase::*;
    let mut s = state.clone();
    match event {
        WorkstreamEvent::WorkstreamCreated {
            name,
            home,
            creator: _,
        } => {
            s.phase = Active;
            s.name = name;
            s.home = home;
        }
        WorkstreamEvent::ChatJoined { chat } => {
            s.members.insert(chat);
        }
        WorkstreamEvent::ChatLeft { chat } => {
            s.members.remove(&chat);
        }
        // ARCHIVE_REHOMES_MEMBERS: archiving empties membership — no chat left on the
        // dead ref (the shell re-homes them to the placement mainline).
        WorkstreamEvent::WorkstreamArchived { rehomed: _ } => {
            s.phase = Archived;
            s.members.clear();
        }
        // Attribution only — no structural state change.
        WorkstreamEvent::ContributionAdmitted { chat: _, by: _ } => {}
    }
    s
}

impl crate::Lifecycle for WorkstreamState {
    type State = WorkstreamState;
    type Command = WorkstreamCommand;
    type Event = WorkstreamEvent;
    const KIND: &'static str = "workstream";
    fn decide(
        state: &WorkstreamState,
        command: WorkstreamCommand,
    ) -> Result<Vec<WorkstreamEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &WorkstreamState, event: WorkstreamEvent) -> WorkstreamState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn apply(state: &WorkstreamState, command: WorkstreamCommand) -> WorkstreamState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }

    fn created() -> WorkstreamState {
        apply(
            &WorkstreamState::default(),
            WorkstreamCommand::CreateWorkstream {
                name: "feature-x".into(),
                home: "hub".into(),
                creator: "alice".into(),
            },
        )
    }

    #[test]
    fn create_then_join_then_member_contributes() {
        let s = created();
        assert_eq!(s.phase, WorkstreamPhase::Active);
        let s = apply(&s, WorkstreamCommand::JoinWorkstream { chat: "a".into() });
        assert!(s.members.contains("a"));
        // a member of an active stream may advance the stream main — and the admitted
        // event carries the driving authority (WS-G attribution).
        let ev = decide(
            &s,
            WorkstreamCommand::Contribute {
                chat: "a".into(),
                by: "alice".into(),
            },
        )
        .unwrap();
        assert_eq!(
            ev,
            vec![WorkstreamEvent::ContributionAdmitted {
                chat: "a".into(),
                by: "alice".into()
            }]
        );
        // a non-member may not.
        assert!(decide(
            &s,
            WorkstreamCommand::Contribute {
                chat: "b".into(),
                by: "alice".into()
            }
        )
        .is_err());
    }

    #[test]
    fn cannot_create_twice() {
        let s = created();
        assert!(decide(
            &s,
            WorkstreamCommand::CreateWorkstream {
                name: "again".into(),
                home: "hub".into(),
                creator: "alice".into(),
            }
        )
        .is_err());
    }

    #[test]
    fn archive_rehomes_members_and_is_terminal() {
        let s = created();
        let s = apply(&s, WorkstreamCommand::JoinWorkstream { chat: "a".into() });
        let s = apply(&s, WorkstreamCommand::JoinWorkstream { chat: "b".into() });
        let s = apply(&s, WorkstreamCommand::ArchiveWorkstream);
        assert_eq!(s.phase, WorkstreamPhase::Archived);
        assert!(s.members.is_empty(), "archive re-homes (empties) members");
        // archived is terminal and accepts no advance.
        assert!(decide(
            &s,
            WorkstreamCommand::Contribute {
                chat: "a".into(),
                by: "alice".into()
            }
        )
        .is_err());
        assert!(decide(&s, WorkstreamCommand::JoinWorkstream { chat: "c".into() }).is_err());
        assert!(decide(&s, WorkstreamCommand::ArchiveWorkstream).is_err());
    }

    #[test]
    fn leave_rehomes_to_mainline() {
        let s = created();
        let s = apply(&s, WorkstreamCommand::JoinWorkstream { chat: "a".into() });
        let s = apply(&s, WorkstreamCommand::LeaveWorkstream { chat: "a".into() });
        assert!(!s.members.contains("a"));
        // a left chat targets the mainline now — no longer advances this stream.
        assert!(decide(
            &s,
            WorkstreamCommand::Contribute {
                chat: "a".into(),
                by: "alice".into()
            }
        )
        .is_err());
    }

    fn arb_command() -> impl Strategy<Value = WorkstreamCommand> {
        let chat = prop_oneof![Just("a"), Just("b"), Just("c")].prop_map(String::from);
        prop_oneof![
            Just(WorkstreamCommand::CreateWorkstream {
                name: "w".into(),
                home: "hub".into(),
                creator: "alice".into(),
            }),
            chat.clone()
                .prop_map(|chat| WorkstreamCommand::JoinWorkstream { chat }),
            chat.clone()
                .prop_map(|chat| WorkstreamCommand::LeaveWorkstream { chat }),
            Just(WorkstreamCommand::ArchiveWorkstream),
            chat.prop_map(|chat| WorkstreamCommand::Contribute {
                chat,
                by: "alice".into()
            }),
        ]
    }

    proptest! {
        /// The workstream invariants — mirrors workstream.qnt — hold over every reachable
        /// trace.
        #[test]
        fn workstream_invariants(commands in prop::collection::vec(arb_command(), 0..60)) {
            let universe = ["a", "b", "c"];
            let mut s = WorkstreamState::default();
            for c in commands {
                s = apply(&s, c);
                // MEMBERSHIP_GATES_AUTOSYNC: a contribution is admissible ONLY for a member
                // of an active stream.
                for chat in universe {
                    if decide(&s, WorkstreamCommand::Contribute { chat: chat.into(), by: "alice".into() }).is_ok() {
                        prop_assert_eq!(s.phase, WorkstreamPhase::Active);
                        prop_assert!(
                            s.members.contains(chat),
                            "a non-member advanced the stream main"
                        );
                    }
                }
                // ARCHIVE_REHOMES_MEMBERS + ARCHIVED_ACCEPTS_NO_ADVANCE: an archived stream
                // has no members and admits no contribution.
                if s.phase == WorkstreamPhase::Archived {
                    prop_assert!(s.members.is_empty(), "archived stream left members stranded");
                    for chat in universe {
                        prop_assert!(
                            decide(&s, WorkstreamCommand::Contribute { chat: chat.into(), by: "alice".into() }).is_err(),
                            "archived stream admitted an advance"
                        );
                    }
                }
            }
        }
    }
}

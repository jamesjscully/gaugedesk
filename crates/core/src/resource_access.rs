//! Resource access lifecycle — the `(decide, evolve)` reducer, ported from
//! `specs/models/resource-access.qnt`.
//!
//! Grants a bounded read/use basis for a run's method/context resources
//! (`INV-10`, `INV-12`). Discharges `ACCESS_REQUIRES_GRANT`: a resource may be
//! *used* only after the access is `Granted` (all required approvers approved)
//! and not revoked. Revocation is future-only (`INV-18`).

use std::collections::BTreeSet;

use crate::boundary::Authority;
use crate::Rejection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AccessPhase {
    Init,
    Requested,
    Granted,
    Revoked,
    /// Terminal: rejected, canceled, or expired.
    Denied,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessState {
    pub phase: AccessPhase,
    pub required: BTreeSet<Authority>,
    pub approvals: BTreeSet<Authority>,
}

impl Default for AccessState {
    fn default() -> Self {
        Self {
            phase: AccessPhase::Init,
            required: BTreeSet::new(),
            approvals: BTreeSet::new(),
        }
    }
}

impl AccessState {
    /// Whether the resource *handle's name* may be shown — its existence, kind,
    /// and label, with no payload (`INV-10`). A handle always names something, so
    /// the name is visible in every phase: a mobile/remote projection can render
    /// a file tree of handle names before (or without) any access grant. This is
    /// deliberately split from [`payload_accessible`](Self::payload_accessible):
    /// holding a handle is not holding the payload.
    pub fn name_visible(&self) -> bool {
        true
    }

    /// Whether the resource *payload* may be read/used. Unlike the name, this is
    /// a separate, granted basis: it is admitted only from a `Granted` state that
    /// has not been revoked — the same precondition that gates `Use`
    /// (`ACCESS_REQUIRES_GRANT`, `INV-10`/`INV-12`). Name visibility never implies
    /// payload access; that is the `HANDLE_HIDES_PAYLOAD` invariant.
    pub fn payload_accessible(&self) -> bool {
        self.phase == AccessPhase::Granted
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessCommand {
    RequestAccess { required: BTreeSet<Authority> },
    Approve(Authority),
    Reject(Authority),
    Revoke,
    Expire,
    Cancel,
    Use,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AccessEvent {
    AccessRequested { required: BTreeSet<Authority> },
    Approved(Authority),
    Granted,
    Revoked,
    Rejected(Authority),
    Expired,
    Canceled,
    Used,
}

fn reject(reason: &'static str) -> Result<Vec<AccessEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(state: &AccessState, command: AccessCommand) -> Result<Vec<AccessEvent>, Rejection> {
    use AccessPhase::*;
    match command {
        AccessCommand::RequestAccess { required } => match state.phase {
            Init => Ok(vec![AccessEvent::AccessRequested { required }]),
            _ => reject("requestAccess: already exists"),
        },
        AccessCommand::Approve(a) => {
            let ok = state.phase == Requested
                && state.required.contains(&a)
                && !state.approvals.contains(&a);
            if !ok {
                return reject("approve: not a pending required approver");
            }
            let mut events = vec![AccessEvent::Approved(a.clone())];
            let mut after = state.approvals.clone();
            after.insert(a);
            if after == state.required {
                events.push(AccessEvent::Granted);
            }
            Ok(events)
        }
        // An approver rejects → denied (the negative of approve).
        AccessCommand::Reject(a) => {
            if matches!(state.phase, Requested | Granted) && state.required.contains(&a) {
                Ok(vec![AccessEvent::Rejected(a)])
            } else {
                reject("rejectAccess: not a required approver in a reviewable phase")
            }
        }
        AccessCommand::Revoke => match state.phase {
            Granted => Ok(vec![AccessEvent::Revoked]),
            _ => reject("revoke: not granted"),
        },
        AccessCommand::Expire => match state.phase {
            Requested | Granted => Ok(vec![AccessEvent::Expired]),
            _ => reject("expire: terminal"),
        },
        AccessCommand::Cancel => match state.phase {
            Requested => Ok(vec![AccessEvent::Canceled]),
            _ => reject("cancel: no pending request"),
        },
        // ACCESS_REQUIRES_GRANT: use only from Granted (not revoked/expired).
        AccessCommand::Use => match state.phase {
            Granted => Ok(vec![AccessEvent::Used]),
            _ => reject("use: access not granted (ACCESS_REQUIRES_GRANT)"),
        },
    }
}

pub fn evolve(state: &AccessState, event: AccessEvent) -> AccessState {
    use AccessPhase::*;
    let mut s = state.clone();
    match event {
        AccessEvent::AccessRequested { required } => {
            s.phase = Requested;
            s.required = required;
            s.approvals.clear();
        }
        AccessEvent::Approved(a) => {
            s.approvals.insert(a);
        }
        AccessEvent::Granted => s.phase = Granted,
        AccessEvent::Revoked => s.phase = Revoked,
        AccessEvent::Rejected(_) | AccessEvent::Expired | AccessEvent::Canceled => s.phase = Denied,
        AccessEvent::Used => {} // evidence; no phase change
    }
    s
}

impl crate::Lifecycle for AccessState {
    type State = AccessState;
    type Command = AccessCommand;
    type Event = AccessEvent;
    const KIND: &'static str = "resource_access";
    fn decide(state: &AccessState, command: AccessCommand) -> Result<Vec<AccessEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &AccessState, event: AccessEvent) -> AccessState {
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
    fn apply(state: &AccessState, command: AccessCommand) -> AccessState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }

    #[test]
    fn grant_then_use() {
        let s = AccessState::default();
        let s = apply(
            &s,
            AccessCommand::RequestAccess {
                required: req(&["A"]),
            },
        );
        assert_eq!(s.phase, AccessPhase::Requested);
        let s = apply(&s, AccessCommand::Approve("A".into()));
        assert_eq!(s.phase, AccessPhase::Granted);
        // use is admitted only now
        assert!(decide(&s, AccessCommand::Use).is_ok());
    }

    #[test]
    fn use_before_grant_rejected() {
        let s = AccessState::default();
        let s = apply(
            &s,
            AccessCommand::RequestAccess {
                required: req(&["A"]),
            },
        );
        assert!(decide(&s, AccessCommand::Use).is_err());
    }

    #[test]
    fn use_after_revoke_rejected() {
        let s = AccessState::default();
        let s = apply(
            &s,
            AccessCommand::RequestAccess {
                required: req(&["A"]),
            },
        );
        let s = apply(&s, AccessCommand::Approve("A".into()));
        let s = apply(&s, AccessCommand::Revoke);
        assert!(
            decide(&s, AccessCommand::Use).is_err(),
            "no use after revoke (INV-18)"
        );
    }

    #[test]
    fn reject_and_cancel_deny_then_block_use() {
        let s = AccessState::default();
        let s = apply(
            &s,
            AccessCommand::RequestAccess {
                required: req(&["A", "B"]),
            },
        );
        let s = apply(&s, AccessCommand::Reject("B".into()));
        assert_eq!(s.phase, AccessPhase::Denied);
        assert!(decide(&s, AccessCommand::Use).is_err(), "no use after deny");
        let c = AccessState::default();
        let c = apply(
            &c,
            AccessCommand::RequestAccess {
                required: req(&["A"]),
            },
        );
        let c = apply(&c, AccessCommand::Cancel);
        assert_eq!(c.phase, AccessPhase::Denied);
    }

    #[test]
    fn name_visible_without_payload_access() {
        // INV-10: a handle's name is visible in every phase, but its payload is
        // not accessible until the access is granted. The two are split.
        let s = AccessState::default();
        assert!(s.name_visible(), "handle name is visible in Init");
        assert!(!s.payload_accessible(), "no payload in Init");

        let s = apply(
            &s,
            AccessCommand::RequestAccess {
                required: req(&["A"]),
            },
        );
        assert!(
            s.name_visible() && !s.payload_accessible(),
            "name only while Requested"
        );

        let s = apply(&s, AccessCommand::Approve("A".into()));
        assert_eq!(s.phase, AccessPhase::Granted);
        assert!(
            s.name_visible() && s.payload_accessible(),
            "payload only after grant"
        );

        let s = apply(&s, AccessCommand::Revoke);
        assert!(s.name_visible(), "name stays visible after revoke");
        assert!(
            !s.payload_accessible(),
            "payload gone after revoke (INV-18)"
        );
    }

    /// The injected fault for `HANDLE_HIDES_PAYLOAD`: a handle whose visible name
    /// is taken to imply payload access. Mirrors the model's
    /// `HANDLE_GRANTS_PAYLOAD` tooth — flipping this true must break the
    /// invariant.
    fn payload_accessible_with_tooth(state: &AccessState, handle_grants_payload: bool) -> bool {
        state.payload_accessible() || (handle_grants_payload && state.name_visible())
    }

    fn arb_command() -> impl Strategy<Value = AccessCommand> {
        let who = prop_oneof![
            Just(Authority::from("A")),
            Just(Authority::from("B")),
            Just(Authority::from("X"))
        ];
        prop_oneof![
            Just(AccessCommand::RequestAccess {
                required: req(&["A", "B"])
            }),
            who.clone().prop_map(AccessCommand::Approve),
            who.prop_map(AccessCommand::Reject),
            Just(AccessCommand::Revoke),
            Just(AccessCommand::Expire),
            Just(AccessCommand::Cancel),
            Just(AccessCommand::Use),
        ]
    }

    proptest! {
        /// ACCESS_REQUIRES_GRANT: `Use` is admitted only from a `Granted` state.
        #[test]
        fn access_requires_grant(commands in prop::collection::vec(arb_command(), 0..40)) {
            let mut s = AccessState::default();
            for c in commands {
                if c == AccessCommand::Use && decide(&s, AccessCommand::Use).is_ok() {
                    prop_assert_eq!(s.phase, AccessPhase::Granted);
                }
                s = apply(&s, c);
            }
        }

        /// HANDLE_HIDES_PAYLOAD (INV-10): name visibility never implies payload
        /// access. Whenever the payload is accessible it is because the access is
        /// `Granted` — not because the handle's name is visible. The
        /// `handle_grants_payload` tooth (off) must break this property when on.
        #[test]
        fn handle_hides_payload(commands in prop::collection::vec(arb_command(), 0..40)) {
            let handle_grants_payload = false; // tooth off
            let mut s = AccessState::default();
            for c in commands {
                // The handle's name is always visible (it always names something).
                prop_assert!(s.name_visible());
                // ...yet that visibility alone never opens the payload.
                if payload_accessible_with_tooth(&s, handle_grants_payload) {
                    prop_assert_eq!(s.phase, AccessPhase::Granted);
                }
                s = apply(&s, c);
            }
        }
    }

    /// Tooth: with `handle_grants_payload` on, a visible name opens the payload
    /// in a non-granted phase — violating `HANDLE_HIDES_PAYLOAD`. Proves the
    /// proptest above has teeth.
    #[test]
    fn handle_grants_payload_tooth_bites() {
        // Init: name visible, not granted. The tooth claims payload access anyway.
        let s = AccessState::default();
        assert_ne!(s.phase, AccessPhase::Granted);
        assert!(
            payload_accessible_with_tooth(&s, true),
            "tooth must open the payload from a visible name pre-grant"
        );
    }
}

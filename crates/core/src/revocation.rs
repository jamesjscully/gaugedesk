//! Revocation + `PAST_PRESERVED` (`INV-18`) — ported from
//! `specs/models/revocation.qnt`.
//!
//! The temporal companion to [`crate::boundary`]: where `boundary` proves a
//! *single* egress is sound (conjunctive consent), this proves the property
//! **across a trace of consent / egress / revoke** steps:
//! - A stakeholder may revoke consent; afterwards **future** egress under the
//!   revoked basis is blocked (consent is conjunctive — any one veto freezes it).
//! - Already-released items are **never clawed back**: `obtained` only grows, so
//!   everything ever released stays held by its recipient (`PAST_PRESERVED`).

use std::collections::{BTreeMap, BTreeSet};

use crate::boundary::Authority;
use crate::resource::ResourceId;

/// An admitted basis: stakeholder `by` permits resource `res` to reach `to`.
/// `res` is the typed handle from the resource primitive ([`ResourceId`], `INV-10`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Basis {
    pub res: ResourceId,
    pub to: Authority,
    pub by: Authority,
}

#[derive(Clone, Debug, Default)]
pub struct EgressState {
    /// What each recipient currently holds. Monotone — only grows.
    pub obtained: BTreeMap<Authority, BTreeSet<ResourceId>>,
    pub bases: BTreeSet<Basis>,
    /// Immutable release history: every (item, recipient) ever released.
    pub ever_released: BTreeSet<(ResourceId, Authority)>,
}

#[derive(Clone, Debug)]
pub enum EgressCommand {
    Consent {
        res: ResourceId,
        to: Authority,
        by: Authority,
    },
    Egress {
        res: ResourceId,
        to: Authority,
    },
    Revoke {
        res: ResourceId,
        to: Authority,
        by: Authority,
    },
}

#[derive(Clone, Debug)]
pub enum EgressEvent {
    Consented(Basis),
    Released { res: ResourceId, to: Authority },
    Revoked(Basis),
}

/// Egress allowed iff the recipient is trusted, or every stakeholder of the item
/// (except the recipient) has admitted a basis to that recipient.
pub fn allowed_egress(
    state: &EgressState,
    stakeholders: &BTreeSet<Authority>,
    tcb: &BTreeSet<Authority>,
    res: &ResourceId,
    to: &Authority,
) -> bool {
    if tcb.contains(to) {
        return true;
    }
    stakeholders.iter().filter(|a| *a != to).all(|a| {
        state.bases.contains(&Basis {
            res: res.clone(),
            to: to.clone(),
            by: a.clone(),
        })
    })
}

/// `decide` here is parameterized by the protection context (stakeholders, tcb)
/// the imperative shell materializes before calling.
pub fn decide(
    state: &EgressState,
    stakeholders: &BTreeSet<Authority>,
    tcb: &BTreeSet<Authority>,
    command: EgressCommand,
) -> Result<Vec<EgressEvent>, crate::Rejection> {
    match command {
        EgressCommand::Consent { res, to, by } => {
            Ok(vec![EgressEvent::Consented(Basis { res, to, by })])
        }
        EgressCommand::Egress { res, to } => {
            if allowed_egress(state, stakeholders, tcb, &res, &to) {
                Ok(vec![EgressEvent::Released { res, to }])
            } else {
                Err(crate::Rejection {
                    reason: "egress: not all stakeholders consent (SOUND)",
                })
            }
        }
        // INV-18: revocation removes the basis (future-only); history untouched.
        EgressCommand::Revoke { res, to, by } => {
            Ok(vec![EgressEvent::Revoked(Basis { res, to, by })])
        }
    }
}

pub fn evolve(state: &EgressState, event: EgressEvent) -> EgressState {
    let mut s = state.clone();
    match event {
        EgressEvent::Consented(b) => {
            s.bases.insert(b);
        }
        EgressEvent::Released { res, to } => {
            s.obtained
                .entry(to.clone())
                .or_default()
                .insert(res.clone());
            s.ever_released.insert((res, to));
        }
        EgressEvent::Revoked(b) => {
            s.bases.remove(&b);
            // obtained is NOT touched — the recipient keeps what it already holds.
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const OUT: &str = "out";
    fn out() -> ResourceId {
        ResourceId::new(OUT)
    }
    fn stakeholders() -> BTreeSet<Authority> {
        ["A", "B"].iter().map(|s| Authority::from(*s)).collect()
    }
    fn tcb() -> BTreeSet<Authority> {
        ["model"].iter().map(|s| Authority::from(*s)).collect()
    }
    fn apply(state: &EgressState, command: EgressCommand) -> EgressState {
        match decide(state, &stakeholders(), &tcb(), command) {
            Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }
    fn held(s: &EgressState, to: &str, item: &ResourceId) -> bool {
        s.obtained
            .get(&Authority::from(to))
            .is_some_and(|set| set.contains(item))
    }

    #[test]
    fn release_then_revoke_keeps_past_blocks_future() {
        let s = EgressState::default();
        let s = apply(
            &s,
            EgressCommand::Consent {
                res: out(),
                to: "ext".into(),
                by: "A".into(),
            },
        );
        let s = apply(
            &s,
            EgressCommand::Consent {
                res: out(),
                to: "ext".into(),
                by: "B".into(),
            },
        );
        let s = apply(
            &s,
            EgressCommand::Egress {
                res: out(),
                to: "ext".into(),
            },
        );
        assert!(held(&s, "ext", &out()));
        let s = apply(
            &s,
            EgressCommand::Revoke {
                res: out(),
                to: "ext".into(),
                by: "A".into(),
            },
        );
        assert!(
            held(&s, "ext", &out()),
            "INV-18: past release still held after revoke"
        );
        assert!(
            !allowed_egress(&s, &stakeholders(), &tcb(), &out(), &"ext".into()),
            "A's revocation freezes future egress"
        );
    }

    fn arb_command() -> impl Strategy<Value = EgressCommand> {
        let who = prop_oneof![Just(Authority::from("A")), Just(Authority::from("B"))];
        let to = prop_oneof![
            Just(Authority::from("ext")),
            Just(Authority::from("A")),
            Just(Authority::from("B"))
        ];
        prop_oneof![
            (to.clone(), who.clone()).prop_map(|(to, by)| EgressCommand::Consent {
                res: out(),
                to,
                by
            }),
            to.clone()
                .prop_map(|to| EgressCommand::Egress { res: out(), to }),
            (to, who).prop_map(|(to, by)| EgressCommand::Revoke { res: out(), to, by }),
        ]
    }

    proptest! {
        /// PAST_PRESERVED (INV-18): everything ever released remains held by its
        /// recipient, no matter how consent is later revoked.
        #[test]
        fn past_preserved(commands in prop::collection::vec(arb_command(), 0..60)) {
            let mut s = EgressState::default();
            for c in commands {
                s = apply(&s, c);
                for (item, to) in &s.ever_released {
                    prop_assert!(held(&s, to.as_str(), item), "INV-18 violated: {to} lost {item:?}");
                }
            }
        }
    }
}

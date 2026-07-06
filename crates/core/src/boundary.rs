//! Boundary egress + engagement-scoped taint — the protection core, ported from
//! `specs/models/engagement-taint.qnt` and `derived-output.qnt`.
//!
//! Discharges the confidentiality goal (`INV-22`): a protected output reaches an
//! authority outside its stakeholders + the placement's trusted set only with an
//! admitted basis from **every** stakeholder (conjunctive consent, `INV-13`).
//! Taint is **engagement-scoped** (`ADR 0026`): an output's stakeholders are the
//! owners of everything *the engagement* read, not just one run.

use std::collections::BTreeSet;

/// The authority responsible for a durable fact (`INV-1`) — the typed identity
/// from [`crate::ids`], re-exported here so the boundary's egress vocabulary
/// (stakeholders, recipients, bases) is the same typed identity the rest of the
/// core uses, never a bare string.
pub use crate::ids::AuthorityId as Authority;

/// An admitted egress basis: authority `by` permits a resource to reach `to`.
pub type Basis = (Authority, Authority); // (by, to)

/// Engagement-scoped conservative taint: the owners of everything the engagement
/// read up to production. Computed by the boundary, never agent-asserted.
pub fn taint(engagement_read_owners: &BTreeSet<Authority>) -> BTreeSet<Authority> {
    engagement_read_owners.clone()
}

/// Egress allowed iff the recipient is trusted (`tcb`), or every stakeholder
/// except the recipient has admitted a basis to that recipient.
pub fn allowed_egress(
    stakeholders: &BTreeSet<Authority>,
    recipient: &Authority,
    tcb: &BTreeSet<Authority>,
    bases: &BTreeSet<Basis>,
) -> bool {
    if tcb.contains(recipient) {
        return true;
    }
    stakeholders
        .iter()
        .filter(|s| *s != recipient)
        .all(|s| bases.contains(&(s.clone(), recipient.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn set(xs: &[&str]) -> BTreeSet<Authority> {
        xs.iter().map(|s| Authority::from(*s)).collect()
    }
    fn bases(pairs: &[(&str, &str)]) -> BTreeSet<Basis> {
        pairs
            .iter()
            .map(|(b, t)| (Authority::from(*b), Authority::from(*t)))
            .collect()
    }

    #[test]
    fn tcb_recipient_always_allowed() {
        assert!(allowed_egress(
            &set(&["A", "B"]),
            &"model".into(),
            &set(&["model"]),
            &BTreeSet::new()
        ));
    }

    #[test]
    fn needs_every_stakeholder() {
        let stake = set(&["A", "B"]);
        let tcb = set(&["model"]);
        assert!(
            !allowed_egress(&stake, &"ext".into(), &tcb, &bases(&[("A", "ext")])),
            "B hasn't consented"
        );
        assert!(allowed_egress(
            &stake,
            &"ext".into(),
            &tcb,
            &bases(&[("A", "ext"), ("B", "ext")])
        ));
    }

    fn auth() -> impl Strategy<Value = &'static str> {
        prop_oneof![Just("A"), Just("B")]
    }
    fn recip() -> impl Strategy<Value = &'static str> {
        prop_oneof![Just("A"), Just("B"), Just("ext"), Just("model")]
    }

    proptest! {
        /// INV-22 / SOUND: an output reaches a non-TCB recipient only when every
        /// engagement-scoped stakeholder consented. (Quint: engagement-taint.qnt.)
        #[test]
        fn sound_release(
            read_owners in prop::collection::vec(auth(), 0..10),
            granted in prop::collection::vec((auth(), recip()), 0..6),
            recipient in recip(),
        ) {
            let tcb = set(&["model"]);
            let stakeholders: BTreeSet<Authority> = read_owners.iter().map(|s| Authority::from(*s)).collect();
            let basis_set: BTreeSet<Basis> = granted.iter().map(|(b, t)| (Authority::from(*b), Authority::from(*t))).collect();
            let recipient = Authority::from(recipient);

            let t = taint(&stakeholders); // engagement-scoped
            if allowed_egress(&t, &recipient, &tcb, &basis_set) && !tcb.contains(&recipient) {
                for s in t.iter().filter(|s| **s != recipient) {
                    prop_assert!(
                        basis_set.contains(&(s.clone(), recipient.clone())),
                        "SOUND violated: {recipient} obtained the output without {s}'s consent"
                    );
                }
            }
        }
    }
}

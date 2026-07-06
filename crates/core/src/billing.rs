//! Settlement-plane billing policy (M3 `SETTLE-1`, [ADR 0060]) — the **pure** split for one
//! consultant↔client engagement, in the **two billing relationships** the settlement plane
//! runs:
//!
//! - **(a) the transaction** — the consultant bills the client as **merchant-of-record** (a
//!   Connect *direct* charge on the consultant's account); the platform takes its cut as the
//!   `application_fee` **take-rate**. This is **rail-contingent**: the platform earns it only
//!   when the consultant routes the client charge through our rail (the realization concern
//!   lives in `payment.rs`, not here).
//! - **(b) the platform charge** — the **metered attested-compute floor** (our cost + margin)
//!   billed **to the consultant**, regardless of engagement value. This is the **guaranteed**
//!   capture and the rail that realizes the floor's owe-case: an internal $0-value engagement
//!   still owes the floor, and relationship (b) can actually charge it (a *positive*
//!   `PlatformCharge`), where the old single destination charge could not.
//!
//! Relay/multi-party metering and consultant-org **seat fees** also ride relationship (b), but
//! at the **org** level (`SETTLE-3`), not per-engagement — so they are out of this pure policy.
//! The metered floor is the only per-engagement term.
//!
//! This is the loopback-buildable half — the *policy arithmetic* over the entitlement grant's
//! `engagement-id + value` (ADR 0048). The deferred half is the Stripe rail in `payment.rs`.
//!
//! [ADR 0060]: ../../specs/decisions/0060-settlement-plane-direct-charges-consultant-mor.md

/// The platform's per-engagement billing knobs (ADR 0060 §2–§3 — policy, not tiers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BillingPolicy {
    /// Platform take on transacted engagement value, in **basis points** (1% = 100 bps,
    /// 100% = 10 000 bps). Realized as the direct charge's `application_fee` — rail-contingent.
    pub take_rate_bps: u32,
    /// The metered attested-compute floor **per sealed run**, in cents (cost + margin),
    /// billed to the consultant via relationship (b).
    pub metered_floor_cents: u64,
}

/// One engagement's transacted facts, as read from the entitlement grant (ADR 0048).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Engagement {
    /// The consultant-set price the consultant bills the client, in cents.
    pub value_cents: u64,
    /// Attested sealed runs in the engagement — the metered-floor multiplier.
    pub attested_runs: u64,
}

/// **(a)** The consultant↔client transaction: a direct charge on the **consultant's** account
/// (consultant = merchant-of-record). The consultant's transaction net is always ≥ 0 — the
/// floor does not come out of this flow (it is relationship (b)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TransactionSettlement {
    /// What the client pays the consultant (the consultant-set price).
    pub client_charge_cents: u64,
    /// The platform's `application_fee` cut of the transaction — the take-rate (rail-contingent).
    pub platform_take_cents: u64,
    /// What the consultant nets on the transaction: charge − take. Always ≥ 0.
    pub consultant_net_cents: u64,
}

/// **(b)** The platform→consultant charge: the metered floor billed to the consultant,
/// owed regardless of engagement value. (Relay/seats are added at the org level, `SETTLE-3`.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlatformCharge {
    /// The metered floor across all attested runs in this engagement.
    pub metered_floor_total_cents: u64,
    /// What the consultant owes the platform for (b) this engagement. (= the floor here.)
    pub total_cents: u64,
}

/// The settled split across both relationships, plus reconciled totals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Settlement {
    /// (a) the consultant↔client direct-charge transaction.
    pub transaction: TransactionSettlement,
    /// (b) the platform→consultant metered charge.
    pub platform_charge: PlatformCharge,
    /// The platform's total cut — take + floor ("a cut of everything").
    pub platform_revenue_cents: u64,
    /// The consultant's overall net across both relationships: transaction net − platform
    /// charge. **Signed**: negative ⇒ the consultant owes (e.g. an internal $0-value run still
    /// owes the floor) — and now relationship (b) actually collects it.
    pub consultant_net_cents: i64,
}

/// The free seats every consultant org gets before the platform seat fee applies
/// (`SETTLE-3`, ADR 0060 §4). A platform fee on *our* relationship with the consultant org
/// (relationship (b)), distinct from the consultant↔client take-rate. ADR 0068 later
/// promotes enterprise governance/SSO packaging as a separate paid entitlement.
pub const FREE_CONSULTANT_SEATS: u64 = 3;

/// The platform seat fee owed by a consultant org on relationship (b) (`SETTLE-3`): the
/// per-seat fee times the seats **beyond** the free allotment. Pure; saturating. Zero when at
/// or below the free seats (e.g. solo/tenant-of-one and small consultancies pay nothing).
pub fn platform_seat_charge(seats_used: u64, free_seats: u64, seat_fee_cents: u64) -> u64 {
    seat_fee_cents.saturating_mul(seats_used.saturating_sub(free_seats))
}

impl BillingPolicy {
    /// Settle one engagement into its two-relationship split. Pure; saturating arithmetic.
    ///
    /// Money is conserved across the transaction: `client_charge == platform_take +
    /// transaction.consultant_net`. The floor sits in relationship (b) and is what makes the
    /// consultant's *overall* net go negative on a low/zero-value engagement — which (b) bills.
    pub fn settle(&self, e: &Engagement) -> Settlement {
        let take = (e.value_cents as u128 * self.take_rate_bps as u128 / 10_000)
            .min(u64::MAX as u128) as u64;
        let transaction_net = e.value_cents.saturating_sub(take);
        let metered_floor_total = self.metered_floor_cents.saturating_mul(e.attested_runs);
        let platform_revenue = take.saturating_add(metered_floor_total);
        let consultant_net = transaction_net as i64 - metered_floor_total as i64;
        Settlement {
            transaction: TransactionSettlement {
                client_charge_cents: e.value_cents,
                platform_take_cents: take,
                consultant_net_cents: transaction_net,
            },
            platform_charge: PlatformCharge {
                metered_floor_total_cents: metered_floor_total,
                total_cents: metered_floor_total,
            },
            platform_revenue_cents: platform_revenue,
            consultant_net_cents: consultant_net,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const TEN_PCT: BillingPolicy = BillingPolicy {
        take_rate_bps: 1000, // 10%
        metered_floor_cents: 500,
    };

    #[test]
    fn take_rate_is_the_application_fee_on_the_direct_charge() {
        // $100.00 engagement, no runs: 10% take, consultant nets $90.00 on the transaction,
        // client pays the consultant $100; nothing owed on relationship (b).
        let s = TEN_PCT.settle(&Engagement {
            value_cents: 10_000,
            attested_runs: 0,
        });
        assert_eq!(s.transaction.client_charge_cents, 10_000);
        assert_eq!(s.transaction.platform_take_cents, 1_000);
        assert_eq!(s.transaction.consultant_net_cents, 9_000);
        assert_eq!(s.platform_charge.total_cents, 0);
        assert_eq!(s.platform_revenue_cents, 1_000);
        assert_eq!(s.consultant_net_cents, 9_000);
    }

    #[test]
    fn metered_floor_is_billed_to_the_consultant_via_relationship_b() {
        // 3 attested runs add 3 × $5.00 floor to relationship (b) — NOT subtracted from the
        // client→consultant transaction (the consultant's transaction net stays $90).
        let s = TEN_PCT.settle(&Engagement {
            value_cents: 10_000,
            attested_runs: 3,
        });
        assert_eq!(s.transaction.consultant_net_cents, 9_000); // transaction unaffected by floor
        assert_eq!(s.platform_charge.metered_floor_total_cents, 1_500);
        assert_eq!(s.platform_charge.total_cents, 1_500);
        assert_eq!(s.platform_revenue_cents, 2_500); // take 1000 + floor 1500
        assert_eq!(s.consultant_net_cents, 7_500); // 9000 transaction net − 1500 floor
    }

    #[test]
    fn an_internal_zero_value_run_owes_the_floor_and_b_can_collect_it() {
        // $0 engagement with 2 attested runs: no transaction, no take, but the floor is owed —
        // a POSITIVE PlatformCharge that relationship (b) actually bills (the owe-case the old
        // destination rail could not realize), and a negative overall net.
        let s = TEN_PCT.settle(&Engagement {
            value_cents: 0,
            attested_runs: 2,
        });
        assert_eq!(s.transaction.client_charge_cents, 0);
        assert_eq!(s.transaction.platform_take_cents, 0);
        assert_eq!(s.platform_charge.total_cents, 1_000); // (b) charges the consultant $10
        assert_eq!(s.platform_revenue_cents, 1_000);
        assert_eq!(s.consultant_net_cents, -1_000); // consultant owes the floor
    }

    #[test]
    fn platform_seat_fee_is_free_up_to_the_allotment_then_per_seat() {
        // 3 free; $20/seat beyond. Solo/small orgs pay nothing; a 5-seat org owes 2 × $20.
        assert_eq!(platform_seat_charge(1, FREE_CONSULTANT_SEATS, 2_000), 0);
        assert_eq!(platform_seat_charge(3, FREE_CONSULTANT_SEATS, 2_000), 0);
        assert_eq!(platform_seat_charge(5, FREE_CONSULTANT_SEATS, 2_000), 4_000);
        assert_eq!(FREE_CONSULTANT_SEATS, 3);
    }

    proptest! {
        /// Money is conserved on the transaction (client charge = take + consultant transaction
        /// net), the consultant's transaction net is never negative (the floor is relationship
        /// (b), not a deduction), and the platform's overall cut equals take + floor.
        #[test]
        fn money_is_conserved(
            value in 0u64..1_000_000_000,
            runs in 0u64..1_000,
            floor in 0u64..1_000_000,
            bps in 0u32..=10_000,
        ) {
            let s = BillingPolicy { take_rate_bps: bps, metered_floor_cents: floor }
                .settle(&Engagement { value_cents: value, attested_runs: runs });
            // (a) the transaction conserves money and never pays the consultant negative.
            prop_assert_eq!(
                s.transaction.client_charge_cents,
                s.transaction.platform_take_cents + s.transaction.consultant_net_cents
            );
            // (b) the floor is owed regardless of value, billed to the consultant.
            prop_assert_eq!(s.platform_charge.total_cents, floor.saturating_mul(runs));
            // The platform's overall cut is take + floor ("a cut of everything").
            prop_assert_eq!(
                s.platform_revenue_cents,
                s.transaction.platform_take_cents + s.platform_charge.total_cents
            );
            // The consultant's overall net reconciles: transaction net − platform charge.
            prop_assert_eq!(
                s.consultant_net_cents,
                s.transaction.consultant_net_cents as i64 - s.platform_charge.total_cents as i64
            );
            // The take never exceeds the value (capped at 100% = 10 000 bps).
            prop_assert!(s.transaction.platform_take_cents <= value);
        }
    }
}

//! Boundary / pairing lifecycle (M1). Ported from
//! `specs/models/boundary-lifecycle.qnt`, with the **placement** attribute (M1
//! BP-1, ADR 0003/0006): a boundary is proposed with its participants, declares a
//! ceiling derived from a `Placement`, collects every participant's acceptance,
//! and only then admits resources / starts sessions. Teardown blocks future use
//! but preserves evidence (INV-18).
//!
//! Discharges: RESOURCE_REQUIRES_ACTIVE_BOUNDARY · SESSION_REQUIRES_ACTIVE_BOUNDARY
//! · NO_GHOST_ACCEPT · CEILING_IMMUTABLE_AFTER_RESOURCE (structural — no command
//! re-declares a ceiling) · EVIDENCE_PRESERVED_ON_TEARDOWN.

use std::collections::{BTreeMap, BTreeSet};

use crate::attestation::AttestationEvidence;
use crate::ids::{BridgeGrantId, DeviceId};
use crate::Rejection;

/// Who physically operates the boundary host. Orthogonal to attestation: it sets
/// the physical-attack / availability threat model (ADR 0040), **not** the
/// confidentiality ceiling.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Operator {
    /// The run owner operates the host (local desktop, or a node you control).
    Local,
    /// The other party operates the host (their hardware).
    Counterparty,
    /// A third party operates the host (neutral cloud).
    Neutral,
}

/// Where the boundary executes, as two **orthogonal** axes (ADR 0040): who operates
/// the host (`operator`) and whether execution is attested (`attested`). Attestation
/// makes confidentiality placement-independent, so the ceiling — does the host see
/// the method in plaintext? — depends only on `attested`; `operator` is the
/// physical-attack/availability threat model. This lets *counterparty-hosted +
/// attested* be expressed (the strong commercial case the old flat enum could not).
/// `attested` execution is M2 prep (D-ATTEST / ADR 0040) — the seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Placement {
    pub operator: Operator,
    /// Attested (TEE) execution removes the host from the trusted set: host-blind
    /// (`boundary.qnt`: host ∈ `tcbAt` ⟺ ¬attested).
    pub attested: bool,
}

impl Placement {
    /// A local, unattested placement (the M0/M1 default: you operate the host).
    pub const fn local() -> Self {
        Self {
            operator: Operator::Local,
            attested: false,
        }
    }

    /// The honest method-secrecy ceiling: is the method hidden from the host? True
    /// iff attested — independent of who operates it (ADR 0040).
    pub fn method_secret(self) -> bool {
        self.attested
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryPhase {
    #[default]
    Init,
    Proposed,
    Declared,
    /// A paired device has been bound to the boundary's bridge grant before it
    /// goes active (D-MOBILE / ADR 0009). Reachable from `Declared`; acceptance
    /// proceeds from here exactly as from `Declared`, so device binding is an
    /// optional refinement of the ceiling-declared boundary, not a new gate on
    /// the accept→active path.
    DeviceBinding,
    Active,
    Draining,
    TornDown,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, serde::Serialize)]
pub struct BoundaryState {
    pub phase: BoundaryPhase,
    pub placement: Option<Placement>,
    /// The participant authorities that must accept before the boundary is active.
    pub required: BTreeSet<String>,
    pub accepted: BTreeSet<String>,
    /// Per-participant attestation evidence collected at acceptance (ATTEST-2).
    /// Populated only when the declared `placement` is `attested`: an attested
    /// acceptance carries the host's [`AttestationEvidence`], an unattested one
    /// carries none. The app `boundary_keeper` gate (ATTEST-5) consults this
    /// before releasing any sealed key.
    pub attestation_evidence: BTreeMap<String, AttestationEvidence>,
    /// The paired device bound to this boundary's bridge grant, recorded by the
    /// `DeviceBinding` phase (D-MOBILE). `Some` once a device has been bound; the
    /// typed `(DeviceId, BridgeGrantId)` pins the device key and grant a later
    /// federated delivery must match (MOB-004).
    pub device_binding: Option<(DeviceId, BridgeGrantId)>,
    pub resources_admitted: bool,
    pub session_started: bool,
    pub torn_down: bool,
    /// Evidence exists and is preserved across teardown (INV-18).
    pub evidence: bool,
}

impl BoundaryState {
    /// Active: declared ceiling + every participant accepted + not torn down.
    pub fn active(&self) -> bool {
        self.phase == BoundaryPhase::Active
            && self.placement.is_some()
            && self.required.is_subset(&self.accepted)
            && !self.torn_down
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundaryCommand {
    Propose(BTreeSet<String>),
    DeclareCeiling(Placement),
    /// A participant accepts the boundary. On an *attested* placement the
    /// acceptance must present [`AttestationEvidence`]; on an unattested one it
    /// must not (ATTEST-2).
    Accept {
        participant: String,
        evidence: Option<AttestationEvidence>,
    },
    /// Bind a paired device to the boundary's bridge grant (D-MOBILE). Only a
    /// ceiling-declared boundary admits a binding, and a boundary binds at most
    /// one device — a second binding is rejected so the bound device key cannot
    /// be silently swapped out from under an active grant.
    BindDevice {
        device: DeviceId,
        bridge_grant: BridgeGrantId,
    },
    Reject(String),
    AdmitResource,
    StartSession,
    EndSession,
    BeginTeardown,
    FinishTeardown,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BoundaryEvent {
    Proposed(BTreeSet<String>),
    CeilingDeclared(Placement),
    /// A participant accepted, carrying any attestation evidence presented for an
    /// attested placement (ATTEST-2). `evidence` is `Some` iff the declared
    /// placement is attested.
    Accepted {
        participant: String,
        evidence: Option<AttestationEvidence>,
    },
    /// A paired device was bound to the boundary's bridge grant (D-MOBILE).
    DeviceBound {
        device: DeviceId,
        bridge_grant: BridgeGrantId,
    },
    Rejected(String),
    ResourceAdmitted,
    SessionStarted,
    SessionEnded,
    TeardownBegun,
    TeardownFinished,
}

fn reject(reason: &'static str) -> Result<Vec<BoundaryEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(
    state: &BoundaryState,
    command: BoundaryCommand,
) -> Result<Vec<BoundaryEvent>, Rejection> {
    use BoundaryPhase::*;
    match command {
        BoundaryCommand::Propose(participants) => match state.phase {
            Init => Ok(vec![BoundaryEvent::Proposed(participants)]),
            _ => reject("propose: boundary already proposed"),
        },
        BoundaryCommand::DeclareCeiling(p) => match state.phase {
            Proposed => Ok(vec![BoundaryEvent::CeilingDeclared(p)]),
            _ => reject("declareCeiling: not in proposed (ceiling is immutable once set)"),
        },
        // NO_GHOST_ACCEPT: only a named participant accepts.
        BoundaryCommand::Accept {
            participant,
            evidence,
        } => {
            // Acceptance proceeds from a ceiling-declared boundary, whether or not
            // a device has since been bound (DeviceBinding is a refinement of
            // Declared, not a separate gate).
            if !matches!(state.phase, Declared | DeviceBinding)
                || !state.required.contains(&participant)
            {
                return reject("accept: not a declared boundary, or not a participant");
            }
            // ATTESTED_ACCEPT_REQUIRES_EVIDENCE: an attested placement admits only
            // acceptances that present attestation evidence; an unattested one
            // admits only acceptances that present none (ATTEST-2). The declared
            // `placement` is always present here (phase == Declared).
            let attested = state.placement.map(|p| p.attested).unwrap_or(false);
            match (attested, &evidence) {
                (true, None) => reject("accept: attested placement requires attestation evidence"),
                (false, Some(_)) => {
                    reject("accept: unattested placement carries no attestation evidence")
                }
                _ => Ok(vec![BoundaryEvent::Accepted {
                    participant,
                    evidence,
                }]),
            }
        }
        // DEVICE_BINDS_DECLARED_BOUNDARY: a device binds only to a ceiling-declared
        // (or already device-bound) boundary, and at most once — no silent swap of
        // the bound device key out from under the grant.
        BoundaryCommand::BindDevice {
            device,
            bridge_grant,
        } => {
            if !matches!(state.phase, Declared | DeviceBinding) {
                return reject("bindDevice: boundary has not declared a ceiling");
            }
            if state.device_binding.is_some() {
                return reject("bindDevice: a device is already bound (no silent swap)");
            }
            Ok(vec![BoundaryEvent::DeviceBound {
                device,
                bridge_grant,
            }])
        }
        BoundaryCommand::Reject(a) => {
            if matches!(state.phase, Proposed | Declared | DeviceBinding)
                && state.required.contains(&a)
            {
                Ok(vec![BoundaryEvent::Rejected(a)])
            } else {
                reject("reject: not a pending participant")
            }
        }
        // RESOURCE_REQUIRES_ACTIVE_BOUNDARY.
        BoundaryCommand::AdmitResource => {
            if state.active() {
                Ok(vec![BoundaryEvent::ResourceAdmitted])
            } else {
                reject("admitResource: boundary not active")
            }
        }
        // SESSION_REQUIRES_ACTIVE_BOUNDARY.
        BoundaryCommand::StartSession => {
            if state.active() {
                Ok(vec![BoundaryEvent::SessionStarted])
            } else {
                reject("startSession: boundary not active")
            }
        }
        BoundaryCommand::EndSession => {
            if state.session_started {
                Ok(vec![BoundaryEvent::SessionEnded])
            } else {
                reject("endSession: no session running")
            }
        }
        BoundaryCommand::BeginTeardown => match state.phase {
            Active | Draining => Ok(vec![BoundaryEvent::TeardownBegun]),
            _ => reject("beginTeardown: boundary not active"),
        },
        BoundaryCommand::FinishTeardown => match state.phase {
            Draining => Ok(vec![BoundaryEvent::TeardownFinished]),
            _ => reject("finishTeardown: not draining"),
        },
    }
}

pub fn evolve(state: &BoundaryState, event: BoundaryEvent) -> BoundaryState {
    use BoundaryPhase as P;
    let mut s = state.clone();
    match event {
        BoundaryEvent::Proposed(participants) => {
            s.phase = P::Proposed;
            s.required = participants;
        }
        BoundaryEvent::CeilingDeclared(p) => {
            s.phase = P::Declared;
            s.placement = Some(p);
            s.evidence = true;
        }
        BoundaryEvent::Accepted {
            participant,
            evidence,
        } => {
            if let Some(ev) = evidence {
                s.attestation_evidence.insert(participant.clone(), ev);
            }
            s.accepted.insert(participant);
            s.evidence = true;
            if matches!(s.phase, P::Declared | P::DeviceBinding)
                && s.required.is_subset(&s.accepted)
            {
                s.phase = P::Active;
            }
        }
        BoundaryEvent::DeviceBound {
            device,
            bridge_grant,
        } => {
            s.device_binding = Some((device, bridge_grant));
            // Refine Declared → DeviceBinding; a re-bind on an already-bound
            // boundary is rejected in decide, so phase only advances here.
            if s.phase == P::Declared {
                s.phase = P::DeviceBinding;
            }
            s.evidence = true;
        }
        BoundaryEvent::Rejected(_) => s.phase = P::Denied,
        BoundaryEvent::ResourceAdmitted => {
            s.resources_admitted = true;
            s.evidence = true;
        }
        BoundaryEvent::SessionStarted => {
            s.session_started = true;
            s.evidence = true;
        }
        BoundaryEvent::SessionEnded => s.session_started = false,
        BoundaryEvent::TeardownBegun => {
            s.phase = P::Draining;
            s.torn_down = true;
            s.session_started = false;
            // EVIDENCE_PRESERVED_ON_TEARDOWN: evidence kept.
        }
        BoundaryEvent::TeardownFinished => {
            s.phase = P::TornDown;
            s.torn_down = true;
            s.session_started = false;
        }
    }
    s
}

/// A restrict-only org **placement policy** (`DEPLOY-2`, [ADR 0059]/[ADR 0061]): which
/// `(operator, attested)` combos are admissible for engagements touching the org's data.
/// Like the resource-floor `Policy` (ADR 0032 / `RBAC-6`) it only **narrows** — the open
/// policy admits everything (the tenant-of-one / no-policy default), and each constraint
/// removes options, never adds (`ABAC_MONOTONE`). Fail-closed on each narrowed axis
/// (`INV-20`).
///
/// [ADR 0059]: ../../specs/decisions/0059-deployment-topology-headless-control-plane-policy-gated-pairing.md
/// [ADR 0061]: ../../specs/decisions/0061-tenant-and-home-governance.md
#[derive(Clone, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct PlacementPolicy {
    /// If true, only attested (host-blind) execution is admissible.
    #[serde(default)]
    pub require_attested: bool,
    /// The operators allowed. **Empty = all operators allowed** (no narrowing on this axis);
    /// a non-empty set narrows to exactly those operators.
    #[serde(default)]
    pub allowed_operators: BTreeSet<Operator>,
}

impl PlacementPolicy {
    /// The open policy: admits every placement (the no-policy / tenant-of-one default).
    pub fn open() -> Self {
        Self::default()
    }

    /// Does this policy admit the declared placement? Restrict-only and fail-closed.
    pub fn admits(&self, p: &Placement) -> bool {
        if self.require_attested && !p.attested {
            return false;
        }
        if !self.allowed_operators.is_empty() && !self.allowed_operators.contains(&p.operator) {
            return false;
        }
        true
    }
}

/// The pure **policy-gated pairing** admission decision (`DEPLOY-3`, [ADR 0059]/[ADR 0061]):
/// the client's boundary `accept` admits the consultant's declared deployment mode **iff** it
/// satisfies the org placement policy **∧** (when attested) the measurement verified against
/// the allow-list. Fail-closed (`INV-20`) — any failing predicate denies. `measurement_verified`
/// is the verdict the attestation verifier produced for an attested placement (irrelevant when
/// the declared placement is unattested).
pub fn pairing_admitted(
    policy: &PlacementPolicy,
    declared: &Placement,
    measurement_verified: bool,
) -> bool {
    policy.admits(declared) && (!declared.attested || measurement_verified)
}

impl crate::Lifecycle for BoundaryState {
    type State = BoundaryState;
    type Command = BoundaryCommand;
    type Event = BoundaryEvent;
    const KIND: &'static str = "boundary_lifecycle";
    fn decide(
        state: &BoundaryState,
        command: BoundaryCommand,
    ) -> Result<Vec<BoundaryEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &BoundaryState, event: BoundaryEvent) -> BoundaryState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attestation::{
        AttestationEvidence, AttestationQuote, CodeMeasurement, QuoteVerificationResult,
    };
    use proptest::prelude::*;

    fn apply(state: &BoundaryState, command: BoundaryCommand) -> BoundaryState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }

    fn parts() -> BTreeSet<String> {
        BTreeSet::from(["A".to_string(), "B".to_string()])
    }

    /// An unattested acceptance (no evidence) — the M0/M1 default shape.
    fn accept(a: &str) -> BoundaryCommand {
        BoundaryCommand::Accept {
            participant: a.into(),
            evidence: None,
        }
    }

    fn evidence() -> AttestationEvidence {
        let measurement = CodeMeasurement::new("a".repeat(64));
        AttestationEvidence::new(
            AttestationQuote::new(measurement.clone(), "nonce-1", vec![1, 2, 3, 4]),
            QuoteVerificationResult::Verified { measurement },
        )
    }

    /// An attested acceptance carrying attestation evidence.
    fn accept_attested(a: &str) -> BoundaryCommand {
        BoundaryCommand::Accept {
            participant: a.into(),
            evidence: Some(evidence()),
        }
    }

    #[test]
    fn activate_admit_start_then_teardown_preserves_evidence() {
        use BoundaryCommand::*;
        let s = BoundaryState::default();
        let s = apply(&s, Propose(parts()));
        let s = apply(
            &s,
            DeclareCeiling(Placement {
                operator: Operator::Counterparty,
                attested: false,
            }),
        );
        let s = apply(&s, accept("A"));
        assert!(!s.active(), "not active until both accept");
        let s = apply(&s, accept("B"));
        assert!(s.active());
        // ceiling honestly looser at an unattested counterparty host: method is not secret.
        assert!(!s.placement.unwrap().method_secret());
        let s = apply(&s, AdmitResource);
        let s = apply(&s, StartSession);
        assert!(s.resources_admitted && s.session_started);
        let s = apply(&s, BeginTeardown);
        assert!(s.torn_down && !s.session_started);
        // EVIDENCE_PRESERVED_ON_TEARDOWN.
        assert!(s.evidence);
    }

    #[test]
    fn ghost_cannot_accept_and_inactive_cannot_admit() {
        use BoundaryCommand::*;
        let s = BoundaryState::default();
        let s = apply(&s, Propose(parts()));
        let s = apply(&s, DeclareCeiling(Placement::local()));
        // a non-participant acceptance is rejected
        let s = apply(&s, accept("ghost"));
        assert!(!s.accepted.contains("ghost"));
        // admit before active is rejected
        let s = apply(&s, AdmitResource);
        assert!(!s.resources_admitted);
    }

    #[test]
    fn method_secret_depends_only_on_attestation() {
        use Operator::*;
        // Ceiling is f(attested), independent of who operates the host (ADR 0040).
        assert!(!Placement {
            operator: Local,
            attested: false
        }
        .method_secret());
        assert!(!Placement {
            operator: Counterparty,
            attested: false
        }
        .method_secret());
        assert!(!Placement {
            operator: Neutral,
            attested: false
        }
        .method_secret());
        // counterparty-hosted + attested = method hidden from the host: the strong
        // commercial case the old flat enum could not even express.
        assert!(Placement {
            operator: Counterparty,
            attested: true
        }
        .method_secret());
        assert!(Placement {
            operator: Neutral,
            attested: true
        }
        .method_secret());
    }

    /// MOB-001: a paired device binds to a ceiling-declared boundary, the typed
    /// `(DeviceId, BridgeGrantId)` is recorded, acceptance still drives the
    /// boundary to active, and a second bind is rejected (no silent device swap).
    #[test]
    fn device_binds_declared_boundary_and_accept_still_activates() {
        use BoundaryCommand::*;
        let device = DeviceId::new("device:pixel-9");
        let grant = BridgeGrantId::new("grant-7");
        let s = BoundaryState::default();
        // A bind before the ceiling is declared is rejected.
        let early = apply(
            &s,
            BindDevice {
                device: device.clone(),
                bridge_grant: grant.clone(),
            },
        );
        assert!(
            early.device_binding.is_none(),
            "no bind before a ceiling is declared"
        );

        let s = apply(&s, Propose(parts()));
        let s = apply(&s, DeclareCeiling(Placement::local()));
        let s = apply(
            &s,
            BindDevice {
                device: device.clone(),
                bridge_grant: grant.clone(),
            },
        );
        assert_eq!(s.phase, BoundaryPhase::DeviceBinding);
        assert_eq!(s.device_binding, Some((device.clone(), grant.clone())));

        // A second bind is rejected — the bound device key cannot be swapped.
        let swapped = apply(
            &s,
            BindDevice {
                device: DeviceId::new("device:other"),
                bridge_grant: BridgeGrantId::new("grant-evil"),
            },
        );
        assert_eq!(
            swapped.device_binding,
            Some((device, grant)),
            "no silent device swap"
        );

        // Acceptance from DeviceBinding still drives the boundary to active.
        let s = apply(&s, accept("A"));
        assert!(!s.active());
        let s = apply(&s, accept("B"));
        assert!(s.active());
    }

    /// ATTEST-2: on an attested placement an acceptance must present evidence and
    /// the evidence is recorded against the participant; an unattested placement
    /// rejects any acceptance that carries evidence.
    #[test]
    fn attested_acceptance_carries_and_records_evidence() {
        use BoundaryCommand::*;
        let s = BoundaryState::default();
        let s = apply(&s, Propose(parts()));
        let s = apply(
            &s,
            DeclareCeiling(Placement {
                operator: Operator::Counterparty,
                attested: true,
            }),
        );
        // attested ceiling: an acceptance without evidence is rejected.
        let bare = apply(&s, accept("A"));
        assert!(
            !bare.accepted.contains("A"),
            "attested accept needs evidence"
        );
        // with evidence it is admitted and the evidence is recorded.
        let s = apply(&s, accept_attested("A"));
        assert!(s.accepted.contains("A"));
        assert_eq!(s.attestation_evidence.get("A"), Some(&evidence()));
        let s = apply(&s, accept_attested("B"));
        assert!(s.active());
        // host-blind ceiling: method is secret under the attested placement.
        assert!(s.placement.unwrap().method_secret());
    }

    /// The dual tooth: an unattested placement refuses an acceptance that smuggles
    /// in evidence (no phantom attestation), and records no evidence on a plain one.
    #[test]
    fn unattested_acceptance_refuses_evidence() {
        use BoundaryCommand::*;
        let s = BoundaryState::default();
        let s = apply(&s, Propose(parts()));
        let s = apply(&s, DeclareCeiling(Placement::local()));
        let smuggled = apply(&s, accept_attested("A"));
        assert!(
            !smuggled.accepted.contains("A"),
            "unattested accept rejects evidence"
        );
        let s = apply(&s, accept("A"));
        assert!(s.accepted.contains("A"));
        assert!(
            s.attestation_evidence.is_empty(),
            "unattested records no evidence"
        );
    }

    fn arb_command() -> impl Strategy<Value = BoundaryCommand> {
        use BoundaryCommand::*;
        let auth = prop_oneof![
            Just("A".to_string()),
            Just("B".to_string()),
            Just("ghost".to_string())
        ];
        let operator = prop_oneof![
            Just(Operator::Local),
            Just(Operator::Counterparty),
            Just(Operator::Neutral),
        ];
        let placement = (operator, any::<bool>())
            .prop_map(|(operator, attested)| Placement { operator, attested });
        // Acceptances arise both with and without evidence, so the reducer's
        // attested/unattested gate is exercised in both directions.
        let accept_cmd = (auth.clone(), any::<bool>()).prop_map(|(participant, with_ev)| Accept {
            participant,
            evidence: with_ev.then(evidence),
        });
        // Device bindings arise with a couple of distinct device/grant pairs so
        // the "no silent swap" rule is exercised against a competing bind.
        let bind_cmd = prop_oneof![
            Just(BindDevice {
                device: DeviceId::new("device:a"),
                bridge_grant: BridgeGrantId::new("grant-a"),
            }),
            Just(BindDevice {
                device: DeviceId::new("device:b"),
                bridge_grant: BridgeGrantId::new("grant-b"),
            }),
        ];
        prop_oneof![
            Just(Propose(parts())),
            placement.prop_map(DeclareCeiling),
            accept_cmd,
            bind_cmd,
            auth.prop_map(Reject),
            Just(AdmitResource),
            Just(StartSession),
            Just(EndSession),
            Just(BeginTeardown),
            Just(FinishTeardown),
        ]
    }

    proptest! {
        /// ATTEST-12: the attested/unattested evidence biconditional holds over
        /// every reachable trace. Under an attested placement every accepted
        /// participant carries recorded evidence (`attested ⇒ evidence`); under an
        /// unattested one no participant ever does (`unattested ⇒ none`). Recorded
        /// evidence and acceptance are in exact lockstep with the declared ceiling —
        /// no phantom attestation, no evidence-less attested accept.
        #[test]
        fn attested_acceptance_requires_evidence(
            commands in prop::collection::vec(arb_command(), 0..60),
        ) {
            let mut s = BoundaryState::default();
            for c in commands {
                s = apply(&s, c);
                let attested = s.placement.map(|p| p.attested).unwrap_or(false);
                if attested {
                    // attested ⇒ evidence: every accepted participant is recorded
                    // with evidence (the ceiling cannot change once declared, so a
                    // participant accepted here was admitted under attestation).
                    prop_assert_eq!(s.accepted.len(), s.attestation_evidence.len());
                    for participant in &s.accepted {
                        prop_assert!(s.attestation_evidence.contains_key(participant));
                    }
                } else {
                    // unattested ⇒ none: no evidence is ever recorded.
                    prop_assert!(s.attestation_evidence.is_empty());
                }
                // Recorded evidence is never phantom: every keyed participant both
                // accepted and did so under an attested ceiling.
                for participant in s.attestation_evidence.keys() {
                    prop_assert!(attested);
                    prop_assert!(s.accepted.contains(participant));
                }
            }
        }

        /// The boundary-lifecycle invariants hold over every reachable trace.
        #[test]
        fn boundary_invariants(commands in prop::collection::vec(arb_command(), 0..60)) {
            let mut s = BoundaryState::default();
            let mut ever_evidence = false;
            let mut bound: Option<(crate::ids::DeviceId, crate::ids::BridgeGrantId)> = None;
            for c in commands {
                s = apply(&s, c);
                ever_evidence |= s.evidence;
                // DEVICE_BINDING_STABLE (MOB-001): once a device is bound it is
                // never silently swapped — the recorded pair only ever appears,
                // and equals the first one bound, for the rest of the trace.
                match (&bound, &s.device_binding) {
                    (None, Some(b)) => bound = Some(b.clone()),
                    (Some(first), Some(now)) => prop_assert_eq!(first, now),
                    (Some(_), None) => prop_assert!(false, "a bound device cannot un-bind"),
                    (None, None) => {}
                }
                // A binding implies a ceiling was declared (evidence exists).
                if s.device_binding.is_some() {
                    prop_assert!(s.evidence);
                }
                // NO_GHOST_ACCEPT: only required participants are ever accepted.
                prop_assert!(s.accepted.is_subset(&s.required));
                // RESOURCE_REQUIRES_ACTIVE_BOUNDARY: admitted ⟹ ceiling + all accepted.
                if s.resources_admitted {
                    prop_assert!(s.placement.is_some());
                    prop_assert!(s.required.is_subset(&s.accepted));
                }
                // SESSION_REQUIRES_ACTIVE_BOUNDARY: a live session ⟹ active-grade state.
                if s.session_started {
                    prop_assert!(s.placement.is_some());
                    prop_assert!(s.required.is_subset(&s.accepted));
                }
                // EVIDENCE_PRESERVED_ON_TEARDOWN.
                if ever_evidence {
                    prop_assert!(s.evidence);
                }
                // ATTESTED_ACCEPT_REQUIRES_EVIDENCE (ATTEST-2): recorded attestation
                // evidence only ever exists under an attested placement, and every
                // key is a genuine acceptance — no evidence without an accept.
                if !s.attestation_evidence.is_empty() {
                    prop_assert!(s.placement.map(|p| p.attested).unwrap_or(false));
                }
                for participant in s.attestation_evidence.keys() {
                    prop_assert!(s.accepted.contains(participant));
                }
            }
        }
    }
}

#[cfg(test)]
mod placement_policy_tests {
    use super::*;
    use proptest::prelude::*;

    fn p(operator: Operator, attested: bool) -> Placement {
        Placement { operator, attested }
    }

    #[test]
    fn open_policy_admits_everything() {
        let pol = PlacementPolicy::open();
        for op in [Operator::Local, Operator::Counterparty, Operator::Neutral] {
            assert!(pol.admits(&p(op, false)));
            assert!(pol.admits(&p(op, true)));
        }
    }

    #[test]
    fn require_attested_refuses_unattested() {
        let pol = PlacementPolicy {
            require_attested: true,
            ..Default::default()
        };
        assert!(!pol.admits(&p(Operator::Counterparty, false)));
        assert!(pol.admits(&p(Operator::Counterparty, true)));
    }

    #[test]
    fn allowed_operators_narrows_to_the_set() {
        let pol = PlacementPolicy {
            allowed_operators: [Operator::Local].into_iter().collect(),
            ..Default::default()
        };
        assert!(pol.admits(&p(Operator::Local, false)));
        assert!(!pol.admits(&p(Operator::Counterparty, false)));
        assert!(!pol.admits(&p(Operator::Neutral, true)));
    }

    #[test]
    fn pairing_is_fail_closed_on_policy_and_measurement() {
        let strict = PlacementPolicy {
            require_attested: true,
            ..Default::default()
        };
        let attested = p(Operator::Counterparty, true);
        let plain = p(Operator::Counterparty, false);
        // attested + verified + policy-satisfied → admitted.
        assert!(pairing_admitted(&strict, &attested, true));
        // attested but measurement NOT verified → denied (the verify-before-trust axis).
        assert!(!pairing_admitted(&strict, &attested, false));
        // plain counterparty under an attested-required policy → denied (the policy axis),
        // and measurement is irrelevant for an unattested declaration.
        assert!(!pairing_admitted(&strict, &plain, true));
        // open policy + unattested → admitted regardless of measurement.
        assert!(pairing_admitted(&PlacementPolicy::open(), &plain, false));
    }

    proptest! {
        /// Restrict-only (`ABAC_MONOTONE`): tightening either axis never admits *more*.
        /// Adding `require_attested` to an open policy, or shrinking the allowed-operator set,
        /// can only remove admitted placements.
        #[test]
        fn tightening_never_widens(attested in any::<bool>(), op_idx in 0usize..3) {
            let op = [Operator::Local, Operator::Counterparty, Operator::Neutral][op_idx];
            let pl = p(op, attested);
            let loose = PlacementPolicy::open();
            let tight_attest = PlacementPolicy { require_attested: true, ..Default::default() };
            let tight_op = PlacementPolicy {
                allowed_operators: [Operator::Local].into_iter().collect(),
                ..Default::default()
            };
            // each tighter policy admits a subset of what the open one admits.
            prop_assert!(!tight_attest.admits(&pl) || loose.admits(&pl));
            prop_assert!(!tight_op.admits(&pl) || loose.admits(&pl));
        }
    }
}

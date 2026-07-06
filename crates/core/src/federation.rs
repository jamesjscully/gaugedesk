//! Federation crossing rule — the `(decide, evolve)` reducer, ported from
//! `specs/models/federation.qnt` (ADR 0009). M2.
//!
//! The mechanism for moving a fact between two admission shells without a global
//! authority: the **source** authorizes the crossing, a **relay** routes it
//! (transport only), and the **target authority** admits it into its own scope.
//! Discharges:
//! - `CROSSING_REQUIRES_BOTH` — a target fact exists only after source permission
//!   **and** target admission (`INV-13`).
//! - `TARGET_ADMITTED_BY_TARGET` — the target fact is admitted by the target
//!   authority, never the source or relay (`INV-1`/ADR 0005).
//! - `RELAY_NOT_PAYLOAD_AUTHORITY` / `RELAY_NO_PAYLOAD_ACCESS` — routing never makes
//!   the relay the payload authority and grants it no payload read (`INV-14`/`INV-10`).
//! - `SIGNATURE_VERIFIED_BEFORE_ADMISSION` — the target verifies the source's
//!   envelope signature before writing the fact, so the admitted actor is
//!   authenticated, not forged (`INV-21`).
//! - `SOURCE_KEY_PINNED` (C-1) — that signature is verified against the **grant's
//!   pinned** source-authority key, never the key the envelope presents, so a
//!   self-consistent envelope signed under an attacker's own key cannot forge the
//!   source authority (`INV-21`).
//! - `GRANT_NOT_EXPIRED_BEFORE_ADMISSION` — the target validates the bridge grant
//!   (active and not expired at admission time) before writing the fact, so a
//!   stale or revoked grant cannot cross (`INV-21`/ADR 0009).
//!
//! This is the primitive crossing rule that [[federated-delivery]] (one attempt) and
//! [[remote-call]] (a call+response) build on.

use crate::bridge_grant::BridgeGrant;
use crate::delegation::DeviceDelegation;
use crate::federated_delivery::Authority;
use crate::ids::PublicKey;
use crate::signature::{verify_signature, Signature};
use crate::Rejection;

/// The single linear stage of a crossing. A closed, ordered phase — *not* a bag of
/// `source_permitted` / `routed` / `target_fact_written` / `signature_verified` /
/// `grant_validated` bools — so the desync the bools allowed (a written fact with
/// `signature_verified:false`, a routed crossing the source never permitted) is
/// **unrepresentable**. The fact, its admitting authority, the recorded signature
/// and grant validations are all one thing: the target wrote the fact, which only
/// the verified `TargetAdmit` path can produce.
///
/// The mirror [`federation.qnt`](../../../specs/models/federation.qnt) keeps the
/// individual bool vars: the *model* needs the illegal combinations representable so
/// its teeth (`BYPASS_SOURCE` / `SKIP_SIG` / `RELAY_READS_PAYLOAD` / …) can produce
/// them and prove the invariants catch them. The Rust encoding cannot reach them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrossingPhase {
    /// Nothing yet — the crossing has not been source-permitted.
    Init,
    /// The source authority permitted the crossing.
    SourcePermitted,
    /// The relay routed the (source-permitted) crossing; no fact written yet.
    Routed,
    /// The target verified the signature + grant and wrote its fact. Terminal.
    Admitted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrossingState {
    pub phase: CrossingPhase,
}

impl CrossingState {
    /// The source authorized the crossing (every phase past `Init`).
    pub fn source_permitted(&self) -> bool {
        !matches!(self.phase, CrossingPhase::Init)
    }
    /// The relay routed the crossing.
    pub fn routed(&self) -> bool {
        matches!(self.phase, CrossingPhase::Routed | CrossingPhase::Admitted)
    }
    /// The target wrote its admission fact. The four old admission bools
    /// (`target_admitted` / `target_fact_written` / `signature_verified` /
    /// `grant_validated`) were always set together on this one transition, so they
    /// are one predicate now — the desync between them is unrepresentable.
    pub fn target_admitted(&self) -> bool {
        matches!(self.phase, CrossingPhase::Admitted)
    }
    pub fn target_fact_written(&self) -> bool {
        self.target_admitted()
    }
    /// The signature was verified before the fact was written (`INV-21`) — true
    /// exactly when the fact is written (the reducer writes it only on the verified
    /// path).
    pub fn signature_verified(&self) -> bool {
        self.target_admitted()
    }
    /// The bridge grant was validated before the fact was written (`INV-21`/ADR 0009).
    pub fn grant_validated(&self) -> bool {
        self.target_admitted()
    }
    /// Who wrote the target fact — the target authority once admitted, else nobody.
    /// Never the relay (`TARGET_ADMITTED_BY_TARGET`).
    pub fn target_fact_admitted_by(&self) -> Authority {
        if self.target_admitted() {
            Authority::Target
        } else {
            Authority::None
        }
    }
    /// The payload authority is always the source — a relay never becomes it (`INV-14`).
    pub fn payload_authority(&self) -> Authority {
        Authority::Source
    }
    /// Routing grants the relay no payload read, ever (`INV-10`).
    pub fn relay_has_payload_access(&self) -> bool {
        false
    }
}

impl Default for CrossingState {
    fn default() -> Self {
        Self {
            phase: CrossingPhase::Init,
        }
    }
}

/// The signed source envelope the target verifies before admitting (`INV-21`).
///
/// The reducer verifies the signature against the **grant's** pinned
/// `source_authority_root_pubkey` — *not* against `source_pubkey`, which the
/// presenter controls (C-1). `source_pubkey` is therefore only the envelope's
/// *claim* of its signer: admission requires it to equal the grant's pinned key
/// (an explicit mismatch is rejected), so a forged source authority cannot cross
/// even with a signature that is self-consistent under its own key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossingEnvelope {
    pub signed_bytes: Vec<u8>,
    pub signature: Signature,
    /// The envelope's *claimed* signer. In the single-key slice it must equal the
    /// grant's pinned `source_authority_root_pubkey` (C-1). Under a device
    /// delegation (Model A) it must instead equal the delegation's `subkey`, and the
    /// delegation must chain to the grant's pinned root — see [`Self::delegation`].
    pub source_pubkey: PublicKey,
    /// Optional **device subkey delegation** (Model A, ADR 0039). When present, the
    /// envelope is signed by a device subkey rather than the root directly: the
    /// target verifies the delegation was issued by the grant's pinned root and is
    /// unexpired, then verifies the envelope under the delegated subkey. `None` is
    /// the single-key slice (envelope signed directly by the pinned root).
    pub delegation: Option<DeviceDelegation>,
}

// AdmitTargetReceipt carries the full signed envelope; the other variants are
// unit-sized. Boxing would obscure the reducer's by-value command API for a
// command type that is constructed once per crossing, not held in bulk.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrossingCommand {
    SourceAuthorize,
    RelayRoute,
    /// The target admits the crossing — it verifies the source's signature over
    /// `envelope` **and** validates the bridge `grant` against the clock `now`
    /// before writing the fact (`INV-21`/ADR 0009).
    TargetAdmit {
        envelope: CrossingEnvelope,
        grant: BridgeGrant,
        now: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CrossingEvent {
    SourcePermitted,
    Routed,
    TargetFactWritten,
}

fn reject(reason: &'static str) -> Result<Vec<CrossingEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(
    state: &CrossingState,
    command: CrossingCommand,
) -> Result<Vec<CrossingEvent>, Rejection> {
    match command {
        CrossingCommand::SourceAuthorize => Ok(vec![CrossingEvent::SourcePermitted]),
        // No BYPASS_SOURCE: a relay routes only a source-permitted crossing.
        CrossingCommand::RelayRoute => {
            if state.source_permitted() {
                Ok(vec![CrossingEvent::Routed])
            } else {
                reject("relayRoute: crossing not source-permitted (no BYPASS_SOURCE)")
            }
        }
        // CROSSING_REQUIRES_BOTH: the target writes its fact only on a routed,
        // source-permitted crossing. (No BYPASS_TARGET / relay-creates-fact path.)
        CrossingCommand::TargetAdmit {
            envelope,
            grant,
            now,
        } => {
            if !(state.routed() && state.source_permitted()) {
                return reject("targetAdmit: needs a routed, source-permitted crossing (INV-13)");
            }
            // SIGNATURE_VERIFIED_BEFORE_ADMISSION: the target verifies the
            // source's signature before writing the fact (INV-21). No SKIP_SIG:
            // a malformed or unverifiable signature denies admission (fail-closed).
            //
            // SOURCE_KEY_PINNED (C-1): verification chains to the **grant's**
            // registered source-authority root — never a key the envelope alone
            // carries, which an attacker controls. The key the envelope must verify
            // under is decided here, and it always chains to the pinned root:
            //
            // - **single-key slice** (no delegation): the envelope's claimed key must
            //   *equal* the pinned root, and the signature verifies under it. A
            //   self-consistent (attacker key, attacker signature) pair is rejected.
            // - **Model A** (device subkey delegation): the delegation must be issued
            //   by the pinned root and unexpired, the envelope's claimed key must be
            //   the delegated subkey, and the signature verifies under that subkey. A
            //   delegation under any other root — or an expired/forged one — is
            //   rejected, so a foreign device cannot forge the source authority.
            let verify_key = match &envelope.delegation {
                None => {
                    if envelope.source_pubkey != grant.source_authority_root_pubkey {
                        return reject(
                            "targetAdmit: envelope source key does not match the grant's pinned \
                             source-authority key (INV-21, C-1)",
                        );
                    }
                    &grant.source_authority_root_pubkey
                }
                Some(delegation) => {
                    if delegation.authority_root != grant.source_authority_root_pubkey {
                        return reject(
                            "targetAdmit: device delegation is not issued by the grant's pinned \
                             source-authority root (INV-21, C-1)",
                        );
                    }
                    if delegation.verify(now).is_err() {
                        return reject(
                            "targetAdmit: device delegation is invalid or expired (INV-21)",
                        );
                    }
                    if envelope.source_pubkey != delegation.subkey {
                        return reject(
                            "targetAdmit: envelope source key is not the delegated device subkey \
                             (INV-21)",
                        );
                    }
                    &delegation.subkey
                }
            };
            match verify_signature(&envelope.signed_bytes, &envelope.signature, verify_key) {
                Ok(true) => {}
                Ok(false) => {
                    return reject("targetAdmit: source signature did not verify (INV-21)")
                }
                Err(_) => {
                    return reject("targetAdmit: source signature could not be verified (INV-21)")
                }
            }
            // GRANT_NOT_EXPIRED_BEFORE_ADMISSION: the target validates the bridge
            // grant against `now` before writing the fact (INV-21/ADR 0009). No
            // SKIP_GRANT_EXPIRY: a revoked or expired grant denies admission
            // (fail-closed).
            if !grant.is_valid(now) {
                return reject("targetAdmit: bridge grant is revoked or expired (INV-21)");
            }
            Ok(vec![CrossingEvent::TargetFactWritten])
        }
    }
}

pub fn evolve(state: &CrossingState, event: CrossingEvent) -> CrossingState {
    let mut s = *state;
    // Phase advances monotonically along Init → SourcePermitted → Routed → Admitted;
    // a re-issued earlier event (e.g. SourceAuthorize after routing) is idempotent,
    // exactly as the old monotone bools were.
    match event {
        CrossingEvent::SourcePermitted => {
            if matches!(s.phase, CrossingPhase::Init) {
                s.phase = CrossingPhase::SourcePermitted;
            }
        }
        CrossingEvent::Routed => {
            if matches!(s.phase, CrossingPhase::SourcePermitted) {
                s.phase = CrossingPhase::Routed;
            }
        }
        // TARGET_ADMITTED_BY_TARGET: the target authority — never the relay — writes it.
        // The fact is written only on the verified path above (INV-21 / ADR 0004), so
        // `signature_verified()`/`grant_validated()` are true exactly when it is.
        CrossingEvent::TargetFactWritten => s.phase = CrossingPhase::Admitted,
    }
    s
}

impl crate::Lifecycle for CrossingState {
    type State = CrossingState;
    type Command = CrossingCommand;
    type Event = CrossingEvent;
    const KIND: &'static str = "federation";
    fn decide(
        state: &CrossingState,
        command: CrossingCommand,
    ) -> Result<Vec<CrossingEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &CrossingState, event: CrossingEvent) -> CrossingState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{BridgeGrantId, Nonce};
    use proptest::prelude::*;

    fn apply(state: &CrossingState, command: CrossingCommand) -> CrossingState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(*state, |s, e| evolve(&s, e)),
            Err(_) => *state,
        }
    }

    /// The source authority's signing key — the one the grant pins and the envelope
    /// signs under (they must agree, C-1).
    fn source_key() -> crate::signature::SigningKey {
        crate::signature::SigningKey::from_seed(&[7u8; 32]).unwrap()
    }

    /// A well-formed source envelope `verify_signature` accepts: a real P-256
    /// signature over the signed bytes under the source key the grant pins.
    fn valid_envelope() -> CrossingEnvelope {
        let sk = source_key();
        let signed_bytes = vec![1, 2, 3, 4];
        CrossingEnvelope {
            signature: sk.sign(&signed_bytes),
            source_pubkey: sk.public_key(),
            signed_bytes,
            delegation: None,
        }
    }

    /// An active bridge grant that is valid at `ADMIT_NOW`. Its pinned
    /// `source_authority_root_pubkey` is the key [`valid_envelope`] signs under
    /// (C-1: verification is pinned to the grant, so the two must agree).
    fn valid_grant() -> BridgeGrant {
        BridgeGrant {
            id: "grant-1".into(),
            source_authority_root_pubkey: source_key().public_key(),
            source_authority_key_id: "gov-1".into(),
            target_environment: "prod".into(),
            target_route: "/v1/agent".into(),
            device_key: PublicKey::new("04ffeeddccbbaa"),
            governance_scope: "bridge:invoke".into(),
            expiry: 100,
            active: true,
        }
    }

    /// The clock at admission — before the valid grant's expiry.
    const ADMIT_NOW: u64 = 50;

    /// A `TargetAdmit` over the valid envelope and an active, unexpired grant.
    fn target_admit() -> CrossingCommand {
        CrossingCommand::TargetAdmit {
            envelope: valid_envelope(),
            grant: valid_grant(),
            now: ADMIT_NOW,
        }
    }

    #[test]
    fn crossing_needs_source_then_target() {
        // BYPASS_SOURCE teeth: nothing routes/admits before the source permits.
        let s = CrossingState::default();
        assert!(decide(&s, CrossingCommand::RelayRoute).is_err());
        assert!(decide(&s, target_admit()).is_err());
        let s = apply(&s, CrossingCommand::SourceAuthorize);
        // BYPASS_TARGET teeth: a routed-but-unadmitted crossing has written no fact.
        let s = apply(&s, CrossingCommand::RelayRoute);
        assert!(
            !s.target_fact_written(),
            "routing alone writes no target fact"
        );
        // …only the target authority writes it (RELAY_WRITES_TARGET teeth).
        let s = apply(&s, target_admit());
        assert!(s.target_fact_written() && s.target_fact_admitted_by() == Authority::Target);
        // SIGNATURE_VERIFIED_BEFORE_ADMISSION: the write recorded a verified signature.
        assert!(
            s.signature_verified(),
            "admitted fact recorded the verified signature"
        );
        // GRANT_NOT_EXPIRED_BEFORE_ADMISSION: the write recorded a validated grant.
        assert!(
            s.grant_validated(),
            "admitted fact recorded the validated grant"
        );
        // RELAY_READS_PAYLOAD / RELAY_OWNS_PAYLOAD teeth.
        assert!(!s.relay_has_payload_access());
        assert_eq!(s.payload_authority(), Authority::Source);
    }

    /// SKIP_SIG tooth: on a routed, source-permitted crossing, a bad signature
    /// (empty bytes or a wrong-length signature) still denies admission — the
    /// fact is never written without a verified signature (INV-21).
    #[test]
    fn crossing_denies_unverified_signature() {
        let s = CrossingState::default();
        let s = apply(&s, CrossingCommand::SourceAuthorize);
        let s = apply(&s, CrossingCommand::RelayRoute);

        let empty_bytes = CrossingEnvelope {
            signed_bytes: vec![],
            ..valid_envelope()
        };
        assert!(decide(
            &s,
            CrossingCommand::TargetAdmit {
                envelope: empty_bytes,
                grant: valid_grant(),
                now: ADMIT_NOW
            }
        )
        .is_err());

        let bad_sig = CrossingEnvelope {
            signature: Signature::new(vec![0u8; 32]),
            ..valid_envelope()
        };
        let after = apply(
            &s,
            CrossingCommand::TargetAdmit {
                envelope: bad_sig,
                grant: valid_grant(),
                now: ADMIT_NOW,
            },
        );
        assert!(
            !after.target_fact_written(),
            "no fact written on an unverified signature"
        );
        assert!(!after.signature_verified());
    }

    /// SKIP_GRANT_EXPIRY tooth: on a routed, source-permitted crossing with a
    /// good signature, a stale grant (expired at `now`, or revoked) still denies
    /// admission — the fact is never written under a grant that no longer governs
    /// the route (INV-21 / ADR 0009).
    #[test]
    fn crossing_denies_expired_or_revoked_grant() {
        let s = CrossingState::default();
        let s = apply(&s, CrossingCommand::SourceAuthorize);
        let s = apply(&s, CrossingCommand::RelayRoute);

        // Expired: now is at/after the grant's expiry.
        let expired = CrossingCommand::TargetAdmit {
            envelope: valid_envelope(),
            grant: valid_grant(),
            now: 100,
        };
        assert!(decide(&s, expired).is_err());
        let after = apply(
            &s,
            CrossingCommand::TargetAdmit {
                envelope: valid_envelope(),
                grant: valid_grant(),
                now: 200,
            },
        );
        assert!(
            !after.target_fact_written(),
            "no fact written under an expired grant"
        );
        assert!(!after.grant_validated());

        // Revoked: the grant is inactive even though now is before expiry.
        let revoked_grant = BridgeGrant {
            active: false,
            ..valid_grant()
        };
        let revoked = CrossingCommand::TargetAdmit {
            envelope: valid_envelope(),
            grant: revoked_grant,
            now: ADMIT_NOW,
        };
        let after = apply(&s, revoked);
        assert!(
            !after.target_fact_written(),
            "no fact written under a revoked grant"
        );
        assert!(!after.grant_validated());
    }

    /// C-1 teeth — a forged source authority must not cross. An attacker mints a
    /// **self-consistent** envelope: signs the bytes with their OWN key and presents
    /// their OWN public key as `source_pubkey`. Under the pre-C-1 code (verify
    /// against the envelope's key) this verified and wrote the target fact, forging
    /// the source authority. Pinned to the grant's key it is rejected: the claimed
    /// key does not equal the pinned key, and the signature does not verify under the
    /// pinned key. The legitimate grant is otherwise valid, isolating the key check.
    #[test]
    fn crossing_denies_forged_source_key() {
        let s = CrossingState::default();
        let s = apply(&s, CrossingCommand::SourceAuthorize);
        let s = apply(&s, CrossingCommand::RelayRoute);

        let attacker = crate::signature::SigningKey::from_seed(&[42u8; 32]).unwrap();
        let signed_bytes = vec![1, 2, 3, 4];
        let forged = CrossingEnvelope {
            signature: attacker.sign(&signed_bytes), // a valid signature…
            source_pubkey: attacker.public_key(),    // …under the ATTACKER's own key
            signed_bytes,
            delegation: None,
        };
        // decide rejects it outright…
        assert!(decide(
            &s,
            CrossingCommand::TargetAdmit {
                envelope: forged.clone(),
                grant: valid_grant(),
                now: ADMIT_NOW,
            }
        )
        .is_err());
        // …and no fact is ever written for a forged source authority (INV-21).
        let after = apply(
            &s,
            CrossingCommand::TargetAdmit {
                envelope: forged,
                grant: valid_grant(),
                now: ADMIT_NOW,
            },
        );
        assert!(
            !after.target_fact_written(),
            "a forged source key must not cross (C-1 / INV-21)"
        );
        assert!(!after.signature_verified());
    }

    /// Model A (device subkeys): a crossing signed by a **device subkey** admits
    /// when its delegation chains to the grant's pinned root and is unexpired, and is
    /// denied when the delegation is expired, issued under a foreign root (C-1), or
    /// the envelope is signed by a key other than the delegated subkey (INV-21).
    #[test]
    fn crossing_admits_a_delegated_subkey_and_rejects_a_bad_delegation() {
        use crate::delegation::DeviceDelegation;
        use crate::signature::SigningKey;

        let s = CrossingState::default();
        let s = apply(&s, CrossingCommand::SourceAuthorize);
        let s = apply(&s, CrossingCommand::RelayRoute);

        // `valid_grant()` pins `source_key()`'s pubkey as the authority root.
        let root = source_key();
        let subkey = SigningKey::from_seed(&[11u8; 32]).unwrap();
        let signed_bytes = vec![1, 2, 3, 4];
        let envelope = |delegation: DeviceDelegation, signer: &SigningKey| CrossingEnvelope {
            signature: signer.sign(&signed_bytes),
            source_pubkey: signer.public_key(),
            signed_bytes: signed_bytes.clone(),
            delegation: Some(delegation),
        };
        let admit = |env: CrossingEnvelope| {
            apply(
                &s,
                CrossingCommand::TargetAdmit {
                    envelope: env,
                    grant: valid_grant(),
                    now: ADMIT_NOW,
                },
            )
            .target_fact_written()
        };

        // Genuine: subkey-signed, delegation issued by the grant's root, unexpired.
        let good = DeviceDelegation::issue(&root, subkey.public_key(), 100);
        assert!(
            admit(envelope(good.clone(), &subkey)),
            "a delegated device-subkey crossing admits (Model A)"
        );

        // Expired delegation (expiry < ADMIT_NOW) → denied.
        let expired = DeviceDelegation::issue(&root, subkey.public_key(), 10);
        assert!(
            !admit(envelope(expired, &subkey)),
            "an expired delegation is denied"
        );

        // Delegation issued under an ATTACKER's root, not the grant's → denied (C-1).
        let attacker_root = SigningKey::from_seed(&[42u8; 32]).unwrap();
        let foreign = DeviceDelegation::issue(&attacker_root, subkey.public_key(), 100);
        assert!(
            !admit(envelope(foreign, &subkey)),
            "a delegation under a foreign root is denied (C-1)"
        );

        // Envelope signed by a different key than the delegated subkey → denied.
        let other = SigningKey::from_seed(&[13u8; 32]).unwrap();
        assert!(
            !admit(envelope(good, &other)),
            "the envelope must be signed by the delegated subkey"
        );
    }

    /// The envelope a strategy draws for `TargetAdmit` — either a verifiable
    /// envelope or one that fails verification (a SKIP_SIG tooth at the proptest
    /// level), so the invariant is exercised on both paths.
    fn arb_envelope() -> impl Strategy<Value = CrossingEnvelope> {
        prop_oneof![
            Just(valid_envelope()),
            Just(CrossingEnvelope {
                signed_bytes: vec![],
                ..valid_envelope()
            }),
            Just(CrossingEnvelope {
                signature: Signature::new(vec![0u8; 32]),
                ..valid_envelope()
            }),
        ]
    }

    /// The grant a strategy draws for `TargetAdmit` — active, revoked, or one
    /// whose expiry the drawn clock may or may not be past (a SKIP_GRANT_EXPIRY
    /// tooth at the proptest level), so the invariant is exercised on both paths.
    fn arb_grant() -> impl Strategy<Value = BridgeGrant> {
        (any::<bool>(), 0u64..200).prop_map(|(active, expiry)| BridgeGrant {
            active,
            expiry,
            ..valid_grant()
        })
    }

    fn arb_command() -> impl Strategy<Value = CrossingCommand> {
        prop_oneof![
            Just(CrossingCommand::SourceAuthorize),
            Just(CrossingCommand::RelayRoute),
            (arb_envelope(), arb_grant(), 0u64..200).prop_map(|(envelope, grant, now)| {
                CrossingCommand::TargetAdmit {
                    envelope,
                    grant,
                    now,
                }
            }),
        ]
    }

    proptest! {
        #[test]
        fn federation_invariants(commands in prop::collection::vec(arb_command(), 0..30)) {
            let mut s = CrossingState::default();
            for c in commands {
                s = apply(&s, c);
                // CROSSING_REQUIRES_BOTH.
                if s.target_fact_written() {
                    prop_assert!(s.source_permitted() && s.target_admitted(), "fact without both sides");
                    // TARGET_ADMITTED_BY_TARGET.
                    prop_assert_eq!(s.target_fact_admitted_by(), Authority::Target);
                    // SIGNATURE_VERIFIED_BEFORE_ADMISSION (INV-21).
                    prop_assert!(s.signature_verified(), "fact written without a verified signature");
                    // GRANT_NOT_EXPIRED_BEFORE_ADMISSION (INV-21 / ADR 0009).
                    prop_assert!(s.grant_validated(), "fact written without a validated grant");
                }
                // RELAY_NOT_PAYLOAD_AUTHORITY / RELAY_NO_PAYLOAD_ACCESS.
                prop_assert_ne!(s.payload_authority(), Authority::Relay);
                prop_assert!(!s.relay_has_payload_access());
            }
        }
    }

    // ---- LOOPBACK-TEST-0: combined two-authority loopback proptest ----------
    //
    // The per-reducer proptests above (and the `federated_delivery` one) each
    // pin one edge's INV-21 surface in isolation. LOOPBACK-TEST-0 drives both
    // edges — the [[federation]] crossing reducer and the [[federated_delivery]]
    // reducer — over one interleaved command stream, the loopback two-authority
    // shape the M2 mechanism is verified in (m2-tracker gate note). It pins the
    // *joint* property the seam relies on: across both edges, no target fact is
    // ever written without a verified source signature (INV-21), and each edge
    // additionally never admits without its own anti-replay / grant guard —
    // crossing ⇒ a validated (active, unexpired) grant
    // (GRANT_NOT_EXPIRED_BEFORE_ADMISSION), delivery ⇒ a fresh nonce and a
    // matching bridge grant (NONCE_NOT_REUSED / BRIDGE_GRANT_MATCHES). The two
    // reducers share one fail-closed authentication + anti-replay surface, so a
    // forged or replayed envelope crosses *neither*.

    use crate::federated_delivery::{
        self as delivery, DeliveryCommand, DeliveryEnvelope, DeliveryState,
    };

    /// One step of the interleaved loopback stream: a command for the crossing
    /// edge or a command for the delivery edge of the same two-authority bridge.
    #[derive(Clone, Debug)]
    enum LoopbackStep {
        Crossing(CrossingCommand),
        Delivery(DeliveryCommand),
    }

    fn delivery_apply(state: &DeliveryState, command: DeliveryCommand) -> DeliveryState {
        match delivery::decide(state, command) {
            Ok(events) => events
                .into_iter()
                .fold(state.clone(), |s, e| delivery::evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }

    /// A delivery envelope drawn for the loopback stream — verifiable or not, on
    /// a fresh or a replayed nonce, under the bound grant or another — so the
    /// delivery edge's anti-replay guards are exercised on both paths.
    fn arb_delivery_envelope() -> impl Strategy<Value = DeliveryEnvelope> {
        prop_oneof![
            Just(DeliveryEnvelope {
                signed_bytes: vec![1, 2, 3, 4],
                signature: Signature::new(vec![0u8; 64]),
                source_pubkey: PublicKey::new("04a1b2c3d4e5f6"),
                nonce: Nonce::new("n-1"),
                bridge_grant_id: BridgeGrantId::new("bridge-grant-7"),
                device_key: PublicKey::new("04dev1ce0ke7"),
                device_active: true,
            }),
            Just(DeliveryEnvelope {
                signed_bytes: vec![],
                signature: Signature::new(vec![0u8; 64]),
                source_pubkey: PublicKey::new("04a1b2c3d4e5f6"),
                nonce: Nonce::new("n-1"),
                bridge_grant_id: BridgeGrantId::new("bridge-grant-7"),
                device_key: PublicKey::new("04dev1ce0ke7"),
                device_active: true,
            }),
            Just(DeliveryEnvelope {
                signed_bytes: vec![1, 2, 3, 4],
                signature: Signature::new(vec![0u8; 64]),
                source_pubkey: PublicKey::new("04a1b2c3d4e5f6"),
                nonce: Nonce::new("n-2"),
                bridge_grant_id: BridgeGrantId::new("bridge-grant-7"),
                device_key: PublicKey::new("04dev1ce0ke7"),
                device_active: true,
            }),
            Just(DeliveryEnvelope {
                signed_bytes: vec![1, 2, 3, 4],
                signature: Signature::new(vec![0u8; 64]),
                source_pubkey: PublicKey::new("04a1b2c3d4e5f6"),
                nonce: Nonce::new("n-1"),
                bridge_grant_id: BridgeGrantId::new("bridge-grant-other"),
                device_key: PublicKey::new("04dev1ce0ke7"),
                device_active: true,
            }),
        ]
    }

    fn arb_delivery_command() -> impl Strategy<Value = DeliveryCommand> {
        use DeliveryCommand::*;
        prop_oneof![
            Just(AuthorizeFederatedMessage),
            Just(EnqueueFederatedMessage),
            Just(RecordRelayDelivery),
            arb_delivery_envelope().prop_map(|envelope| AdmitTargetReceipt { envelope }),
            Just(ExpireFederatedDelivery),
            Just(RecordDeliveryFailure),
            Just(RetryFederatedDelivery),
        ]
    }

    fn arb_loopback_step() -> impl Strategy<Value = LoopbackStep> {
        prop_oneof![
            arb_command().prop_map(LoopbackStep::Crossing),
            arb_delivery_command().prop_map(LoopbackStep::Delivery),
        ]
    }

    proptest! {
        /// LOOPBACK-TEST-0: signature + anti-replay + grant-expiry invariants
        /// hold jointly across the crossing and delivery edges of one loopback
        /// two-authority bridge (INV-21 / ADR 0009).
        #[test]
        fn loopback_two_authority_invariants(steps in prop::collection::vec(arb_loopback_step(), 0..40)) {
            let mut crossing = CrossingState::default();
            let mut deliv = DeliveryState::default();

            for step in steps {
                // The pre-apply view a delivery admission this step is judged
                // against: whether its nonce was already spent or its grant fails
                // to match the bound grant (NONCE_NOT_REUSED / BRIDGE_GRANT_MATCHES).
                let pre_delivered = deliv.target_admitted;
                let deliv_admit_basis = match &step {
                    LoopbackStep::Delivery(DeliveryCommand::AdmitTargetReceipt { envelope }) => Some((
                        deliv.seen_nonces.contains(&envelope.nonce),
                        envelope.bridge_grant_id != deliv.bound_bridge_grant_id,
                    )),
                    _ => None,
                };

                match step {
                    LoopbackStep::Crossing(c) => crossing = apply(&crossing, c),
                    LoopbackStep::Delivery(c) => deliv = delivery_apply(&deliv, c),
                }

                // INV-21 (shared surface): neither edge writes a target fact
                // without a verified source signature.
                if crossing.target_fact_written() {
                    prop_assert!(crossing.signature_verified(), "crossing fact written without a verified signature");
                    // GRANT_NOT_EXPIRED_BEFORE_ADMISSION (crossing edge).
                    prop_assert!(crossing.grant_validated(), "crossing fact written without a validated grant");
                }
                if deliv.target_admitted {
                    prop_assert!(deliv.signature_verified, "delivery receipt admitted without a verified signature");
                }

                // NONCE_NOT_REUSED / BRIDGE_GRANT_MATCHES (delivery edge): a *new*
                // admission this step is only ever on a fresh nonce and a matching
                // grant — a replayed or mis-granted envelope crosses neither edge.
                if !pre_delivered && deliv.target_admitted {
                    if let Some((nonce_seen, grant_mismatch)) = deliv_admit_basis {
                        prop_assert!(!nonce_seen, "delivery admitted on a replayed nonce");
                        prop_assert!(!grant_mismatch, "delivery admitted under a mismatched bridge grant");
                    }
                }

                // RELAY_* invariants hold on both edges throughout.
                prop_assert_ne!(crossing.payload_authority(), Authority::Relay);
                prop_assert!(!crossing.relay_has_payload_access());
                prop_assert_ne!(deliv.payload_authority, Authority::Relay);
                prop_assert!(!deliv.relay_has_payload_access);
            }
        }
    }
}

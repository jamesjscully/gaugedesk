//! Federated delivery lifecycle — the `(decide, evolve)` reducer, ported from
//! `specs/models/federated-delivery.qnt` (ADR 0022). M2.
//!
//! One bridge-carried message delivery attempt: source authorizes → relay queues
//! and delivers (transport only) → the **target authority** admits. Discharges:
//! - `TARGET_ADMISSION_REQUIRES_SOURCE_AND_TARGET` — a target fact needs source
//!   authorization, relay delivery, **and** the target authority (`INV-13`/`INV-1`);
//!   relay delivery alone is not admission.
//! - `RELAY_NO_PAYLOAD_ACCESS` / `RELAY_NOT_PAYLOAD_AUTHORITY` — routing grants the
//!   relay no payload read and never makes it the payload authority (`INV-10`/`INV-14`).
//! - `RETRY_DOES_NOT_WIDEN` — a retry's handles stay within the original (`INV-17`).
//! - `CORRELATION_PRESERVED` — relay/retry cannot rewrite correlation (`INV-7`).
//! - `EXPIRY_BLOCKS_TARGET_ADMISSION` — an expired attempt cannot admit later.
//! - `SIGNATURE_VERIFIED_BEFORE_ADMISSION` — the target verifies the source's
//!   envelope signature before admitting the receipt, so the admitted actor is
//!   authenticated, not forged (`INV-21`); this is the delivery-level twin of
//!   the same check in the [[federation]] crossing reducer (`CROSSING-1`).
//! - `NONCE_NOT_REUSED` — the target admits a receipt only on a *fresh* envelope
//!   nonce; a replayed nonce is rejected, so the same signed envelope cannot
//!   write a second fact (`INV-21` anti-replay).
//! - `BRIDGE_GRANT_MATCHES` — the target admits only when the envelope's
//!   `bridge_grant_id` matches the grant this delivery is bound to, so an
//!   envelope minted under a different (or revoked-and-re-issued) grant cannot
//!   be replayed onto this delivery (`INV-21` / ADR 0009).
//! - `DEVICE_BINDING_MATCHES` — the target admits only when the envelope
//!   presents the device key this delivery is bound to **and** that device's
//!   bridge grant is still active; a foreign device key or a revoked device
//!   denies admission, so a revoked device cannot keep delivering (`INV-21` /
//!   ADR 0009, D-MOBILE / MOB-004). `ValidateDeviceBinding` is the standalone
//!   pre-check the imperative shell runs before it routes a delivery.

use std::collections::BTreeSet;

use crate::ids::{BridgeGrantId, DeviceId, Nonce, PublicKey};
use crate::signature::{verify_signature, Signature};
use crate::Rejection;

/// The authorities at a federation edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Authority {
    None,
    Source,
    Target,
    Relay,
}

fn original_handles() -> BTreeSet<String> {
    ["method", "context"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// The bridge grant a default delivery is bound to. The imperative shell binds a
/// real delivery to the grant it authorized; a loopback/default delivery uses
/// this fixed id, and an admitting envelope must present a matching
/// `bridge_grant_id` (`BRIDGE_GRANT_MATCHES`).
const DEFAULT_BRIDGE_GRANT_ID: &str = "bridge-grant-7";

/// The device key a default delivery is bound to (D-MOBILE). The imperative shell
/// binds a real delivery to the device key its bridge grant pinned; a
/// loopback/default delivery uses this fixed key, and an admitting envelope must
/// present a matching, still-active device (`DEVICE_BINDING_MATCHES`).
const DEFAULT_DEVICE_KEY: &str = "04dev1ce0ke7";
const DEFAULT_DEVICE_ID: &str = "device:pixel-9";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DeliveryPhase {
    Draft,
    Authorized,
    Queued,
    Delivered,
    TargetAdmitted,
    TargetRejected,
    Failed,
    Expired,
    Canceled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryState {
    pub phase: DeliveryPhase,
    pub source_authorized: bool,
    pub delivered: bool,
    pub target_admitted: bool,
    pub target_admitted_by: Authority,
    /// The payload authority — always `Source`; a relay never becomes it (`INV-14`).
    pub payload_authority: Authority,
    /// A relay never gains payload read while routing (`INV-10`).
    pub relay_has_payload_access: bool,
    /// The message's content handles — never widened by retry (`INV-17`).
    pub handles: BTreeSet<String>,
    /// Correlation is never rewritten in transit (`INV-7`).
    pub correlation_intact: bool,
    pub expired: bool,
    pub retried: bool,
    /// The target verified the source's envelope signature before admitting the
    /// receipt — true whenever `target_admitted` is on the verified path
    /// (`INV-21`); reset by a retry so a re-admission must re-verify.
    pub signature_verified: bool,
    /// The bridge grant this delivery is bound to; an admitting envelope must
    /// present a matching `bridge_grant_id` (`BRIDGE_GRANT_MATCHES`).
    pub bound_bridge_grant_id: BridgeGrantId,
    /// The envelope nonces the target has already admitted — a replayed nonce is
    /// rejected so the same signed envelope cannot write a second fact
    /// (`NONCE_NOT_REUSED`). Carried across retries so a previously admitted
    /// nonce stays spent.
    pub seen_nonces: BTreeSet<Nonce>,
    /// The paired device this delivery is bound to (D-MOBILE / MOB-004). The
    /// boundary's `DeviceBinding` phase pins a `(DeviceId, BridgeGrantId)`; here
    /// the delivery keeps the device id and its device key, and an admitting
    /// envelope must present this same, still-active device
    /// (`DEVICE_BINDING_MATCHES`).
    pub bound_device: DeviceId,
    /// The device key the bound device presents — an admitting envelope's
    /// `device_key` must match it (`DEVICE_BINDING_MATCHES`).
    pub bound_device_key: PublicKey,
}

impl Default for DeliveryState {
    fn default() -> Self {
        Self {
            phase: DeliveryPhase::Draft,
            source_authorized: false,
            delivered: false,
            target_admitted: false,
            target_admitted_by: Authority::None,
            payload_authority: Authority::Source,
            relay_has_payload_access: false,
            handles: original_handles(),
            correlation_intact: true,
            expired: false,
            retried: false,
            signature_verified: false,
            bound_bridge_grant_id: BridgeGrantId::new(DEFAULT_BRIDGE_GRANT_ID),
            seen_nonces: BTreeSet::new(),
            bound_device: DeviceId::new(DEFAULT_DEVICE_ID),
            bound_device_key: PublicKey::new(DEFAULT_DEVICE_KEY),
        }
    }
}

/// The signed source envelope the target verifies before admitting a receipt
/// (`INV-21`). Mirrors [`crate::federation::CrossingEnvelope`]: the reducer
/// borrows only the bytes `verify_signature` needs — the canonical metadata the
/// source signed, the signature over them, and the source's pinned public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryEnvelope {
    pub signed_bytes: Vec<u8>,
    pub signature: Signature,
    pub source_pubkey: PublicKey,
    /// The single-use nonce the source stamped on this envelope — the target
    /// rejects an admission whose nonce it has already admitted
    /// (`NONCE_NOT_REUSED`).
    pub nonce: Nonce,
    /// The bridge grant this envelope was minted under — must match the grant
    /// the delivery is bound to (`BRIDGE_GRANT_MATCHES`).
    pub bridge_grant_id: BridgeGrantId,
    /// The device key the presenting client signed this envelope with — must
    /// match the device key the delivery is bound to (`DEVICE_BINDING_MATCHES`).
    pub device_key: PublicKey,
    /// Whether the presenting device's bridge grant is still active at admission
    /// time — the imperative shell materializes this from the grant's `active`
    /// flag (mirrors [`crate::bridge_grant::BridgeGrant::is_valid`]). A revoked
    /// device (`false`) denies admission, so it cannot keep delivering.
    pub device_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryCommand {
    /// Bind this delivery to the **real** bridge grant + device the imperative shell
    /// authorized (FED-3), replacing the loopback defaults — so an admitting
    /// envelope must match the *actual* grant the source's key was pinned under, not
    /// the fixed `bridge-grant-7`. **Draft-only**: the binding is fixed before the
    /// message is authorized, so it cannot be flipped mid-flight to admit a foreign
    /// envelope. The bound values are still checked exactly as before
    /// (`BRIDGE_GRANT_MATCHES` / `DEVICE_BINDING_MATCHES`); only their source changes.
    BindDelivery {
        bridge_grant_id: BridgeGrantId,
        device_key: PublicKey,
        device: DeviceId,
    },
    AuthorizeFederatedMessage,
    EnqueueFederatedMessage,
    RecordRelayDelivery,
    /// The target admits the receipt — it verifies the source's signature over
    /// `envelope` before writing the target fact (`INV-21`/`CROSSING-1`).
    AdmitTargetReceipt {
        envelope: DeliveryEnvelope,
    },
    /// Validate that an envelope presents the bound, still-active device before
    /// the delivery is routed (D-MOBILE / MOB-004). A pure pre-check: it admits
    /// no fact and emits no event — it only succeeds (the device matches and its
    /// grant is active) or rejects (a foreign device key or a revoked device).
    /// The imperative shell runs it as a gate so a revoked device's delivery is
    /// stopped before it ever reaches `AdmitTargetReceipt`.
    ValidateDeviceBinding {
        envelope: DeliveryEnvelope,
    },
    RejectTargetReceipt,
    RecordDeliveryFailure,
    ExpireFederatedDelivery,
    CancelFederatedDelivery,
    RetryFederatedDelivery,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DeliveryEvent {
    /// The delivery was bound to a real grant + device (FED-3).
    DeliveryBound {
        bridge_grant_id: BridgeGrantId,
        device_key: PublicKey,
        device: DeviceId,
    },
    MessageAuthorized,
    MessageQueued,
    RelayDeliveryRecorded,
    /// The receipt was admitted; carries the envelope nonce so the target marks
    /// it spent (`NONCE_NOT_REUSED`).
    TargetReceiptAdmitted {
        nonce: Nonce,
    },
    TargetReceiptRejected,
    DeliveryFailed,
    DeliveryExpired,
    DeliveryCanceled,
    DeliveryRetried,
}

fn reject(reason: &'static str) -> Result<Vec<DeliveryEvent>, Rejection> {
    Err(Rejection { reason })
}

/// `DEVICE_BINDING_MATCHES`: the envelope must present the device key this
/// delivery is bound to and that device's grant must still be active. No
/// WRONG_DEVICE / DEVICE_REVOKED: a foreign device key or a revoked device
/// denies admission (fail-closed), so a revoked device cannot keep delivering
/// (`INV-21` / ADR 0009, D-MOBILE / MOB-004). Returns the rejection reason or
/// `Ok(())` — shared by `AdmitTargetReceipt` and `ValidateDeviceBinding`.
fn check_device_binding(
    state: &DeliveryState,
    envelope: &DeliveryEnvelope,
) -> Result<(), Rejection> {
    if envelope.device_key != state.bound_device_key {
        return Err(Rejection {
            reason: "device binding: envelope device key does not match the bound device (INV-21)",
        });
    }
    if !envelope.device_active {
        return Err(Rejection {
            reason: "device binding: presenting device's bridge grant is revoked (INV-21)",
        });
    }
    Ok(())
}

pub fn decide(
    state: &DeliveryState,
    command: DeliveryCommand,
) -> Result<Vec<DeliveryEvent>, Rejection> {
    use DeliveryPhase::*;
    match command {
        // BIND_BEFORE_AUTHORIZE (FED-3): the grant/device binding is fixed while the
        // delivery is still a draft, so it cannot be changed after the source has
        // authorized to retroactively admit a foreign envelope.
        DeliveryCommand::BindDelivery {
            bridge_grant_id,
            device_key,
            device,
        } => match state.phase {
            Draft => Ok(vec![DeliveryEvent::DeliveryBound {
                bridge_grant_id,
                device_key,
                device,
            }]),
            _ => {
                reject("bindDelivery: can only bind a draft delivery (binding precedes authorize)")
            }
        },
        DeliveryCommand::AuthorizeFederatedMessage => match state.phase {
            Draft => Ok(vec![DeliveryEvent::MessageAuthorized]),
            _ => reject("authorize: not a draft"),
        },
        DeliveryCommand::EnqueueFederatedMessage => {
            if state.phase == Authorized && state.source_authorized && !state.expired {
                Ok(vec![DeliveryEvent::MessageQueued])
            } else {
                reject("enqueue: requires a source-authorized message (no BYPASS_SOURCE)")
            }
        }
        // Relay delivery is **transport evidence only** — never target admission.
        DeliveryCommand::RecordRelayDelivery => {
            if state.phase == Queued && !state.expired {
                Ok(vec![DeliveryEvent::RelayDeliveryRecorded])
            } else {
                reject("recordRelayDelivery: not queued")
            }
        }
        // TARGET_ADMISSION_REQUIRES_SOURCE_AND_TARGET + EXPIRY_BLOCKS_TARGET_ADMISSION:
        // only the target authority admits, only after delivery, only before expiry.
        DeliveryCommand::AdmitTargetReceipt { envelope } => {
            if state.phase != Delivered {
                return reject(
                    "admitTargetReceipt: needs a delivered, non-expired message (INV-13)",
                );
            }
            // SIGNATURE_VERIFIED_BEFORE_ADMISSION: the target verifies the
            // source's envelope signature before writing the target fact
            // (INV-21 / CROSSING-1). No SKIP_SIG: a malformed or unverifiable
            // signature denies admission (fail-closed).
            match verify_signature(
                &envelope.signed_bytes,
                &envelope.signature,
                &envelope.source_pubkey,
            ) {
                Ok(true) => {}
                Ok(false) => {
                    return reject("admitTargetReceipt: source signature did not verify (INV-21)")
                }
                Err(_) => {
                    return reject(
                        "admitTargetReceipt: source signature could not be verified (INV-21)",
                    )
                }
            }
            // BRIDGE_GRANT_MATCHES: the envelope must be minted under the grant
            // this delivery is bound to. No MISMATCH_GRANT: an envelope under a
            // different grant denies admission (fail-closed).
            if envelope.bridge_grant_id != state.bound_bridge_grant_id {
                return reject("admitTargetReceipt: envelope bridge grant does not match the bound grant (INV-21)");
            }
            // DEVICE_BINDING_MATCHES: the envelope must present the bound,
            // still-active device. A foreign device key or a revoked device
            // denies admission (fail-closed), so a revoked device cannot keep
            // delivering (D-MOBILE / MOB-004).
            check_device_binding(state, &envelope)?;
            // NONCE_NOT_REUSED: the envelope nonce must be fresh. No REPLAY_NONCE:
            // an already-admitted nonce denies admission (fail-closed), so a
            // replayed signed envelope cannot write a second fact.
            if state.seen_nonces.contains(&envelope.nonce) {
                return reject(
                    "admitTargetReceipt: envelope nonce was already admitted — replay (INV-21)",
                );
            }
            Ok(vec![DeliveryEvent::TargetReceiptAdmitted {
                nonce: envelope.nonce,
            }])
        }
        // ValidateDeviceBinding is a pure pre-check: it admits no fact and emits
        // no event, succeeding only when the envelope presents the bound,
        // still-active device (DEVICE_BINDING_MATCHES). The imperative shell runs
        // it as a gate before routing so a revoked device is stopped early.
        DeliveryCommand::ValidateDeviceBinding { envelope } => {
            check_device_binding(state, &envelope)?;
            Ok(vec![])
        }
        DeliveryCommand::RejectTargetReceipt => match state.phase {
            Delivered => Ok(vec![DeliveryEvent::TargetReceiptRejected]),
            _ => reject("rejectTargetReceipt: nothing delivered to reject"),
        },
        DeliveryCommand::RecordDeliveryFailure => match state.phase {
            Authorized | Queued | Delivered => Ok(vec![DeliveryEvent::DeliveryFailed]),
            _ => reject("recordDeliveryFailure: not in flight"),
        },
        DeliveryCommand::ExpireFederatedDelivery => match state.phase {
            Draft | Authorized | Queued | Delivered => Ok(vec![DeliveryEvent::DeliveryExpired]),
            _ => reject("expire: already terminal"),
        },
        DeliveryCommand::CancelFederatedDelivery => match state.phase {
            Draft | Authorized | Queued | Delivered => Ok(vec![DeliveryEvent::DeliveryCanceled]),
            _ => reject("cancel: already terminal"),
        },
        DeliveryCommand::RetryFederatedDelivery => match state.phase {
            Failed | Expired | Canceled => Ok(vec![DeliveryEvent::DeliveryRetried]),
            _ => reject("retry: not in a retryable terminal state"),
        },
    }
}

pub fn evolve(state: &DeliveryState, event: DeliveryEvent) -> DeliveryState {
    use DeliveryPhase::*;
    let mut s = state.clone();
    match event {
        // FED-3: adopt the real grant/device the shell bound (Draft-only, so this
        // never changes after authorize). The bound values are checked unchanged at
        // admission — only their provenance moves from the default to the real grant.
        DeliveryEvent::DeliveryBound {
            bridge_grant_id,
            device_key,
            device,
        } => {
            s.bound_bridge_grant_id = bridge_grant_id;
            s.bound_device_key = device_key;
            s.bound_device = device;
        }
        DeliveryEvent::MessageAuthorized => {
            s.phase = Authorized;
            s.source_authorized = true;
        }
        DeliveryEvent::MessageQueued => s.phase = Queued,
        // delivered=true, but NOT target-admitted: relay gains no payload access /
        // authority and cannot rewrite correlation (RELAY_* invariants hold by omission).
        DeliveryEvent::RelayDeliveryRecorded => {
            s.phase = Delivered;
            s.delivered = true;
        }
        // The fact is admitted only on the verified path above, so the
        // verification is recorded alongside it (INV-21 / ADR 0004 — recorded,
        // not just checked).
        DeliveryEvent::TargetReceiptAdmitted { nonce } => {
            s.phase = TargetAdmitted;
            s.target_admitted = true;
            s.target_admitted_by = Authority::Target; // never Relay (RELAY_WRITES_TARGET)
            s.signature_verified = true;
            // NONCE_NOT_REUSED: the admitted nonce is now spent (kept across
            // retries so a later replay of the same nonce is rejected).
            s.seen_nonces.insert(nonce);
        }
        DeliveryEvent::TargetReceiptRejected => s.phase = TargetRejected,
        DeliveryEvent::DeliveryFailed => s.phase = Failed,
        DeliveryEvent::DeliveryExpired => {
            s.phase = Expired;
            s.expired = true;
        }
        DeliveryEvent::DeliveryCanceled => s.phase = Canceled,
        // RETRY_DOES_NOT_WIDEN: a retry resets to authorized with the original handles.
        DeliveryEvent::DeliveryRetried => {
            s.phase = Authorized;
            s.source_authorized = true;
            s.delivered = false;
            s.target_admitted = false;
            s.target_admitted_by = Authority::None;
            s.handles = original_handles();
            s.expired = false;
            s.retried = true;
            // A re-admission must re-verify the source signature (INV-21).
            s.signature_verified = false;
        }
    }
    s
}

impl crate::Lifecycle for DeliveryState {
    type State = DeliveryState;
    type Command = DeliveryCommand;
    type Event = DeliveryEvent;
    const KIND: &'static str = "federated_delivery";
    fn decide(
        state: &DeliveryState,
        command: DeliveryCommand,
    ) -> Result<Vec<DeliveryEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &DeliveryState, event: DeliveryEvent) -> DeliveryState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn apply(state: &DeliveryState, command: DeliveryCommand) -> DeliveryState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }
    fn delivered() -> DeliveryState {
        let s = DeliveryState::default();
        let s = apply(&s, DeliveryCommand::AuthorizeFederatedMessage);
        let s = apply(&s, DeliveryCommand::EnqueueFederatedMessage);
        apply(&s, DeliveryCommand::RecordRelayDelivery)
    }

    /// A well-formed source envelope `verify_signature` accepts: non-empty
    /// signed bytes and a 64-byte P-256 signature (mirrors `federation`).
    fn valid_envelope() -> DeliveryEnvelope {
        let sk = crate::signature::SigningKey::from_seed(&[7u8; 32]).unwrap();
        let signed_bytes = vec![1, 2, 3, 4];
        DeliveryEnvelope {
            signature: sk.sign(&signed_bytes),
            source_pubkey: sk.public_key(),
            signed_bytes,
            nonce: Nonce::new("nonce-1"),
            bridge_grant_id: BridgeGrantId::new(DEFAULT_BRIDGE_GRANT_ID),
            device_key: PublicKey::new(DEFAULT_DEVICE_KEY),
            device_active: true,
        }
    }

    /// An `AdmitTargetReceipt` over the valid envelope.
    fn admit() -> DeliveryCommand {
        DeliveryCommand::AdmitTargetReceipt {
            envelope: valid_envelope(),
        }
    }

    /// FED-3: a bound delivery admits an envelope under the **real** grant + device
    /// it was bound to, and rejects one under the old loopback default — the binding
    /// is honored at admission. Binding is draft-only (cannot be flipped mid-flight).
    #[test]
    fn bound_delivery_honors_the_real_grant_and_device() {
        let real_device = PublicKey::new("04rea1dev1ce");
        let bind = || DeliveryCommand::BindDelivery {
            bridge_grant_id: BridgeGrantId::new("grant-real-1"),
            device_key: real_device.clone(),
            device: DeviceId::new("device:peer-b"),
        };
        // An envelope under the real grant + device the delivery was bound to.
        let real_envelope = || {
            let mut e = valid_envelope();
            e.bridge_grant_id = BridgeGrantId::new("grant-real-1");
            e.device_key = real_device.clone();
            e
        };

        // Bind (draft) → authorize → enqueue → deliver → admit the real envelope.
        let s = apply(&DeliveryState::default(), bind());
        assert_eq!(s.bound_bridge_grant_id.as_str(), "grant-real-1");
        let s = apply(&s, DeliveryCommand::AuthorizeFederatedMessage);
        let s = apply(&s, DeliveryCommand::EnqueueFederatedMessage);
        let s = apply(&s, DeliveryCommand::RecordRelayDelivery);
        let admitted = apply(
            &s,
            DeliveryCommand::AdmitTargetReceipt {
                envelope: real_envelope(),
            },
        );
        assert_eq!(
            admitted.phase,
            DeliveryPhase::TargetAdmitted,
            "the envelope under the bound real grant admits"
        );

        // The old loopback-default grant no longer matches a delivery bound to the
        // real one — it is rejected (BRIDGE_GRANT_MATCHES against the bound grant).
        let rejected = apply(
            &s,
            DeliveryCommand::AdmitTargetReceipt {
                envelope: valid_envelope(), // bridge-grant-7 / default device
            },
        );
        assert_ne!(
            rejected.phase,
            DeliveryPhase::TargetAdmitted,
            "an envelope under the old default grant is denied once bound to the real grant"
        );

        // BIND_BEFORE_AUTHORIZE: binding a non-draft delivery is rejected.
        let authorized = apply(
            &DeliveryState::default(),
            DeliveryCommand::AuthorizeFederatedMessage,
        );
        assert!(
            decide(&authorized, bind()).is_err(),
            "binding can only happen while the delivery is a draft"
        );
    }

    #[test]
    fn relay_delivery_is_not_target_admission() {
        // RELAY_DELIVERY_ADMITS teeth: delivery is transport only, not admission.
        let s = delivered();
        assert_eq!(s.phase, DeliveryPhase::Delivered);
        assert!(!s.target_admitted, "relay delivery is not target admission");
        // …and only the target authority admits (RELAY_WRITES_TARGET teeth).
        let s = apply(&s, admit());
        assert!(s.target_admitted && s.target_admitted_by == Authority::Target);
        // SIGNATURE_VERIFIED_BEFORE_ADMISSION: the admitted fact recorded the
        // verified signature.
        assert!(
            s.signature_verified,
            "admitted receipt recorded the verified signature"
        );
        assert_ne!(s.payload_authority, Authority::Relay); // RELAY_OWNS_PAYLOAD teeth
        assert!(!s.relay_has_payload_access); // RELAY_READS_PAYLOAD teeth
    }

    /// SKIP_SIG tooth: on a delivered message, a bad signature (empty bytes or a
    /// wrong-length signature) still denies admission — no receipt is admitted
    /// without a verified signature (INV-21).
    #[test]
    fn admission_denies_unverified_signature() {
        let s = delivered();

        let empty = DeliveryCommand::AdmitTargetReceipt {
            envelope: DeliveryEnvelope {
                signed_bytes: vec![],
                ..valid_envelope()
            },
        };
        assert!(decide(&s, empty).is_err());

        let bad_sig = DeliveryCommand::AdmitTargetReceipt {
            envelope: DeliveryEnvelope {
                signature: Signature::new(vec![0u8; 32]),
                ..valid_envelope()
            },
        };
        let after = apply(&s, bad_sig);
        assert!(
            !after.target_admitted,
            "no receipt admitted on an unverified signature"
        );
        assert!(!after.signature_verified);
    }

    /// REPLAY_NONCE tooth: an envelope whose nonce the target has already
    /// admitted is denied — a replayed envelope cannot write a second fact
    /// (`NONCE_NOT_REUSED`, INV-21). `seen_nonces` is the target's spent-nonce
    /// set, carried across delivery attempts; here we seed it with a prior
    /// admission and re-present the same nonce on a freshly delivered attempt.
    #[test]
    fn admission_denies_replayed_nonce() {
        // First admission spends nonce-1.
        let admitted = apply(&delivered(), admit());
        assert!(admitted.target_admitted);
        assert!(
            admitted.seen_nonces.contains(&Nonce::new("nonce-1")),
            "admitted nonce is now spent"
        );

        // A subsequent delivery attempt at the same target carries the spent
        // nonce forward (the target remembers it). Re-presenting nonce-1 replays.
        let mut replay_attempt = delivered();
        replay_attempt.seen_nonces = admitted.seen_nonces.clone();
        assert_eq!(replay_attempt.phase, DeliveryPhase::Delivered);

        assert!(decide(&replay_attempt, admit()).is_err());
        let after = apply(&replay_attempt, admit());
        assert!(
            !after.target_admitted,
            "no receipt admitted on a replayed nonce"
        );

        // A fresh nonce on that same delivered attempt still admits.
        let fresh = DeliveryCommand::AdmitTargetReceipt {
            envelope: DeliveryEnvelope {
                nonce: Nonce::new("nonce-2"),
                ..valid_envelope()
            },
        };
        let after = apply(&replay_attempt, fresh);
        assert!(after.target_admitted, "a fresh nonce admits");
    }

    /// MISMATCH_GRANT tooth: an envelope minted under a bridge grant other than
    /// the one this delivery is bound to is denied admission
    /// (`BRIDGE_GRANT_MATCHES`, INV-21 / ADR 0009).
    #[test]
    fn admission_denies_mismatched_bridge_grant() {
        let s = delivered();
        assert_eq!(s.bound_bridge_grant_id.as_str(), DEFAULT_BRIDGE_GRANT_ID);

        let other_grant = DeliveryCommand::AdmitTargetReceipt {
            envelope: DeliveryEnvelope {
                bridge_grant_id: BridgeGrantId::new("bridge-grant-other"),
                ..valid_envelope()
            },
        };
        assert!(decide(&s, other_grant.clone()).is_err());
        let after = apply(&s, other_grant);
        assert!(
            !after.target_admitted,
            "no receipt admitted under a mismatched grant"
        );
        assert!(
            after.seen_nonces.is_empty(),
            "a denied admission spends no nonce"
        );
    }

    /// WRONG_DEVICE / DEVICE_REVOKED teeth: an envelope presenting a device key
    /// other than the bound one, or a device whose grant has been revoked, is
    /// denied admission — a revoked device cannot keep delivering
    /// (`DEVICE_BINDING_MATCHES`, INV-21 / ADR 0009, MOB-004).
    #[test]
    fn admission_denies_wrong_or_revoked_device() {
        let s = delivered();
        assert_eq!(s.bound_device_key, PublicKey::new(DEFAULT_DEVICE_KEY));

        // WRONG_DEVICE: a foreign device key denies admission.
        let wrong_device = DeliveryCommand::AdmitTargetReceipt {
            envelope: DeliveryEnvelope {
                device_key: PublicKey::new("04deadbeef00"),
                ..valid_envelope()
            },
        };
        assert!(decide(&s, wrong_device.clone()).is_err());
        let after = apply(&s, wrong_device);
        assert!(
            !after.target_admitted,
            "no receipt admitted under a foreign device key"
        );
        assert!(
            after.seen_nonces.is_empty(),
            "a denied admission spends no nonce"
        );

        // DEVICE_REVOKED: the bound device whose grant is no longer active.
        let revoked = DeliveryCommand::AdmitTargetReceipt {
            envelope: DeliveryEnvelope {
                device_active: false,
                ..valid_envelope()
            },
        };
        assert!(decide(&s, revoked.clone()).is_err());
        let after = apply(&s, revoked);
        assert!(
            !after.target_admitted,
            "no receipt admitted for a revoked device"
        );
    }

    /// `ValidateDeviceBinding` is the standalone pre-check: it admits no fact and
    /// changes no state, succeeding only on the bound, still-active device and
    /// rejecting a foreign or revoked one (MOB-004). This is the gate the shell
    /// runs before routing so a revoked device is stopped before admission.
    #[test]
    fn validate_device_binding_gates_routing() {
        let s = delivered();

        let ok = DeliveryCommand::ValidateDeviceBinding {
            envelope: valid_envelope(),
        };
        assert_eq!(
            decide(&s, ok.clone()),
            Ok(vec![]),
            "the bound, active device validates"
        );
        // A pure pre-check: no state change, no admission.
        assert_eq!(
            apply(&s, ok),
            s,
            "validation admits no fact and changes no state"
        );

        let foreign = DeliveryCommand::ValidateDeviceBinding {
            envelope: DeliveryEnvelope {
                device_key: PublicKey::new("04deadbeef00"),
                ..valid_envelope()
            },
        };
        assert!(
            decide(&s, foreign).is_err(),
            "a foreign device fails the gate"
        );

        let revoked = DeliveryCommand::ValidateDeviceBinding {
            envelope: DeliveryEnvelope {
                device_active: false,
                ..valid_envelope()
            },
        };
        assert!(
            decide(&s, revoked).is_err(),
            "a revoked device fails the gate"
        );
    }

    #[test]
    fn bypass_source_and_after_expire_are_rejected() {
        // BYPASS_SOURCE teeth: cannot enqueue/admit without source authorization.
        let s = DeliveryState::default();
        assert!(decide(&s, DeliveryCommand::EnqueueFederatedMessage).is_err());
        // TARGET_AFTER_EXPIRE teeth: an expired attempt cannot admit a target fact.
        let s = apply(&delivered(), DeliveryCommand::ExpireFederatedDelivery);
        assert_eq!(s.phase, DeliveryPhase::Expired);
        assert!(decide(&s, admit()).is_err());
    }

    #[test]
    fn retry_keeps_handles_within_original() {
        // RETRY_WIDENS teeth: retry never adds a handle.
        let s = apply(&delivered(), DeliveryCommand::RecordDeliveryFailure);
        let s = apply(&s, DeliveryCommand::RetryFederatedDelivery);
        assert!(s.retried);
        assert_eq!(
            s.handles,
            original_handles(),
            "retry did not widen the handles"
        );
    }

    /// The envelope a strategy draws for `AdmitTargetReceipt` — either a
    /// verifiable envelope or one that fails verification (a SKIP_SIG tooth at
    /// the proptest level), so the invariant is exercised on both paths.
    fn arb_envelope() -> impl Strategy<Value = DeliveryEnvelope> {
        prop_oneof![
            Just(valid_envelope()),
            Just(DeliveryEnvelope {
                signed_bytes: vec![],
                ..valid_envelope()
            }),
            Just(DeliveryEnvelope {
                signature: Signature::new(vec![0u8; 32]),
                ..valid_envelope()
            }),
            // REPLAY_NONCE / MISMATCH_GRANT teeth at the proptest level: a second
            // nonce, the spent nonce, and a grant other than the bound one, so
            // NONCE_NOT_REUSED / BRIDGE_GRANT_MATCHES are exercised on both paths.
            Just(DeliveryEnvelope {
                nonce: Nonce::new("nonce-2"),
                ..valid_envelope()
            }),
            Just(DeliveryEnvelope {
                bridge_grant_id: BridgeGrantId::new("bridge-grant-other"),
                ..valid_envelope()
            }),
            // WRONG_DEVICE / DEVICE_REVOKED teeth at the proptest level: a foreign
            // device key and a revoked device, so DEVICE_BINDING_MATCHES is
            // exercised on both paths.
            Just(DeliveryEnvelope {
                device_key: PublicKey::new("04deadbeef00"),
                ..valid_envelope()
            }),
            Just(DeliveryEnvelope {
                device_active: false,
                ..valid_envelope()
            }),
        ]
    }

    fn arb_command() -> impl Strategy<Value = DeliveryCommand> {
        use DeliveryCommand::*;
        prop_oneof![
            Just(AuthorizeFederatedMessage),
            Just(EnqueueFederatedMessage),
            Just(RecordRelayDelivery),
            arb_envelope().prop_map(|envelope| AdmitTargetReceipt { envelope }),
            arb_envelope().prop_map(|envelope| ValidateDeviceBinding { envelope }),
            Just(RejectTargetReceipt),
            Just(RecordDeliveryFailure),
            Just(ExpireFederatedDelivery),
            Just(CancelFederatedDelivery),
            Just(RetryFederatedDelivery),
        ]
    }

    proptest! {
        #[test]
        fn federated_delivery_invariants(commands in prop::collection::vec(arb_command(), 0..50)) {
            let mut s = DeliveryState::default();
            let orig = original_handles();
            for c in commands {
                // EXPIRY_BLOCKS_TARGET_ADMISSION.
                if s.expired {
                    prop_assert!(decide(&s, admit()).is_err());
                }
                // The pre-apply view the next admission is judged against:
                // whether this command's envelope nonce was already spent or its
                // bridge grant fails to match the bound grant.
                let pre_admitted = s.target_admitted;
                let admit_basis = match &c {
                    DeliveryCommand::AdmitTargetReceipt { envelope } => Some((
                        s.seen_nonces.contains(&envelope.nonce),
                        envelope.bridge_grant_id != s.bound_bridge_grant_id,
                        envelope.device_key != s.bound_device_key || !envelope.device_active,
                    )),
                    _ => None,
                };
                s = apply(&s, c);
                // TARGET_ADMISSION_REQUIRES_SOURCE_AND_TARGET.
                if s.target_admitted {
                    prop_assert!(s.source_authorized && s.delivered && s.target_admitted_by == Authority::Target);
                    // SIGNATURE_VERIFIED_BEFORE_ADMISSION (INV-21 / CROSSING-1).
                    prop_assert!(s.signature_verified, "receipt admitted without a verified signature");
                }
                // NONCE_NOT_REUSED / BRIDGE_GRANT_MATCHES: a *new* admission this
                // step is only ever on a fresh nonce and a matching grant.
                if !pre_admitted && s.target_admitted {
                    if let Some((nonce_seen, grant_mismatch, device_invalid)) = admit_basis {
                        prop_assert!(!nonce_seen, "receipt admitted on a replayed nonce");
                        prop_assert!(!grant_mismatch, "receipt admitted under a mismatched bridge grant");
                        // DEVICE_BINDING_MATCHES: a new admission is only ever on
                        // the bound, still-active device — never a foreign or
                        // revoked one (MOB-004).
                        prop_assert!(!device_invalid, "receipt admitted under a foreign or revoked device");
                    }
                }
                // RELAY_NO_PAYLOAD_ACCESS / RELAY_NOT_PAYLOAD_AUTHORITY.
                prop_assert!(!s.relay_has_payload_access);
                prop_assert_ne!(s.payload_authority, Authority::Relay);
                // RETRY_DOES_NOT_WIDEN.
                prop_assert!(s.handles.iter().all(|h| orig.contains(h)));
                // CORRELATION_PRESERVED.
                prop_assert!(s.correlation_intact);
            }
        }
    }
}

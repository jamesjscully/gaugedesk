//! Sealed-key release — the typed records of the attested key-release flow
//! (D-ATTEST / ADR 0040, ATTEST-4).
//!
//! A confidential workload only earns the keys that unseal its inputs once it has
//! *proven* what code it is running. A [`SealedKeyRecord`] binds opaque key
//! material to the [`CodeMeasurement`] it is sealed to: the key may be released
//! only to a host that attests to that very measurement. A [`KeyReleaseRequest`]
//! carries the [`AttestationEvidence`] a host presented; the pure
//! [`KeyReleaseRequest::decide`] turns it into a [`KeyReleaseDecision`] —
//! `Released` with the key material, or `Denied` with a structured reason.
//!
//! These are pure domain types: the decision is a total function over the request
//! and the sealed record, with no I/O. The app-layer key-release service
//! (`LoopbackKeyReleaseService`, ATTEST-5) and the sealed-key store (ATTEST-6)
//! attach behind this seam; the real KMS (Azure Key Vault / AWS KMS / GCP KMS)
//! that binds sealing to attestation replaces the loopback impl with no change
//! here.

use crate::attestation::{AttestationEvidence, CodeMeasurement};

/// Why a presented entitlement does not authorize a release — the structured,
/// fail-closed reasons the gate refuses on (ADR 0048).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EntitlementIneligibility {
    /// No active entitlement exists for the engagement (none requested, still
    /// pending, or denied) — the commercial relationship to bill against is absent.
    NoActiveEntitlement,
    /// An entitlement exists but is suspended or closed — future use is blocked
    /// (`INV-18`, future-only).
    Blocked,
    /// The entitlement's TTL elapsed before this release — "valid, *unexpired*".
    Expired,
}

/// A shell-evaluated verdict on the governed-deployment entitlement presented at
/// key-release time (ADR 0048, "attestation is the meter").
///
/// This mirrors [`QuoteVerificationResult`](crate::attestation::QuoteVerificationResult):
/// the imperative shell folds the deployment-entitlement reducer and checks the
/// grant's freshness against the wall clock, then materializes the verdict here. The
/// pure gate *trusts* the verdict rather than re-deriving it — non-determinism
/// (the clock, the fold) enters as data on the request (`INV-9`), and the gate stays
/// a total function. Anything other than [`Active`](EntitlementVerdict::Active)
/// denies, fail-closed (`INV-20`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EntitlementVerdict {
    /// An entitlement that is active and unexpired as of the evaluation instant.
    Active,
    /// No usable entitlement — the gate denies, for the given structured reason.
    Ineligible { reason: EntitlementIneligibility },
}

impl EntitlementVerdict {
    /// Whether this verdict authorizes a release (active and unexpired).
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }
}

/// The entitlement presented alongside a [`KeyReleaseRequest`] (ADR 0048): which
/// engagement/client it provisions and the shell-evaluated [`EntitlementVerdict`].
///
/// The grant carries the engagement id (and, in the durable store record, the
/// engagement's value) so the eligibility model can move from metered attested
/// compute to a marketplace take-rate as a grant-policy change, not a re-plumbing
/// (ADR 0048). The pure gate consumes only the verdict; the engagement rides through
/// so the durable release grant can record *which* engagement was billed.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EntitlementProof {
    /// The engagement/deployment context the entitlement provisions.
    pub engagement: String,
    /// The shell-evaluated verdict the gate trusts.
    pub verdict: EntitlementVerdict,
}

impl EntitlementProof {
    /// A proof carrying `verdict` for `engagement`.
    pub fn new(engagement: impl Into<String>, verdict: EntitlementVerdict) -> Self {
        Self {
            engagement: engagement.into(),
            verdict,
        }
    }

    /// An active (release-authorizing) proof for `engagement` — the happy-path shape.
    pub fn active(engagement: impl Into<String>) -> Self {
        Self::new(engagement, EntitlementVerdict::Active)
    }

    /// An ineligible proof for `engagement`, denying for `reason`.
    pub fn ineligible(engagement: impl Into<String>, reason: EntitlementIneligibility) -> Self {
        Self::new(engagement, EntitlementVerdict::Ineligible { reason })
    }
}

/// Key material sealed to a single [`CodeMeasurement`]: the bytes may be released
/// only to a host whose attested measurement equals `measurement`.
///
/// The `key_bytes` are opaque (a wrapped data-encryption key, a recovery secret);
/// the core never interprets them — it only releases them to a trustworthy,
/// matching measurement. They are private so callers go through
/// [`KeyReleaseDecision::released_key`] rather than reading the field of a denied
/// request.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SealedKeyRecord {
    /// Stable identifier for this sealed key.
    pub id: String,
    /// The measurement the key is sealed to — release is gated on a host
    /// attesting to exactly this measurement.
    pub measurement: CodeMeasurement,
    /// Opaque sealed key material — released only to a matching, trusted host.
    key_bytes: Vec<u8>,
}

impl SealedKeyRecord {
    /// Seal `key_bytes` to `measurement`.
    pub fn new(
        id: impl Into<String>,
        measurement: CodeMeasurement,
        key_bytes: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            id: id.into(),
            measurement,
            key_bytes: key_bytes.into(),
        }
    }
}

/// A request to release the key sealed under `sealed_key_id`, carrying the
/// attestation evidence the requesting host presented (ATTEST-2).
///
/// The request names the sealed key it wants; the decision is taken against the
/// matching [`SealedKeyRecord`] and the carried [`AttestationEvidence`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KeyReleaseRequest {
    /// The sealed key the requester is asking to unseal.
    pub sealed_key_id: String,
    /// The evidence the requesting host presented for its current measurement.
    pub evidence: AttestationEvidence,
    /// The entitlement presented for the engagement this release bills against
    /// (ADR 0048). No sealed key flows without an [`Active`](EntitlementVerdict::Active)
    /// verdict — the commercial gate collapsed onto the attestation checkpoint.
    pub entitlement: EntitlementProof,
}

impl KeyReleaseRequest {
    /// Construct a release request for `sealed_key_id` backed by `evidence` and the
    /// `entitlement` presented for the billing engagement (ADR 0048).
    pub fn new(
        sealed_key_id: impl Into<String>,
        evidence: AttestationEvidence,
        entitlement: EntitlementProof,
    ) -> Self {
        Self {
            sealed_key_id: sealed_key_id.into(),
            evidence,
            entitlement,
        }
    }

    /// Decide whether to release `sealed_key` to this request — a pure, total
    /// function the app-layer service (ATTEST-5) drives.
    ///
    /// The key is released only when all four hold:
    /// 1. the request names the offered record (`sealed_key_id` matches),
    /// 2. the evidence is *trustworthy* — its verdict verified the very
    ///    measurement the quote claimed ([`AttestationEvidence::is_trustworthy`]),
    /// 3. that trusted measurement equals the one the key is sealed to,
    /// 4. a valid, unexpired **entitlement** is presented (ADR 0048) — the meter is
    ///    the same checkpoint as the seal, fail-closed (`INV-20`).
    ///
    /// The attestation checks (1–3, the security floor) are evaluated before the
    /// entitlement gate (4, the commercial layer on top): a host that fails to prove
    /// trustworthy code gets a security denial and never learns the engagement's
    /// billing status. Any failure yields a structured [`KeyReleaseDenial`] so callers
    /// branch on *why* without string-matching.
    pub fn decide(&self, sealed_key: &SealedKeyRecord) -> KeyReleaseDecision {
        if self.sealed_key_id != sealed_key.id {
            return KeyReleaseDecision::Denied {
                reason: KeyReleaseDenial::UnknownSealedKey,
            };
        }
        if !self.evidence.is_trustworthy() {
            return KeyReleaseDecision::Denied {
                reason: KeyReleaseDenial::UntrustedAttestation,
            };
        }
        if self.evidence.quote.measurement != sealed_key.measurement {
            return KeyReleaseDecision::Denied {
                reason: KeyReleaseDenial::MeasurementMismatch,
            };
        }
        // ADR 0048: a trustworthy host still earns no key without a valid, unexpired
        // entitlement. The shell already evaluated the verdict (fold + clock); the
        // gate trusts it and fails closed on anything but `Active`.
        if let EntitlementVerdict::Ineligible { reason } = self.entitlement.verdict {
            return KeyReleaseDecision::Denied {
                reason: KeyReleaseDenial::Unentitled { reason },
            };
        }
        KeyReleaseDecision::Released {
            key_bytes: sealed_key.key_bytes.clone(),
        }
    }
}

/// The verdict of a [`KeyReleaseRequest`]: the sealed bytes were released, or
/// release was denied for a structured reason.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum KeyReleaseDecision {
    /// Release granted — the unsealed key material the requester may now use.
    Released { key_bytes: Vec<u8> },
    /// Release denied, for the given reason.
    Denied { reason: KeyReleaseDenial },
}

impl KeyReleaseDecision {
    /// The released key material, if the request was granted.
    pub fn released_key(&self) -> Option<&[u8]> {
        match self {
            Self::Released { key_bytes } => Some(key_bytes),
            Self::Denied { .. } => None,
        }
    }

    /// Whether the key was released.
    pub fn is_released(&self) -> bool {
        matches!(self, Self::Released { .. })
    }
}

/// Why a [`KeyReleaseRequest`] was denied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum KeyReleaseDenial {
    /// No sealed key matched the request's `sealed_key_id`.
    UnknownSealedKey,
    /// The presented evidence did not attest a trusted measurement (the quote was
    /// rejected, or the verdict vouched for a different measurement than claimed).
    UntrustedAttestation,
    /// The trusted measurement is not the one the key is sealed to.
    MeasurementMismatch,
    /// The host attested correctly, but no valid, unexpired entitlement authorizes
    /// the release (ADR 0048) — the commercial gate, fail-closed.
    Unentitled { reason: EntitlementIneligibility },
    /// The gate authorized release, but the key-release backend (a real KMS / Key
    /// Vault Secure Key Release) could not produce the key — the policy rejected the
    /// attestation token or the call failed. Fail-closed: no key flows (ADR 0049).
    ReleaseUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attestation::{AttestationQuote, QuoteRejection, QuoteVerificationResult};
    use proptest::prelude::*;

    fn measurement() -> CodeMeasurement {
        CodeMeasurement::new("a".repeat(64))
    }

    fn other_measurement() -> CodeMeasurement {
        CodeMeasurement::new("b".repeat(64))
    }

    fn quote_for(m: CodeMeasurement) -> AttestationQuote {
        AttestationQuote::new(m, "nonce-1", vec![1, 2, 3, 4])
    }

    fn trustworthy_evidence(m: CodeMeasurement) -> AttestationEvidence {
        AttestationEvidence::new(
            quote_for(m.clone()),
            QuoteVerificationResult::Verified { measurement: m },
        )
    }

    fn sealed() -> SealedKeyRecord {
        SealedKeyRecord::new("sealed-1", measurement(), vec![9, 8, 7])
    }

    /// An active entitlement proof — the happy-path billing gate (ADR 0048). Used by
    /// the attestation-floor tests so they exercise *those* checks, not the gate.
    fn entitled() -> EntitlementProof {
        EntitlementProof::active("engagement-1")
    }

    /// Trustworthy evidence for the very measurement a key is sealed to releases
    /// the sealed bytes — the path ATTEST-5 takes on an attested acceptance.
    #[test]
    fn releases_key_to_trusted_matching_measurement() {
        let request =
            KeyReleaseRequest::new("sealed-1", trustworthy_evidence(measurement()), entitled());
        let decision = request.decide(&sealed());
        assert!(decision.is_released());
        assert_eq!(decision.released_key(), Some([9, 8, 7].as_slice()));
    }

    /// A request that names a different sealed key is denied as unknown.
    #[test]
    fn denies_when_sealed_key_id_does_not_match() {
        let request =
            KeyReleaseRequest::new("other", trustworthy_evidence(measurement()), entitled());
        let decision = request.decide(&sealed());
        assert_eq!(decision.released_key(), None);
        assert_eq!(
            decision,
            KeyReleaseDecision::Denied {
                reason: KeyReleaseDenial::UnknownSealedKey
            }
        );
    }

    /// A rejected quote is untrusted attestation — no release.
    #[test]
    fn denies_when_evidence_not_trustworthy() {
        let evidence = AttestationEvidence::new(
            quote_for(measurement()),
            QuoteVerificationResult::Rejected {
                reason: QuoteRejection::StaleNonce,
            },
        );
        let request = KeyReleaseRequest::new("sealed-1", evidence, entitled());
        let decision = request.decide(&sealed());
        assert_eq!(
            decision,
            KeyReleaseDecision::Denied {
                reason: KeyReleaseDenial::UntrustedAttestation
            }
        );
    }

    /// A verdict over a *different* measurement than the quote claimed is not
    /// trustworthy — denied as untrusted, never released to the wrong key.
    #[test]
    fn denies_when_verdict_measurement_differs_from_quote() {
        let evidence = AttestationEvidence::new(
            quote_for(measurement()),
            QuoteVerificationResult::Verified {
                measurement: other_measurement(),
            },
        );
        let request = KeyReleaseRequest::new("sealed-1", evidence, entitled());
        let decision = request.decide(&sealed());
        assert_eq!(
            decision,
            KeyReleaseDecision::Denied {
                reason: KeyReleaseDenial::UntrustedAttestation
            }
        );
    }

    /// Trustworthy evidence for a measurement the key is *not* sealed to is denied
    /// as a mismatch — a correctly-attested host still cannot take another's key.
    #[test]
    fn denies_when_trusted_measurement_is_not_the_sealed_one() {
        let request = KeyReleaseRequest::new(
            "sealed-1",
            trustworthy_evidence(other_measurement()),
            entitled(),
        );
        let decision = request.decide(&sealed());
        assert_eq!(
            decision,
            KeyReleaseDecision::Denied {
                reason: KeyReleaseDenial::MeasurementMismatch
            }
        );
    }

    /// ADR 0048: a perfectly trustworthy host, attesting the very measurement the key
    /// is sealed to, is still denied the key when no active entitlement is presented —
    /// the meter is the same checkpoint as the seal, fail-closed.
    #[test]
    fn denies_a_trustworthy_request_without_an_active_entitlement() {
        let request = KeyReleaseRequest::new(
            "sealed-1",
            trustworthy_evidence(measurement()),
            EntitlementProof::ineligible(
                "engagement-1",
                EntitlementIneligibility::NoActiveEntitlement,
            ),
        );
        let decision = request.decide(&sealed());
        assert_eq!(decision.released_key(), None);
        assert_eq!(
            decision,
            KeyReleaseDecision::Denied {
                reason: KeyReleaseDenial::Unentitled {
                    reason: EntitlementIneligibility::NoActiveEntitlement
                }
            }
        );
    }

    /// Each ineligibility reason (suspended/closed → Blocked; TTL elapsed → Expired)
    /// propagates through the gate unchanged, so the caller learns *why* it was denied.
    #[test]
    fn entitlement_ineligibility_reason_propagates() {
        for reason in [
            EntitlementIneligibility::Blocked,
            EntitlementIneligibility::Expired,
        ] {
            let request = KeyReleaseRequest::new(
                "sealed-1",
                trustworthy_evidence(measurement()),
                EntitlementProof::ineligible("engagement-1", reason),
            );
            assert_eq!(
                request.decide(&sealed()),
                KeyReleaseDecision::Denied {
                    reason: KeyReleaseDenial::Unentitled { reason }
                }
            );
        }
    }

    /// The security floor is evaluated before the commercial gate: an *untrustworthy*
    /// host that also lacks entitlement is denied as `UntrustedAttestation`, never
    /// `Unentitled` — a host that cannot prove its code never learns the engagement's
    /// billing status (no information leak to an unattested caller).
    #[test]
    fn attestation_failure_dominates_the_entitlement_gate() {
        let untrusted = AttestationEvidence::new(
            quote_for(measurement()),
            QuoteVerificationResult::Rejected {
                reason: QuoteRejection::StaleNonce,
            },
        );
        let request = KeyReleaseRequest::new(
            "sealed-1",
            untrusted,
            EntitlementProof::ineligible(
                "engagement-1",
                EntitlementIneligibility::NoActiveEntitlement,
            ),
        );
        assert_eq!(
            request.decide(&sealed()),
            KeyReleaseDecision::Denied {
                reason: KeyReleaseDenial::UntrustedAttestation
            }
        );
    }

    proptest! {
        /// The gate mirrors `entitlement-guard.qnt`: across arbitrary combinations of
        /// attestation trustworthiness, measurement match, and entitlement verdict, a
        /// key is released *iff* the host is trustworthy for the sealed measurement
        /// **and** an active entitlement is presented — `RELEASE_REQUIRES_ATTESTATION`
        /// ∧ `RELEASE_REQUIRES_ENTITLEMENT` ∧ `FRESH_AT_RELEASE`, fail-closed.
        #[test]
        fn release_iff_trustworthy_matching_and_entitled(
            trustworthy in any::<bool>(),
            matching in any::<bool>(),
            verdict_active in any::<bool>(),
            expired in any::<bool>(),
        ) {
            let sealed_to = measurement();
            // The measurement the host attests: the sealed one, or a different one.
            let attested_m = if matching { sealed_to.clone() } else { other_measurement() };
            let result = if trustworthy {
                QuoteVerificationResult::Verified { measurement: attested_m.clone() }
            } else {
                QuoteVerificationResult::Rejected { reason: QuoteRejection::UntrustedSignature }
            };
            let evidence = AttestationEvidence::new(
                AttestationQuote::new(attested_m, "nonce-1", vec![1, 2, 3, 4]),
                result,
            );
            let verdict = if verdict_active {
                EntitlementVerdict::Active
            } else if expired {
                EntitlementVerdict::Ineligible { reason: EntitlementIneligibility::Expired }
            } else {
                EntitlementVerdict::Ineligible { reason: EntitlementIneligibility::NoActiveEntitlement }
            };
            let request = KeyReleaseRequest::new(
                "sealed-1",
                evidence,
                EntitlementProof::new("engagement-1", verdict),
            );
            let released = request.decide(&sealed()).is_released();
            // Release happens exactly when all three floors hold.
            prop_assert_eq!(released, trustworthy && matching && verdict_active);
            // And whenever released, the entitlement was active and fresh.
            if released {
                prop_assert!(verdict.is_active());
            }
        }
    }

    fn cbor_round_trip<T>(value: &T) -> T
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let mut bytes = Vec::new();
        ciborium::into_writer(value, &mut bytes).unwrap();
        ciborium::from_reader(bytes.as_slice()).unwrap()
    }

    #[test]
    fn sealed_key_record_serde_round_trips() {
        let record = sealed();
        assert_eq!(cbor_round_trip(&record), record);
    }

    #[test]
    fn request_and_decision_serde_round_trip() {
        let request =
            KeyReleaseRequest::new("sealed-1", trustworthy_evidence(measurement()), entitled());
        assert_eq!(cbor_round_trip(&request), request);

        let decision = request.decide(&sealed());
        assert_eq!(cbor_round_trip(&decision), decision);
    }
}

//! The wire envelope for one bridge-carried federated message (D-REMOTE / ADR
//! 0009 / ADR 0022). M2.
//!
//! A [`FederatedEnvelope`] is the signed unit a source authority hands to a
//! relay for delivery to a target authority. It carries only **metadata and
//! handles** — never inline payload content (content lives behind the handles
//! in `payload_handles`). The signature is computed over the canonical encoding
//! of that metadata, and anti-replay fields (`nonce`, `correlation`,
//! `timestamp`, `bridge_grant_id`) let the target reject replays and bind the
//! delivery to a specific bridge grant.
//!
//! The type is pure data: it round-trips through serde so the imperative shell
//! can ferry it through the log and over the wire (`INV-8`). Verification of the
//! signature is the imperative shell's job, using
//! [`crate::signature::verify_signature`] over [`FederatedEnvelope::signed_bytes`].

use std::collections::BTreeSet;

use crate::ids::{AuthorityId, BridgeGrantId, Nonce};
use crate::signature::Signature;

/// A signed, bridge-carried federated message — metadata, content handles, and
/// anti-replay binding, plus the signature over the canonical metadata bytes
/// (D-REMOTE).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FederatedEnvelope {
    /// The authority that authorized and signed this message (`INV-1`).
    pub source: AuthorityId,
    /// The authority the message is addressed to — only it can admit a fact
    /// from this envelope (`INV-13`).
    pub target: AuthorityId,
    /// The acting principal within the source authority.
    pub actor: String,
    /// The message kind discriminator (e.g. a remote-call method tag).
    pub kind: String,
    /// Content-addressed handles for the payload — never inline content; a
    /// relay gains no payload read by routing these (`INV-10`).
    pub payload_handles: BTreeSet<String>,
    /// The source's basis for authorizing this message (grant / entitlement ref).
    pub source_basis: String,
    /// Correlation id — never rewritten in transit (`INV-7`).
    pub correlation: String,
    /// A single-use nonce for anti-replay at the target.
    pub nonce: Nonce,
    /// Source-stamped time (unix epoch seconds), part of the anti-replay window.
    pub timestamp: u64,
    /// The bridge grant this delivery is bound to (D-REMOTE).
    pub bridge_grant_id: BridgeGrantId,
    /// The signature over [`Self::signed_bytes`] (the canonical metadata).
    pub signature: Signature,
    /// The exact canonical metadata bytes the signature covers — kept alongside
    /// the envelope so the target verifies the same bytes the source signed.
    pub signed_bytes: Vec<u8>,
}

impl FederatedEnvelope {
    /// The canonical byte encoding of this envelope's metadata — everything
    /// EXCEPT [`Self::signature`] and [`Self::signed_bytes`]. The signature is
    /// computed over these bytes, and the target re-derives and verifies them.
    ///
    /// The encoding is deterministic canonical CBOR (`ciborium`): the metadata
    /// is serialized as a CBOR map whose fields are declared in lexicographic
    /// key order (so the emitted keys are sorted), and `ciborium` uses the
    /// minimal fixed-width encoding for integers. Identical metadata therefore
    /// produces identical bytes across implementations and platforms, which is
    /// what lets a target re-derive and verify the same bytes the source signed
    /// (D-REMOTE / ADR 0009 / ADR 0022).
    pub fn envelope_canonical_cbor(&self) -> Vec<u8> {
        // Fields are listed in sorted key order so the CBOR map keys come out
        // canonically sorted (`ciborium` preserves serialization order).
        #[derive(serde::Serialize)]
        struct Metadata<'a> {
            actor: &'a str,
            bridge_grant_id: &'a str,
            correlation: &'a str,
            kind: &'a str,
            nonce: &'a str,
            payload_handles: &'a BTreeSet<String>,
            source: &'a AuthorityId,
            source_basis: &'a str,
            target: &'a AuthorityId,
            timestamp: u64,
        }

        let metadata = Metadata {
            actor: &self.actor,
            bridge_grant_id: self.bridge_grant_id.as_str(),
            correlation: &self.correlation,
            kind: &self.kind,
            nonce: self.nonce.as_str(),
            payload_handles: &self.payload_handles,
            source: &self.source,
            source_basis: &self.source_basis,
            target: &self.target,
            timestamp: self.timestamp,
        };

        let mut bytes = Vec::new();
        ciborium::into_writer(&metadata, &mut bytes).expect("envelope metadata serializes to CBOR");
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_envelope() -> FederatedEnvelope {
        FederatedEnvelope {
            source: AuthorityId::new("did:gaugewright:acme"),
            target: AuthorityId::new("did:gaugewright:globex"),
            actor: "agent:planner".to_string(),
            kind: "remote_call".to_string(),
            payload_handles: ["method", "context"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            source_basis: "grant:bridge-7".to_string(),
            correlation: "corr-123".to_string(),
            nonce: Nonce::new("nonce-abc"),
            timestamp: 1_700_000_000,
            bridge_grant_id: BridgeGrantId::new("bridge-grant-7"),
            signature: Signature::new(vec![0u8; 64]),
            signed_bytes: vec![1, 2, 3, 4],
        }
    }

    /// The envelope round-trips through serde unchanged — the invariant the
    /// imperative shell relies on to ferry it through the log and over the wire
    /// (D-REMOTE / `INV-8`).
    #[test]
    fn envelope_serde_round_trip() {
        let envelope = sample_envelope();
        let mut bytes = Vec::new();
        ciborium::into_writer(&envelope, &mut bytes).unwrap();
        let decoded: FederatedEnvelope = ciborium::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(decoded, envelope);
    }

    /// The canonical encoding covers the metadata but neither the signature nor
    /// the stored signed bytes — changing only the signature leaves it stable.
    #[test]
    fn envelope_canonical_excludes_signature() {
        let mut a = sample_envelope();
        let canonical = a.envelope_canonical_cbor();
        a.signature = Signature::new(vec![9u8; 64]);
        a.signed_bytes = vec![7, 7, 7];
        assert_eq!(a.envelope_canonical_cbor(), canonical);
    }

    /// The canonical encoding is deterministic CBOR, not JSON: the bytes decode
    /// as a CBOR map and re-encode to the exact same bytes (`ciborium`), which
    /// is the cross-implementation contract a target relies on to re-derive and
    /// verify the signed bytes (CANONCBOR-1 / D-REMOTE).
    #[test]
    fn envelope_canonical_is_deterministic_cbor() {
        let envelope = sample_envelope();
        let canonical = envelope.envelope_canonical_cbor();

        // Re-encoding the same metadata yields byte-identical output.
        assert_eq!(envelope.envelope_canonical_cbor(), canonical);

        // The bytes are valid CBOR (a map), not JSON. A JSON object would begin
        // with `{` (0x7b); a CBOR map with ten entries begins with 0xaa.
        let value: ciborium::value::Value =
            ciborium::from_reader(canonical.as_slice()).expect("canonical bytes are CBOR");
        assert!(value.is_map(), "canonical metadata encodes as a CBOR map");
        assert_eq!(canonical[0], 0xaa, "CBOR map header for ten fields");

        // Round-tripping through the CBOR value preserves the bytes (canonical).
        let mut reencoded = Vec::new();
        ciborium::into_writer(&value, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, canonical);
    }
}

//! The resource primitive — a first-class noun (`specs/primitives/resource.md`),
//! ported from `specs/models/derived-output.qnt`.
//!
//! A resource is a protected thing the system records, references, or transports:
//! a `method`, a `context`, or a derived `output`. It carries a handle (`id`), a
//! `kind`, an `owner`, and — for a derived output — its `provenance` (the
//! resources it was computed from). Its **stakeholders** (the authorities whose
//! consent governs egress, `INV-13`) are the conservative union of the owner with
//! the owners of everything in its provenance. The boundary computes this from
//! provenance — **never** from the agent's claim of what it used: under-approximating
//! taint launders an asset out (the `LAUNDER` probe; `launder_taint_would_leak`).
//!
//! This is a noun, not a lifecycle (`primitives/README.md`): types plus the one
//! pure provenance→stakeholders law. The egress gate it feeds is reused unchanged
//! from [`crate::boundary::allowed_egress`]; the engagement-scoped taint superset
//! (the M1 reading, ported from `engagement-taint.qnt`) is the peer module
//! [`crate::taint`].

use std::collections::BTreeSet;

use crate::abac::ResourceAttributes;
use crate::boundary::Authority;

/// A resource handle. Holding or transporting it conveys **no** payload access
/// (`INV-10`); access is a separate, explicit decision evaluated at the boundary.
#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ResourceId(String);

impl ResourceId {
    /// Construct from an already-validated string (parsed at the imperative shell).
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The kind of a resource — `method | context | output`, an **open** set
/// (`primitives/resource.md`; mirrors `boundary.qnt`'s `type Kind = str`). A
/// newtype rather than a closed enum so a new kind is not a breaking change, and
/// so neither method nor context is privileged (`INV-12`).
#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ResourceKind(String);

impl ResourceKind {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn method() -> Self {
        Self("method".into())
    }
    pub fn context() -> Self {
        Self("context".into())
    }
    pub fn output() -> Self {
        Self("output".into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A protected resource. Stores the **inputs** to the stakeholder computation
/// (`owner` + `provenance`), never the forgeable result: an input resource has
/// empty provenance and stakeholders `{owner}`; a derived output's provenance is
/// the resources it was computed from, and its stakeholders union their owners in
/// (its *taint*).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Resource {
    pub id: ResourceId,
    pub kind: ResourceKind,
    pub owner: Authority,
    pub provenance: BTreeSet<ResourceId>,
}

impl Resource {
    /// An input resource (no provenance): its stakeholder set is the singleton
    /// `{owner}` — the degenerate case of the stakeholder-set model.
    pub fn input(id: ResourceId, kind: ResourceKind, owner: Authority) -> Self {
        Self {
            id,
            kind,
            owner,
            provenance: BTreeSet::new(),
        }
    }

    /// A derived `output` computed from `provenance`, produced under `owner`.
    pub fn derived(id: ResourceId, owner: Authority, provenance: BTreeSet<ResourceId>) -> Self {
        Self {
            id,
            kind: ResourceKind::output(),
            owner,
            provenance,
        }
    }
}

/// The conservative stakeholders law (`derived-output.qnt`): a resource's
/// stakeholders are its own `owner` together with the owners of **everything in
/// its provenance**. Empty provenance ⇒ the singleton `{owner}` (an input
/// resource). `owner_of` is the boundary's over-approximating oracle; supplying
/// the agent's under-approximation here is the `LAUNDER` bug the model rejects.
pub fn stakeholders(
    res: &Resource,
    owner_of: impl Fn(&ResourceId) -> Authority,
) -> BTreeSet<Authority> {
    let mut s = BTreeSet::new();
    s.insert(res.owner.clone());
    for p in &res.provenance {
        s.insert(owner_of(p));
    }
    s
}

/// Where a resource's protected payload bytes live (`data.md` content; the
/// `sqlite-local-store.md` content layout). A handle resolving the locator still
/// requires an access basis — the locator is *not* access (`INV-10`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContentLocator {
    /// Impl-resident in a workspace/worktree: resolves to `(path, commit)`, where
    /// `commit` is the workspace impl's opaque revision id. Per
    /// `sqlite-local-store.md`, workspace files live in the workspace, not the blob store.
    Workspace { path: String, commit: String },
    /// A non-workspace payload addressed by a content-store handle (attached
    /// binaries, exported deliverables). The seam for the `content/` store; unused
    /// while only context (workspace) resources exist.
    Content { handle: String },
}

/// The durable **resource-metadata record** (`data.md`): a declarative fact, not
/// lifecycle state, whose current value is source of truth. It carries the
/// primitive plus the boundary-materialized `stakeholders` snapshot (frozen per
/// `INV-9` so replay never re-derives it), the content locator, and tombstone
/// state. The payload itself stays behind the handle (`INV-10`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourceRecord {
    pub resource: Resource,
    /// Conjunctive-consent stakeholder set, computed once via [`stakeholders`].
    pub stakeholders: BTreeSet<Authority>,
    pub locator: ContentLocator,
    /// `true` once [[content-erasure]] tombstoned the payload: the handle, record,
    /// and history remain, but future payload resolution is blocked (`INV-6`/`INV-18`).
    pub tombstoned: bool,
    /// Enterprise ABAC attributes (`data.md` extension, ADR 0032): classification,
    /// residency region, purpose tags. Read by [`crate::abac::evaluate`] to *narrow*
    /// the floor's verdict, never to widen it (`ABAC_MONOTONE`). `#[serde(default)]`
    /// so records written before this field round-trip as the fail-closed default
    /// (the most-protected classification, no region/purpose).
    #[serde(default)]
    pub attributes: ResourceAttributes,
}

impl ResourceRecord {
    /// Build a record for `resource`, materializing its stakeholders from the
    /// boundary's `owner_of` oracle (never the agent's claim). Attributes default to
    /// the fail-closed [`ResourceAttributes::default`]; attach real ones with
    /// [`ResourceRecord::with_attributes`].
    pub fn new(
        resource: Resource,
        locator: ContentLocator,
        owner_of: impl Fn(&ResourceId) -> Authority,
    ) -> Self {
        let stakeholders = stakeholders(&resource, owner_of);
        Self {
            resource,
            stakeholders,
            locator,
            tombstoned: false,
            attributes: ResourceAttributes::default(),
        }
    }

    /// Attach ABAC attributes to a record (builder style). The admission shell sets
    /// these from the ingest context / IdP-classified source; they then gate egress
    /// via [`crate::abac`].
    pub fn with_attributes(mut self, attributes: ResourceAttributes) -> Self {
        self.attributes = attributes;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::{allowed_egress, Basis};
    use proptest::prelude::*;

    // Fixed config mirroring derived-output.qnt: method owned by A, context owned
    // by B, tcb = {model}. `owner_of` is the boundary's truthful oracle.
    fn owner_of(r: &ResourceId) -> Authority {
        match r.as_str() {
            "context" => Authority::from("B"),
            _ => Authority::from("A"), // "method" (and any other input) → A
        }
    }
    fn tcb() -> BTreeSet<Authority> {
        BTreeSet::from([Authority::from("model")])
    }

    #[test]
    fn input_resource_is_singleton() {
        let m = Resource::input(
            ResourceId::new("method"),
            ResourceKind::method(),
            "A".into(),
        );
        assert_eq!(
            stakeholders(&m, owner_of),
            BTreeSet::from([Authority::from("A")])
        );
    }

    #[test]
    fn record_materializes_stakeholders() {
        let ctx = Resource::input(ResourceId::new("ctx"), ResourceKind::context(), "B".into());
        let rec = ResourceRecord::new(
            ctx,
            ContentLocator::Workspace {
                path: "docs".into(),
                commit: "abc123".into(),
            },
            owner_of,
        );
        assert_eq!(rec.stakeholders, BTreeSet::from([Authority::from("B")]));
        assert!(!rec.tombstoned);
    }

    #[test]
    fn derived_output_unions_provenance_owners() {
        let out = Resource::derived(
            ResourceId::new("output"),
            "A".into(),
            BTreeSet::from([ResourceId::new("method"), ResourceId::new("context")]),
        );
        // {A} (owner) ∪ {A (method), B (context)} = {A, B}.
        assert_eq!(
            stakeholders(&out, owner_of),
            BTreeSet::from([Authority::from("A"), Authority::from("B")])
        );
    }

    fn auth() -> impl Strategy<Value = &'static str> {
        prop_oneof![Just("A"), Just("B")]
    }
    fn recip() -> impl Strategy<Value = &'static str> {
        prop_oneof![Just("A"), Just("B"), Just("ext"), Just("model")]
    }

    proptest! {
        /// SOUND_RELEASE (derived-output.qnt): a derived output reaches a non-TCB
        /// recipient only when every REAL stakeholder except the recipient
        /// consented (conjunctive, `INV-13`). The boundary tracks taint
        /// conservatively (here it equals the real taint), so the gate never leaks.
        /// The `launder_taint_would_leak` teeth show the same gate leaks the moment
        /// the tracked taint under-approximates the truth.
        #[test]
        fn sound_release(
            inputs in prop::collection::vec(prop_oneof![Just("method"), Just("context")], 0..4),
            granted in prop::collection::vec((auth(), recip()), 0..6),
            recipient in recip(),
        ) {
            let tcb = tcb();
            let provenance: BTreeSet<ResourceId> = inputs.iter().map(|s| ResourceId::new(*s)).collect();
            let out = Resource::derived(ResourceId::new("output"), "A".into(), provenance);
            let tracked = stakeholders(&out, owner_of); // conservative
            let truth = tracked.clone();                // == the real taint here
            let bases: BTreeSet<Basis> =
                granted.iter().map(|(b, t)| (Authority::from(*b), Authority::from(*t))).collect();
            let recipient = Authority::from(recipient);

            if allowed_egress(&tracked, &recipient, &tcb, &bases) && !tcb.contains(&recipient) {
                for s in truth.iter().filter(|s| **s != recipient) {
                    prop_assert!(
                        bases.contains(&(s.clone(), recipient.clone())),
                        "SOUND_RELEASE violated: {recipient} obtained the output without {s}'s consent"
                    );
                }
            }
        }
    }

    /// The teeth (derived-output.qnt's `LAUNDER` probe): an output that really
    /// derives from A's method and B's context has true taint {A, B}. If the
    /// boundary under-approximates it to {B} — the adversarial agent's claim of
    /// what it "used" — egress to `ext` is wrongly permitted with only B's consent,
    /// and A's asset leaks. Conservative taint blocks it until A consents too. This
    /// is why taint must be boundary-computed from provenance, not agent-asserted.
    #[test]
    fn launder_taint_would_leak() {
        let out = Resource::derived(
            ResourceId::new("output"),
            "A".into(),
            BTreeSet::from([ResourceId::new("method"), ResourceId::new("context")]),
        );
        let conservative = stakeholders(&out, owner_of); // {A, B}
        let laundered: BTreeSet<Authority> = BTreeSet::from([Authority::from("B")]); // under-approx
        let tcb = tcb();
        // Only B consents to ext; A never does.
        let bases: BTreeSet<Basis> =
            BTreeSet::from([(Authority::from("B"), Authority::from("ext"))]);

        // Laundered taint leaks: ext obtains the output with only B's consent…
        assert!(allowed_egress(
            &laundered,
            &Authority::from("ext"),
            &tcb,
            &bases
        ));
        // …while conservative taint correctly blocks it (A never consented).
        assert!(!allowed_egress(
            &conservative,
            &Authority::from("ext"),
            &tcb,
            &bases
        ));
    }
}

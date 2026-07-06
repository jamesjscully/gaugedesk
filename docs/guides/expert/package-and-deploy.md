# Package & deploy

<span class="status built">Built</span> <span class="status planned">live deployment Planned</span>

You've built an [archetype](../../concepts/glossary.md#archetype) and proven it on
real work. The next step is to hand it to a client **without handing over your
method**. That's what packaging and deployment are for: making the agent
transferable across parties while the instructions, skills, and context all stay
where they belong.

!!! warning "Status — read this first"
    The packaging and cross-party deployment model is **implemented and tested in
    the core**, but **there is no go-live path you can use today**. You cannot, in
    the shipped product, publish a package and have a remote client install and run
    it across the wire. Cross-party deployment, attested compute, and enterprise
    identity are <span class="status built">Built</span> or
    <span class="status planned">Planned</span> — not
    <span class="status available">Available</span>.

    What *is* available today is the **local desktop workbench** (build · run ·
    review) and **multi-authority federation** for collaborating across machines.
    This page describes the intended deployment sequence in future tense and will
    gain concrete step-by-step instructions as each leg goes live. The single
    source of truth for status is the [roadmap](../../reference/status.md); the
    security detail is in [How GaugeWright protects your work](../../concepts/protection.md).

!!! info "Where your data goes"
    GaugeWright orchestrates locally, but **inference is remote**. During any run —
    yours or a client's — the prompts and the in-scope
    [context](../../concepts/glossary.md#context) are sent to the third-party
    LLM provider configured for that agent. GaugeWright runs **no local model**.
    Whoever sets the provider key chooses who can see plaintext prompts and
    context. See [Where your data goes](../../concepts/protection.md#where-your-data-goes).

!!! note "Network egress default — open per project"
    The canonical [boundary](../../concepts/glossary.md#boundary) decision
    (2026-06-17) sets the runtime sandbox's **network-egress default to open**, not
    deny-by-default: a chat can reach the configured model out of the box, and the
    operator **opts into** kernel-enforced network isolation per project. The egress
    chokepoint and conjunctive consent are always on — this changes the honest
    ceiling, not the mechanism. If another page describes egress as
    "deny-by-default", treat that as the *isolated*-project posture; the per-project
    default is open.

## What a package is

A **[package](../../concepts/glossary.md#package)** is a durable, shareable
*manifest* — it makes an archetype transferable across parties **without** turning
transfer into execution or payload release. Three things are true of it:

- **It carries handles and metadata, not payload.** Publishing a package does not
  hand over your method's contents. Moving protected payload across a boundary is a
  separate, explicitly-permitted step.
- **A published version is immutable.** When you package an archetype its version
  is fixed, so a deployed agent never silently changes underneath a client. You
  [upgrade deliberately](#upgrades-and-withdrawal). <span class="status built">Built</span>
- **Installing a package grants nothing by itself.** A client admitting your
  package creates a local record that *this version is eligible* — it does not
  grant method-payload access, context access, run admission, or output release.
  Those remain governed by the boundary on every run.

??? note "For the spec-minded"
    Packaging and install are the **package-distribution** lifecycle
    (`specs/lifecycles/package-distribution.md`). Install requires a *published*
    source version **and** target admission, and it provably does not confer
    payload or run authority (`INSTALL_REQUIRES_PUBLISHED_TARGET_ADMISSION`,
    `INSTALL_DOES_NOT_GRANT_PAYLOAD_OR_RUN`). Withdrawal and removal are
    future-only and never erase history (`INV-18`).

## The intended deployment sequence

This is the path as designed. Each leg's status badge tells you whether you can
walk it today.

### 1. Package the archetype <span class="status built">Built</span>

From your library, you will publish the archetype you want to share. The published
version is immutable and becomes the unit a client can install. Publishing records
the manifest, its provenance, and the protection posture (next step) — it does
**not** release your method's payload.

### 2. Declare the protection posture <span class="status built">Built</span>

At publish time you declare *how the agent is allowed to run at the client*: the
[deployment mode](../../concepts/deployment-modes.md) (private/federated vs.
attested), what the method may read, and what crossing requires. The posture is
part of the package manifest, so the client sees the terms **before** admitting.
The runtime configuration the agent carries — model/provider, skills, tools — is
the same **Configuration** surface you set when you
[built the agent](build-an-agent.md#define-the-method); see
[Deployment modes](../../concepts/deployment-modes.md) for which posture fits which
engagement.

!!! warning "The posture chooses who can see plaintext"
    The protection posture governs the GaugeWright [boundary](../../concepts/glossary.md#boundary)
    — not the LLM provider. Even under the strongest posture, prompts and in-scope
    context still go to the configured inference provider. Choosing that provider
    is part of declaring the posture honestly. See
    [Where your data goes](../../concepts/protection.md#where-your-data-goes).

### 3. Deploy — private/federated, or attested

You will deploy in one of two shapes, depending on the posture:

=== "Private (federated)"

    The method runs **on the client's machine** as a placement. Collaboration
    between you and the client travels over certificate-pinned TLS through a relay
    that routes encrypted bytes and **never reads payload**. Nothing crosses
    without the source permitting it to leave *and* the target admitting it.

    - Federation transport: <span class="status available">Available</span>
    - End-to-end cross-party deploy (publish → install → run across the wire):
      <span class="status built">Built</span> <span class="status planned">live Planned</span>

=== "Attested compute"

    The agent runs in a **sealed confidential VM** (AMD SEV-SNP) with cryptographic
    proof that the method and context stayed sealed; both parties verify the
    [attestation](../../concepts/glossary.md#attestation) measurement before
    trusting a result. Sealed-key release is gated on a valid, unexpired deployment
    entitlement — the security checkpoint and the commercial meter are the *same*
    checkpoint.

    - Verifier / key-release gate: <span class="status built">Built</span>
    - Live hosting on real confidential hardware:
      <span class="status planned">Planned</span> (needs cloud TEE + KMS infra)

For governed (commercial) deployments, an **eligible install is necessary but not
sufficient** to run. The deployment must also hold an **active deployment
entitlement** before the install can be shown as run-ready.

??? note "For the spec-minded"
    Governed run-readiness is the **deployment-entitlement** lifecycle
    (`specs/lifecycles/deployment-entitlement.md`). Active entitlement is
    *eligibility*, never execution — actual runs still require the run, resource,
    boundary, and runtime lifecycles
    (`RUN_ELIGIBILITY_REQUIRES_ACTIVE_ENTITLEMENT`). For attested placements the
    entitlement is enforced where the attestation is *issued*: the issuer refuses
    to seal/quote a run without a valid, unexpired entitlement (fail-closed,
    `INV-20`; ADR 0048). Billing and support receipts are correlated evidence —
    they never create entitlement or run authority (`INV-16`).

### 4. The client admits the package <span class="status built">Built</span>

Deployment is **target admission**, not a push. The client reviews the published
posture and explicitly admits the package into their project, where it becomes a
[placement](../../concepts/glossary.md#placement). Admission creates a local
package record; it still grants no payload access, no context access, and no run
authority. From there the client runs the agent under their own
[boundary](../../concepts/glossary.md#boundary), and outputs are held until
explicitly reviewed and released — see
[Review & release outputs](../client/review-and-release.md).

## The guarantees, and what backs them

GaugeWright separates guarantees that are **machine-checked invariants** from ones
that are **policy or operational**. Both matter; only the first are structural.

| Guarantee | Kind | Status |
|---|---|---|
| Install requires a published version **and** target admission | Structural — model-checked | <span class="status built">Built</span> |
| Install grants no payload / context / run authority | Structural — model-checked | <span class="status built">Built</span> |
| Withdrawal/removal block future use but never erase history (`INV-18`) | Structural — model-checked | <span class="status built">Built</span> |
| Governed runs require an active entitlement | Structural — model-checked | <span class="status built">Built</span> |
| Method runs read-only at the client; never exported or revealed | Structural — kernel-enforced | <span class="status available">Available (Linux/macOS)</span> |
| Crossing requires source-permit **and** target-admit | Structural — federation | <span class="status available">Available</span> |
| Attested run actually sealed on real hardware | Operational — needs live TEE/KMS | <span class="status planned">Planned</span> |
| Confidential inference (provider **outside** your trust boundary) | Operational — needs attested inference | <span class="status planned">Planned</span> |

!!! warning "Per-OS caveat"
    Kernel-enforced method isolation is
    <span class="status available">Available</span> on **Linux and macOS** only. A
    Windows method-isolation sandbox is <span class="status planned">Planned</span>.
    On Windows the structural read-only-method guarantee does not yet hold.

## Upgrades and withdrawal <span class="status built">Built</span>

- **Upgrade is explicit.** Replacing a deployed version requires you to publish the
  replacement *and* the client to admit it. An upgrade never smuggles broader
  access or execution authority.
- **Withdrawal is future-only.** Withdrawing a published version (or a client
  removing an install) blocks new installs and future runs under that version but
  **does not erase** prior records, runs, or outputs.
- **Suspend / close a governed deployment.** Suspending or closing a deployment
  entitlement blocks future governed runs while preserving all prior
  package / install / run / output / receipt history.

## Known gaps (today)

- **No usable go-live path.** You cannot deploy to a remote client end-to-end in
  the shipped product. Everything above the federation transport is
  <span class="status built">Built</span> or <span class="status planned">Planned</span>.
- **Attested compute has no live hardware.** The verifier and the key-release gate
  are <span class="status built">Built</span>, but live confidential-VM hosting
  (real TEE + KMS) is <span class="status planned">Planned</span>.
- **Enterprise identity is not deployed.** OIDC / SAML / SCIM / RBAC are
  <span class="status built">Built</span> but not operationally available;
  MFA enforcement is <span class="status none">Not implemented</span>.
- **Inference provider is inside the trust boundary.** The third-party LLM
  provider you configure sees prompts and in-scope context in plaintext today —
  it is part of your boundary, not outside it. Confidential inference (removing
  the provider from the trust boundary) is <span class="status planned">Planned</span>.

See the full table and the security gaps (no SBOM / dependency scanning, no
production monitoring, unsigned builds) in the
[roadmap](../../reference/status.md) and the
[architecture & security documentation](../../security.md).

## Next

- **[Deployment modes](../../concepts/deployment-modes.md)** — where the agent runs
  and who's involved, mode by mode.
- **[How GaugeWright protects your work](../../concepts/protection.md)** — the
  protection model behind every deployment, and where your data goes.
- **[Run & review work](run-and-review.md)** — the local loop these deployments
  reuse.
- **[Review & release outputs](../client/review-and-release.md)** — what the client
  does after a run produces a result.

# Security & trust

GaugeWright's product *is* its security model: an expert's
[method](concepts/glossary.md#method) and a client's
[context](concepts/glossary.md#context) meet only inside an enforced
[boundary](concepts/glossary.md#boundary), and the guarantees are expressed as
formal **invariants** that are **machine-checked**. This page is the user-facing
summary and the entry point for a security or IT reviewer — it states what is
true today, points you at the in-repo detail, and hands you off to the
reviewer-grade trust documents.

!!! warning "The one fact to read first"
    **Local orchestration is not local inference.** The workbench runs on your
    machine, but the agent's reasoning is performed by the **third-party LLM
    provider you configure** — so your prompts and the in-scope context are sent
    to that provider over the network, in plaintext, today. Who can and cannot
    see plaintext is spelled out in
    [Where your data goes](concepts/protection.md#where-your-data-goes). The
    provider is in the trust boundary until confidential inference ships
    (<span class="status planned">Planned</span>).

## In brief

- **The boundary is structural.** [Handles](concepts/glossary.md#handle) don't
  grant access; method and context reads are both explicit;
  [runs](concepts/glossary.md#run) have no ambient authority; everything is
  fail-closed. These are [invariant](#structural-vs-operational)-backed and
  machine-checked. <span class="status available">Available</span>
- **Your method/data don't leak to the other party or the
  [runtime](concepts/glossary.md#runtime)** — but the agent's reasoning runs at
  the **third-party LLM provider you configure**, so prompts and in-scope context
  are sent there. The no-leak property is a structural, machine-checked invariant
  of the code; the **cross-party** [federation](concepts/glossary.md#federation)
  it governs is not yet operationally available (see the
  [federation footnote](#fed-caveat)). See
  [Where your data goes](concepts/protection.md#where-your-data-goes).
  <span class="status available">structural Available</span>
- **Everything is audited** in an append-only log, exportable to your SIEM.
  <span class="status available">Available</span>
- **Cross-party deployment, attested compute, and enterprise identity** are
  implemented and tested in code but **not yet operationally available**.
  <span class="status built">Built</span>
  <span class="status planned">live Planned</span>
- **No third-party certifications yet** (SOC 2 / ISO 27001 / pen test are
  committed and planned). <span class="status planned">Planned</span>

!!! note "Status vocabulary — one source of truth"
    Every badge on this page follows the same convention and defers to one table:
    <span class="status available">Available</span> = in the product you can
    download today · <span class="status built">Built</span> = implemented and
    tested in code, not operationally deployed ·
    <span class="status planned">Planned</span> = committed, not built ·
    <span class="status none">Not implemented</span> = absent today. The canonical
    list is **[Roadmap & status](reference/status.md)** — if anything here
    disagrees with that table, the table wins.

## What's Available today vs. what's coming

Today, the only capability you can download and use **end-to-end** is the
**local desktop [workbench](concepts/glossary.md#run)**.
[Federation](concepts/glossary.md#federation) is Available *only* for a
device set you own (a [self-federated](concepts/deployment-modes.md#federation)
pairing of your own machines over cert-pinned TLS); **cross-party,
multi-authority federation across two parties' machines is not operationally
available** — its code is Built and exercised in CI on a loopback + NAT-isolated
harness, but the hosted relay infrastructure is not live. Everything else on the
security surface is either Built (code-complete and tested, not deployed) or
Planned. The [Deployment modes](concepts/deployment-modes.md#federation) page
states this split precisely: federation works *locally* across your own devices,
while multi-party relay deployment is Planned.

| Capability | Status |
|---|---|
| Local desktop workbench (build · run · review) | <span class="status available">Available</span> |
| [Federation](concepts/glossary.md#federation) across **your own devices** (self-federated, cert-pinned TLS) | <span class="status available">Available</span> |
| Multi-authority [federation](concepts/glossary.md#federation) (**cross-party**, two parties' machines) [^fed] | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| Append-only audit log + SIEM export | <span class="status available">Available</span> |
| Kernel-enforced method isolation (Linux/macOS) | <span class="status available">Available</span> |
| Encryption at rest (KMS-backed, server deployments) | <span class="status built">Built</span> |
| Cross-party [packaging](concepts/glossary.md#package) & deployment | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| [Attested compute](concepts/glossary.md#attestation) (confidential VM) | <span class="status built">verifier Built</span> <span class="status planned">live Planned</span> |
| Enterprise identity (OIDC / SAML / SCIM / RBAC) | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| Windows method-isolation sandbox | <span class="status planned">Planned</span> |
| MFA enforcement | <span class="status none">Not implemented</span> |
| Confidential inference (provider out of trust boundary) | <span class="status planned">Planned</span> |
| SOC 2 Type II · DPA · penetration test | <span class="status planned">Planned</span> |

For the full table and what each status means, see
**[Roadmap & status](reference/status.md)**.

## Structural vs. operational guarantees

Not every promise on this page is the same *kind* of promise. Reviewers should
keep two buckets separate.

=== "Structural (machine-checked)"

    These are **invariants** of the design — properties the code is *built around*,
    each paired with a formal [Quint](#reviewer-handoff) model and an adversarial
    "teeth" test that fails CI if the protection is removed. They hold by
    construction, not by configuration, and cannot be misconfigured away.

    - A reference is not access: handles convey no payload read (`INV-10`).
    - Method and context reads are both explicit (`INV-12`).
    - A run can only do what it was admitted to do — no ambient authority
      (`INV-11`).
    - Fail-closed: uncertainty denies (`INV-20`).
    - Append-only history: durable facts are immutable events (`INV-6`).
    - A running agent cannot rewrite its own method — enforced by an OS sandbox
      at the kernel (`INV-24`). <span class="status available">Available
      (Linux/macOS)</span> · Windows sandbox
      <span class="status planned">Planned</span>.

    <span class="status available">Available</span>. The invariant IDs are
    optional deep links for reviewers — you do not need to read them to use the
    product.

=== "Operational / policy"

    These depend on how a deployment is *operated and configured*, on external
    infrastructure, or on programs that are still in progress. They are real but
    they are not machine-checked invariants.

    - **Encryption at rest** — the AES-256-GCM encryptor is
      <span class="status available">Available</span> locally; the KMS-backed data
      key (Azure Key Vault) is <span class="status built">Built</span> and awaits
      live wiring.
    - **Enterprise identity** (OIDC / SAML / SCIM / RBAC) — verifiers are
      <span class="status built">Built</span> and tested; live IdP interop and the
      admin console are <span class="status planned">Planned</span>.
    - **MFA enforcement** — <span class="status none">Not implemented</span>; under
      enforce-SSO the factor is enforced by your IdP.
    - **SOC 2 / ISO 27001 / penetration test / DPA** —
      <span class="status planned">Planned</span>; none are in hand today.
    - **Production monitoring, alerting, incident response** —
      <span class="status none">Not implemented</span>.

The short rule: a structural guarantee survives a misconfigured or hostile
operator; an operational one assumes the operator and the infrastructure behave.

## Per-OS caveats

| Protection | Linux | macOS | Windows |
|---|---|---|---|
| Kernel-enforced method isolation (`INV-24`) | <span class="status available">Available</span> | <span class="status available">Available</span> | <span class="status planned">Planned</span> |
| Local encryption at rest | <span class="status available">Available</span> | <span class="status available">Available</span> | <span class="status available">Available</span> |
| Federation over cert-pinned TLS (self-federated devices) [^fed] | <span class="status available">Available</span> | <span class="status available">Available</span> | <span class="status available">Available</span> |

!!! warning "Windows"
    The kernel sandbox that keeps a *work chat* from reading or rewriting the
    method (`INV-24`) is a Linux/macOS feature today. On Windows the
    method-isolation sandbox is <span class="status planned">Planned</span> — until
    it ships, treat method confidentiality on Windows as not kernel-enforced.

## Where your data goes (and who sees plaintext)

The most important review fact, stated plainly:

- **You can see your own plaintext.** Method, context, and outputs are resolvable
  to the authority that owns them, through the boundary.
- **The third-party LLM provider you configure sees plaintext** — your prompts and
  the in-scope context, sent over the network for inference. With your own
  provider credentials the provider is *your* subprocessor, not GaugeWright's; its
  retention and training terms are the provider's.
- **The other party does not see your side.** When cross-party federation is
  deployed, the expert's method does not leak to the client and the client's
  context does not leak to the method-owner (`INV-12` / `INV-22`). This is a
  structural guarantee of the code; note that cross-party federation is not
  operationally available yet (see the [federation footnote](#fed-caveat)) — it
  holds across your own self-federated devices today.
- **A federation relay cannot see plaintext, when deployed.** Relays route
  encrypted bytes only and are never payload authorities (`INV-14`). This is a
  structural property of the federation code; the relay infrastructure that
  would carry real cross-party traffic is not yet live (see the
  [federation footnote](#fed-caveat)).

Full treatment, including the data-flow crossings and the loud egress override:
[How GaugeWright protects your work → Where your data goes](concepts/protection.md#where-your-data-goes).

<a id="fed-caveat"></a>

!!! warning "Federation scope — tested in CI, not yet deployed for cross-party use"
    Federation is tested in CI with a loopback + NAT-isolated harness; **real
    cross-machine, multi-party deployment awaits hosted relay infrastructure**
    (<span class="status planned">Planned</span>). Federation across **your own
    devices** is <span class="status available">Available</span> today; the
    structural relay guarantees above (`INV-14`, `INV-10`) hold by construction
    in code, but the operational guarantee — "a relay routes encrypted bytes for
    two strangers' machines" — is not something you can switch on today. See
    [Deployment modes → Federation](concepts/deployment-modes.md#federation) and
    [Limitations & known gaps](reference/limitations.md) for the full statement.

## Limitations & known gaps (in the docs, not only off-site)

Honesty is a product feature here. The most actionable near-term gaps are stated
in-repo so a reviewer never has to leave the docs to find them:

- **No SBOM, no dependency-CVE scanning, no SAST/secret-scanning in CI.**
  <span class="status none">Not implemented</span>
- **Desktop builds are unsigned / un-notarized.**
  <span class="status none">Not implemented</span>
- **No production monitoring, alerting, runbooks, or incident-response.**
  <span class="status none">Not implemented</span>
- **Audit log is tamper-evident *semantically*** (immutable event log) **but not
  yet cryptographically** (no signature/merkle chain).
  <span class="status planned">Planned</span>
- **No third-party attestation** (SOC 2 / ISO 27001 / pen test).
  <span class="status planned">Planned</span>

These are named gaps, not oversights. For the full, structured list — and what is
in scope to fix next — see **[Limitations & known gaps](reference/limitations.md)**.

## How a reviewer verifies the claims

These are structural guarantees you can check yourself, not assertions you have to
take on faith.

1. **Read the invariants.** Each protection claim above maps to a numbered
   invariant in `specs/principles.md` and to a Quint model in `specs/models/`.
2. **Confirm each model holds and its tooth bites.** CI runs
   `quint typecheck specs/models/*.qnt` and verifies both that every invariant
   holds (`quint run --invariant`) and that its adversarial "teeth" probe fails
   when the protection is removed.
3. **Trace a guarantee to the reducer.** Property tests in the pure core
   (`crates/core`) tie the Rust reducers to the models — run
   `cargo test --workspace`.
4. **Check the live legs.** OIDC is verified per-commit against a self-hosted
   Keycloak; KMS wrap/unwrap against Azure Key Vault; attestation against real
   AMD SEV-SNP Milan vectors.

A step-by-step verification walkthrough — exact files, commands, and what each
proves — lives at **[Verifying our claims](reference/verifying-claims.md)**.

## In brief, for IT and compliance

| Area | Where it lives |
|---|---|
| Identity & access (OIDC / SAML / SCIM / RBAC) | [For admins (IT)](guides/admin/index.md) · <span class="status built">Built</span> |
| Output review & release | [Review & release outputs](guides/client/review-and-release.md) |
| Cross-party deployment & attestation | [Package & deploy](guides/expert/package-and-deploy.md) · <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| Deployment modes (local / federated / hosted / attested) | [Deployment modes](concepts/deployment-modes.md) |
| Vocabulary | [Glossary](concepts/glossary.md) |

## Reviewer handoff — full documentation

The reviewer-grade documents are published on the trust site. Each is available to
view, download as PDF, or read as Markdown.

- **[Security overview](https://gaugewright.com/security)** — the one-page summary.
- **[Architecture & security](https://gaugewright.com/architecture)** — system
  context, data-flow diagrams with trust boundaries, the invariant→control
  crosswalk (SOC 2 / ISO 27001 / NIST), threat model (STRIDE + OWASP LLM Top 10),
  AI governance, supply chain, and an honest compliance posture.
- **[Control crosswalk (CAIQ)](https://gaugewright.com/caiq)** — a control-by-control
  responses sheet you can drop into a vendor assessment.

For questions a questionnaire raises that these don't answer, contact the security
team at jack@gaugewright.com.

[^fed]: Federation is tested in CI with a loopback + NAT-isolated harness; real
    cross-machine, multi-party deployment awaits hosted relay infrastructure
    (Planned). Federation across **your own devices** is Available today. See the
    [federation scope note](#fed-caveat),
    [Deployment modes → Federation](concepts/deployment-modes.md#federation), and
    [Limitations & known gaps](reference/limitations.md).

---

!!! question "Where do I go next?"
    - New to the model? Start with
      [How GaugeWright protects your work](concepts/protection.md).
    - Want the timeline? [Roadmap & status](reference/status.md).
    - Standing up the org layer? [For admins (IT)](guides/admin/index.md).
    - Need the gaps and the proof method?
      [Limitations & known gaps](reference/limitations.md) ·
      [Verifying our claims](reference/verifying-claims.md).

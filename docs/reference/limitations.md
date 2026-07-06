# Known limitations &amp; gaps

This page is the honest, in-docs list of what GaugeWright **does not** do today —
written so a reviewer can read it directly without leaving the documentation. It
exists because the project's first rule is honesty: every gap below is a known,
named gap, not an oversight.

It is the companion to the [roadmap &amp; status](status.md) table, which is the
**single source of truth** for capability status. Where a capability *is* in
progress, this page says what state it's in and links back to the status table
rather than restating it.

!!! warning "The one thing to read first"
    GaugeWright orchestrates **locally**, but it **does not run inference
    locally**. The agent's reasoning is performed by the **third-party LLM
    provider you configure**, so your prompts and the in-scope
    [context](../concepts/glossary.md#context) are sent to that provider
    over the network in plaintext. This is the single highest-scrutiny data flow
    in the product. See
    [Where your data goes](../concepts/protection.md#where-your-data-goes).

## How to read the status badges

The same vocabulary is used everywhere in these docs. It defers to the
[status table](status.md):

- <span class="status available">Available</span> — shipped in the product you can
  download today.
- <span class="status built">Built</span> — implemented and tested in code, but
  **not operationally deployed** (usually waiting on infrastructure or go-live
  wiring). A Built feature is **not usable from the shipped product today.**
- <span class="status planned">Planned</span> — committed and designed, not yet
  built.
- <span class="status none">Not implemented</span> — absent today.

!!! note "What's actually usable today"
    Only the **local desktop [workbench](../concepts/glossary.md#workbench)** — build,
    run, and review agents on your own machine — is
    <span class="status available">Available</span> end-to-end. **Multi-authority
    [federation](../concepts/glossary.md#federation)** is
    <span class="status available">Available</span> in code, but its
    "Available" status is qualified: it has been **verified only in a loopback +
    NAT-isolated CI harness** (lab conditions), **not in an operationally deployed,
    cross-party setting**. Do not assume two real, independently administered
    machines collaborate in production on the strength of this row alone. Cross-party
    deployment, attested compute, enterprise identity, and hosted/embedded modes are
    <span class="status built">Built</span> or
    <span class="status planned">Planned</span>. See
    [Deployment modes](../concepts/deployment-modes.md).

---

## Limitations that affect your data and trust decision

These are the gaps a security or procurement reviewer will care about most. They
are grouped so that **structural guarantees** (machine-checked, built into how the
system works) are kept separate from **policy and operational** items (process,
infrastructure, and paperwork that are not yet in place).

### Inference goes to a third-party provider

| Limitation | Status |
|---|---|
| No local-only / on-device inference | <span class="status none">Not implemented</span> |
| Confidential inference (provider removed from the trust boundary) | <span class="status planned">Planned</span> |

The LLM provider is **inside the trust [boundary](../concepts/glossary.md#boundary)
today**: it receives prompts and in-scope context in plaintext. GaugeWright does not
bundle or run a model itself.

What this means in practice:

- You choose and authenticate the provider (e.g. OpenAI, Anthropic, Azure OpenAI).
  Its retention and training terms are the **provider's**, not GaugeWright's.
- With your own provider credentials (bring-your-own-credentials), the LLM
  relationship is *yours* — the provider is **your** subprocessor, not
  GaugeWright's.
- The agent has **no ambient authority** ([`run`](../concepts/glossary.md#run)) and
  acts only on admitted work. Be precise about network egress, because the honest
  framing matters here:
    - **Network-egress default is *open* per [project](../concepts/glossary.md#project)**,
      not deny-by-default. A chat can reach the network out of the box; the operator
      **opts into** isolation per project (`ProjectRecord.network_isolated`, default
      **off**). Most deployments therefore have open host egress unless they
      explicitly turn isolation on.
    - **Egress to the model endpoint is the known, disclosed exposure**
      (OWASP LLM02) — that is the data flow that carries your prompts and in-scope
      [context](../concepts/glossary.md#context) to the third-party provider.
    - Because the per-host egress proxy is not yet built, "open" means **unfiltered
      host egress**: opening it on a project where it is not already open requires a
      loud, explicit operator override (`UNTIE_ALLOW_UNFILTERED_EGRESS=1`) — never
      silent — and enabling `network_isolated` restores kernel-enforced network
      containment for that project.

!!! question "Can I keep data off a third-party model?"
    Not within GaugeWright today. If your data may not leave for a third-party
    model, use a provider you've contracted (so it is your subprocessor under your
    terms), or wait for **confidential inference**
    (<span class="status planned">Planned</span>), which removes the provider from
    the trust boundary. See [Protection → Where your data goes](../concepts/protection.md#where-your-data-goes).

### Encryption at rest awaits a deployed KMS

| Limitation | Status |
|---|---|
| AEAD encryption of payload at rest (AES-256-GCM) | <span class="status available">Available</span> |
| KMS-backed envelope encryption (Azure Key Vault) for server deployments | <span class="status built">Built</span> |

The local encryptor — AES-256-GCM authenticated encryption — is shipped and used
on the desktop. The **envelope-encryption** layer (a data key wrapped by a
key-encryption key held in Azure Key Vault) is implemented and has been verified
live against a real key vault, but **operational deployment needs a configured
service principal** and is therefore tied to the (not-yet-live) server modes. Treat
KMS-backed at-rest encryption as <span class="status built">Built</span>, not
something you can switch on in the shipped desktop product.

### Identity, MFA, and access control

| Limitation | Status |
|---|---|
| Multi-factor authentication (MFA) enforcement | <span class="status none">Not implemented</span> |
| Enterprise identity — OIDC / SAML verifiers | <span class="status built">Built</span> |
| SCIM **outbound** sync | <span class="status planned">Planned</span> |
| RBAC/ABAC admin console UI | <span class="status planned">Planned</span> |
| Live interop with specific IdP vendors (Okta, Entra, Google) | <span class="status planned">Planned</span> |

The shipped desktop product is single-party and **needs no account or sign-in** —
so there is no MFA to enforce on it. MFA enforcement is an organization-layer
concern and is <span class="status none">Not implemented</span> in the product
today. The OIDC and SAML id-token verifiers are
<span class="status built">Built</span> (OIDC verified per-commit against a
self-hosted Keycloak; SAML behind a hardened sidecar with single-use replay
defense), but enterprise identity as a whole is **not operationally available** —
see the [admin guide](../guides/admin/index.md) and the
[status table](status.md).

### No third-party attestation yet (SOC 2 / ISO / pen test / DPA)

| Limitation | Status |
|---|---|
| SOC 2 Type II report | <span class="status planned">Planned</span> |
| ISO/IEC 27001 certification | <span class="status none">Not implemented</span> (no roadmap committed) |
| ISO/IEC 42001 (AI management system) | <span class="status planned">Planned</span> (named future target) |
| Independent penetration test (SAML-scoped first) | <span class="status planned">Planned</span> |
| Data Processing Agreement (DPA) + published subprocessor list | <span class="status planned">Planned</span> |

There is **no independent third-party audit, certification, or penetration test
today.** The security *model* is verified by machine-checked invariants (see
below); operational *readiness* for a regulated deployment is in progress. GDPR
right-to-erasure is modeled and implemented in the reducer
(<span class="status built">Built</span>), but the accompanying DPA and bulk/admin
erasure UI are <span class="status planned">Planned</span>.

??? note "What *is* defensible today (structural vs. policy)"
    The protection guarantees are **structural** and **machine-checked** — each is
    a formal invariant paired with an adversarial "teeth" test that fails if the
    protection is removed. In plain operational terms:

    - **Handles don't grant access (`INV-10`).** Holding a
      [handle](../concepts/glossary.md#handle) (the name/reference for a
      [resource](../concepts/glossary.md#resource)) is not the same as being able to
      read its content — reading still requires explicit admission.
    - **Method and context reads are both explicit (`INV-12`).** Neither the
      [method](../concepts/glossary.md#method) definition nor its
      [context](../concepts/glossary.md#context) is read implicitly; each read is an
      authorized, recorded event.
    - **Runs have no ambient authority (`INV-11`).** A [run](../concepts/glossary.md#run)
      can act *only* on the work it was admitted to — it cannot reach for files,
      scopes, or capabilities it was not explicitly granted ("ambient authority"
      means powers a process holds just by existing; here there are none).
    - **The system is fail-closed (`INV-20`).** When a decision is ambiguous or a
      check cannot be completed, access is denied rather than allowed.
    - **A running agent cannot rewrite its own method (`INV-24`).** The
      [method](../concepts/glossary.md#method) the agent runs under is
      edit-authored; a [work chat](../concepts/glossary.md#chat) cannot mutate the
      definition it is executing.

    These are claims about how the code behaves, not policy promises. **`INV-24` is
    structurally enforced at the OS kernel only on Linux/macOS — see the per-OS note
    below.** The items in *this* section are **policy and operational** —
    certifications, audits, and paperwork — which a formal model cannot supply. For
    the full invariant → control crosswalk, see the
    [security documentation](../security.md).

---

## Supply-chain and build limitations

These are the most actionable near-term hardening items. The dependency lockfile
is pinned and the build is reproducible, but several standard supply-chain controls
are not yet wired in.

| Limitation | Status |
|---|---|
| Pinned dependency lockfile (`Cargo.lock`) | <span class="status available">Available</span> |
| Reproducible build + measurement digest (`flake.nix`) | <span class="status built">Built</span> |
| Code signing / notarization of desktop builds | <span class="status none">Not implemented</span> — **builds are unsigned** |
| SBOM (CycloneDX / SPDX) | <span class="status none">Not implemented</span> |
| SLSA provenance attestation | <span class="status none">Not implemented</span> |
| Dependency CVE scanning (`cargo audit` / Dependabot) | <span class="status none">Not implemented</span> |
| SAST / secret scanning in CI | <span class="status none">Not implemented</span> |

!!! warning "Desktop builds are unsigned"
    Downloaded desktop builds are **not code-signed or notarized** today. On macOS
    and Windows your OS will warn that the publisher is unverified, and you must
    explicitly allow the app to run. Verify you obtained the build from the
    official download page before allowing it.

---

## Operational limitations

These concern running GaugeWright as a service, and reflect that **no server mode
is operationally deployed yet** — the shipped product is a local desktop app.

| Limitation | Status |
|---|---|
| Production monitoring / alerting / observability | <span class="status none">Not implemented</span> |
| Incident-response runbooks / procedures | <span class="status none">Not implemented</span> |
| Uptime / SLA + status page | <span class="status planned">Planned</span> |
| Cryptographic (signature / merkle-chain) tamper-evidence of the audit log | <span class="status planned">Planned</span> |
| Platform rate-limiting / quotas | <span class="status planned">Planned</span> (limited today) |

The append-only audit log is enforced **semantically** (an immutable event log) and
exportable to your SIEM (<span class="status available">Available</span>), but it is
**not yet cryptographically** tamper-evident — there is no signature or merkle
chain over log entries, and cross-party log non-repudiation is a
<span class="status planned">Planned</span> federation guarantee. There is **no
production observability, alerting, or incident-response procedure** in the product
today.

---

## Platform and per-OS limitations

Some structural guarantees depend on an OS kernel sandbox, which is **not
available on every platform**.

=== "Linux / macOS"

    Kernel-enforced [method](../concepts/glossary.md#method) isolation is
    <span class="status available">Available</span>. The method-definition surface
    runs read-only at the kernel, so even a shell inside a
    [run](../concepts/glossary.md#run) cannot rewrite the agent's own method
    (`INV-24`).

=== "Windows"

    The kernel sandbox that enforces [method](../concepts/glossary.md#method)
    isolation is <span class="status planned">Planned</span> — **not implemented on
    Windows today.** Until it ships, the OS-kernel-level protection described above
    is not in force on Windows.

    **Running untrusted methods on Windows today provides no kernel-enforced method
    isolation.** Method definitions are **not read-only at the OS level**, so a
    running agent could theoretically rewrite the method definition or escape
    containment bounds — only mitigated by the actor's operational discipline (process
    and container controls), **not** by the OS sandbox. This is a critical asymmetry
    versus Linux/macOS that you must account for when assessing compliance risk.

    !!! note "`INV-24` on Windows is a design guarantee, not a structural one"
        Until the Windows kernel sandbox ships, method integrity (`INV-24`) is **not
        enforced at the OS level on Windows** — the guarantee holds in logic and
        design but **operationally depends on process and container controls, not
        structural guarantees.** Treat method integrity on Windows as defended by
        operational discipline, not by the platform.

Cross-party modes carry their own platform notes:

| Limitation | Status |
|---|---|
| Windows method-isolation sandbox | <span class="status planned">Planned</span> |
| Cross-party packaging &amp; deployment (live) | <span class="status built">Built</span> · <span class="status planned">live Planned</span> |
| Attested compute (confidential VM) — live host | <span class="status built">verifier Built</span> · <span class="status planned">live Planned</span> |
| Hosted multi-tenant platform | <span class="status planned">Planned</span> |
| Public hosting / embedded agents | <span class="status planned">Planned</span> |

The SEV-SNP [attestation](../concepts/glossary.md#attestation) story is **asymmetric**,
and the asymmetry matters for anyone planning an enterprise deployment:

- **Verification works.** The SEV-SNP attestation **verifier** is
  <span class="status built">Built</span> and tested against **real Milan vectors** —
  you *can* verify someone else's attestation report (parse the quote, check the
  measurement against the registry).
- **Generation does not, yet.** You **cannot generate a fresh attestation quote**
  without a **confidential VM**, which is not operationally available. So live
  attested compute — actually *producing* a trustworthy quote from a running host —
  is <span class="status planned">Planned</span>.

Cross-party deployment (the [package](../concepts/glossary.md#package) and
[deploy](../guides/expert/package-and-deploy.md) path) is implemented in the core but
**not operationally live** — see
[Package &amp; deploy](../guides/expert/package-and-deploy.md).

---

## Certification timeline framing

The honest framing: **the protection model is verified; the certifications that
attest to it operationally are not yet in place.**

- **Highest priority** is **SOC 2 Type II** — it is also the intended trust source
  for the own-built SSO/SCIM stack (rather than a third-party identity broker).
  <span class="status planned">Planned</span>
- A **SAML-scoped penetration test** and a **DPA with a published subprocessor
  list** are committed alongside it. <span class="status planned">Planned</span>
- **ISO/IEC 27001** has **no committed roadmap** yet.
  <span class="status none">Not implemented</span>
- **ISO/IEC 42001** (AI management system) is named as a future target.
  <span class="status planned">Planned</span>

There is no published completion date for these; this page and the
[status table](status.md) will move them to a later badge as they land. Treat any
of them as **not yet available** when making a procurement decision.

---

## Where to go next

- **[Roadmap &amp; status](status.md)** — the single capability-status table this
  page defers to.
- **[How GaugeWright protects your work](../concepts/protection.md)** — the
  structural guarantees, and where your data goes.
- **[Deployment modes](../concepts/deployment-modes.md)** — what's usable in each
  mode.
- **[Security &amp; trust](../security.md)** — the reviewer-grade architecture,
  threat model, and control crosswalk.
- **[Concepts](../concepts/index.md)** · **[Glossary](../concepts/glossary.md)** —
  the vocabulary used throughout.

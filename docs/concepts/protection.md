# How GaugeWright protects your work

GaugeWright exists to let an expert's **method** and a client's **context** meet
without either leaking — the method doesn't escape to the client, and the context
doesn't escape to the method-owner or the runtime. Protection is **structural**:
built into how the system records and runs work, not a setting you can forget to
turn on.

This page is the plain-language boundary contract every other page links back to.
It answers two questions a reviewer actually asks: **where does my data go**, and
**which guarantees are real today versus modeled or planned**. Each claim carries a
status badge. The single source for those badges is the
[roadmap & status](../reference/status.md) page; nothing here is stated in usable
present tense without the right caveat.

!!! warning "The one counterintuitive truth — read this first"
    **Local orchestration is not local inference.** Today the only shipped product
    is the **local desktop [workbench](glossary.md#workbench)**
    <span class="status available">Available</span> — it runs on *your* machine and
    stores work *locally*. But the agent's *reasoning* is performed by the
    **third-party LLM provider you configure**, so your prompts and the in-scope
    [context](glossary.md#context) are sent to that provider over the
    network. **There is no local-only inference.** See
    [Where your data goes](#where-your-data-goes).

## The boundary in one paragraph

A [project](glossary.md#project) is a trust and data boundary — a body of work plus
the parties allowed to see it. Inside it, every protected thing is a
[resource](glossary.md#resource) (a method, a context, or a derived
[output](glossary.md#output)), addressed by [handle](glossary.md#handle), never by
raw content. Work happens as a [run](glossary.md#run) inside the
[boundary](glossary.md#boundary) — the single point where what a run may read,
call, and export is enforced. A run gets no ambient power; it acts only on what it
was admitted to do. That is the whole shape; the rest of this page is what makes it
hold.

---

## Where your data goes

This is the anchor other pages point at. Read it before you trust GaugeWright with
anything sensitive.

| Stays on your machine | Leaves your machine |
|---|---|
| The event log (your history), resources, handles, and stored payloads — held locally by the workbench. <span class="status available">Available</span> | Your **prompts and the in-scope context** for each run, sent to the **third-party LLM provider you configure** for inference. <span class="status available">Available</span> |
| Orchestration, [run](glossary.md#run) admission, and the [boundary](glossary.md#boundary) decisions — all local. <span class="status available">Available</span> | Anything you explicitly move to another party via [federation](glossary.md#federation) — only with the source's permission *and* the target's admission. <span class="status available">Available</span> |

**Who can see plaintext, plainly:**

- **You can** — it's your machine, your log, your data.
- **Your configured LLM provider can** see the prompts and in-scope context you
  send it for reasoning. Its retention and training terms are the **provider's**,
  not GaugeWright's.
- **The other party in a [federation](glossary.md#federation)** can see only what
  you permitted to leave and they admitted — never your whole project.
- **A federation relay cannot** read payload: cross-machine traffic is
  certificate-pinned TLS through a blind rendezvous relay that routes encrypted
  bytes and never sees plaintext. <span class="status available">Available</span>

!!! note "Your provider is your subprocessor"
    When you authenticate with **your own** provider credentials, the LLM
    relationship is *yours* — the provider is **your** subprocessor, not
    GaugeWright's. You choose OpenAI, Anthropic, Azure OpenAI, or another, and the
    contract/retention terms that govern your data are the ones you accepted with
    them.

!!! warning "If your data may not leave for a third-party model"
    Use a provider you have contracted (with the data terms you need), or wait for
    **confidential inference** — taking the provider out of the trust boundary —
    which is on the roadmap. <span class="status planned">Planned</span>

??? question "How do I see or change which provider gets my data?"
    The model/provider is part of the [archetype](glossary.md#archetype)'s
    configuration, edited from an **edit chat** (a chat rooted on the archetype),
    in the `.agent-config.json` definition surface. Provider credentials are
    supplied as environment keys (e.g. your provider's API-key variable). To change
    where inference goes, open the archetype's edit chat and update its
    configuration — never from a work chat. See
    [Build an agent](../guides/expert/build-an-agent.md).

---

## Two kinds of guarantee — don't conflate them

GaugeWright's claims fall in two buckets. Keeping them separate is what keeps the
claims defensible.

=== "Structural (machine-checked)"

    These are stated as formal **invariants** and verified in **Quint** models with
    machine-checked properties, each paired with an adversarial "teeth" probe that
    must fail if the protection is removed. The pure core is property-tested against
    those models. This is the bucket most vendors cannot claim.

    > These are *modeled and tested* guarantees about the core's logic. They are a
    > strong foundation, but a third-party attestation that lets a reviewer take
    > them on trust (SOC 2, penetration test) is **not yet in hand** — see
    > [limitations](#honest-limitations-and-known-gaps).

=== "Policy / operational"

    These are configurable rules and deployment behaviors (org policy, encryption
    infrastructure, audit export). They are real, but they are *settings and
    integrations*, not invariants — some have the code seam built while the external
    half (a KMS, a SIEM connector, a confidential VM) is not yet wired live.

### The structural guarantees (machine-checked)

Each line is an invariant of the local desktop product.

- **A handle is not access.** Holding or transporting a
  [handle](glossary.md#handle) conveys no read of the payload; reading requires a
  separate, explicit grant evaluated at the boundary. (INV-10)
  <span class="status available">Available</span>
- **Method and context reads are both explicit.** Neither is readable just because
  code is running inside the boundary; each needs its own grant. (INV-12)
  <span class="status available">Available</span>
- **A run has no ambient authority.** It acts only on the work, handles, and
  export basis it was admitted with — no power to read, call, retain, reveal, or
  export beyond that. (INV-11) <span class="status available">Available</span>
- **Fail-closed.** If a required grant is missing, stale, or uncertain, the action
  is **denied** — never allowed on doubt. (INV-20, model `fail-closed.qnt`)
  <span class="status available">Available</span>
- **Append-only, immutable history.** Every durable fact is an immutable event;
  corrections are new events, never edits. Every run is auditable and reversible.
  (INV-6) <span class="status available">Available</span>
- **Revocation stops the future, not the past.** Revoking a grant blocks use from
  that point on; it never rewrites prior events. (INV-18)
  <span class="status available">Available</span>
- **Cross-party movement is two-key.** Nothing crosses authorities without the
  **source** permitting it to leave **and** the **target** admitting it. (INV-13;
  relays never become payload authorities, INV-14)
  <span class="status available">Available</span>

!!! warning "Kernel-enforced method isolation is Linux/macOS only"
    **A working agent cannot rewrite its own method.** The definition surface —
    `.pi/SYSTEM.md`, `AGENTS.md`, `.agent-config.json` — is writable **only** from
    an [edit chat](glossary.md#chat) (rooted on the archetype). A
    [work chat](glossary.md#chat) (rooted on a [placement](glossary.md#placement))
    may *read* the definition but never mutate it — so a running agent can't
    escalate by loosening its own policy or rewriting its own system prompt.
    (INV-24, model `method-integrity.qnt`)

    This is enforced by an **OS sandbox at the kernel** that makes the surface a
    read-only root — so *every* write path, including a shell `bash` call inside a
    run, is blocked by the kernel, not by gating individual tools.

    - **Linux / macOS:** <span class="status available">Available</span>
    - **Windows:** the kernel sandbox is <span class="status planned">Planned</span>.
      Until it ships, run untrusted methods on Linux or macOS.

    **What this does *not* do — the load-bearing honest limitation.** This stops a
    *working agent* from rewriting its own method. It does **not** hide the method's
    plaintext from the **host**. At an **unattested local desktop placement** the
    host *is* the context owner and the method endpoint sits in its trusted computing
    base — so the host can read the method's definition in the clear. Method secrecy
    from the operator is **not achievable at a local placement** (only obfuscation).
    It activates only at an **[attested](glossary.md#attestation) placement**, which
    is host-blind regardless of who runs it — and attested compute is
    <span class="status built">verifier Built</span>
    <span class="status planned">live Planned</span>, not Available today.
    (`boundary.qnt`: `METHOD_HIDDEN_FROM_B` *fails* at an unattested local placement
    and passes only when attested.)

### The policy / operational controls

These are the organization-facing and infrastructure controls. Read the status
badge on each — several have the code seam built but await an external integration.

| Control | What it gives you | Status |
|---|---|---|
| **Restrict-only org policy** | Enterprise policy (e.g. `viewer ⇒ no export`, `pii ⇒ attested + same-region`) can only **narrow** the verified protection floor, never widen it — proven monotone in Quint (`abac.qnt`). | <span class="status built">Built</span> |
| **Egress containment** | Every channel out of the boundary is mediated through a single chokepoint, and taint/consent gate disclosure. Network reach itself, though, is **open by default** per project — a chat can reach the model out of the box; deny-by-default is the *intent* you opt into per project (`network_isolated`, off by default). The per-host **model-endpoint allowlist** (the egress proxy that would make "deny-by-default with an allow-list" literally true) is **deferred infrastructure**, so today "isolated" means kernel-enforced *unfiltered* containment, not a host allowlist. | <span class="status built">seam Built</span> <span class="status planned">allowlist Planned</span> |
| **Per-actor audit trail + export** | Every governance action is recorded immutably, attributed to the authenticated actor, and exportable (CSV/JSON, references only). | <span class="status available">Available</span> |
| **SIEM streaming** | Stream the audit log to Splunk/Datadog. The streaming sink is built; the exporter attaches behind it per deployment. | <span class="status built">Built</span> |
| **Encryption in transit** | Cross-machine federation runs over cert-pinned TLS through a blind relay. | <span class="status available">Available</span> |
| **Encryption at rest** | AES-256-GCM seam is built; the KMS-managed data key (server deployments) is the deferred infra half. | <span class="status built">Built</span> |
| **Enterprise identity** | OIDC SSO, SCIM provisioning, RBAC (default-deny, `rbac.qnt`), per-actor audit. | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| **Attested compute** | A run sealed in a confidential VM (AMD SEV-SNP) with a verifiable quote that the method and context stayed sealed. The quote *verifier* is built and tested against genuine AMD material; live quote generation + hardware-bound key release run on a confidential VM. | <span class="status built">verifier Built</span> <span class="status planned">live Planned</span> |
| **MFA enforcement** | Org-level require-MFA factor. | <span class="status none">Not implemented</span> (policy modeled; factor would be enforced by your IdP) |

For the full reviewer-grade detail behind each row — the invariant→control
crosswalk (SOC 2 / ISO 27001 / NIST), data-flow diagrams, and threat model — see
[Security & trust](../security.md).

---

## How to keep your work protected

Concrete, ordered steps. Each names the real artifact you touch.

### 1. Keep method edits in the edit chat

1. Open the [archetype](glossary.md#archetype) you want to change.
2. Start (or open) its **edit chat** — the chat rooted on the archetype itself.
3. Edit the definition surface there: `.pi/SYSTEM.md` (instructions), `AGENTS.md`,
   and `.agent-config.json` (skills, tools, model/provider).
4. To *use* the agent, switch to a **work chat** (rooted on a [placement](glossary.md#placement)). It can
   read the method but the kernel sandbox blocks any write — so a running agent
   cannot alter its own instructions. (Linux/macOS; Windows sandbox is
   <span class="status planned">Planned</span>.)

### 2. Control what inference sees

1. In the archetype's edit chat, open `.agent-config.json`.
2. Set the **model/provider** and reference your provider's API-key environment
   variable — do not paste secrets into instruction files.
3. Remember: whatever the run reads as in-scope context is what gets sent to that
   provider. Scope the context to what the work needs. See
   [Where your data goes](#where-your-data-goes).

### 3. Review before anything is released

1. Generating an [output](glossary.md#output) does **not** release it — outputs are
   held until reviewed. (INV-16)
2. The context-owner (or a designated reviewer) approves release to authorized
   stakeholders; release requires consent from every stakeholder.
3. See [Review & release outputs](../guides/client/review-and-release.md).

### 4. Audit and, if needed, revoke

1. Every action lands in the append-only log; filter by actor or action.
2. Export the (authorization-scoped) timeline as CSV/JSON — references only, never
   protected payload.
3. Revoking a grant blocks future use immediately; prior history stays intact
   (INV-18). To make a payload unresolvable for erasure, use content tombstoning —
   the audit fact remains.

---

## Honest limitations and known gaps

Stated here, not hidden behind a link:

- **No local-only inference.** Inference always goes to your configured
  third-party provider. Confidential inference is
  <span class="status planned">Planned</span>.
- **Windows method-isolation sandbox is not shipped.** Kernel-enforced method
  isolation is Linux/macOS only today. <span class="status planned">Planned</span>
  for Windows.
- **Method plaintext is visible to the local host.** Kernel isolation stops a
  *working agent* from rewriting its own method, but at an unattested local
  desktop placement the host is the context owner and can read the method's
  definition in the clear. Hiding the method from the operator requires an
  **[attested](glossary.md#attestation) placement** (host-blind), which is
  <span class="status built">verifier Built</span>
  <span class="status planned">live Planned</span>, not Available today.
- **Cross-party deployment is not live.** Packaging, attested compute, enterprise
  identity, and hosted/embed modes are <span class="status built">Built</span> or
  <span class="status planned">Planned</span>, not operationally available — see
  the [roadmap](../reference/status.md) and
  [deployment modes](deployment-modes.md).
- **No third-party certifications yet.** SOC 2 Type II, a DPA with a published
  subprocessor list, and an independent penetration test are committed and
  prioritized but <span class="status planned">Planned</span>.
- **Young-product gaps.** Unsigned builds (code-signing/notarization in setup), no
  published SBOM/dependency scanning, no production monitoring. See the
  [FAQ](../faq.md) and [Security & trust](../security.md).

---

## Keep reading

- **[Concepts](index.md)** · **[Glossary](glossary.md)** — the vocabulary used here.
- **[Deployment modes](deployment-modes.md)** — the same protection model in every mode.
- **[Roadmap & status](../reference/status.md)** — the single source for every status badge.
- **[Security & trust](../security.md)** — reviewer-grade controls and crosswalk.
- Role guides: **[For clients](../guides/client/index.md)** ·
  **[For experts](../guides/expert/index.md)** ·
  **[For admins](../guides/admin/index.md)**.

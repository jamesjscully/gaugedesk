# Deployment modes

The *same* [protection model](protection.md) applies in every mode — governance is
**added, never re-architected**. What differs between modes is three things only:

1. **Where the agent runs** — whose machine executes the [run](glossary.md#run).
2. **Who is inside the trust boundary** — who *can* see plaintext prompts and
   in-scope [context](glossary.md#context).
3. **Operational status** — what you can actually use today versus what is built or
   planned.

!!! warning "The one fact every mode shares: inference is remote"
    GaugeWright orchestrates **locally**, but it does **not** run a local model.
    Every mode sends the agent's prompts and the in-scope context to the
    **third-party LLM provider you configure** (OpenAI, Anthropic, or Azure
    OpenAI), in plaintext, over the network. The provider is **inside the trust
    boundary in every mode shipping today.** Removing it
    ([confidential inference](#confidential-inference)) is
    <span class="status planned">Planned</span>. See
    **[Where your data goes](protection.md#where-your-data-goes)** before reading
    further — it is the single most important caveat on this page.

This page is the per-mode **security matrix**: for each mode, where the agent runs,
who is in the trust boundary, and its honest [status](../reference/status.md). It is
written for an admin or security reviewer who needs to know exactly who can see
plaintext in each posture.

---

## Status vocabulary

This page uses the project's single status vocabulary; the canonical table lives in
**[Roadmap &amp; status](../reference/status.md)**, which **wins** if any badge here
ever disagrees.

- <span class="status available">Available</span> — shipped in the product you can
  download and use today (the local desktop workbench).
- <span class="status built">Built</span> — implemented and tested in the codebase,
  but **not operationally deployed** (waiting on infrastructure or go-live wiring).
  Not a usable end-user feature today.
- <span class="status planned">Planned</span> — committed and designed, not yet built.
- <span class="status none">Not implemented</span> — absent today.

!!! note "Only one mode is usable end-to-end today"
    The **local desktop workbench** is the only fully
    <span class="status available">Available</span> mode. Federation is
    <span class="status available">Available</span> for a **self-federated device set
    you own**, but its **cross-party** infrastructure is metered and not live. Hosted,
    attested, and embed are <span class="status built">Built</span> in code or
    <span class="status planned">Planned</span>. Do not read a Built badge as "you can
    switch this on today."

---

## The security matrix

Read each row as: *in this mode, the agent executes on `<machine>`, and the parties
able to read plaintext are `<trust boundary>`.* The **LLM provider is always in the
trust boundary** — that column is therefore implied in every row, and called out
explicitly where it is the *only* external reader.

| Mode | Where the agent runs | Who is in the trust boundary (can see plaintext) | Status |
|---|---|---|---|
| **[Local desktop](#local-desktop)** | Your own machine; storage local; inference to your configured provider | **You** (machine owner) **+ your LLM provider** | <span class="status available">Available</span> |
| **[Federation](#federation) (self-federated devices)** | Your own devices, paired peer-to-peer over cert-pinned TLS | **You** (across your devices) **+ your LLM provider**. A [relay](glossary.md#relay) routes opaque bytes and is **not** in the boundary | <span class="status available">Available</span> |
| **[Federation](#federation) (cross-party)** | The **[host](glossary.md#host)** authority's machine (e.g. the client's box) | **The host machine's owner + the LLM provider.** The operator (other party) drives but reads no payload by holding a handle | <span class="status built">Built</span> infra metered, not live |
| **[Hosted multi-tenant](#hosted-multi-tenant)** | A cloud relay + compute platform we operate | **The platform operator + the LLM provider** (a non-attested host reads plaintext) | <span class="status planned">Planned</span> |
| **[Attested compute](#attested-compute)** | A sealed confidential VM (AMD SEV-SNP) — neutral cloud, or a counterparty's CVM-capable box | **Only the LLM provider** — the **host is removed** from the trust boundary by attestation | <span class="status built">verifier Built</span> · <span class="status planned">live Planned</span> |
| **[Public hosting / embed](#public-hosting--embed)** | An always-on hosted deployment behind the consultant's website | **The consultant + the platform operator + the LLM provider.** End-users are scoped, non-authority principals | <span class="status built">core Built</span> · <span class="status planned">live Planned</span> |

!!! note "Structural guarantees vs. operational ones"
    The trust-boundary column splits into two kinds of claim, and a reviewer should
    keep them separate:

    - **Structural / machine-checked** — "a handle is not access," "no ambient
      authority," "fail-closed," "a relay can't read payload" — these are formal
      [invariants](protection.md#two-kinds-of-guarantee-dont-conflate-them)
      (`INV-10`, `INV-11`, `INV-14`, `INV-20`) backed by Quint models and adversarial
      tests. They hold by construction.
    - **Policy / operational / infrastructure** — "the host is removed from the trust
      boundary" depends on a **live TEE + key-release service** that is
      <span class="status planned">Planned</span>; "encryption at rest" depends on a
      live KMS whose code seam is <span class="status built">Built</span> but is not
      live. These are true only once the infrastructure is stood up.

---

## Local desktop

<span class="status available">Available</span>

You build and run agents on **your own machine**. Storage is local: an append-only
event log plus a git-backed content store. Inference calls the
[LLM provider you configure](protection.md#where-your-data-goes).

=== "Where the agent runs"

    Your machine, in an [OS-sandboxed](glossary.md#boundary) Pi subprocess. Nothing
    executes off-box except the model call.

=== "Trust boundary"

    **You** (the machine owner) and **your LLM provider**. No other party is involved.
    Because you own the box, this mode makes **no method-secrecy claim against you** —
    you can read anything on your own machine.

=== "Structural guarantees here"

    - A running [work chat](glossary.md#chat) **cannot rewrite its own method** — the
      [archetype](glossary.md#archetype) definition (`.pi/SYSTEM.md`, `AGENTS.md`,
      `.pi/extensions/`, `.agent-config.json`) is editable only from an *edit chat*,
      enforced by a **kernel sandbox** (`INV-24`).
      <span class="status available">Available (Linux/macOS)</span>; the Windows
      method-isolation sandbox is <span class="status planned">Planned</span>.
    - Network egress is **open by default** in the sandbox unless the project opts
      into isolation. The operator can enable per-project network containment via
      `ProjectRecord.network_isolated` (default off); when enabled, declared model
      hosts record intent and kernel-enforced egress is deny-by-default to those
      hosts only. Only a loud, explicit operator override
      (`UNTIE_ALLOW_UNFILTERED_EGRESS=1`) opens unfiltered egress.

    !!! warning "Honest ceiling: the default-open sandbox does not contain network reach"
        Because the per-host egress proxy is deferred (no model-endpoint allowlist
        yet), "open" means **unfiltered host egress**: with the default posture the
        sandbox does **not** contain a [run](glossary.md#run)'s network reach. This
        is a deliberate low-friction trade-off, not a relaxation of the
        [boundary](glossary.md#boundary) membrane — every channel is still mediated
        and taint/consent still gate disclosure — but it lowers the *honest ceiling*
        for local desktop mode until the project opts into isolation. Enabling
        `ProjectRecord.network_isolated` restores kernel-enforced network
        containment.

??? question "How do I confirm where my prompts go?"
    1. Open the archetype's **edit chat** and inspect `.agent-config.json` — it names
       the model/provider selection your runs use.
    2. The provider you authenticate there (via your provider's API-key environment
       variable) is the **only** external party that sees plaintext in local mode. Its
       retention and training terms are the *provider's*, not GaugeWright's — see
       [Where your data goes](protection.md#where-your-data-goes).

**Next:** [Build an agent](../guides/expert/build-an-agent.md) ·
[Run &amp; review work](../guides/expert/run-and-review.md).

---

## Federation

Self-federated devices: <span class="status available">Available</span> ·
Cross-party infrastructure: <span class="status built">Built</span> (metered, not live)

[Federation](glossary.md#federation) moves resources, commands, events, and outputs
**between authorities** — across machines or parties. Nothing crosses without the
**source** permitting it to leave *and* the **target** admitting it (`INV-13`).

=== "Where the agent runs"

    On the **host** authority's machine — the authority whose event log owns the
    [project](glossary.md#project). After a **handoff**, the project's single home
    relocates to the host (e.g. a consultant hands a project to a client, where the
    data and runs live). The **operator** (the other party) can *drive* work, but
    every crossing is admitted by the host.

=== "Trust boundary"

    - **The host machine's owner** sees plaintext that lives on the host (at rest or
      at execution). This is explicit in the spec: ownership is enforced as
      **licensing, not secrecy** — a payload owner can *revoke* access at any time
      (`INV-18`, future-only, fail-closed `INV-20`), but a payload present on a
      machine **is readable by that machine's owner**.
    - **The relay is NOT in the trust boundary.** It is a *transport role*: a dumb
      rendezvous relay piping cert-pinned TLS, or a WireGuard overlay forwarding
      opaque packets. It can prove routing happened but **cannot read payload or
      become a payload authority** (`INV-14`, `INV-10`) — a structural guarantee.
    - **Your LLM provider**, as always.

=== "Structural guarantees here"

    - **Both sides are load-bearing.** A crossing needs source permission **and**
      target admission; a relay receipt or delivery attempt is neither (`INV-13`).
    - **Encryption in transit.** Cross-machine federation runs over **cert-pinned
      TLS** through a blind rendezvous relay that never sees plaintext.
      <span class="status available">Available</span>
    - **Authority identity is a signed governance keypair.** Crossings are signed by
      short-lived **device subkeys** chaining to a root pinned in the bridge grant; a
      forged source is rejected (`INV-21`), and a compromised device is bounded by
      subkey expiry or a root-signed revocation pushed over the bridge.

!!! warning "What federation does NOT give you today"
    - Federation across **your own devices** is <span class="status available">Available</span>.
    - **Cross-party** federation (relayed crossings beyond a device set you own) is a
      **commercially metered** capability whose hosted relay infrastructure is **not
      live** — the code is <span class="status built">Built</span> and verified on a
      loopback + NAT-isolated CI harness, but you cannot broker trust between two
      strangers' machines over our infrastructure today.
    - Federation **does not** seal the host owner out of the payload. If you need the
      host to *run but not read* the other party's method or data, that requires
      [attested compute](#attested-compute) — federation alone protects by
      revocation, not secrecy.

**Next:** [Package &amp; deploy](../guides/expert/package-and-deploy.md) for the
cross-party deploy flow.

---

## Hosted multi-tenant

<span class="status planned">Planned</span> (needs infrastructure)

A cloud-hosted relay **plus compute** platform that runs consultants' deployments, so
neither party has to keep a machine online. This is the production data plane that
hosted/managed-SaaS operation depends on.

=== "Where the agent runs"

    On compute **we operate** in the cloud, not on either party's machine.

=== "Trust boundary"

    On a **non-attested** hosted host, the **platform operator** is inside the trust
    boundary (the host reads plaintext to run the agent), **plus the LLM provider**.
    To remove the platform operator from the boundary you must combine hosting with
    [attested compute](#attested-compute).

!!! note "Why this is Planned, not Built"
    Hosted operation depends on standing up the production data plane: a status page,
    backup/restore + DR, regional data-plane isolation, and the per-deployment microVM
    host. The protection *model* is unchanged from local; the *infrastructure* is the
    missing half. See [Roadmap &amp; status](../reference/status.md).

---

## Attested compute

<span class="status built">verifier Built</span> · <span class="status planned">live host Planned</span>

The agent runs inside a **sealed confidential VM** (AMD SEV-SNP) that attests its
launch **measurement**. Both parties can verify a cryptographic proof that the
attested code — and only the attested code — handled the method and context. This is
the only mode that **removes the host operator from the trust boundary.**

=== "Where the agent runs"

    In a confidential VM running the full node (shell + Pi subprocess + git +
    filesystem + pinned egress) largely unmodified. The **first target is a neutral
    cloud CVM**; a **counterparty-hosted** CVM follows on the same mechanism once that
    party has CVM-capable hardware. Attestation is **orthogonal** to *who* operates
    the host — you can be client-hosted **and** attested.

=== "Trust boundary"

    **Only the LLM provider.** After attestation the residual trust boundary is
    *{the attested GaugeWright code, the model provider}* — **not the host**. With
    cert-pinned TLS *inside* the attested code, even a host that controls the network
    cannot MITM the model egress.

    > **The expert's method can run on the client's own hardware and the client cannot
    > see the method** — it is encrypted in TEE memory, the code is attested, egress
    > is pinned. The only party seeing plaintext prompts is the LLM provider.

=== "What is Built vs. Planned"

    - <span class="status built">Built</span> — a **real AMD SEV-SNP quote verifier**
      (ARK→ASK→VCEK chain + ECDSA-P384 signature), tested against genuine AMD material;
      a nonce-bound, measurement-checking acceptance gate; and the **entitlement-gated
      sealed-key-release** logic (the issuer refuses to seal a run without a valid,
      unexpired deployment entitlement — the security checkpoint and the billing meter
      are the **same** checkpoint).
    - <span class="status planned">Planned</span> — **live** quote generation +
      hardware-bound key release on a running confidential VM, the production KMS, and
      the reproducible-build tooling that publishes the measurement a party checks the
      quote against.

!!! warning "Attestation removes the host — not the model provider"
    A TEE protects memory from the **host operator**, but the prompt still reaches the
    **model provider in plaintext**. So attestation removes the *host* from the trust
    boundary, **not the model provider.** Closing that last gap is
    [confidential inference](#confidential-inference) (the model also runs attested) —
    a later increment, deliberately *not* the first ceiling, so we never imply a
    "hidden from everyone" guarantee we cannot yet make.

??? note "For the spec reader (optional)"
    The ceiling is defined by the [boundary](glossary.md#boundary) lifecycle:
    `declareCeiling(Attested) → each participant verifies a fresh quote →
    acceptBoundary → admit resources`. The placement model is two axes —
    **operator** (`Local`/`Counterparty`/`Neutral`) × **`attested: bool`** — and the
    method-secrecy ceiling depends only on the attested bit (ADR 0040; `boundary.qnt`
    `METHOD_HIDDEN_FROM_B`).

---

## Public hosting / embed

<span class="status built">core Built</span> · <span class="status planned">live Planned</span>
(MVP scope — see the [embed status table](../reference/status.md#public-hosting-embed-mvp-vs-later))

A browser-embeddable agent a consultant publishes behind their **own website**, for
their **end-users**. Each visitor session is isolated, behind an origin allowlist and
budget caps.

=== "Where the agent runs"

    On an **always-on hosted deployment** (the [hosted](#hosted-multi-tenant) data
    plane), one ephemeral per-visitor microVM per session. A visit is a sequence of
    *local* runs sharing one boundary — **not** a cross-authority crossing: the
    visitor is a principal *inside* the consultant's authority, never a foreign
    authority.

=== "Trust boundary"

    The **consultant** (the responsible authority, `INV-1`), the **platform operator**
    (a non-attested host, unless combined with attestation), and the **LLM provider**.
    The **end-user is a scoped, non-authority [audience](glossary.md#audience)** —
    identified but never an authority, never minting an [account](glossary.md#account)
    keypair.

=== "Structural guarantees here"

    - **Composition is scope.** The embed is built from web-component panels; the
      **panel set chosen is the redaction**. A panel beyond the granted ceiling **does
      not render** (`INV-20`) — fail-closed is a *designed* state, not a crash.
    - **Isolation is total.** One end-user never sees another's session or chats, and
      no panel exposes the consultant's private workspace or method (`INV-22`).
    - **Handles are not payload.** Downloads cross only through resource-export; a
      shown handle conveys no bytes (`INV-10`).

!!! warning "Honest-ceiling disclosure for embed"
    The consultant's Deploy Config carries a **required credential-ceiling
    acknowledgement**: by default the agent runs on the consultant's **sealed model
    credential on a non-attested host**, so the **platform operator can read
    plaintext**. The [attested host](#attested-compute) is the premium alternative.
    The deploy flow *must not* let the consultant deploy without acknowledging this.

The two end-user modes (per the embed spec):

- **Anonymous** — ephemeral, identity-less; on teardown the conversation is
  **discarded** (one session-occurred fact + a retained transcript only).
- **Authenticated** — durable, resumable, identity-scoped; the conversation persists
  as a [chat](glossary.md#chat) keyed to the end-user in **my-chats**.

!!! note "What \"core Built\" means here"
    The audience-identity seam, the durable-chat data layer, the scoped session, and
    the `<gw-session>`/`<gw-chat>` web-component elements are implemented and tested in
    code. They **cannot run end-to-end** without the managed host that serves live
    per-visitor sessions, so the surface stays
    <span class="status planned">Planned</span> for end-users. The MVP/later split is
    in the [embed status table](../reference/status.md#public-hosting-embed-mvp-vs-later).

**Next:** [For embedded end-users](../guides/embed/index.md).

---

## Confidential inference

<span class="status planned">Planned</span>

The single capability that would **remove the LLM provider from the trust boundary** —
the model also runs attested, on confidential GPUs, so prompts never reach a provider
in plaintext. Until this ships, the truth in the warning at the top of this page holds
in **every** mode: your prompts and in-scope context go to the third-party provider
you configure.

!!! note "If your data may not leave for a third-party model"
    Use a provider you have **contracted** (so the LLM relationship is *yours* — the
    provider is your subprocessor, not GaugeWright's), or wait for confidential
    inference. See [Where your data goes](protection.md#where-your-data-goes).

---

## Known gaps and limitations

Stated here, not only behind the trust site:

- **Only local desktop is usable end-to-end today.** Federation across your own
  devices is live; cross-party, hosted, attested-live, and embed are not.
- **The LLM provider is in the trust boundary in every shipping mode.** No mode hides
  prompts from the model provider until [confidential inference](#confidential-inference).
- **Encryption at rest** (KMS-backed) has its code seam
  <span class="status built">Built</span> but the live KMS half is deferred; the
  **SIEM** streaming sink is built and the exporter attaches per deployment.
- **Windows method isolation** is <span class="status planned">Planned</span>; the
  kernel-enforced method sandbox is **Linux/macOS only** today.
- **MFA enforcement** as a first-party factor is
  <span class="status none">Not implemented</span> — MFA is enforced by your IdP under
  enforce-SSO.
- **No third-party attestations yet** — SOC 2 Type II, DPA, and a penetration test are
  <span class="status planned">Planned</span>. No SBOM / dependency-CVE scanning and
  unsigned builds are open supply-chain gaps.

---

## See also

- **[How GaugeWright protects your work](protection.md)** — the structural protection
  model and [Where your data goes](protection.md#where-your-data-goes).
- **[Roadmap &amp; status](../reference/status.md)** — the canonical status table.
- **[Security &amp; trust](../security.md)** — reviewer-grade documentation, threat
  model, and compliance posture.
- **[Glossary](glossary.md)** — every term used here.
- Role guides: **[For experts](../guides/expert/index.md)** ·
  **[For clients](../guides/client/index.md)** ·
  **[For admins (IT)](../guides/admin/index.md)** ·
  **[For embedded end-users](../guides/embed/index.md)**.

# For clients

You're the **[context](../../concepts/glossary.md#context)-owner**. An expert
(a consultant) builds an agent — a [method](../../concepts/glossary.md#method) — and
you bring the private data it works on. The promise is that your data doesn't leak to
the expert or to the runtime, and the expert's method isn't exposed to you. This page
is your honest starting point: it leads with the one thing most people get wrong, then
tells you plainly what works today and what doesn't.

!!! warning "Read this first — your data IS sent to a third-party LLM provider"
    **Local does not mean private.** GaugeWright orchestrates work on *your* machine
    and stores your event log, [resources](../../concepts/glossary.md#resource),
    and payloads locally — but it does **not** run the model locally. The agent's
    reasoning is performed by the **third-party LLM provider that's configured for the
    agent** (OpenAI, Anthropic, Azure OpenAI, or another). So on every
    [run](../../concepts/glossary.md#run), your prompts **and the in-scope
    [context](../../concepts/glossary.md#context)** are sent over the network to that
    provider. **There is no local-only inference today.**
    <span class="status available">Available</span>

    The provider can see the plaintext you send it. Its retention and training terms
    are the **provider's**, not GaugeWright's. If your data may not leave for a
    third-party model, that is a blocker you must resolve *before* you attach
    anything sensitive — see [What to check before you attach data](#what-to-check-before-you-attach-data).
    Full detail: **[Where your data goes](../../concepts/protection.md#where-your-data-goes)**.

## What you can actually do today

The single source of truth for every status badge is the
**[Roadmap &amp; status](../../reference/status.md)** table. In plain terms:

| What you might expect | Status today |
|---|---|
| Use the **local desktop [workbench](../../concepts/glossary.md#workbench)** — install an agent on a project on *your own* machine and run it | <span class="status available">Available</span> |
| Collaborate across **devices you own** (self-federated), via [federation](../../concepts/glossary.md#federation) | <span class="status available">Available</span> |
| Collaborate **cross-party** (with a separate party's authority) over that same federation transport | <span class="status built">Built</span> infra metered, not live |
| [Review &amp; release](review-and-release.md) the [outputs](../../concepts/glossary.md#output) an agent produces | <span class="status built">Built</span> <span class="status planned">reviewer UI Planned</span> |
| Append-only audit trail of every action, each fact bound to its [authority](../../concepts/glossary.md#authority-scope) | <span class="status available">Available</span> |
| **Receive a packaged agent from a remote expert, end-to-end** | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| Receive it under **[attested compute](../../concepts/glossary.md#attestation)** (sealed confidential VM you can verify) | <span class="status built">verifier Built</span> <span class="status planned">live Planned</span> |
| Enterprise [SSO](../../concepts/glossary.md#account) / hardware-key onboarding | <span class="status built">Built</span> <span class="status planned">live Planned</span> |

## The honest part: you cannot receive a remote agent end-to-end today

This is the capability the client role is *for*, so read the caveat carefully rather
than the workflow below.

!!! warning "Cross-party deployment is not operationally live"
    Receiving a packaged agent from a remote expert depends on **cross-party
    deployment**. The building blocks are real and tested in code — the
    [federation](../../concepts/glossary.md#federation) transport, the
    [boundary](../../concepts/glossary.md#boundary) that gates every read and export,
    [output review](review-and-release.md), and the
    [attestation](../../concepts/glossary.md#attestation) **quote verifier** (checked
    against genuine AMD SEV-SNP material). But the **go-live wiring** — a hosted
    confidential VM, live quote generation, hardware-bound key release, and the
    SSO-fronted onboarding flow — is **not deployed**. So the end-to-end
    "expert hands me an agent over the internet and I verify it" experience is
    <span class="status built">Built</span> <span class="status planned">live Planned</span>,
    **not something you can do today**.

    What you *can* do today is the **local desktop workbench**
    <span class="status available">Available</span>: run an agent on a project on
    your own machine, and collaborate with a peer you've paired with directly. See
    the [Roadmap &amp; status](../../reference/status.md) and
    [Deployment modes](../../concepts/deployment-modes.md).

??? question "Then why is the receive-an-agent flow documented at all?"
    Because the *design* is fixed and the *core* is built — it's the next thing to go
    live, and knowing the shape of it tells you what guarantees to expect. The flow
    below is grounded in the
    [attested onboarding contract](../../concepts/protection.md). Treat it as
    "how it will work," badged honestly, not as a feature you can use right now.

## How the receive-an-agent flow will work <span class="status built">Built</span> <span class="status planned">live Planned</span>

When cross-party deployment goes live, this is the sequence you'll follow as the
joining party. It's asymmetric **by design**: the expert holds the sovereign
identity and brings the proprietary method; you hold a hardware-resident consent key
(no seed phrase), often behind your company's SSO, and bring the sensitive context.
You carry the least identity weight on purpose — you're the adoption-critical party.

1. **Receive the invite.** The expert sends an out-of-band link or QR code carrying
   their identity fingerprint. It's pinned **trust-on-first-use**, so the first
   contact establishes who you're talking to.
2. **Install the client app** (or open the web client). Native is preferred when
   hardware-key custody matters; the web client lowers install friction at a
   weaker-custody cost — a deliberate, surfaced tradeoff.
3. **Enroll.** First run generates a **hardware-resident consent key** invisibly —
   **no recovery phrase to lose**. For an enterprise client, **SSO sign-in comes
   first and gates** key provisioning.
4. **(High-stakes only) compare the safety number** with the expert out-of-band to
   close the man-in-the-middle window. Optional, made easy, never mandatory for
   low-stakes pairings.
5. **Verify before you trust.** <span class="status built">verifier Built</span>
   <span class="status planned">live quote Planned</span> The app fetches the
   confidential-VM [attestation](../../concepts/glossary.md#attestation) **quote**,
   checks its **measurement** against the published reproducible build and a fresh
   nonce, then shows the **honest-ceiling disclosure** (step 6). The **quote
   verifier** is implemented and tested against genuine AMD SEV-SNP material; what is
   **not** live is the hosted confidential VM that *generates* a fresh quote, so this
   step is not something you can exercise end-to-end today. Accepting is a
   hardware-signed action, and the boundary goes active **only when both parties
   verify** — neither side can activate it alone. See
   **[Verifying our claims](../../reference/verifying-claims.md)**.
6. **Read the honest-ceiling disclosure, then admit context.** Before your first
   attach, the app states plainly **who can see plaintext** and who cannot (next
   section). Then you pick files or folders; they're admitted as
   [context resources](../../concepts/glossary.md#resource), encrypted client-side
   and carried **by [handle](../../concepts/glossary.md#handle)**, never inline.
   Admitting context **is your consent to the declared ceiling.**

!!! note "Lost device? Re-enroll, don't recover"
    There's no seed phrase, so there's nothing to write down or lose. If you lose
    your device, the expert sends a fresh invite and you re-enroll (enterprise
    clients re-authenticate via SSO); your in-flight engagement re-attaches to the
    new device.

## Who can see your plaintext — stated plainly

Honesty means naming who *can* read your data as clearly as who can't. This mirrors
the boundary's declared ceiling.

=== "Can see plaintext"

    - **You** — it's your machine, your log, your data.
    - **The configured LLM provider** — it sees the prompts and in-scope context
      sent to it for reasoning, under **its** terms.
      <span class="status available">Available</span>
    - **The attested code you verified** (in the live attested mode) — the specific,
      measured agent build you checked in step 5, and nothing else.
      <span class="status built">verifier Built</span> <span class="status planned">live Planned</span>

=== "Cannot see plaintext"

    - **GaugeWright** the company — not in your trust boundary.
    - **The host operator** of an attested VM — the environment is *host-blind*.
    - **The expert / consultant** — your data does **not** reach them unless **you**
      release it through an explicit [review](review-and-release.md)
      declassification ([INV-16](../../concepts/protection.md#the-structural-guarantees-machine-checked)).
    - **A federation relay** — cross-machine traffic is certificate-pinned TLS
      through a blind relay that routes encrypted bytes and never sees plaintext.
      <span class="status available">Available</span>

!!! warning "What this does NOT mean"
    It does **not** mean "hidden from everyone." Your provider sees what you send it.
    If your data is governed by terms that forbid sending it to a third-party model,
    use a provider you've contracted with, or wait for **confidential inference**
    (taking the provider out of the trust boundary), which is on the roadmap.
    <span class="status planned">Planned</span>

## What to check before you attach data

Concrete, ordered checks. Do these *before* the first attach, not after.

1. **Find out which provider gets your data.** The model/provider lives in the
   agent's `.agent-config.json` (the agent's governance configuration), set by the
   expert in an [edit chat](../../concepts/glossary.md#chat). Ask the expert which
   provider and account the agent is configured to use, and confirm it's one you're
   allowed to send your data to.
2. **Match it to your data terms.** If a contract or regulation forbids that
   provider, stop here — resolve it before attaching anything. Don't rely on "it's
   local"; inference is not.
3. **Scope the context to what the work needs.** Whatever a run reads in-scope is
   what gets sent to the provider. Attach the narrowest set of files/folders that
   does the job, not your whole drive.
4. **(Attested mode, when live) verify the quote before you accept.** Don't treat a
   "connected" indicator or a rendered context list as proof of anything — a
   reachable boundary is not an admitted one, and seeing a handle is not access. The
   measurement check in step 5 is the real gate.

## Your guarantees — structural vs. operational

Keep these two buckets separate; it's what keeps the claims defensible. Full detail
and the per-OS caveats live in
**[How GaugeWright protects your work](../../concepts/protection.md)**.

=== "Structural (machine-checked invariants)"

    Built into how the system records and runs work, and paired with adversarial
    tests that fail if the protection is removed:

    - **A handle is not access.** Holding or transporting a
      [handle](../../concepts/glossary.md#handle) reveals nothing; reading needs a
      separate, explicit grant. ([INV-10](../../concepts/protection.md#the-structural-guarantees-machine-checked))
    - **A run has no ambient authority.** It acts only on what it was admitted to do.
      (INV-11)
    - **Fail-closed.** A missing, stale, or uncertain grant is **denied**, never
      allowed on doubt. (INV-20)
    - **Cross-party movement is two-key.** Nothing crosses without the **source**
      permitting it to leave *and* the **target** admitting it. (INV-13)
    - **Generating an output is not releasing it.** Outputs are held until reviewed
      and released. (INV-16) — see [Review &amp; release](review-and-release.md).
    - **Append-only history; revocation stops the future, not the past.** (INV-6,
      INV-18)

    These are <span class="status available">Available</span> on the local desktop
    product. The **kernel-enforced method isolation** that backs them is
    <span class="status available">Available</span> on **Linux/macOS** and
    <span class="status planned">Planned</span> on **Windows** — until then, run
    untrusted methods on Linux or macOS.

=== "Policy / operational (settings &amp; integrations)"

    Real, but they're configuration and infrastructure — not invariants. Several
    have the code seam built while the external half isn't wired live. See the full
    table at
    **[The policy / operational controls](../../concepts/protection.md#the-policy-operational-controls)**:

    - **Audit export** (CSV/JSON, references only) and **SIEM streaming**.
    - **Encryption at rest** (KMS-backed, server deployments) —
      <span class="status built">Built</span>.
    - **Enterprise identity** (OIDC / SCIM / RBAC) —
      <span class="status built">Built</span> <span class="status planned">live Planned</span>.
    - **Inference confidentiality** depends on **your provider's terms** today —
      confidential inference is <span class="status planned">Planned</span>.

## Known gaps — stated here, not hidden

- **No local-only inference.** Inference always goes to the configured third-party
  provider. Confidential inference is <span class="status planned">Planned</span>.
- **Cross-party deployment is not live.** Receiving a remote agent end-to-end —
  including attested compute and SSO onboarding — is
  <span class="status built">Built</span> or <span class="status planned">Planned</span>,
  not operationally available.
- **Windows method-isolation sandbox isn't shipped.** Kernel-enforced isolation is
  Linux/macOS only today.
- **No third-party certifications yet.** SOC 2 Type II, a DPA with a published
  subprocessor list, and an independent penetration test are
  <span class="status planned">Planned</span>.

## Keep reading

- **[Review &amp; release outputs](review-and-release.md)** — how outputs are held
  until you (or a designated reviewer) approve their release.
- **[How GaugeWright protects your work](../../concepts/protection.md)** — the boundary
  contract, where your data goes, and structural vs. operational guarantees.
- **[Verifying our claims](../../reference/verifying-claims.md)** — how to check the
  attestation and the protections for yourself.
- **[Roadmap &amp; status](../../reference/status.md)** — the single source for every
  status badge.
- **[Deployment modes](../../concepts/deployment-modes.md)** · **[Glossary](../../concepts/glossary.md)**.
- The other side of the engagement: **[For experts](../expert/index.md)**.

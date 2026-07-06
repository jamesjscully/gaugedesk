# Verify before you trust &amp; what you control

<span class="status built">verifier Built</span> · <span class="status planned">live quote generation Planned</span>

You're the **[client](../../concepts/glossary.md#core-vocabulary)** — the
context-owner. An expert built an [archetype](../../concepts/glossary.md#archetype)
(their proprietary *method*); you bring the sensitive *context* it works on. Before
you hand that context over, this page shows you how to **check the boundary for
yourself** instead of taking anyone's word — and exactly what stays in your hands
afterwards: the [grants](../../concepts/glossary.md#resource) you give, the access
you can pull back, and the way the system fails *closed* when anything is uncertain.

!!! warning "The one counterintuitive truth — read this first"
    **Local orchestration is not local inference.** Even in an attested
    [boundary](../../concepts/glossary.md#boundary), the agent's *reasoning* is done
    by the **third-party LLM provider configured for the boundary** — so your
    prompts and the in-scope context are sent to that provider over the network.
    **There is no local-only model.** The model provider remains **inside** the
    trust boundary in the attested confidential VM today; confidential inference
    (removing the provider) is <span class="status planned">Planned</span>. That is
    the honest ceiling, spelled out in
    [Where your data goes](../../concepts/protection.md#where-your-data-goes) and in
    [step 4 below](#step-4-read-the-honest-ceiling-before-you-admit-anything).

!!! note "Status — what this page describes vs. what ships today"
    The only product you can download today is the **local desktop
    [workbench](../../concepts/glossary.md#workbench)**
    <span class="status available">Available</span>. The cross-party,
    **attested boundary** this page walks through depends on
    [attested compute](../../concepts/glossary.md#attestation) — the quote
    *verifier* is <span class="status built">Built</span> and tested against genuine
    AMD material, but **live** quote generation on a confidential VM is
    <span class="status planned">Planned</span>. So the verification flow below is
    how it works when an expert sends you an attested boundary; the building
    blocks exist in code, the live host does not yet. The single source for every
    badge is [Roadmap &amp; status](../../reference/status.md).

---

## What "verify the boundary" actually means

When an expert invites you into an attested boundary, the agent doesn't run on
your machine or theirs — it runs in a sealed **[boundary](../../concepts/glossary.md#boundary)**
(a confidential VM). The promise is that the method and your context stay sealed
inside it. **You should not take that promise on faith.** The boundary hands your
app an **[attestation](../../concepts/glossary.md#attestation) quote**: a piece of
cryptographic evidence about *which exact code* is running and *that it is genuinely
sealed in confidential hardware*. Verifying is the act of checking that evidence
before you let any context in. The order is deliberate: you **verify** the boundary
(cryptographic proof) → **accept** it → only then **admit** context (the consent to
move data). Verification is not admission, and reaching a boundary never admits your
data to it.

!!! note "Evidence, not truth — until it checks out"
    The quote and the measurement your app displays are **evidence to verify**, not
    a guarantee in themselves. Nothing is trusted until verification passes — and
    *reachability is not admission*: the fact that you can connect to a boundary
    never means your data has been admitted to it. (Grounded in the
    [attested-onboarding contract](../../concepts/glossary.md#attestation); the
    underlying invariant is INV-10 — a [handle](../../concepts/glossary.md#handle)
    is not access.)

---

## How to verify the boundary — the ordered steps

These are the steps the client app walks you through at first contact. Each is a
concrete action; the app performs the cryptography, you confirm the result.

### Step 1 — Open the invite and pin the expert's identity

1. Open the **out-of-band invite** the expert sent you — a link or QR code. It
   carries the expert's root public key (or its fingerprint).
2. Install the **client app** (preferred where hardware-key custody matters) or open
   the **browser-based client** (lower install friction, weaker custody — a
   deliberate, surfaced tradeoff).
3. On first run the app generates a **hardware-resident consent key** for you
   automatically — **no seed phrase, nothing to write down**. For an enterprise
   client, your **SSO** sign-in happens first and gates this step.

The expert's key is pinned **trust-on-first-use** at this moment, so every later
message is checked against the identity you accepted here.

### Step 2 — (High-stakes) compare the safety fingerprint out of band

1. The app shows a short **safety number** (fingerprint) for the pairing.
2. Over a *separate* channel you already trust — a phone call, a known-good chat —
   read it aloud or paste it and confirm it matches what the expert sees.
3. If the numbers match, the man-in-the-middle window at pairing is closed.

!!! note "Optional by design"
    This compare is **easy but not mandatory** — it exists for high-stakes
    pairings. Low-stakes pairings can skip it; the trust-on-first-use pin from
    step 1 still holds. (Grounded in the attested-onboarding fingerprint-check
    surface.)

### Step 3 — Fetch the attestation quote and check the measurement

1. The app fetches the boundary's **attestation quote** (an AMD SEV-SNP report).
2. It verifies the quote's authenticity end to end: it validates the
   **ARK → ASK → VCEK** certificate chain and the **ECDSA-P384 / SHA-384**
   signature, so a forged or replayed quote is rejected.
3. It checks the quote's **measurement** — a digest of the exact code image — against
   the **published reproducible build**. A measurement that isn't on the allow-list
   fails the check. (The build is reproducible from `flake.nix`; the image digest is
   published, so "the measurement matches the published build" is something you can
   independently re-derive.)
4. It checks the **nonce / freshness** (`report_data`) so the quote is for *this*
   session and not an old one being replayed.

If any of these fail, **the boundary does not go active** — you are never asked to
admit data to an environment that didn't verify. <span class="status built">Built</span>
(verifier) <span class="status planned">live quote Planned</span>

??? question "Why can I trust the measurement at all?"
    Because the build is **reproducible**: the same source produces the same image,
    and that image's digest is published. The measurement in the quote is that same
    digest, signed by the AMD hardware's key (the VCEK, chained up to AMD's root,
    the ARK). So "the code running is the published build" is a cryptographic chain
    you can check, not a claim you have to believe. The current
    <span class="status built">Built</span> verifier has been tested against **real
    AMD Milan vectors**; what's <span class="status planned">Planned</span> is the
    live confidential VM that *generates* a fresh quote.

### Step 4 — Read the honest ceiling before you admit anything

Before your first admit, the app shows a plain-language **honest-ceiling
disclosure**. Read it. It is a required surface precisely so that no one can imply
your data is "hidden from everyone." It states:

=== "Who CAN see your plaintext"

    - **The attested code you just verified** — running inside the sealed boundary.
    - **The configured LLM provider** — your prompts and in-scope context are sent
      to it for reasoning. The provider is **inside** the trust boundary today; its
      retention and training terms are the *provider's*, not GaugeWright's. See
      [Where your data goes](../../concepts/protection.md#where-your-data-goes).

=== "Who CANNOT see your plaintext"

    - **GaugeWright** — the software vendor.
    - **The host operator** — the cloud running the confidential VM is host-blind.
    - **The expert's (consultant's) machine** — the expert's machine never admits
      your context handles without an explicit release (INV-10 — a handle is not
      access). Your context does **not** reach the expert unless *you* later consent
      to release a specific [output](../../concepts/glossary.md#output) through
      [review](review-and-release.md); the expert's access to released outputs is
      gated by that review decision (INV-16).

!!! warning "If your data may not go to a third-party model at all"
    The honest ceiling means inference leaves to a provider. If your data terms
    forbid that, use a provider you have *contracted* with the terms you need, or
    wait for **confidential inference** — which takes the provider *out* of the
    trust boundary and is <span class="status planned">Planned</span>.

### Step 5 — Accept the boundary (both parties must verify)

1. Accepting is a **hardware-signed `acceptBoundary`** — signed by the consent key
   from step 1, so the acceptance is provably yours.
2. The boundary goes **active only when *both* parties have verified** — your
   acceptance and the expert's, together. One party alone cannot bring it live.
   (Conjunctive verification.)

### Step 6 — Admit context, by handle, under your control

1. Pick the files or folders to admit. They are admitted as **context
   [resources](../../concepts/glossary.md#resource)**, encrypted client-side and
   carried **by [handle](../../concepts/glossary.md#handle)** — the reference moves,
   not the raw payload. (INV-10)
2. The admit panel shows **exactly what is exposed**, and that admitting **is your
   consent to the declared ceiling** from step 4.
3. Admitting is *not* a blanket grant: the agent reads a resource only under a
   specific access grant evaluated at the boundary, and method and context each
   require their **own** grant. (INV-12)

---

## What you control after you've joined

Verification is the start; the controls below are what stay in your hands for the
life of the boundary.

### Grants — nothing is read without one

A [resource-access](../../concepts/glossary.md#resource) grant is a **bounded right
to read or use a specific resource**, and it follows a strict lifecycle: a use can
only consume a grant while it is in the **granted** phase. Holding the handle, or a
run simply executing inside the boundary, conveys **no** read on its own — reading
always needs its own explicit grant. (INV-10, INV-12)
<span class="status available">Available</span> (single-party) ·
<span class="status built">Built</span> (cross-party)

### Revocation — stops the future, not the past

You can **revoke** a grant. Revocation is **future-only**: from that moment on, new
reads and uses of that basis are blocked, but it does **not** delete or rewrite the
historical events and observations a previously-granted read already produced.
(INV-18 — `REVOCATION_BLOCKS_FUTURE`, with an adversarial `USE_AFTER_REVOKE` probe
that must fail.) <span class="status available">Available</span>

!!! warning "Revocation is not erasure"
    Revoking stops future access; it does not unsend what already left or scrub
    history. To make a *payload* permanently unresolvable — your right-to-erasure
    path — use **content tombstoning**: the payload becomes unreadable, but the
    immutable audit *fact* that it existed remains (it has to, for the log to stay
    tamper-evident). See
    [How GaugeWright protects your work](../../concepts/protection.md#how-to-keep-your-work-protected).

### Release — generating an output is not releasing it

Any [output](../../concepts/glossary.md#output) the agent produces from your context
starts **held** — it is not delivered or exported by default, and it is *tainted* by
the context that touched it. It reaches anyone only when **you** (or a designated
reviewer) make an explicit, recorded release decision, with provenance shown.
Cross-party release additionally requires the source to permit the crossing and the
target to admit it. (INV-16) See **[Review &amp; release outputs](review-and-release.md)**.
<span class="status built">Built</span>

### Fail-closed — doubt means denied

If a required grant is **missing, stale, expired, revoked, or uncertain**, the
action is **denied** — never allowed on doubt. This is the default posture for every
access, egress, and export decision, and it is a machine-checked invariant, not a
setting you can forget. (INV-20, model `fail-closed.qnt`)
<span class="status available">Available</span>

### Audit — everything is on the record

Every governance action (your admits, grants, revocations, releases) is recorded
**immutably** in the append-only log, attributed to the authenticated actor, and
filterable by actor or action. You can **export** the authorization-scoped timeline
as CSV or JSON — **references only, never protected payload**.
<span class="status available">Available</span>

---

## Structural guarantees vs. policy — don't conflate them

Keeping these separate is what keeps the claims defensible. (Full treatment in
[How GaugeWright protects your work → Two kinds of guarantee](../../concepts/protection.md#two-kinds-of-guarantee-dont-conflate-them).)

=== "Structural (machine-checked)"

    These are formal **invariants** verified in **Quint** models, each paired with
    an adversarial probe that must fail if the protection is removed:

    - Handle is not access (INV-10) · method &amp; context reads are each explicit
      (INV-12) · runs have no ambient authority (INV-11) · fail-closed (INV-20) ·
      append-only/immutable history (INV-6) · revocation is future-only (INV-18) ·
      cross-party movement is two-key (INV-13/14).
    - On **Linux/macOS** the method-isolation part is enforced by a **kernel
      sandbox** so a running agent cannot rewrite its own method.
      <span class="status available">Available</span>. **Windows** sandbox is
      <span class="status planned">Planned</span>.

=== "Policy / operational"

    Configurable rules and infrastructure — real, but *settings and integrations*,
    not invariants, and some have the code seam built while the external half isn't
    wired live:

    - SSO (OIDC) sign-in, RBAC, per-actor audit + export, **encryption in transit**
      (cert-pinned TLS through a blind relay). <span class="status available">Available</span> /
      <span class="status built">Built</span>.
    - **Encryption at rest** (AES-256-GCM seam built; KMS data key deferred),
      **SIEM streaming** (sink built; exporter per-deployment), **attested compute**
      (verifier built; live host Planned), **data residency** policy (region match).
      <span class="status built">Built</span>.

---

## For your IT / security reviewer

Hand your reviewer this section. It points them at the controls a standard security
questionnaire asks for, and is honest about what is not yet in hand.

| What your reviewer asks | Where it stands |
|---|---|
| **Audit export** | Per-actor, append-only, immutable; CSV/JSON export of the authorization-scoped timeline (references only). <span class="status available">Available</span> |
| **SIEM streaming** | Stream the audit log to Splunk/Datadog; sink built, exporter attaches per deployment. <span class="status built">Built</span> |
| **SSO / enterprise identity** | OIDC sign-in (Okta, Microsoft Entra ID, Google Workspace), SCIM provisioning, RBAC (default-deny, `rbac.qnt`), enforce-SSO with owner break-glass. <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| **Data residency** | Resources carry a region; policy can require a resource's region to match the actor's. Regional data-plane isolation is a later increment. <span class="status built">Built</span> (policy) |
| **MFA** | Org-level require-MFA *policy* is modeled; the factor itself is enforced by your IdP under enforce-SSO. <span class="status none">Not implemented</span> (as a GaugeWright-enforced factor) |
| **Encryption** | In transit: cert-pinned TLS via a blind relay <span class="status available">Available</span>. At rest: AES-256-GCM seam built, KMS data key deferred <span class="status built">Built</span>. |
| **Attestation evidence** | AMD SEV-SNP quote verifier (ARK→ASK→VCEK + ECDSA-P384), tested against real Milan vectors; reproducible build + published image digest. <span class="status built">Built</span>; live quote <span class="status planned">Planned</span>. |
| **Certifications (SOC 2 / DPA / pen test)** | Committed and prioritized, **not yet in hand**. <span class="status planned">Planned</span> |
| **GDPR / CCPA erasure** | Access + audit supported; right-to-erasure via tombstoning (payload becomes unresolvable; audit fact remains). |

!!! warning "Stated plainly, not hidden behind a link"
    - **No third-party certifications yet** — SOC 2 Type II, a DPA with a published
      subprocessor list, and an independent penetration test are
      <span class="status planned">Planned</span>.
    - **No local-only inference** — inference always goes to the configured
      third-party provider (the honest ceiling); confidential inference is
      <span class="status planned">Planned</span>.
    - **Young-product gaps** — unsigned builds (no code-signing/notarization yet),
      no published SBOM / dependency scanning, no production monitoring.
    - Your reviewer's deepest dive lives in **[Security &amp; trust](../../security.md)**
      — the reviewer-grade control crosswalk (SOC 2 / ISO 27001 / NIST), data-flow,
      and threat model.

---

## Keep reading

- **[How GaugeWright protects your work](../../concepts/protection.md)** — the full
  boundary contract and *where your data goes*.
- **[Review &amp; release outputs](review-and-release.md)** — your release control in
  depth.
- **[For clients](index.md)** — your role and the whole client workflow.
- **[Deployment modes](../../concepts/deployment-modes.md)** — local, federation,
  attested, hosted/embed.
- **[Glossary](../../concepts/glossary.md)** — every term used here.
- **[Roadmap &amp; status](../../reference/status.md)** — the single source for every
  status badge.
- **[Security &amp; trust](../../security.md)** — reviewer-grade detail.

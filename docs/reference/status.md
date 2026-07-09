# Roadmap &amp; status

GaugeWright is built in shippable increments. This page is the **single source of
truth** for what you can use today and what's coming — every other page defers
here, so a capability described anywhere on the site carries the same status badge
it has on this table.

!!! warning "The one truth to read first"
    GaugeWright orchestrates locally, but **it does not run the model locally.**
    The agent's reasoning is performed by the **third-party LLM provider you
    configure**, so your prompts and the in-scope
    [context](../concepts/glossary.md#context) are sent to that provider over the
    network, in plaintext, today. What this means for who can and cannot see
    plaintext is spelled out in
    [Where your data goes](../concepts/protection.md#where-your-data-goes).

!!! note "Network egress: filtered where enforceable, else open-by-default (disclosed)"
    A non-isolated [chat](../concepts/glossary.md#chat) runs under **filtered egress**
    (CORE-5, [ADR 0079](https://github.com/jamesjscully/gaugedesk-src/blob/main/specs/decisions/0079-per-host-egress-filtering.md))
    — the sandbox may reach **only the model endpoints**, enforced by a host-filtering
    `CONNECT` proxy — **on hosts that can enforce it** (a rootless netns route via
    `slirp4netns`/`pasta`). Where that enforcement isn't available, a non-isolated chat
    keeps the accepted **open-by-default** posture (unfiltered egress with a *disclosed
    lower ceiling* — the 2026-06-17 product decision): the model is reachable out of the
    box, but the agent can reach any host. An operator can **opt into** full network
    isolation **per project** (`network_isolated`), which denies network entirely.
    **Honest status of enforcement:** the proxy and the whole policy path are
    **built and tested**; the last-mile netns routing that makes the proxy the
    sandbox's *sole* outbound path (`slirp4netns`/`pasta` + an nft default-drop)
    is **designed but not yet verified on a routing-capable host**
    (`FILTERED_ROUTING_VERIFIED = false`). Until it is verified, the engine does **not**
    request `Filtered` (which the harness fails closed to isolation), so the default
    stays the disclosed open-by-default posture and model access is **never silently
    broken**; filtering **upgrades** the default automatically once a host can enforce
    it. `GAUGEWRIGHT_ALLOW_UNFILTERED_EGRESS=1` remains a conscious, logged opt-in to
    unfiltered egress regardless. The egress chokepoint and taint/consent gating are
    *always on* regardless.

## What the statuses mean

These four badges are used identically across the whole docs site; if any other
page ever disagrees with this table, **this table wins.**

- <span class="status available">Available</span> — in the product you can
  **download and use today** (the local desktop workbench).
- <span class="status built">Built</span> — **implemented and tested in the
  codebase**, but not yet operationally deployed (typically waiting on hosting
  infrastructure or go-live wiring). Not a usable end-user feature yet.
- <span class="status planned">Planned</span> — committed and designed, **not yet
  built**.
- <span class="status none">Not implemented</span> — **absent today**.

!!! note "Reading the split badges (e.g. \"Built · live Planned\")"
    Some rows carry **two badges** because the code and the deployment have different
    statuses. The first badge is the **code** state — `core Built`, `verifier Built`,
    `adapter Built` all mean the component is implemented and tested in the codebase.
    The second, **`live Planned`**, means that even though the code exists, the
    capability is **not operationally deployed for end-users** — it is waiting on the
    hosting infrastructure to go live. A row with `live Planned` is **not usable
    today**, no matter what its code badge says.

!!! note "Available means today; Built does not"
    If a page describes a Built feature, it is describing the design and the code
    behind it, **not** something you can switch on. The only mode you can use
    end-to-end today is the **local desktop workbench**.

!!! warning "Today's ceiling: method secrecy from the context owner is not achievable"
    The only Available placement is the **local desktop workbench** — an
    **unattested** placement where the host *is* the
    [context](../concepts/glossary.md#context) owner. At this placement, per the
    [boundary](../concepts/glossary.md#boundary) ceiling, **method secrecy from the
    context owner is not achievable** — only obfuscation, via isolating the run from
    the host network. The model endpoint and the host are both in the trust set.
    Raising the [method](../concepts/glossary.md#method) ceiling to host-blind
    requires an **attested** placement, which is <span class="status built">Built</span>
    in code but not operationally live.

## At a glance

| Capability | Status |
|---|---|
| Local desktop workbench (build · run · review) | <span class="status available">Available</span> |
| Multi-authority [federation](../concepts/glossary.md#federation) (collaborate across machines) | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| Append-only audit log + SIEM export | <span class="status available">Available</span> |
| Kernel-enforced method isolation (Linux/macOS) | <span class="status available">Available</span> |
| Windows method-isolation sandbox | <span class="status planned">Planned</span> |
| Encryption at rest — local envelope encryption (AES-256-GCM) | <span class="status available">Available</span> |
| Encryption at rest — KMS-backed (server deployments) | <span class="status built">Built</span> |
| Cross-party [packaging](../concepts/glossary.md#package) &amp; deployment | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| [Attested](../concepts/glossary.md#attestation) compute (confidential VM, host-blind — provider stays in TCB) | <span class="status built">verifier Built</span> <span class="status planned">live Planned</span> |
| Enterprise identity (OIDC / SAML / SCIM / RBAC) | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| Output review &amp; release lifecycle | <span class="status built">Built</span> |
| Hosted multi-tenant platform | <span class="status planned">Planned</span> |
| Public hosting / embedded agents | <span class="status built">core Built</span> <span class="status planned">live Planned</span> |
| MFA enforcement | <span class="status none">Not implemented</span> |
| Confidential inference (provider out of trust boundary) | <span class="status planned">Planned</span> |
| SOC 2 Type II · DPA · penetration test | <span class="status planned">Planned</span> |

The [Deployment modes](../concepts/deployment-modes.md) page reads this same table
from the angle of *where the agent runs and who's involved*; the
[Glossary](../concepts/glossary.md) carries the same badge on each term.

!!! note "Two encryption-at-rest claims, kept apart"
    **Local envelope encryption** (AES-256-GCM, a random per-instance data key) is
    <span class="status available">Available</span> in the desktop workbench today.
    Separately, a **KMS-backed** adapter — the data key wrapped by a cloud KMS key,
    so nothing usable sits at rest and key access is KMS-gated — is
    <span class="status built">Built</span>: the adapter is implemented and verified
    live against a real Key Vault, but it is **not operationally deployed** at server
    scale (server hosting is the infra gap). Built ≠ switched on.

!!! warning "What \"attested\" does and does not remove"
    Attested compute (confidential VM) raises the [method](../concepts/glossary.md#method)
    ceiling to **host-blind**: who operates the host no longer decides who sees
    plaintext. It does **not** remove your **model provider** from the trust boundary.
    Even with attestation Built, the residual trust set is **{the attested GaugeWright
    code, the model provider}** — the prompt still reaches the provider in plaintext.
    Only **confidential inference** (<span class="status planned">Planned</span>) takes
    the provider out of the boundary. See
    [Where your data goes](../concepts/protection.md#where-your-data-goes).

!!! note "Enterprise identity is Built in code, not yet operationally live"
    The enterprise-identity row reads <span class="status built">Built</span> because
    the OIDC and SAML SSO adapters, SCIM 2.0 provisioning, fixed-role RBAC, and the
    org-level enforce-SSO toggle are implemented and tested in the codebase (verified
    live against real IdPs) — but they cannot be switched on end-to-end without the
    hosted control plane, which is the infra gap. Built is the code, not a usable
    end-user feature.

## Public hosting &amp; embed — MVP vs. later

The browser-embeddable agent surface (a consultant embedding their agent in their
own website for end-users) is being shipped as a clearly-scoped MVP first, then
widened. Per-piece status below reflects what is **built and tested in code**
versus what is **operationally live**. None of it is usable end-to-end today: the
managed host that serves a live per-visitor session is infrastructure that does
not run in the local scaffold, so the whole surface stays
<span class="status planned">Planned</span> for end-users until that host ships.

!!! warning "The entire embedded-agent surface is not operationally available today"
    **No item in either table below is end-to-end usable today.** Every row is either
    a <span class="status built">Built</span> component (implemented and tested in
    code, awaiting the managed host) or <span class="status planned">Planned</span>.
    The per-row badges describe code-readiness, not a feature you can switch on.

=== "MVP scope"

    | Embed capability | Status |
    |---|---|
    | Anonymous [chat](../concepts/glossary.md#chat) (ephemeral, identity-less, discarded on teardown) | <span class="status built">core Built</span> <span class="status planned">live Planned</span> |
    | Managed-auth signed-in chat ([audience](../concepts/glossary.md#audience) via email / magic-link / social) | <span class="status built">adapter Built</span> <span class="status planned">live Planned</span> |
    | My-chats — your own durable chats (drawer + standalone element) | <span class="status built">data layer Built</span> <span class="status planned">live Planned</span> |
    | Chat panel + `<gw-session>` provider + `embed.js` bundle | <span class="status built">Built</span> <span class="status planned">publish Planned</span> |
    | "Powered by" branding mark | <span class="status planned">Planned</span> |
    | Consultant Deploy Config · Embed/Preview · basic Monitor | <span class="status planned">Planned</span> |

=== "Later"

    | Embed capability | Status |
    |---|---|
    | Embedded [output](../concepts/glossary.md#output) panel (read + download your own artifacts) | <span class="status planned">Planned</span> |
    | Embedded files panel (browse your own worktree) | <span class="status planned">Planned</span> |
    | BYO-OIDC sign-in + silent token pass-through | <span class="status built">adapter Built</span> <span class="status planned">live Planned</span> |
    | Anonymous → authenticated **[claim](../concepts/glossary.md#claim-token)** flow (one-time claim token) | <span class="status planned">Planned</span> (decided, build deferred) |
    | [White-label](../concepts/glossary.md#powered-by-white-label) — paid removal of the powered-by mark | <span class="status planned">Planned</span> |
    | [Attested](../concepts/glossary.md#attestation) host as a selectable deployment ceiling | <span class="status planned">Planned</span> |
    | Server-side **secret** API key (backend proxying) | <span class="status planned">Planned</span> |

!!! note "What \"core Built\" covers here"
    The audience-identity seam, the durable-chat data layer, the scoped remote
    session, and the web-component elements (`<gw-session>` / `<gw-chat>`) are
    implemented and tested in the codebase. They cannot run end-to-end without the
    managed host that serves live per-visitor sessions, so the surface stays
    <span class="status planned">Planned</span> for end-users. The end-user side is
    [For embedded end-users](../guides/embed/index.md); the consultant side is
    [Package &amp; deploy](../guides/expert/package-and-deploy.md).

## How the guarantees are backed

Not every status caveat is the same *kind* of promise. The docs keep two kinds
apart so claims stay defensible:

- **Structural, machine-checked guarantees.** Built into how the system works and
  paired with an adversarial test that fails if the protection is removed. These
  are the boundary properties on
  [How GaugeWright protects your work](../concepts/protection.md): a
  [handle](../concepts/glossary.md#handle) is not access; method and
  [context](../concepts/glossary.md#context) reads are both explicit; a
  [run](../concepts/glossary.md#run) has no ambient authority; everything is
  fail-closed; history is append-only. <span class="status available">Available</span>
- **Policy / operational properties.** Things that depend on configuration,
  process, or a third party — your LLM provider's retention terms, certifications,
  monitoring, build signing. These are *not* invariants and are stated as such.

The [Security &amp; trust](../security.md) page splits the two bucket-by-bucket for
reviewers (with the optional `INV-*` deep links).

!!! note "Per-OS caveat on the sandbox"
    The kernel-enforced sandbox that stops a work
    [chat](../concepts/glossary.md#chat) from rewriting its own method is
    <span class="status available">Available</span> on **Linux and macOS**.
    **Windows** method-isolation is <span class="status planned">Planned</span> —
    until it ships, run untrusted methods on Linux/macOS.

## Known limitations &amp; gaps (today)

Stated here in the docs, not only on the external trust site:

- **Inference is remote and inside the trust boundary.** Prompts and in-scope
  context go to the third-party provider you configure; its retention and training
  terms are the provider's, not GaugeWright's. Confidential inference (removing the
  provider from the boundary) is <span class="status planned">Planned</span>. See
  [Where your data goes](../concepts/protection.md#where-your-data-goes).
- **Only the local desktop workbench is usable today.** Cross-party deployment,
  attested compute, enterprise identity, and hosted/embed are
  <span class="status built">Built</span> or <span class="status planned">Planned</span> —
  not operationally live.
- **No third-party certifications yet.** SOC 2 Type II, DPA, and a penetration
  test are <span class="status planned">Planned</span>.
- **MFA enforcement is <span class="status none">Not implemented</span> as a
  GaugeWright-native factor.** The org-level **require-SSO / require-MFA policy** is
  <span class="status built">Built</span> (the `enforce_sso` and `require_mfa`
  org-security toggles landed 2026-06-18, tracker SEC-1 / ID-5); the **MFA factor
  itself is delegated to your IdP** under the enforce-SSO flag — GaugeWright declares
  the policy it honors, the IdP enforces the factor. The last-`owner` break-glass
  guard means enforce-SSO can never lock the org out.
- **Supply-chain &amp; ops gaps.** No SBOM / dependency scanning, no production
  monitoring, and unsigned builds today. These are tracked on the
  [Security &amp; trust](../security.md) page and its linked architecture
  documentation.

## Where to go next

- **What protects the work, and who can see plaintext** →
  [How GaugeWright protects your work](../concepts/protection.md)
- **Where the agent runs and who's involved** →
  [Deployment modes](../concepts/deployment-modes.md)
- **Reviewer-grade detail** (data-flow diagrams, the invariant→control crosswalk,
  threat model, honest compliance posture) → [Security &amp; trust](../security.md)
- **Role guides** → [Experts](../guides/expert/index.md) ·
  [Clients](../guides/client/index.md) · [Admins](../guides/admin/index.md) ·
  [Embedded end-users](../guides/embed/index.md)

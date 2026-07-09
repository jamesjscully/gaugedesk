# GaugeDesk documentation

GaugeWright lets an expert's **[method](concepts/glossary.md#method)** (an
AI agent — its instructions, skills, and tools) run against a client's **private
[context](concepts/glossary.md#context)** (their data) inside an enforced
**[boundary](concepts/glossary.md#boundary)**, so that neither the method nor the
data leaks to the other party or to the [runtime](concepts/glossary.md#runtime).

These docs cover everyone who works with GaugeWright: experts who build and
deploy agents, clients who receive them, IT admins who govern a deployment, and
end-users who use an embedded agent.

## Status legend

Every capability claim on every page carries one of four badges. They mean exactly
this — and the **single source of truth is the [status table](reference/status.md)**.
When a page and the table disagree, the table wins.

<span class="status available">Available</span> shipped in the product you can
download today &nbsp;·&nbsp;
<span class="status built">Built</span> implemented &amp; tested in code, **not yet
operationally deployed** &nbsp;·&nbsp;
<span class="status planned">Planned</span> committed, designed, not yet built &nbsp;·&nbsp;
<span class="status none">Not implemented</span> absent today

!!! warning "The one truth that surprises people: local orchestration is not local inference"
    GaugeWright orchestrates on **your machine**, but the agent's *reasoning* is
    performed by the **third-party LLM provider you configure** (e.g. OpenAI,
    Anthropic, Azure OpenAI). Your prompts and the in-scope context are sent to
    that provider over the network. The provider is **in the trust boundary today**
    — it sees plaintext. <span class="status available">Available</span>; removing
    it (confidential inference) is <span class="status planned">Planned</span>.
    Read [where your data goes](concepts/protection.md#where-your-data-goes) before
    you put anything sensitive through a run.

## Is this for me?

=== "GaugeWright is for you if…"
    - You want to **build an AI agent, run it against your own files, and review
      every change** before keeping it — all on your own desktop.
      <span class="status available">Available</span>
    - You're a consultant who wants to **collaborate with a counterpart across two
      machines**, where neither side's relay can read the payload.
      <span class="status available">Available</span>
    - You need an **append-only audit trail** of what every run read and produced.
      <span class="status available">Available</span>
    - You're comfortable sending prompts + in-scope context to an **LLM provider you
      contract with yourself**.

=== "Not yet, if…"
    - You need a **hosted or embedded agent on a public website** — that's
      <span class="status planned">Planned</span>.
    - You need **enterprise SSO/SCIM/RBAC live** for a customer org — the code is
      <span class="status built">Built</span> but not operationally deployed.
    - You need **attested confidential-VM compute** in production — the verifier is
      <span class="status built">Built</span>, live hosting is
      <span class="status planned">Planned</span>.
    - Your data **may not leave for any third-party model** — wait for confidential
      inference (<span class="status planned">Planned</span>) or use a provider you've
      contractually bound.

## What works today

The product you can download is the **local desktop workbench**. Here is the honest
cut of what is usable right now versus what exists only in code.

| What | Status | Notes |
|---|---|---|
| **Build** an agent ([archetype](concepts/glossary.md#archetype)) in an edit chat | <span class="status available">Available</span> | Authoring on your machine |
| **Run** it against your context, isolated per run | <span class="status available">Available</span> | Reasoning goes to your LLM provider |
| **Review** each run's diff and keep or discard it | <span class="status available">Available</span> | Reviewing diffs locally on your machine. Releasing [outputs](concepts/glossary.md#output) to stakeholders is <span class="status built">Built</span>, not yet live |
| Multi-stakeholder output release lifecycle (release crossing parties) | <span class="status built">Built</span> | The cross-party release gate (`SOUND_RELEASE`) is implemented + tested; local diff review above is the live part |
| **Federate** across two machines (consultant ↔ client) | <span class="status available">Available</span> | Code-complete and CI-tested (loopback + NAT-isolated harness), not yet operationally deployed |
| Append-only audit log + SIEM export | <span class="status available">Available</span> | Tamper-evident at the app log |
| Kernel-enforced method isolation | <span class="status available">Available</span> | **Linux/macOS only**; Windows <span class="status planned">Planned</span> |
| Cross-party packaging &amp; deployment | <span class="status built">Built</span> <span class="status planned">live Planned</span> | Implemented + tested, not live |
| Attested confidential-VM compute | <span class="status built">verifier Built</span> <span class="status planned">live Planned</span> | Verifier in code; no live hosting |
| Enterprise identity (OIDC / SAML / SCIM / RBAC) | <span class="status built">Built</span> <span class="status planned">live Planned</span> | Not operationally deployed |
| Hosted / public / embedded agents | <span class="status planned">Planned</span> | Not buildable locally |
| Confidential inference (provider out of trust boundary) | <span class="status planned">Planned</span> | Today the provider sees plaintext |

**Rule of thumb:** anything **single-party, on your own machine** is
<span class="status available">Available</span> today; anything **cross-party,
hosted, or attested in production** is <span class="status built">Built</span> or
<span class="status planned">Planned</span>. For the full table — including known
gaps (no SBOM / dependency scanning, no production monitoring, unsigned builds, no
third-party audit) — see **[Roadmap &amp; status](reference/status.md)**.

### Where an agent can run today

The *same* protection model applies in every deployment mode; governance is added,
not re-architected. Only one mode is usable end-to-end today. The
[Deployment modes](concepts/deployment-modes.md) page covers each in full.

| Mode | What it is | Status |
|---|---|---|
| **Local desktop** | Orchestration + storage on your machine; inference calls your configured LLM provider | <span class="status available">Available</span> |
| **Multi-authority [federation](concepts/glossary.md#federation)** | Expert ↔ client collaborate across machines; relay routes opaque bytes only | <span class="status built">Built</span> — code-complete, CI-tested (loopback + NAT-isolated), not operationally deployed |
| **Hosted multi-tenant** | Cloud-hosted relay + compute for consultants' deployments | <span class="status planned">Planned</span> (needs infra) |
| **[Attested](concepts/glossary.md#attestation) compute** | Confidential VM; both parties verify the measurement | <span class="status built">verifier Built</span> <span class="status planned">live Planned</span> |
| **Public hosting / embed** | Browser-embeddable agent for end-users | <span class="status built">core Built</span> <span class="status planned">live Planned</span> |

!!! note "Two kinds of guarantee — keep them separate"
    - **Structural guarantees** are built into how the system works and are
      **machine-checked** against formal invariants with adversarial tests (a
      reference is not access; a run has no ambient authority; history is
      append-only; a work chat cannot rewrite its own method). These hold by
      construction. See [How GaugeWright protects your work](concepts/protection.md).
    - **Policy / operational guarantees** depend on configuration and deployment
      (which LLM provider you trust, whether enterprise identity is wired up, code
      signing). These are only as strong as how you run it.

    One structural guarantee is **per-OS**: kernel-enforced
    [method](concepts/glossary.md#method) isolation runs on **Linux and
    macOS** today; the **Windows** method-isolation sandbox is
    <span class="status planned">Planned</span> (egress gating —
    deny-by-default network, fail-closed admission at the
    [boundary](concepts/glossary.md#boundary) — is
    <span class="status available">Available</span>). Windows users are not left
    unprotected: they get the boundary's egress gates and fail-closed admission;
    what they lack is the kernel sandbox that stops a work chat from rewriting its
    own method.

## Start here

1. **[Getting started](getting-started.md)** — download, install, configure a
   provider, and complete your first reviewed run.
   <span class="status available">Available</span>
2. **[Concepts](concepts/index.md)** — the mental model
   ([project](concepts/glossary.md#project) →
   [archetype](concepts/glossary.md#archetype) →
   [placement](concepts/glossary.md#placement) →
   [chat](concepts/glossary.md#chat)) and the full
   [glossary](concepts/glossary.md). Collaboration runs along a
   [workstream](concepts/glossary.md#workstream) — a shared line of work that a set
   of chats auto-sync into; an output's taint is
   [engagement](concepts/glossary.md#engagement)-scoped (read: chat-scoped).
3. **[How GaugeWright protects your work](concepts/protection.md)** — the boundary
   and where your data goes, in plain language.

## Find your role

| You are… | Start here | What you'll do |
|---|---|---|
| An **expert / consultant** | [For experts](guides/expert/index.md) | [Build an agent](guides/expert/build-an-agent.md), [run &amp; review](guides/expert/run-and-review.md), [package &amp; deploy](guides/expert/package-and-deploy.md) |
| A **client** receiving an agent | [For clients](guides/client/index.md) | Provide context, [review &amp; release outputs](guides/client/review-and-release.md) |
| An **admin / IT** governing a deployment | [For admins](guides/admin/index.md) | SSO, provisioning, audit, policy <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| An **embedded end-user** | [For embedded users](guides/embed/index.md) | Use an agent embedded in a site <span class="status planned">Planned</span> |

## Trust &amp; reference

- **[How GaugeWright protects your work](concepts/protection.md)** — the structural
  guarantees and the data-flow truth.
- **[Roadmap &amp; status](reference/status.md)** — the single status table this
  whole site defers to.
- **[Deployment modes](concepts/deployment-modes.md)** — local, federated, hosted,
  embedded — and which are real today.
- **[Security &amp; trust](security.md)** — data flow, threat model, invariant →
  control crosswalk, and honest compliance posture.
- **[FAQ](faq.md)**

For deep readers, the user-facing terms above link to the
[glossary](concepts/glossary.md); the formal invariants (`INV-n`) and decision
records (ADRs) live in the product specification and are referenced from the
[protection](concepts/protection.md) and [security](security.md) pages — optional,
never required reading.

---

*This documentation is the single source of truth for using GaugeWright. It lives
in the product repository alongside the code it describes, so guidance and behavior
change together. Capability status defers to [reference/status.md](reference/status.md).*

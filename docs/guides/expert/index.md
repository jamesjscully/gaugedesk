# For experts (consultants)

You're the **method-owner**. You encode your expertise as an
**[archetype](../../concepts/glossary.md#archetype)** (a reusable agent
definition), run it on real work as a **[placement](../../concepts/glossary.md#placement)**,
and — when it's ready — **[package](../../concepts/glossary.md#package)** it for a
client without handing over your method. This page is your workflow hub: each step
is badged with its honest status, and the harder questions (versioning, deploy
configuration, what to do when something breaks) are linked below.

!!! info "What you can actually do today"
    Only the **local desktop workbench** is shipped:
    **build → run → review** on your own machine, plus collaboration across
    machines via [federation](../../concepts/glossary.md#federation).
    <span class="status available">Available</span>

    Handing an agent to a remote client (cross-party packaging and deployment) is
    implemented and tested in the core but **not yet operationally live**.
    <span class="status built">Built</span> <span class="status planned">live deployment Planned</span>
    The single source of truth for every capability's status is the
    **[Roadmap &amp; status](../../reference/status.md)** table.

!!! warning "Where your data goes — read this once"
    GaugeWright orchestrates **locally**, but it does **not** run the model
    locally. The agent's reasoning is performed by the **third-party LLM provider
    you configure** (e.g. OpenAI, Anthropic, Azure OpenAI), so your prompts and the
    in-scope [context](../../concepts/glossary.md#context) are sent to that
    provider over the network. With your own credentials, the provider is *your*
    subprocessor, not GaugeWright's. There is no local-only inference today.
    See [Where your data goes](../../concepts/protection.md#where-your-data-goes).
    <span class="status available">Available</span>

## The three-step workflow

| Step | What you do | Status |
|---|---|---|
| **1. [Build an agent](build-an-agent.md)** | Define an archetype and refine it in an **edit chat**. | <span class="status available">Available</span> |
| **2. [Run &amp; review work](run-and-review.md)** | Place it on a project, give it tasks, keep or discard the diffs. | <span class="status available">Available</span> |
| **3. [Package &amp; deploy](package-and-deploy.md)** | Make it shareable and deploy it to a client (privately, or under [attestation](../../concepts/glossary.md#attestation)). | <span class="status built">Built</span> <span class="status planned">live deployment Planned</span> |

The progression is deliberate: you **build** the method in isolation, **run** it
against real (or sample) context to prove it works, and only then **package** it
for someone else. Each step reads from and writes to an append-only history, so the
whole trail — every method change, every run, every release — is auditable and
reversible. <span class="status available">Available</span>

## Step 1 — Build an agent <span class="status available">Available</span>

You author the method in an **[edit chat](../../concepts/glossary.md#chat)** — a
chat rooted on the archetype itself. The definition is the **Pi-native surface** in
the archetype's own repo (ADR 0029); you edit these files directly in the content
viewer, or tell the agent in the edit chat to improve them:

1. In your library, **create a new archetype** and name it for the method
   (e.g. `price-leveling`).
2. Edit the **persona / system prompt** in **`.pi/SYSTEM.md`** — who the agent is
   and how it works.
3. Edit **working conventions** in **`AGENTS.md`** — the rules and conventions the
   agent should follow.
4. Set **model/provider, declared tool requests, and the protection posture** in
   **`.agent-config.json`** (this is GaugeWright's own governance file — the
   membrane `policy` — not a Pi method file).
5. **Install skills/packages in chat**: tell the edit chat to install what you
   need; it runs the install as a tool call, writing the archetype's own
   `.pi/skills/`, `.pi/extensions/`, and `.pi/settings.json`. There is no separate
   skills form.

!!! note "A running agent cannot rewrite its own method — structurally"
    The method surface (`.pi/**`, `AGENTS.md`, `.agent-config.json`) is writable
    **only** from an edit chat. A **[work chat](../../concepts/glossary.md#chat)**
    (rooted on a placement) can *read* the definition but never mutate it. This is
    a **structural, machine-checked guarantee** (`INV-24`): the definition surface
    is mounted read-only by an **OS kernel sandbox**, so even a `bash` shell inside
    a run can't change it — bubblewrap on Linux, Seatbelt on macOS.
    <span class="status available">Available (Linux/macOS)</span> The Windows
    sandbox is <span class="status planned">Planned</span> (AppContainer /
    restricted token) — see [Roadmap &amp; status](../../reference/status.md).

Full walkthrough: **[Build an agent](build-an-agent.md)**.

## Step 2 — Run &amp; review work <span class="status available">Available</span>

To do work you install the archetype onto a project — that's a **placement** — and
chat with it in a **work chat**:

1. From the project, **install the archetype**. The placement's identity always
   shows its lineage as `archetype · project` (e.g. `price-leveling · Peach`), so
   which method is running and where is never hidden.
2. **Attach context** — the files or folders the agent works on, as
   [context resources](../../concepts/glossary.md#resource). Attaching records
   intent; the [boundary](../../concepts/glossary.md#boundary) decides what is
   actually revealed during a [run](../../concepts/glossary.md#run).
3. **Give it a task** in the work chat. Each run executes inside an isolated
   sandbox and produces a **diff** — the proposed change.
4. **Keep or discard** the diff. Nothing is applied without your decision, and
   every run is recorded in the append-only history.

!!! warning "Inference leaves your machine on every run"
    During a run your prompts and the in-scope context are sent to your configured
    LLM provider. If data may not leave for a third-party model, use a provider
    you've contracted, or wait for confidential inference
    (<span class="status planned">Planned</span>). See
    [Where your data goes](../../concepts/protection.md#where-your-data-goes).

**Collaborate** by syncing multiple chats into a shared
**[workstream](../../concepts/glossary.md#workstream)**, and across machines via
**[federation](../../concepts/glossary.md#federation)** — nothing crosses without
the source permitting it to leave *and* the target admitting it.
<span class="status available">Available</span>

Full walkthrough: **[Run &amp; review work](run-and-review.md)**.

## Step 3 — Package &amp; deploy <span class="status built">Built</span> <span class="status planned">live deployment Planned</span>

When the method is ready, freeze a version and package it. A
**[package](../../concepts/glossary.md#package)** is a shareable manifest that makes
the archetype transferable across parties **without** turning transfer into
execution or payload release. A published version is **immutable** — a deployed
agent never silently changes under a client.

The cross-party guarantees:

- **Your method stays protected** — it runs read-only at the client and is never
  exported or revealed.
- **The client's data stays protected** — the agent reads context only under the
  client's explicit grants.
- **Crossing requires both sides** — anything moving carries proof that you
  permitted it to leave *and* the client admitted it.
  <span class="status available">Available</span> (federation transport)

!!! warning "Not yet operationally live"
    Packaging and cross-party deployment are implemented and tested in the core,
    but **operational deployment to a remote client is not yet live**. The
    building blocks (federation transport, the boundary,
    [output](../../concepts/glossary.md#output) review, the
    [attestation](../../concepts/glossary.md#attestation) verifier) exist today;
    go-live wiring is pending. See the
    [Roadmap &amp; status](../../reference/status.md).

Full walkthrough: **[Package &amp; deploy](package-and-deploy.md)**.

## Going deeper

These pages answer the questions experts hit once the three-step loop is familiar:

- **[Versioning &amp; forking](versioning-and-forking.md)** — freezing an immutable
  version, how placements take an upgrade deliberately, and **forking** an
  archetype (owner-only) to branch genuinely different behavior.
  <span class="status built">Built</span>
- **Deploy configuration** — the `.agent-config.json` knobs (model/provider
  selection, declared tool/capability requests, and the membrane `policy` —
  the protection posture) live in
  **[Build an agent → Set the model and protection posture](build-an-agent.md#4-set-the-model-and-protection-posture)**;
  the deployment options (private peer-to-peer vs.
  [attested](../../concepts/glossary.md#attestation) compute) live in
  **[Package &amp; deploy → Deploy](package-and-deploy.md#3-deploy-privatefederated-or-attested)**.
  <span class="status built">Built</span> <span class="status planned">live Planned</span>
- **[Troubleshooting](troubleshooting.md)** — unsigned-build prompts, provider
  credential and model-resolution issues, sandbox/permission errors, and what to
  check when a run is denied (fail-closed by design).
  <span class="status available">Available</span>

## Structural guarantees vs. operational posture

Keep these two kinds of claim separate when you reason about trust:

=== "Structural (invariant-backed, machine-checked)"
    These are built into how the system works and paired with adversarial tests
    that fail if the protection is removed:

    - A [handle](../../concepts/glossary.md#handle) is not access; method and
      context reads are both explicit.
    - A run has no ambient [authority](../../concepts/glossary.md#authority-scope) —
      it does only what it was admitted to do.
    - A work chat cannot rewrite the method (`INV-24`) — kernel-enforced on
      **Linux/macOS**; Windows <span class="status planned">Planned</span>.
    - Fail-closed: a missing, stale, or uncertain grant is denied.
    - Append-only history: every durable fact is an immutable event.

    See [How GaugeWright protects your work](../../concepts/protection.md).

=== "Operational / policy (not yet machine-guaranteed)"
    These depend on configuration, infrastructure, or process and are *not*
    structural guarantees today:

    - **Inference confidentiality** depends on your provider's terms — the LLM
      sees prompts and in-scope context. <span class="status planned">Planned</span>
      (confidential inference).
    - **Live cross-party deployment / hosted / embed** are
      <span class="status built">Built</span> or <span class="status planned">Planned</span>,
      not live.
    - **Third-party certifications** (SOC 2 Type II, DPA, pen test) are
      <span class="status planned">Planned</span>; builds are currently unsigned.

## Related reading

- New here? Start with **[Getting started](../../getting-started.md)** and the
  **[Concepts](../../concepts/index.md)**.
- The vocabulary: **[Glossary](../../concepts/glossary.md)**.
- The safety model: **[How GaugeWright protects your work](../../concepts/protection.md)**
  and **[Security &amp; trust](../../security.md)**.
- Where it runs and who's involved: **[Deployment modes](../../concepts/deployment-modes.md)**.
- The single status source: **[Roadmap &amp; status](../../reference/status.md)**.
- The other side of the [engagement](../../concepts/glossary.md#engagement):
  **[For clients](../client/index.md)**.

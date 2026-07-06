# Concepts

GaugeWright has a small, consistent vocabulary. Learn a handful of ideas here and
the rest of the product follows. This page is the five-minute mental model; for
precise one-line definitions of every term, see the **[Glossary](glossary.md)**.

!!! note "The one counterintuitive truth, up front"
    Orchestration is **local**; inference is **not**. The workbench runs on your
    machine, but the agent's reasoning is performed by the **third-party LLM
    provider you configure** — your prompts and the in-scope context are sent to
    that provider over the network. There is no local model. See
    [Where your data goes](protection.md#where-your-data-goes) before you trust
    anything sensitive to a run.

## What you can use today

GaugeWright ships in increments, so the docs are honest about what is live versus
designed. Throughout, capability claims carry a badge that defers to the single
status table in **[Roadmap & status](../reference/status.md)**:

- <span class="status available">Available</span> — in the product you can download today
- <span class="status built">Built</span> — implemented and tested in code, not yet operationally deployed
- <span class="status planned">Planned</span> — committed, not yet built
- <span class="status none">Not implemented</span> — absent today

Today, only the **local desktop workbench** — build a method, run it on local
context, review the result — is <span class="status available">Available</span>.
Cross-party packaging, attested compute, enterprise identity, and hosted/embedded
agents are <span class="status built">Built</span> or
<span class="status planned">Planned</span>. The concepts below describe the whole
model; the badges tell you which parts are live.

## The shape of the product

**You build an agent once, then run it on real work — without your method or the
data leaking.** Three nouns carry that idea:

- An **[archetype](glossary.md#archetype)** is the reusable agent *definition* — its
  instructions, skills, and tools. It lives in your **library**, not in a project,
  and you build and refine it in an **edit chat**. Concretely it is the Pi-native
  surface in the archetype's repo: `.pi/SYSTEM.md` (system prompt), `AGENTS.md`
  (instructions), `.pi/skills/`, `.pi/extensions/`, plus `.agent-config.json` (model
  selection and the declared protection posture). <span class="status available">Available</span>
- A **[placement](glossary.md#placement)** is an archetype *installed onto a
  project* — the durable deployment you actually do work with. Its identity is
  `archetype · project`, so the method's lineage is always visible. One archetype
  can be placed on many projects; one project can hold many placements. <span class="status available">Available</span>
- A **[chat](glossary.md#chat)** is a durable line of conversation. Its kind is
  **fixed by what it is rooted on** — there is no edit/use toggle: <span class="status available">Available</span>

    === "Edit chat"
        Rooted on an **archetype**. Its subject is the
        [method](glossary.md#method); its output is a published version.
        This is the **only** place the method is authored.

    === "Work chat"
        Rooted on a **placement**. Its subject is the project's *work*; it consumes
        the method **read-only** and cannot change it.

## Where work lives

- A **[project](glossary.md#project)** is a **trust and data boundary** — a body of
  work *plus the parties who may see it* (`personal`, `client`, `expert`,
  `auditor`…). Two bodies of work that must stay apart are two projects, and that
  non-mixing *is* the boundary. A project holds context, placements, and runs; it is
  not a folder or a git repo. <span class="status available">Available</span>

!!! note "Three load-bearing words: resource, handle, boundary"
    These three turn the nouns above into something protectable. They are the heart
    of [how GaugeWright protects your work](protection.md).

- A **[resource](glossary.md#resource)** is any protected thing the system records
  or moves: a [method](glossary.md#method), a [context](glossary.md#context),
  or a derived [output](glossary.md#output). Method and context are both resources —
  **neither is privileged**.
- A **[handle](glossary.md#handle)** is the reference by which a resource is
  addressed. **A handle is not access.** Holding or transporting a handle conveys no
  read of the payload; reading the data requires a separate, explicit grant
  evaluated at the boundary. <span class="status available">Available</span>
- A **[boundary](glossary.md#boundary)** is the protected execution context where
  resources meet and run, and the **single point where egress is enforced**. Every
  channel out — output, tool call, log, export — routes through it. A run inside the
  boundary has **no ambient power** beyond what it was admitted to do.

## How work happens

- A **[run](glossary.md#run)** is one episode of agent work — one per chat turn —
  performed inside the boundary. It reads/edits/runs in a sandboxed worktree, streams
  progress, and stops at the end of the turn. It consumes only the work it was
  admitted to do. <span class="status available">Available</span>
- **Generating an [output](glossary.md#output) is not releasing it.** Outputs are held
  until **reviewed and explicitly released** to authorized stakeholders. The full
  review/release lifecycle is <span class="status built">Built</span>; the local
  keep-or-reject diff is <span class="status available">Available</span>.
- An **[engagement](glossary.md#engagement)** is the
  [method](glossary.md#method)-owner ↔ [context](glossary.md#context)-owner
  relationship a chat works under — the unit at which **taint** is tracked so an
  output's conservative provenance is the owners of everything that chat read. "Engagement"
  no longer names a mode (ADR 0045 retired the autonomous-vs-interactive axis); what
  survives is **chat-scoped taint**. <span class="status available">Available (as chat-scoped taint)</span>

## Working with others

- A **[workstream](glossary.md#workstream)** is a shared line of work that multiple
  chats greedily auto-sync into. The single-placement sync/promote loop is live;
  cross-authority and real-time multi-user sync are a later (M2) axis.
  <span class="status available">Available</span>
- **[Federation](glossary.md#federation)** moves things between parties — across
  machines or authorities — always with the source's permission *and* the target's
  admission. <span class="status available">Available</span>

    !!! note "What \"Available\" means for federation today"
        Federation is exercised in CI over a **loopback + NAT-isolated harness**
        (cert-pinned TLS, blind relay) — the protocol and its invariants are
        implemented and tested. It is not yet wired into a turnkey, real-world
        cross-machine deployment; standing up two machines is still a manual recipe.
        Treat it as proven in code, not as a one-click feature.

## What this model does not give you today

Honest limits, so you don't over-trust a concept:

- **Local placement does not hide your [method](glossary.md#method) from the
  [context](glossary.md#context) owner.** When you are
  both the host and the context owner (the MVP desktop case), the method and the
  model endpoint are inside the trusted set — secrecy *from yourself* is not
  achievable, only obfuscation. Host-blind method secrecy needs **attested compute**
  ([attestation](glossary.md#attestation)), which is
  <span class="status built">verifier Built</span> /
  <span class="status planned">live Planned</span>.
- **The third-party LLM provider sees plaintext.** Removing the provider from the
  trust boundary (confidential inference) is <span class="status planned">Planned</span>.
  Until then, use a provider you have contracted, or keep sensitive data out of runs.
- **Kernel-enforced method isolation is Linux/macOS only.** The Windows sandbox is
  <span class="status planned">Planned</span>; on Windows the edit/work write-gate is
  not yet kernel-enforced.

??? question "Who can see plaintext, plainly?"
    The **operating host** of the boundary reads plaintext by necessity — it runs the
    work — and so does the **LLM provider you configured**. Other *parties* in a
    project are walled off by resource ownership, taint, and conjunctive egress — but
    those are structural guarantees about *parties*, not about the host or the model.
    Full detail: [Where your data goes](protection.md#where-your-data-goes).

## Next

- **[Glossary](glossary.md)** — one-line definition of every term above
- **[How GaugeWright protects your work](protection.md)** — what's structural vs policy, and where your data goes
- **[Roadmap & status](../reference/status.md)** — the single source for what is Available, Built, or Planned

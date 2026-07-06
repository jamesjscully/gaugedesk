# Glossary

Plain-language definitions of GaugeWright's vocabulary. These are the
user-facing summaries; the formal definitions live in the product specification.

Each term carries a status badge so you can tell at a glance what is usable today
versus what is built-but-not-live or still planned. Status uses one vocabulary,
and the single source of truth for it is the **[Roadmap &amp; status](../reference/status.md)**
table — if a badge here ever disagrees with that table, the table wins.

!!! note "What the badges mean"
    <span class="status available">Available</span> — shipped in the product you can
    download today (the local desktop workbench).
    <span class="status built">Built</span> — implemented and tested in the code,
    but not yet operationally deployed.
    <span class="status planned">Planned</span> — committed and designed, not yet
    built.
    <span class="status none">Not implemented</span> — absent today.

!!! warning "The one counterintuitive truth"
    **Local orchestration is not local inference.** The workbench runs on your
    machine, but the agent's reasoning is performed by the **third-party LLM
    provider you configure** — so your prompts and the in-scope [context](#context)
    are sent to that provider over the network. There is no local model. Who can
    and cannot see plaintext is spelled out in
    **[How GaugeWright protects your work → Where your data goes](protection.md#where-your-data-goes)**.

---

## Core vocabulary

### Project
A **trust and data boundary** — a body of work *plus the parties who may see it*.
It's the root unit: the home for everything one piece of work needs (its context,
its placements, its people). Not a folder — a boundary.
<span class="status available">Available</span>

### Archetype
A named bundle of **method resources + configuration** — the reusable agent
*definition* you build in the workbench (its instructions, skills, tools). On disk
this is the agent's `.pi/SYSTEM.md`, `AGENTS.md`, and `.agent-config.json` that
the [edit chat](#chat) authors. You can [package](#package) an archetype into a
shareable object. <span class="status available">Available</span>

### Placement
An **archetype installed onto a project** — the durable deployment, and the thing
you chat with to do work. Its identity always shows its lineage as
`archetype · project` (e.g. `price-leveling · Peach`), so where a deployed agent
came from is never hidden. <span class="status available">Available</span>

### Chat
A durable line of conversation. **Rooting** — the object you open the conversation
against — fixes the chat's **kind and its write authority** at creation, and is
never toggled (per ADR 0035, rooting is the mechanism, not a mode):

- an **edit chat** is **rooted on an [archetype](#archetype)** — its subject is the
  [method](#method); its output is a published archetype version. It lives
  in the **library**, *not* in a project;
- a **work chat** is **rooted on a [placement](#placement)** — it does the job,
  consumes the method **read-only**, and **cannot** change it. It lives **inside
  that placement, inside a [project](#project)**.

That the write-gate falls out of *what the chat is rooted on* (rather than a runtime
check) is what makes method protection structural.

!!! note "There is one kind of chat"
    Beyond its root, a chat has **no engagement-mode axis** (no
    autonomous-vs-interactive picker) — that axis was retired by ADR 0045. A chat is
    a single agentic loop that runs to completion and stops at the end of a turn; how
    closely you watch and steer is a choice you make in the same chat, not a mode.

<span class="status available">Available</span>

### Run
One unit of work the agent performs, inside the [boundary](#boundary). A run
consumes only the work it was *admitted* to do; in the workbench it produces a
diff you keep or discard. A run's reasoning is performed remotely — see the
[data-flow truth](#core-vocabulary) above. <span class="status available">Available</span>

### Resource
A protected thing the system records, references, or transports — a **method**, a
**context**, or a derived **output**. Method and context are both resources;
neither is privileged. <span class="status available">Available</span>

### Handle
The reference by which resources are addressed. **Holding a handle is not holding
the data** — reading a resource's payload requires a separate, explicit access
grant evaluated at the boundary. <span class="status available">Available</span>

### Method
**Method** is the expert's agent definition (the "how"). **Context** is the
client's data the agent works on (the "what"). Both are
[resources](#resource) — neither is privileged; they are kept apart by the same
boundary, not by being different kinds of thing. The core promise: the method
doesn't leak to the client, and the context doesn't leak to the method-owner or
the runtime. <span class="status available">Available</span>

### Context
The client's data the agent works on — the "what." The in-scope context for a run
is sent to your configured LLM provider for inference; see
[Where your data goes](protection.md#where-your-data-goes).
<span class="status available">Available</span>

### Boundary
The **protected execution context** where an agent runs and the single point where
egress is enforced. It mediates what a run can read, call, and export — a run has
no ambient authority beyond what it was admitted. On Linux and macOS the
method-isolation part of the boundary is enforced by a **kernel sandbox**; the
Windows sandbox is <span class="status planned">Planned</span>.
<span class="status available">Available (Linux/macOS)</span>

### Output
What the agent produces. Generating an output is not releasing it: outputs are
held until reviewed and explicitly released to authorized stakeholders.
<span class="status available">Available</span>

### Workstream
A named, shared line of work inside one placement that a set of chats greedily
auto-sync into — the collaboration axis for working together.
<span class="status available">Available</span>

### Engagement
**Not a standalone product term anymore.** "Engagement" once named both a *mode* on a
chat (autonomous vs interactive) and the chat *container* itself. ADR 0045 retired the
mode axis — there is **one** [chat](#chat), one agentic loop — and the container is now
just the chat. What survives is one piece of vocabulary: **taint is "engagement-scoped"
= [chat](#chat)-scoped**. An output's conservative taint is the owners of everything
*that chat* read up to production, tracked at the boundary at chat granularity. So when
the principles or older specs say *engagement-scoped taint*, read **chat-scoped taint**.
For everything else, see [Chat](#chat).
<span class="status available">Available (as chat-scoped taint)</span>

### Package
A shareable object: a durable manifest that makes an archetype transferable across
parties **without** turning transfer into execution or payload release. The
packaging machinery is implemented; live cross-party hand-off is not yet
operational. <span class="status built">Built</span> <span class="status planned">live deploy Planned</span>

### Runtime
The execution adapter inside a boundary that runs admitted work. It is not an
authority over truth; it consumes admitted work, resolves resources only through
the boundary, and reports observations back.
<span class="status available">Available</span>

### Authority &amp; scope
Every durable fact names the **authority** responsible for it and the **scope** it
affects. No authority may write into a scope it isn't authorized over — this is
how projects and parties stay isolated. <span class="status available">Available</span>

### Account
The **person** behind one or more placements and devices — the "this is all me"
identity. Its identity is a governance root keypair (one root = one person), and a
person's devices act under it. The always-on **blind directory** (sealed account
state available when your own machine is off) and the cross-device account sync are
not built yet. This is **Post-M3** work and has not started.
<span class="status planned">Planned</span>

### Organization
The enterprise-deployment container: the people of one buying company, the
identities they sign in as, and the admin roles (invite, configure, audit, pay).
The home of the enterprise (SSO / SCIM / RBAC / audit) layer. This is the **M3**
enterprise-readiness layer; it is not in production — no SSO/SCIM/RBAC runs today.
<span class="status planned">Planned</span>

### Federation
Moving things (resources, commands, events, outputs) **between parties** — across
machines or authorities. Nothing crosses without the **source** permitting it to
leave **and** the **target** admitting it; relays route encrypted bytes but never
read payload. <span class="status available">Available</span>

### Attestation
Cryptographic proof that an agent ran inside a sealed, verified environment
(a confidential VM), so both parties can trust the method and context stayed
sealed. <span class="status built">verifier Built</span> <span class="status planned">live hosting Planned</span>

---

## Embedding &amp; hosting vocabulary

!!! warning "All Planned"
    The terms in this section describe the **embed surface** — embedding a
    consultant's agent in their own website for end-users. The whole surface is
    <span class="status planned">Planned</span> (see
    [Public hosting / embedded agents](../reference/status.md) and
    [Deployment modes](deployment-modes.md)). Nothing here is usable today; the
    definitions fix the vocabulary so the design and the docs agree. They are
    grounded in the embed-surface contract behind the
    [embed guide](../guides/embed/index.md).

### Audience
The consultant's **end-users** — an identified-but-**non-authority** principal
*inside* the consultant's [authority](#authority-scope), not a separate
[account](#account). The audience never owns product truth; everything it sees is
a projection scoped to its own session (and, when signed in, its own
[chats](#chat)). An audience identity is **provider-asserted**, with
provider-style recovery (email reset / re-auth) — it never mints a keypair.
<span class="status planned">Planned</span>

### Publishable key
The **client-safe** key that a `<gw-session>` snippet carries to talk to a
deployment. It grants only the verbs the deployment chose; anything else is
rejected at admission (fail-closed). It is *publishable* precisely because it
conveys no secret — a separate server-side **secret** key (for backend proxying)
is itself <span class="status planned">Planned</span>.
<span class="status planned">Planned</span>

### Panel ceiling
The **maximum set of panels** a deployment exposes — chat, optionally +viewer,
optionally +files. **Deploy sets the ceiling; the embed picks within it.** The
panel set chosen *is* the redaction: a panel beyond the granted ceiling **does not
render** at all (no broken pane), rather than rendering and failing. Composition is
scope. <span class="status planned">Planned</span>

### Allowed origins
The **origin allowlist** a consultant sets on a deployment — the web origins
permitted to load its embed. A request from an origin that isn't listed is refused;
a [claim token](#claim-token) presented from the wrong origin is rejected
(fail-closed). Paired with budget/quota caps, this bounds where and how much a
deployment can be used. <span class="status planned">Planned</span>

### &lt;gw-session&gt;
The web-component **provider** element that wraps an embed. One `<gw-session>`
holds the deployment target and [publishable key](#publishable-key); the panel
elements inside it render scoped projections for *this* session. The consultant
copies a generated `<gw-session>…</gw-session>` snippet from the desktop Embed /
Preview surface. <span class="status planned">Planned</span>

### &lt;gw-chat&gt;
The **chat panel** web component — the conversation and streaming assistant turns
for one session, placed inside a `<gw-session>`. In authenticated mode it can carry
a **my-chats drawer** for switching between the end-user's own durable chats.
<span class="status planned">Planned</span>

### &lt;gw-chats&gt;
The **standalone history pane** web component — the end-user's own durable chats as
a conversation switcher in its own element, an alternative to the `<gw-chat>`
drawer. Both ship from v1 (authenticated mode only; there is no history in
anonymous mode). <span class="status planned">Planned</span>

### Powered-by / white-label
**Powered-by** is the subtle GaugeWright mark that rides the embed panels on the
free/standard tier. **White-label** — removing the mark — is a **paid upgrade**, a
fourth paid lever beside hosting, compute, and attestation.
<span class="status planned">Planned</span>

### Claim token
A **one-time token** issued at the teardown of an *anonymous* conversation that
lets an end-user **claim that conversation** by signing up — the sole sanctioned,
fail-closed bridge from anonymous to authenticated. A spent, expired, or
wrong-origin token is refused. <span class="status planned">Planned</span>

### Workbench
The local desktop application you download and run — where you build
[archetypes](#archetype), open [chats](#chat), run agents, and review diffs.
"Workbench" is the product; the [runtime](#runtime) is the execution adapter
inside it. <span class="status available">Available</span>

### Stakeholder
A party with a legitimate interest in a [resource](#resource) or
[output](#output) — typically the [method](#method) owner and the
[context](#context) owner of a [project](#project). Protected payload is never
released to a non-stakeholder without an explicit basis.
<span class="status available">Available</span>

### Taint
The conservative record of *whose* data an [output](#output) was derived from —
the owners of everything a [chat](#chat) read up to producing it, tracked at the
[boundary](#boundary). Release checks taint against the stakeholder set, so an
output cannot be released to someone its inputs did not belong to. (Tracked at
chat granularity; see [Engagement](#engagement).)
<span class="status available">Available</span>

### Host
In [federation](#federation), the authority whose machine holds a project's data
and executes its runs after a [handoff](#handoff). The host admits what crosses
to it. <span class="status built">Built</span>

### Operator
In [federation](#federation), an authority that can *drive* runs on a project it
does not [host](#host) (e.g. the consultant after handing a project's home to the
client). The operator places work; the host admits it.
<span class="status built">Built</span>

### Relay
The transport role in [federation](#federation): it queues, retries, and forwards
encrypted bridge messages between machines, but is never a payload authority and
gains no access from carrying a [handle](#handle) (machine-checked, `INV-14`).
<span class="status built">Built</span>

### Handoff
The federation operation that **relocates a project's home authority** from one
party to another (e.g. a consultant creates a project, then hands its home to the
client who will host the data). Each side keeps what it owns; either can drive,
the [host](#host) admits. <span class="status built">Built</span>

---

## See also

- **[Concepts](index.md)** — how these nouns fit together.
- **[How GaugeWright protects your work](protection.md)** — the structural
  guarantees behind handle, boundary, and fail-closed, and where your data goes.
- **[Deployment modes](deployment-modes.md)** — where an agent can run (local,
  federation, attested, hosted/embed).
- **[Roadmap &amp; status](../reference/status.md)** — the single status table this
  page defers to.
- Role guides: **[For experts](../guides/expert/index.md)** ·
  **[For clients](../guides/client/index.md)** ·
  **[For embedded end-users](../guides/embed/index.md)**.

# Run &amp; review work

<span class="status available">Available</span> *(local desktop workbench, Linux/macOS)*

Once you've [built an archetype](build-an-agent.md), you put it to work by
installing it onto a project — a **[placement](../../concepts/glossary.md#placement)** —
and chatting with it in a **work chat**. Each task you give it becomes a
**[run](../../concepts/glossary.md#run)**: one episode of agent execution inside an
isolated **[boundary](../../concepts/glossary.md#boundary)** that produces a *diff*
you review before anything touches your work.

This page walks the loop end to end — place, attach context, run, review — with a
concrete worked example, and states plainly what is and isn't reversible today.

!!! info "Where your data goes"
    GaugeWright orchestrates locally, but it does **not** run the model locally.
    During a run, your prompts and the in-scope context are sent over the network
    to the **third-party LLM provider you configured** for inference. With your own
    provider credentials that relationship is yours — the provider is your
    subprocessor, not GaugeWright's. Read
    [Where your data goes](../../concepts/protection.md#where-your-data-goes) before
    you attach anything sensitive.

---

## 1. Place the archetype on a project

1. Open the project you want to work on (or create one).
2. From the project, **install** the archetype you built. This creates a
   **[placement](../../concepts/glossary.md#placement)** — a live instance of the
   **[method](../../concepts/glossary.md#method)** bound to that project.
3. The placement's identity shows its lineage as `archetype · project`, so it's
   always unambiguous which method is running and where.

A placement opens with a **work chat** rooted on it. A work chat can *read* the
agent's definition but never *change* it — the method-definition surface
(`.pi/**`, `AGENTS.md`, `.agent-config.json`) is editable only from an
**edit chat** (see [Build an agent](build-an-agent.md)).

!!! note "Edit chat vs. work chat"
    You author the method in an **edit chat** (rooted on the archetype). You do
    work in a **work chat** (rooted on a placement). The split is enforced at the
    OS kernel on Linux and macOS, so even a shell inside a run cannot rewrite the
    agent's own instructions. <span class="status available">Available (Linux/macOS)</span> ·
    Windows isolation is <span class="status planned">Planned</span>. See
    [How GaugeWright protects your work](../../concepts/protection.md).

---

## 2. Attach the context the agent needs

Attach the files or folders the agent should work with as
**[context resources](../../concepts/glossary.md#resource)**.

1. In the work chat, attach a **folder** as context.
2. The attachment records *intent*, not access. Resources are addressed by
   **[handle](../../concepts/glossary.md#handle)**; holding a handle conveys no read
   of the payload. The **[boundary](../../concepts/glossary.md#boundary)** decides
   what is actually revealed during a run, under an explicit grant.

!!! warning "Folder-granular today"
    The desktop workbench attaches context **at folder granularity**. Single-file
    attach is <span class="status planned">Planned</span> (tracker **UX-1**) — to
    scope tightly today, point the agent at a folder that contains only what it
    should see.

---

## 3. Run a task

Type the task into the work chat and let it run. Each turn is one
**[run](../../concepts/glossary.md#run)**: it executes inside the boundary's
sandbox and yields a **diff** — the proposed change to your project's files.

A run can only do what it was *admitted* to do — there is no path to executing
work without an explicit admission of its **[method](../../concepts/glossary.md#method)**
and **[context](../../concepts/glossary.md#context)** grants, and a run has
no ambient power to read, retain, or export beyond that. On Linux and macOS this is
enforced at the OS kernel; on Windows the sandbox is
<span class="status planned">Planned</span>. <span class="status available">Available (Linux/macOS)</span>

!!! warning "Network egress is OPEN by default — opt in to contain it"
    The run's sandbox contains the **method** and **context** surfaces, but it does
    **not** contain network reach by default. Out of the box a run can reach the
    LLM provider — and any other host — over the network; deny-by-default network
    isolation is an **opt-in you enable per project** (`network_isolated`, off by
    default), and even then it is kernel-enforced *unfiltered* containment, not a
    per-host allowlist (that egress proxy is deferred infrastructure). Treat the
    network as open unless you have turned isolation on for the project. See
    [How GaugeWright protects your work](../../concepts/protection.md) for the
    honest ceiling on each platform.

### Worked example

Say your archetype is a `price-leveling` analyst and you've attached a folder
`quotes/` containing three vendor quote spreadsheets exported to CSV.

**Context attached:** the folder `quotes/` (3 files).

**Task you type into the work chat:**

> Read every CSV in `quotes/`. Build a single `leveled-comparison.md` that lists
> each line item across all three vendors in one table, normalizes units to a
> per-1000-unit price, and flags any line where the cheapest vendor is more than
> 20% below the median.

The run reasons over the in-scope CSVs (sent to your configured provider), then
proposes a diff. A reviewed diff looks like this — one new file, nothing else
touched:

```diff
+ leveled-comparison.md   (new file, +41 lines)

  | Line item        | Acme    | Brandt  | Corso   | Median  | Flag         |
  |------------------|---------|---------|---------|---------|--------------|
  | Hex bolt M6      |  12.40  |  11.90  |   9.10  |  11.90  | ! Corso -24% |
  | Washer, flat     |   3.05  |   3.20  |   3.10  |   3.10  |              |
  | Gasket, 40mm     |  18.00  |  17.60  |  17.80  |  17.80  |              |
  ...
```

You can read exactly which files the run touched, and why, before deciding —
nothing has been applied to your project yet.

---

## 4. Review, then keep or discard

The work chat surfaces the diff for review. Nothing is applied to your project
until you decide.

1. **Inspect** the diff — the proposed file changes for that run.
2. **Keep** it to apply the change to your project, or **discard** it to throw the
   candidate away.
3. Every run, and your keep/discard decision, is recorded as an immutable event in
   the **append-only history**. <span class="status available">Available</span>

??? question "Can I undo a change after I keep it?"
    **History is auditable, but not yet one-click-undoable.** Every run is an
    immutable event, so you can always *see* what happened and when, and the
    underlying content is git-backed. But there is **no surfaced “undo / revert
    this kept diff” command in the workbench today** (tracker **UX-5**) — the
    review surface exposes discard/isolate of a *candidate* diff, not a revert of
    one you've already applied.

    To reverse a kept change today you work in the project's git history directly,
    outside the GaugeWright UI. A first-class revert command is **Open** on the
    tracker, not yet built — so plan to review carefully before you keep.

!!! note "Structural guarantee vs. operational convenience"
    The **append-only, machine-checked audit log** is a structural guarantee — a
    formal invariant paired with an adversarial test, true on every install. The
    presence of a *surfaced undo button* is an operational/UX matter, and that
    button is not here yet. Keep the two apart when you reason about what you can
    rely on.

---

## 5. Releasing outputs is a separate step

Producing an output inside a run is **not** the same as releasing it to anyone.
An **[output](../../concepts/glossary.md#output)** carries the taint of every
**[stakeholder](../../concepts/glossary.md#stakeholder)** whose data fed it, and
stays **held** until each required stakeholder has reviewed and consented to an
explicit, terminal release.

In the **local single-party workbench** you are the only stakeholder, so review is
trivial for you. The **cross-party review &amp; release surface** — where multiple
stakeholders must consent: a recipient sees a held output and another party's
stakeholder must consent before it crosses the boundary — is
[<span class="status built">Built</span>](../../reference/status.md) in code (the
reducer and per-resource review/export routes exist) but has **no cross-party UI
yet** (tracker **UX-11**).
See [Review &amp; release outputs](../client/review-and-release.md).

---

## 6. Collaborating across chats

Multiple chats can sync into a shared
**[workstream](../../concepts/glossary.md#workstream)** so a team can drive one line
of work together, including across machines via multi-authority
**[federation](../../concepts/glossary.md#federation)**.
<span class="status available">Available</span>

---

## Where this fits

- **Concepts:** [run](../../concepts/glossary.md#run) ·
  [boundary](../../concepts/glossary.md#boundary) ·
  [resource](../../concepts/glossary.md#resource) ·
  [handle](../../concepts/glossary.md#handle)
- **Glossary:** [method](../../concepts/glossary.md#method) ·
  [context](../../concepts/glossary.md#context) ·
  [stakeholder](../../concepts/glossary.md#stakeholder)
- **Protection:** [How GaugeWright protects your work](../../concepts/protection.md)
  and [Where your data goes](../../concepts/protection.md#where-your-data-goes)
- **Roadmap &amp; status:** [What's Available, Built, and Planned](../../reference/status.md)
- **Next in the expert workflow:**
  [Package &amp; deploy](package-and-deploy.md) — share the method with a client
  without handing it over.

# Versioning, freezing &amp; forking

<span class="status built">freeze/version core Built</span> <span class="status available">archetype &amp; chat fork Available</span> <span class="status planned">version-publish &amp; placement-upgrade UI Planned</span>

You're the **method-owner**. This page is about the *evolution* of your method
over time: how a version becomes immutable, how you branch a new line of work,
and how a placement moves onto a newer version. The headline up front is honest:
the **safety model that makes a published version immutable is built and proven in
the core**, but the **day-to-day version UI is not what ships today**. Read the
status badges carefully — this page deliberately tells you which buttons exist and
which are still routes-only or core-only.

!!! info "Where your data goes"
    Versioning and forking are local, append-only operations on your workbench —
    nothing about them sends data anywhere. But the moment you *run* an agent
    (any version), your prompt and the in-scope context go to the third-party LLM
    provider you configured. GaugeWright does no local inference. See
    [Where your data goes](../../concepts/protection.md#where-your-data-goes).

---

## The three things you can do, and who can do them

| Action | What it does | Who | Status |
|---|---|---|---|
| **Freeze a version** | Snapshot a draft archetype into an immutable version that a package can reference | Archetype **owner** | <span class="status built">core Built</span> |
| **Fork** | Branch a *new, named* archetype from an existing one to change behavior | Archetype **owner only** | <span class="status available">Available</span> (coarse) |
| **Edit the method** | Change instructions, skills, tools, config of an archetype | Archetype **owner**, via an **edit chat** | <span class="status available">Available</span> |
| **Upgrade a placement** | Move an installed placement onto a newer archetype version | Placement holder, **deliberately** (opt-in) | <span class="status planned">Planned</span> (no UI route) |
| **Edit a placement** | Change *config / context / notes* — **never** the method | Placement holder | <span class="status available">Available</span> |

The split that matters: **fork and edit-the-method are owner-only and act on the
[archetype](../../concepts/glossary.md#archetype)**. Everything you can do to a
[placement](../../concepts/glossary.md#placement) is **config-only by
construction** — there is no method to write on a placement, because its method is
an installed, read-only version. This is not a permission check you could
mis-set; it falls out of the topology. A client running *your* packaged archetype
therefore cannot alter your method — only fork is the escape hatch, and **only the
owner can fork**. *(Deep link: ADR 0035 — archetype / placement / chat-rooting;
INV-24.)*

---

## Concepts: draft → frozen → packaged → retired

A version moves through a small, fixed set of states. This is the **agent-version
lifecycle** — the safety boundary between editable workbench state and a shareable
method definition. *(Deep link: `specs/lifecycles/agent-version.md`, validated in
Quint.)*

```text
draft  ──freeze──▶  frozen  ──package──▶  packaged
  │                   │
  │abandon            │retire
  ▼                   ▼
abandoned          retired
```

- **draft** — your editable archetype definition. Mutable. **Not packageable.**
- **frozen** — an immutable version snapshot that *records* method resource
  handles, configuration, declared protection posture, provenance, and content
  hashes. A package may reference *only* a frozen version. (The freeze transition
  captures these fields onto the version record; it does not today *validate* their
  completeness as an admission precondition — see [The freeze model](#the-freeze-model).)
- **packaged** — at least one [package](../../concepts/glossary.md#package)
  references this frozen version.
- **retired** — future packaging of this version is blocked; packages that already
  reference it keep working and stay auditable.
- **abandoned** — a draft cancelled before it was ever frozen. Terminal.

!!! note "What 'immutable' actually buys you"
    After freeze, **no method handle, config, protection posture, provenance, or
    hash can change** for that version — ever. A deployed agent therefore never
    silently changes underneath a client. To change behavior you publish a *new*
    version; placements move onto it deliberately, never silently. These are
    machine-checked structural guarantees, not policy promises (see
    [Guarantees](#what-is-guaranteed-vs-what-is-policy) below).

---

## The freeze model

<span class="status built">core Built</span> · <span class="status planned">surfacing UI Planned</span>

!!! warning "This describes the mechanism, not a button you can click today"
    There is **no "Publish version" button** in the shipped workbench. The freeze
    *transition* is implemented and proven in the core reducer
    (<span class="status built">Built</span>), but the author-facing
    version-publish surface is <span class="status planned">Planned</span> (see
    [Status &amp; gaps](#status-and-gaps)). The numbered steps below describe the
    **underlying model** — what the transition does when a flow drives it — not a
    user-facing workflow you can perform end-to-end in the product right now.

The model the freeze transition implements:

1. **Versioning acts on the archetype.** Freeze targets the
   [archetype](../../concepts/glossary.md#archetype), not a placement. The surface
   where a method changes is an *edit chat* — a
   [chat](../../concepts/glossary.md#chat) rooted on the archetype.
2. **The snapshot records the method's defining fields.** The freeze captures the
   method (instructions, skills, tools, `.agent-config.json`, declared protection
   posture) along with provenance and content/resource hashes *where available*
   onto the version record. **These fields are recorded, not enforced:** the
   `freezeVersion` transition's only structural precondition is that the version is
   a `draft`. The core does not today validate that handles, config, posture,
   provenance, and hashes are all present and complete before admitting the freeze
   — completeness is the author's responsibility, not an admission check. *(The
   `VersionRecord` carries these fields; neither `package_flow.rs` nor the Quint
   model `agent-version.qnt` gates the freeze on them.)*
3. **Freeze.** This is the `freezeVersion` transition: it requires a `draft` and
   emits `agentVersionFrozen`. The result is an immutable version snapshot.
4. **Reference it from a package (optional).** Creating a package records the
   *exact* frozen revision (`recordPackageReference`). Packaging records a manifest
   path; it does **not** grant method-payload access or run admission — those are
   separate boundaries. See [Package &amp; deploy](package-and-deploy.md).

!!! warning "Drafts are not packageable"
    You cannot package or deploy a draft. If you try to reference a non-frozen
    version, the core rejects it (`PACKAGE_REQUIRES_FROZEN_VERSION`). This is the
    safety boundary between editable workbench state and a shareable method.

### Retiring a version

<span class="status built">core Built</span>

`retireVersion` blocks *future* package references to a version but does not touch
packages that already reference it (`RETIREMENT_BLOCKS_FUTURE_PACKAGING`,
`PAST_PACKAGES_PRESERVED`, INV-18). Use it to steer new installs onto a newer
version without breaking anyone already on the old one. Retirement is future-only
and reversibility is not part of the model — treat it as a one-way "stop shipping
this version" signal.

---

## How-to: fork an archetype

<span class="status available">Available</span> (coarse)

Fork is the **escape hatch when a project needs genuinely different *behavior***,
not just different config. A placement is config-only, so when "tweak the notes"
isn't enough, you branch a new archetype and own it independently.

1. **Decide you need new behavior, not new config.** If you only need different
   context, notes, or preferences, edit the *placement* instead — don't fork.
2. **Fork the archetype.** From the archetype, fork it. This creates a **new,
   named archetype** that records its parent (`forked_from`). The route the
   workbench calls is `POST /archetypes/:id/fork`.
3. **Edit and publish the fork normally.** The fork is a fresh archetype: open an
   edit chat, change the method, and freeze a version of *it* just like any other
   archetype.

!!! warning "Fork is owner-only — and that is the IP protection"
    Only the **owning authority** can fork an archetype. A placement of *someone
    else's* archetype (a vendor's package you installed) is config-only **by
    construction** — you cannot fork it to extract or re-author their method. This
    is what protects a packaged method's IP in distribution; it is structural, not
    merely a setting. *(Deep link: ADR 0035.)*

??? question "What about forking a *chat* instead of an archetype?"
    You can also fork a [chat](../../concepts/glossary.md#chat) (`POST
    /chats/:id/fork`) <span class="status available">Available</span> — that
    branches a *conversation* (same placement/archetype and kind, recording its
    `forked_from` parent) to explore an alternative line of work. It does **not**
    branch the method. Forking the archetype is what gives you a new method to own.

### What is *not* there yet

The fork that ships is **coarse**: it branches a whole archetype (or a whole
chat). **Fine-grained point fork** — `fork(entryId)` from a specific point in
history — and a visual **fork-tree view** are **not built**
(<span class="status none">Not implemented</span>, tracked as **UX-8**). If you
want to branch from a precise moment, the coarse fork is your only tool today.

---

## How-to: move a placement onto a newer version

<span class="status planned">Planned</span> — no user-facing route today

This is the most important honesty on the page. The **intended** model is:

1. You edit an archetype and **publish a new version**.
2. Each [placement](../../concepts/glossary.md#placement) pinned to an older
   version is **notified an update is available**.
3. The placement holder **upgrades deliberately** — opt-in, never silent — with
   conflict resolution handled conversationally by the Builder Agent.

What **actually ships**: placements pin a method version read-only, and the pin is
surfaced (`pinned_version`). But there is **no `/placements/:id/upgrade` route and
no upgrade-notice surface** in the product today. The package-distribution upgrade
reducer exists in core, yet the placement-level upgrade UX is **open/deferred**
(tracked as **UX-9**). The project-home display of a placement's version and any
"update available" cue likewise waits on the real version-publish flow (**UX-2**).

!!! warning "Do not expect a one-click placement upgrade today"
    Until UX-9 lands, treat placement version as **pinned and static** from the
    UI's perspective. The guarantee you *do* have today is the one that matters
    most: because a frozen version is immutable, a placement's method **cannot
    change underneath you without a deliberate action** — there simply is no live
    path that mutates it silently.

---

## What is guaranteed vs. what is policy

Keep these two registers separate when you reason about safety.

=== "Structural (machine-checked)"

    Proven in the Quint model `specs/models/agent-version.qnt` and enforced by the
    pure `(decide, evolve)` reducer:

    - `PACKAGE_REQUIRES_FROZEN_VERSION` — you cannot package a draft.
    - `FROZEN_VERSION_IMMUTABLE` — a frozen snapshot never changes.
    - `RETIREMENT_BLOCKS_FUTURE_PACKAGING` — retired versions take no new
      package references.
    - `PAST_PACKAGES_PRESERVED` — retiring a version does not break existing
      packages.
    - **Owner-only fork / config-only placement** — a work chat is rooted on a
      read-only installed version, so there is no method to write (INV-24, falls
      out of topology; *deep link: ADR 0035*).

=== "Policy / operational"

    Not invariants — they depend on the flow being wired and on you acting:

    - **Deliberate upgrade** — placements upgrade opt-in, not silently. The
      *intent* is structural, but the upgrade surface itself is
      <span class="status planned">Planned</span> (UX-9), so today this holds
      because no live upgrade path exists at all.
    - **Polished publish UX** — the freeze transition is core-Built; a smooth
      author-facing publish flow is limited today.
    - **Per-OS isolation when you run a version** — method isolation is
      kernel-enforced on **Linux/macOS** (<span class="status available">Available</span>);
      **Windows is <span class="status planned">Planned</span>**. Versioning itself
      is OS-independent; this caveat applies to *running* any version.

---

## Status &amp; gaps

This page mixes statuses on purpose. The single source of truth is
[Roadmap &amp; status](../../reference/status.md); the safety detail lives in
[How GaugeWright protects your work](../../concepts/protection.md).

| Piece | Status | Note |
|---|---|---|
| Freeze / retire / immutability (core reducer + Quint) | <span class="status built">Built</span> | Proven; not a polished publish button |
| Coarse archetype fork (`/archetypes/:id/fork`) | <span class="status available">Available</span> | Owner-only |
| Coarse chat fork (`/chats/:id/fork`) | <span class="status available">Available</span> | Branches a conversation, not the method |
| Edit-the-method (edit chat) | <span class="status available">Available</span> | Owner-only |
| Edit-the-placement (config/context/notes) | <span class="status available">Available</span> | Config-only by construction |
| Point fork `fork(entryId)` + fork-tree view | <span class="status none">Not implemented</span> | **UX-8** |
| Placement upgrade route + update notice | <span class="status planned">Planned</span> | **UX-9** — no `/placements/:id/upgrade` route today |
| Project-home placement version/upgrade display | <span class="status planned">Planned</span> | **UX-2**, waits on version-publish flow |

---

## Where to go next

- **[Build an agent](build-an-agent.md)** — author and refine the archetype you'll
  freeze. <span class="status available">Available</span>
- **[Package &amp; deploy](package-and-deploy.md)** — reference a frozen version
  from a package and deploy it.
  <span class="status built">Built</span> <span class="status planned">live Planned</span>
- **[How GaugeWright protects your work](../../concepts/protection.md)** — the
  structural guarantees behind immutability and owner-only fork.
- **[Roadmap &amp; status](../../reference/status.md)** — the canonical status table.
- **Glossary:** [archetype](../../concepts/glossary.md#archetype) ·
  [placement](../../concepts/glossary.md#placement) ·
  [package](../../concepts/glossary.md#package) ·
  [chat](../../concepts/glossary.md#chat) ·
  [authority &amp; scope](../../concepts/glossary.md#authority-scope)

# Build an agent

<span class="status available">Available</span> (Linux/macOS) ·
<span class="status planned">Windows method-isolation Planned</span>

You're the method-owner. An agent's reusable definition is an
**[archetype](../../concepts/glossary.md#archetype)** — its system prompt,
working instructions, skills, tools, and configuration. You author and change an
archetype from an **edit [chat](../../concepts/glossary.md#chat)**: a chat rooted
on the archetype itself, whose workspace *is* the archetype's own definition repo.

There is no separate form-based editor and no mode to flip. **The chat's root
decides what it can write:** rooted on an archetype ⇒ an *edit chat* that may
write the method; rooted on a [placement](../../concepts/glossary.md#placement) ⇒
a *work chat* that runs the method read-only. This is the same chat lane and
content viewer you use for any work — just pointed at the archetype's repo.

!!! info "Where your reasoning goes"
    Authoring is local orchestration, but **inference is not local**. When you ask
    the edit chat's agent to improve instructions, your prompt and the in-scope
    files are sent to the third-party **LLM provider you configured**. Its
    retention and training terms are the provider's, not GaugeWright's. See
    [Where your data goes](../../concepts/protection.md#where-your-data-goes).
    <span class="status available">Available</span>

## The files you're actually editing

An archetype is a **Pi-native definition surface** living in its repo — there is
no hidden GaugeWright model behind it. The runtime discovers these files from the
working directory:

| File / directory | What it holds |
|---|---|
| `.pi/SYSTEM.md` | The agent's **persona / system prompt** — its identity and how it reasons. |
| `AGENTS.md` | **Working instructions / conventions** the agent follows on every run. |
| `.pi/skills/**/SKILL.md` | The agent's **own skills**. |
| `.pi/extensions/*.{ts,js}` | The agent's **own tools / extensions**. |
| `.pi/prompts/*.md` | Reusable prompt templates. |
| `.pi/settings.json` (+ `.pi/npm`, `.pi/git`) | **Installed packages** (written by `pi install`). |
| `.agent-config.json` | GaugeWright's own governance: model/provider selection and the **declared protection posture** (the boundary policy). Not a Pi file. |

!!! note "`.agent-config.json` is GaugeWright's; the rest is Pi's"
    Everything under `.pi/` and `AGENTS.md` are *method resources* the runtime
    loads natively. `.agent-config.json` is the one file GaugeWright owns — the
    model selection passed to the runtime plus the protection posture the boundary
    enforces. It never duplicates a method resource.
    *(Deep link: ADR 0029 — agent definition is the Pi-native surface.)*

## 1. Create an archetype

1. Open the **Library** facet.
2. Create a new archetype and give it a name that describes the method (for
   example `price-leveling`). This is the thing you'll iterate on and later
   package for reuse.
3. Opening the archetype shows its edit chats, its versions, and everywhere it is
   placed.

## 2. Write the persona and instructions

1. From the archetype, open (or start) an **edit chat**. The workspace tree shows
   the archetype's own repo.
2. Edit `.pi/SYSTEM.md` to set the agent's **persona** — who it is and how it
   reasons.
3. Edit `AGENTS.md` to set the **working conventions** it follows on every run.

You can edit these files directly in the content viewer, or tell the agent in the
edit chat to improve them — the edit chat's agent runs under an *editor* persona
whose job is to write the definition surface. Either way, each change is a
git-backed auto-commit with a reviewable diff, exactly like any other workspace,
so you can see how the definition evolved and roll back.

## 3. Install skills and packages

There is **no structured skills UI** yet — skills and packages are
**installed in the chat** by the agent running an install as a tool line:

```text
pi install -l <package-or-skill>
```

Running `pi install -l …` from the edit chat writes the archetype's own
`.pi/settings.json` (and `.pi/skills/` / `.pi/extensions/`), so the capability
travels with the archetype. Because this writes the definition surface, it only
works in an **edit chat** — a work chat has no method surface to install into.

!!! note "Editor capabilities never run during work"
    Authoring capabilities (edit the definition, install skills, scaffold) live
    only in edit chats. The agent's *own declared* work capabilities are the ones
    that run during a work chat. The two surfaces are separate by construction, so
    a running agent cannot reach the authoring tools.
    *(Deep link: ADR 0033 — capabilities are role-scoped and policy-gated.)*

## 4. Set the model and protection posture

Configuration lives in `.agent-config.json` — edit it through the config panel or
the file directly, from the edit chat.

1. **Model / provider** — select the LLM the runtime passes through to the
   provider. (Leave the model unset to use the provider's default unless you have
   a reason to pin one.)
2. **Declared protection posture** — the boundary `policy`: the *ceiling* this
   method needs to run safely, declared across these axes:

    | Axis | Governs |
    |---|---|
    | `read` | which paths the agent may read |
    | `write` | which paths it may write |
    | `execute` | what it may run |
    | `egress` | what may leave the boundary (network / output) |
    | `controlPlane` | access to control-plane operations |

3. Keep the posture **as tight as the method actually needs**. At publish time it
   is frozen into the [package](../../concepts/glossary.md#package) and becomes the
   minimum boundary ceiling a client must admit before the agent can run there —
   see [Package &amp; deploy](package-and-deploy.md).

!!! warning "The posture is a declaration; the boundary is the enforcement"
    Setting a tight posture in `.agent-config.json` *declares* intent. The actual
    protection — what a run may read, write, run, or send — is decided at the
    [boundary](../../concepts/glossary.md#boundary) at run time, **fail-closed**:
    if a grant is missing or stale, the action is denied. See
    [How GaugeWright protects your work](../../concepts/protection.md).

## 5. Test on a sample project

Place the archetype onto a throwaway project and task it in a **work chat** on
that placement — the method is consumed read-only there, exactly as a real client
would consume it. See [Run &amp; review work](run-and-review.md).

## The edit / work write-gate

The single most important guarantee while authoring:

!!! note "A running agent cannot rewrite its own method"
    The definition surface — `.pi/**`, `AGENTS.md`, and `.agent-config.json` — is
    writable **only from an edit chat**. A work chat runs an installed, read-only
    version: there is no method surface for it to write, so the gate is
    **structural**, not a runtime mode check.

    On Linux and macOS this is enforced by an **OS kernel sandbox** — the
    definition surface is a read-only root, so *every* write path (`edit`,
    `write`, and even a `bash` child process or `chmod`) fails at the kernel. The
    membrane gate stays as defense-in-depth, clean errors, and audit.
    <span class="status available">Available (Linux/macOS)</span>

    Windows method-isolation is not yet available.
    <span class="status planned">Planned</span>

This is a **structural, machine-checked** guarantee (invariant `INV-24`, paired
with an adversarial test that fails if the protection is removed) — not an
operational policy. The full crosswalk is in
[How GaugeWright protects your work](../../concepts/protection.md).

!!! info "Linux/macOS today"
    The kernel-enforced write-gate ships on Linux and macOS. On a platform without
    the sandbox the membrane gate still applies, but the structural kernel
    guarantee does not — check the [roadmap](../../reference/status.md) for current
    platform coverage.

## Versioning &amp; forking

Freezing an immutable version, upgrading placements deliberately, and forking a
new archetype from a version are covered on the
**[Versioning &amp; forking](versioning-and-forking.md)** page.
<span class="status built">Built</span>

## Next

- **[Run &amp; review work](run-and-review.md)** — put the agent to work and review
  its diffs. <span class="status available">Available</span>
- **[Versioning &amp; forking](versioning-and-forking.md)** — freeze a version, upgrade
  placements, fork. <span class="status built">Built</span>
- **[Package &amp; deploy](package-and-deploy.md)** — share it with a client
  without handing over your method.
  <span class="status built">Built</span> <span class="status planned">live deployment Planned</span>
- **[How GaugeWright protects your work](../../concepts/protection.md)** ·
  **[Roadmap &amp; status](../../reference/status.md)**

# Getting started

This guide takes you from download to your first **reviewed run** on the **local
desktop workbench**. <span class="status available">Available</span>

The workbench is the one mode you can use today. Cross-party deployment, attested
compute, enterprise identity, and hosted/embedded agents are
<span class="status built">Built</span> or <span class="status planned">Planned</span> —
see the **[Roadmap &amp; status](reference/status.md)** table, which is the single
source of truth for what's shipped.

!!! warning "Read this first: where your work goes"
    GaugeWright **orchestrates locally, but it does not run a model locally.** The
    workbench runs on your machine; the agent's reasoning is performed by a
    **third-party [LLM provider](concepts/glossary.md#runtime) you configure**, so
    during a [run](concepts/glossary.md#run) your prompts and the in-scope
    [context](concepts/glossary.md#context) are sent to that provider over
    the network. The provider sees that plaintext. Its retention and training terms
    are the provider's, not GaugeWright's.

    This is the one counterintuitive truth to internalise before your first run.
    Full detail: **[Where your data goes](concepts/protection.md#where-your-data-goes)**.
    Removing the provider from the trust boundary (confidential inference) is
    <span class="status planned">Planned</span>, not available today.

---

## Step 0 — Pick a provider and place your credentials

Do this **before** you create anything. Without a working provider credential, a
[run](concepts/glossary.md#run) cannot reach a model and will fail.

### 0.1 Choose a provider

The agent harness inside the workbench calls a standard LLM API. Supported
providers today are the ones the harness speaks to natively:

| Provider | What you supply |
|---|---|
| OpenAI | OAuth sign-in **or** an API key |
| Anthropic | OAuth sign-in **or** an API key |
| Azure OpenAI | endpoint + API key (enterprise / contracted) — the option to reach for when data may not go to a third-party model without a contract/DPA (see warning below) |

!!! note "Bring your own credentials"
    GaugeWright does not resell inference. You authenticate **your own** account,
    so the LLM relationship — and the provider's data terms — is *yours*: the
    provider is *your* subprocessor, not GaugeWright's. This is the BYO-credentials
    model.

!!! warning "Choose a provider you're allowed to send this data to"
    Because prompts and in-scope context leave your machine for the provider, if
    your data may **not** go to a third-party model, use a provider you have a
    contract/DPA with (e.g. Azure OpenAI with a no-training term), or wait for
    confidential inference. <span class="status planned">Planned</span>

### 0.2 Place the credential

The credential is an **[LLM provider](concepts/glossary.md#runtime) credential
(used by the runtime when a run executes)** — the agent harness presents it to the
provider during a [run](concepts/glossary.md#run). You have two ways to provide it:

=== "Sign in (OAuth)"
    The simplest path: link your OpenAI or Anthropic account from the workbench's
    provider settings. The token is **encrypted at rest on the local desktop**
    (AES-256-GCM) so it can be used by a run without sitting in plaintext on disk.
    <span class="status available">Available</span> Live multi-party
    (KMS-backed) encryption is <span class="status built">Built</span>. See
    **[Roadmap &amp; status](reference/status.md)** for the current state of
    KMS-backed encryption.

=== "Environment variable (API key)"
    If you prefer a key, set it in the environment GaugeWright launches in, using
    the provider's standard variable:

    - OpenAI: `OPENAI_API_KEY`
    - Anthropic: `ANTHROPIC_API_KEY`
    - Azure OpenAI: `AZURE_OPENAI_API_KEY` plus your endpoint

### 0.3 Leave the model unset (recommended)

Each [archetype](concepts/glossary.md#archetype) records its model/provider choice
in its **`.agent-config.json`** (you'll see this in Step 4). For your first run,
**don't pin a specific model** — let the harness resolve the default for the
provider you authenticated. Pinning an exact model that your credential isn't
entitled to is the most common first-run failure.

---

## Step 1 — Download and install

Supported platforms: macOS 12+ (Apple Silicon or Intel), Windows 10+ (64-bit), or
a recent x86-64 Linux with WebKitGTK and `git` installed.

!!! note "Which platform to run untrusted methods on"
    The kernel-enforced [method](concepts/glossary.md#method)-isolation sandbox is
    <span class="status available">Available</span> on **Linux and macOS**;
    **Windows** method-isolation is <span class="status planned">Planned</span>.
    Until it ships, run untrusted methods on Linux/macOS. Exact per-build OS and
    runtime requirements ship with each build on the download page — see
    **[Roadmap &amp; status](reference/status.md)** for what is guaranteed.

!!! warning "Builds are currently unsigned"
    Code-signing and notarization are <span class="status none">Not implemented</span>
    today, so desktop builds are unsigned. Your OS will warn on first launch —
    macOS may even say the app is *"damaged and can't be opened"*. The per-OS
    override is below — this is expected, **not** a corrupt or bad download.

Get the build for your platform from
[gaugewright.com/download](https://gaugewright.com/download), then:

=== "macOS"
    1. Open the `.dmg` and drag **GaugeDesk** to Applications.
    2. Because the build is un-notarized, first launch is blocked. Open
       **System Settings → Privacy &amp; Security**, scroll to the **Security**
       section, and click **Open Anyway** next to the GaugeDesk notice, then
       confirm.
    3. If macOS instead says the app *"is damaged and can't be opened"* and no
       **Open Anyway** button appears, clear the download-quarantine flag from
       **Terminal**, then launch normally:
       `xattr -dr com.apple.quarantine /Applications/GaugeDesk.app`

=== "Windows"
    1. Run the `.msi` installer.
    2. At the SmartScreen prompt, click **More info → Run anyway**.

=== "Linux"
    1. Either run the `.AppImage` (`chmod +x` it first) or install the `.deb`.
    2. Confirm `git` is installed (`git --version`) — the workbench uses git to
       store and version your workspace.

---

## Step 2 — Create a project

A **[project](concepts/glossary.md#project)** is the home for one body of work — a
trust and data **[boundary](concepts/glossary.md#boundary)**, not a folder. It
holds the work's context, the agent placed on it, and (later) the people allowed
to see it.

1. From the workbench, create a project and give it a name.

A project opens with a **default [archetype](concepts/glossary.md#archetype)
already installed** as its default [placement](concepts/glossary.md#placement), so
you can run immediately without authoring anything. (Building your own agent is
Step 4, optional for a first run.)

---

## Step 3 — Add your context

Attach the files or folder the agent should work with. These become
**context [resources](concepts/glossary.md#resource)**, addressed by
**[handle](concepts/glossary.md#handle)**.

1. Attach a local folder as the working context.
2. The folder becomes the placement's **git workspace** (its `main` branch is the
   settled state).

!!! note "A reference is not access"
    Attaching context records *intent*; it does not hand the agent the data. A
    handle conveys no read on its own — the [boundary](concepts/glossary.md#boundary)
    decides what's actually revealed during a run, under an explicit grant, and
    **fails closed** if a grant is missing.
    <span class="status available">Available</span> See
    [How GaugeWright protects your work](concepts/protection.md).

---

## Step 4 — (Optional) Define your own agent

For a first run you can skip this and use the default archetype. To build your
own, open an **edit [chat](concepts/glossary.md#chat)** rooted on the archetype and
edit its definition surface:

- **`.pi/SYSTEM.md`** — the agent's persona / system prompt.
- **`AGENTS.md`** — working conventions / instructions.
- **`.agent-config.json`** — model/provider selection and protection posture.

!!! note "Edit chat vs. work chat"
    Only an **edit chat** (rooted on an archetype) can change these files. A **work
    chat** (rooted on a placement, Step 5) can *read* the method but **never write
    it** — so a running agent can't rewrite its own instructions. This is enforced
    by an **OS kernel sandbox**, not convention.
    <span class="status available">Available (Linux/macOS)</span>; the Windows
    method-isolation sandbox is <span class="status planned">Planned</span>.

Full authoring workflow: **[Build an agent](guides/expert/build-an-agent.md)**.

---

## Step 5 — Run a task and review the diff

This is the first reviewed run — the whole point of the guide.

1. Open a **work chat** on the project's placement. Its identity shows its lineage
   as `archetype · project` (e.g. `default · acme-report`), so it's always clear
   which method is running and where.
2. Type a concrete task, e.g.:

    > Read `notes.md` and write a one-paragraph summary into `summary.md`.

3. Watch it work. The run executes in an **isolated worktree** off `main`. Your
   prompt and the in-scope context are sent to your configured provider for
   inference — see [Where your data goes](concepts/protection.md#where-your-data-goes).
4. When the turn finishes, the workbench shows a **diff** — the proposed change to
   your workspace.
5. **Keep** it (the diff merges to `main`) or **discard** it. Nothing is applied
   without your decision, and every run is recorded in the **append-only history**,
   so you can audit what happened and roll it back with `git`.
   <span class="status available">Available</span>

That's a complete reviewed run: point an agent at your files, give it a task, get
a result you reviewed and chose to keep.

---

## First-contact troubleshooting

??? question "macOS/Windows won't let me open the app"
    Expected — builds are unsigned (see Step 1). On **macOS**, right-click the app
    icon and choose **Open** (the double-click "Move to Trash" path won't offer the
    override). On **Windows**, click **More info → Run anyway** at the SmartScreen
    prompt. Code-signing is <span class="status none">Not implemented</span> today.

??? question "A run was denied / nothing happened"
    A denial usually means a **missing grant**, not a bug — the boundary
    **fails closed**, so if a required access grant is missing, stale, or
    uncertain, the action is refused rather than allowed on doubt.
    <span class="status available">Available</span> Check that the file the agent
    tried to read is attached as context (Step 3) and inside the project's
    workspace. See [How GaugeWright protects your work](concepts/protection.md).

??? question "The run failed to reach a model / authentication error"
    Re-check **Step 0**: the provider credential must be linked (OAuth) or its
    `*_API_KEY` set in the environment GaugeWright launched in. If you pinned a
    specific model in `.agent-config.json`, your credential may not be entitled to
    it — clear the model selection and let the provider's default resolve.

??? question "Can I keep my data fully on my machine?"
    Not for inference — there is **no local model**. Orchestration and storage are
    local, but reasoning goes to your configured provider. If that's not
    acceptable, use a provider you've contracted, or wait for confidential
    inference (<span class="status planned">Planned</span>). Detail:
    [Where your data goes](concepts/protection.md#where-your-data-goes).

??? question "Can the agent reach the network or read files outside the project?"
    The boundary's egress chokepoint and consent rules are always on, but the
    workbench's **network-egress default is open per project** for low-friction
    first runs — a non-isolated project's containment ceiling is lower. You can opt
    a project into kernel-enforced network isolation (Linux/macOS).
    <span class="status available">Available (Linux/macOS)</span>

---

## Next steps

- **[Concepts](concepts/index.md)** — the mental model (archetype, placement, chat,
  project) and the [glossary](concepts/glossary.md).
- **[How GaugeWright protects your work](concepts/protection.md)** — the boundary,
  the data-flow truth, and which guarantees are structural vs. operational.
- **[Build an agent](guides/expert/build-an-agent.md)** · **[Run &amp; review work](guides/expert/run-and-review.md)** — go deeper as an expert.
- **[Roadmap &amp; status](reference/status.md)** — what's shipped and what's next.

# Embedding an agent (integrator & end-user)

<span class="status built">core Built</span> <span class="status planned">live Planned</span>

This page is for two readers: the **integrator** — a consultant (or their web
developer) who wants to embed a GaugeWright agent in their own website — and the
**end-user** — that consultant's customer, who chats with the agent in the browser.
The two sit on opposite sides of a trust line, and the page keeps them apart on
purpose.

!!! warning "Status — read this first"
    **Nothing embeddable ships today.** You cannot, in the product you can download
    now, deploy a public agent and paste a snippet into a live site. The whole embed
    surface is <span class="status planned">Planned</span> for end-users.

    What is <span class="status built">Built</span> (implemented and tested in the
    codebase, but **not operationally live**) is the *core*: the
    [audience](../../concepts/glossary.md#audience)-identity seam, the durable-chat
    data layer, the scoped remote session, and the web-component elements
    (`<gw-session>` / `<gw-chat>` / `<gw-chats>`). They cannot run end to end without
    the **managed host** that serves a live per-visitor session — and that host is
    infrastructure that does not run in the local scaffold. Until it ships, treat
    every snippet and config field below as the **designed shape**, not a switch you
    can flip. The single source of truth for status is the
    [roadmap](../../reference/status.md).

!!! info "Where your data goes"
    GaugeWright orchestrates locally, but **it does not run the model.** Every turn
    an end-user takes sends their prompt and the in-scope
    [context](../../concepts/glossary.md#context) to the **third-party LLM provider
    the consultant configured**, over the network, in plaintext. There is no local
    inference. On top of that, an embedded agent runs on a **non-attested** managed
    host the platform operates — so the platform operator can, in principle, see
    what runs there. Both facts must be disclosed to end-users (see
    [End-user disclosure](#end-user-disclosure-what-to-tell-your-visitors)). Full
    detail: [Where your data goes](../../concepts/protection.md#where-your-data-goes).

---

## The two sides, and the one rule that separates them

An embed has two audiences with completely different authority postures. Conflating
them is the mistake the whole design exists to prevent.

| Side | Who | Authority | Where they work |
|---|---|---|---|
| **Consultant / integrator** | the agent's owner | full [authority](../../concepts/glossary.md#authority-scope), acting as their [account](../../concepts/glossary.md#account) | the **desktop** workbench (deploy + monitor) |
| **End-user** | the consultant's customer | **never an authority** — a provider-asserted [audience](../../concepts/glossary.md#audience) principal *inside* the consultant's scope | the **browser** (embedded panels) |

The governing rule is the workbench rule applied **across a trust boundary**:
embedded panels read **scoped** projections and submit **scoped** commands; the
end-user never owns product truth, and anything outside the granted
[scope](../../concepts/glossary.md#authority-scope) **fails closed** rather than
degrading. An end-user is an *identified actor*, not an authority — so rendering to
them is not a cross-authority crossing, and the consultant stays the one responsible
party for everything the deployment emits.

!!! note "Three identity classes — keep them straight"
    An [account](../../concepts/glossary.md#account) is a self-sovereign person
    (keypair, seed recovery) — that's the consultant. An
    [organization](../../concepts/glossary.md#organization) is a company you join via
    SSO. The end-user is the **audience**: a *third* class, **provider-asserted**
    (managed login or the consultant's identity provider), never holding a keypair or
    a seed phrase. The embed surface never mints an account for an end-user.

---

## For the integrator

You build and deploy from the **desktop app**, where you already build agents —
there is no separate web console. You act there as your
[account](../../concepts/glossary.md#account).

### What you will embed (the snippet shapes)

Panels ship as framework-agnostic **custom elements**, so you compose them inside
your own page in any stack (shadow-DOM isolation, CSS-variable theming). A single
`<gw-session>` provider opens the scoped session and holds the connection; the panels
nested inside bind to it.

!!! warning "All shapes below are <span class=&quot;status planned&quot;>Planned</span> for live use"
    The elements are <span class="status built">Built</span> in code but there is no
    published `embed.js` bundle and no live host to point them at yet. These are the
    designed shapes.

=== "Chat only (the narrowest keyhole)"

    ```html
    <script src="cdn.gaugewright.com/embed.js"></script>

    <gw-session deployment="book-bot" key="pk_live_…">
      <gw-chat></gw-chat>
    </gw-session>
    ```

    The narrowest [panel ceiling](../../concepts/glossary.md#panel-ceiling): the
    end-user can send messages and receive streamed answers, nothing more.

=== "Chat + viewer + files"

    ```html
    <gw-session deployment="report-bot" key="pk_live_…">
      <gw-chat></gw-chat>
      <gw-viewer></gw-viewer>   <!-- read + download this session's own artifacts -->
      <gw-files></gw-files>     <!-- browse this session's own worktree -->
    </gw-session>
    ```

    Widening the ceiling: `+viewer` exposes this session's produced
    [outputs](../../concepts/glossary.md#output); `+files` exposes its own worktree.
    The viewer and files panels are <span class="status planned">Planned</span>
    (later than the MVP chat panel).

=== "Authenticated, with history"

    ```html
    <gw-session deployment="advisor-bot" key="pk_live_…" auth="managed">
      <gw-chats></gw-chats>     <!-- standalone history pane: this user's own chats -->
      <gw-chat></gw-chat>       <!-- a drawer switcher also lives inside <gw-chat> -->
    </gw-session>
    ```

    In authenticated mode an end-user signs in and can return to their own durable
    [chats](../../concepts/glossary.md#chat). History ships **two ways** from v1: a
    drawer inside [`<gw-chat>`](../../concepts/glossary.md#gw-chat) *and* a standalone
    [`<gw-chats>`](../../concepts/glossary.md#gw-chats) element.

!!! note "Composition **is** scope"
    Choosing a panel set is not cosmetic — it **is** the redaction. Each panel
    carries a fixed projection scope and verb set. **Deploy sets the ceiling; the
    embed picks within it.** A `<gw-files>` panel against a chat-only key does **not
    render** — never a silent widening, never a broken pane. That is the fail-closed
    rule made visible.

### How to deploy and embed (the designed sequence)

Each step is <span class="status planned">Planned</span> for live use; the badges
mark what exists in code.

1. **Open the placement you want to publish.** In the desktop, select the
   [placement](../../concepts/glossary.md#placement) (an
   [archetype](../../concepts/glossary.md#archetype) on a project at a pinned
   version) you have proven on real work. A **public deployment** (this same
   [placement](../../concepts/glossary.md#placement), marked public and hosted on the
   managed platform host) is the deployable form of that placement — the words name
   the same thing seen from two sides: *placement* is the local, durable install;
   *deployment* is that placement published to serve end-users. Every `deployment=`
   attribute in the snippets above targets one of these.
   <span class="status built">model Built</span>
2. **Set the panel ceiling.** Choose the *maximum* panel set the deployment will ever
   expose: `chat`, `+viewer`, or `+files`. This is the scope ceiling the publishable
   key will be bound to. <span class="status planned">Planned</span>
3. **Choose the auth mode.** One of:
    - **`anonymous`** — ephemeral, identity-less visitors; no history, conversation
      discarded on teardown.
    - **`managed`** — the platform runs a lightweight login (email / magic-link /
      social) so a consultant with no identity provider gets sign-in for free.
    - **`byo-oidc`** — you point the deployment at your own IdP (reuses the operator
      OIDC/JWKS/PKCE machinery), or silent token pass-through when your host site has
      already signed the user in.

    <span class="status built">adapters Built</span>
    <span class="status planned">live Planned</span>
4. **Register your allowed origins.** List the exact site domain(s) the key may mint
   sessions from. A lifted key used on another site **fails closed**. See
   [allowed origins](../../concepts/glossary.md#allowed-origins).
   <span class="status planned">Planned</span>
5. **Set budget and quota caps.** A hard spend ceiling plus per-visitor/IP rate
   limits and a max-concurrent-sessions cap. When the budget is hit the deployment
   **fails closed** — sessions show "temporarily unavailable", they do not silently
   degrade or overspend. (Because anonymous agents let *anyone* spend your compute,
   these caps are how you stay safe.) <span class="status planned">Planned</span>
6. **Acknowledge the credential ceiling, then deploy.** The hosted agent runs on
   **your account's sealed model credential** (your linked OpenAI/Anthropic OAuth
   token) on a **non-attested** host. Deploy requires an explicit acknowledgement
   that your credential runs on the platform's non-attested host; you cannot deploy
   without it. (The attested host is the premium alternative —
   <span class="status planned">Planned</span>.)
   <span class="status planned">Planned</span>
7. **Copy the snippet and the publishable key.** The desktop generates the exact
   `<gw-session>…</gw-session>` block plus the **publishable** key to paste into your
   site. <span class="status built">elements Built</span>
   <span class="status planned">publish Planned</span>
8. **Preview before going live.** A live in-desktop preview points our panels at the
   deployment so you see exactly what a visitor sees. The preview cannot bypass the
   deployment's panel/verb scope. <span class="status planned">Planned</span>

### Keys: publishable vs. secret, and rotation

The embed key lives in your page's HTML, where anyone can read it — so it is a
**publishable** key (like a `pk_…`), **not** a bearer secret.

| | **Publishable key** | **Secret server-side key** |
|---|---|---|
| Lives in | page source (public) | your backend only |
| Grants | only the chat/panel verbs the ceiling allows | backend API proxying |
| Protected by | origin allowlist + quotas + budget cap | not exposed to the browser |
| Status | <span class="status built">Built</span> <span class="status planned">live Planned</span> | <span class="status planned">Planned</span> (deferred) |

Because the publishable key is public, it is **never enough on its own**: it only
mints sessions from your [allowed origins](../../concepts/glossary.md#allowed-origins),
within your quotas, up to your budget cap — all fail-closed. **Rotate the key** from
the desktop's Embed/Preview surface if it leaks or on a schedule; rotation issues a
new key and invalidates the old. A snippet must **never** carry a secret key — the
desktop emits publishable keys only. A separate secret key for backend proxying is
<span class="status planned">Planned</span>.

??? note "For the spec-minded"
    The publishable key is a capability-scoped principal: the admission shell's pure
    `decide` checks its verb set, and rejects anything outside it (fail-closed,
    `INV-20`). The non-attested honest ceiling, the credential carry, and the
    hosting-as-meter tier come from
    ADR 0050;
    the panel surface, the two auth modes, and end-user identity from
    ADR 0051.
    The full surface contract is `specs/experience/embed-surface.md`.

### Monitoring a live deployment

From the desktop's monitor surface (all <span class="status planned">Planned</span>
for live use; projections are <span class="status built">Built</span>) you will see
live and recent sessions, **spend vs. the budget cap** with the fail-closed ceiling
visible, the **audience directory** (authenticated mode only — end-users and their
durable chats), and a "what visitors ask" topic insight rolled up over retained
transcripts. From there you can pause or disable the deployment, **redeploy** a new
version, edit origins/budget/quotas, rotate the key, open a session transcript, and
request erasure of a transcript or an
end-user's data.

!!! warning "What the monitor must never do"
    These are scoped projections, **not** authority — rebuilding them changes no
    product truth. The monitor never surfaces one end-user's chats to another or into
    the aggregate insight without scope, and it can never permit spend past the cap.

### Pricing levers

Embedding is billed on **hosting / compute**, not on
[attestation](../../concepts/glossary.md#attestation) — a public book-explainer has
nothing confidential to seal, so attestation cannot be the meter. There are four paid
levers:

- **Hosting** — having an always-on managed deployment at all.
- **Per-visitor compute** — snapshot storage, restore count, runtime-seconds.
- **Attestation** — the premium *attested host* ceiling, when a deployment does need
  sealing. <span class="status planned">Planned</span>
- **White-label** — removing the **"powered by"** mark. A subtle powered-by mark
  rides the panels on the free/standard tier; removing it is a paid upgrade gated
  under the deployment's hosting entitlement. See
  [powered-by / white-label](../../concepts/glossary.md#powered-by-white-label).

---

## For the end-user

This section is what the consultant's customer experiences in the browser. (It is
all <span class="status planned">Planned</span> — no embedded agent runs end to end
today.)

### Anonymous vs. signed-in

- **Anonymous.** You use the agent without logging in. The session is ephemeral and
  isolated; when it ends, the conversation is discarded — there is no history to come
  back to. (A single "a session happened" record and the retained transcript are kept
  on the consultant's side; the live conversation itself is not durable truth.)
- **Signed-in.** You sign in via whatever the consultant configured (a managed login,
  or "sign in with {your IdP}"), and your [chats](../../concepts/glossary.md#chat)
  become **durable**. You can return later — from any device — and pick up an old
  conversation in **my-chats**, the history switcher (a drawer inside the chat, or its
  own pane). You see **only your own** chats, never anyone else's.

!!! note "You are an identified actor, not an authority"
    Signing in gives you a **provider-asserted** identity inside the consultant's
    project — not a GaugeWright [account](../../concepts/glossary.md#account), no
    keypair, no seed phrase. Recovery is provider-style (email reset / re-auth). The
    consultant's scoping governs everything you can see and do.

### Claiming an anonymous conversation

If you start anonymously and then decide to sign up, you can **carry that one
conversation into your new identity** via a one-time
[claim token](../../concepts/glossary.md#claim-token) offered when the anonymous
session ends. This is the **only** sanctioned bridge across the
anonymous/signed-in isolation line, and it is opt-in and fail-closed: a spent,
expired, or wrong-site token grants nothing, and without a presented token the two
modes stay completely separate. The claim flow is
<span class="status planned">Planned</span> (decided, build deferred).

### What you should know about your data

This is the disclosure language a consultant should surface to visitors, stated
plainly:

- **Your messages are read by a third-party LLM.** The agent does not run on the
  consultant's computer or "locally". Every message you send, and the context the
  agent works over, is sent to a **third-party LLM provider** over the network to
  generate the reply. That provider's retention and training terms are the
  *provider's*, not GaugeWright's.
- **The host is not attested.** The agent runs on a managed host the platform
  operates, which is **not** a sealed/attested environment — the platform operator
  can, in principle, see what runs there. (Attested hosting is a separate, premium
  option that is <span class="status planned">Planned</span>.)
- **You are isolated from other visitors.** Your session sees only what *you* made
  here; you never see another visitor's conversation, files, or artifacts, and no
  panel exposes the consultant's private workspace or method.
- **Retention and erasure.** Anonymous conversations are discarded when the session
  ends. Signed-in chats are retained so you can return to them, and can be **deleted**
  on request (the consultant can erase a transcript or your data;
  signed-in users can delete their own chats). See
  [Where your data goes](../../concepts/protection.md#where-your-data-goes).

---

## Guarantees: structural vs. operational

GaugeWright keeps machine-checked structural guarantees apart from policy/operational
ones so claims stay defensible.

| Guarantee | Kind | Status |
|---|---|---|
| A panel beyond the granted ceiling does not render (fail-closed) | Structural — model-checked | <span class="status built">Built</span> |
| Commands outside the key's verb scope are rejected at admission | Structural — model-checked | <span class="status built">Built</span> |
| One end-user never sees another's session or chats | Structural — model-checked | <span class="status built">Built</span> |
| A [handle](../../concepts/glossary.md#handle) is not the bytes; downloads cross only via resource-export | Structural — model-checked | <span class="status built">Built</span> |
| Spend past the budget cap is impossible (fail-closed) | Structural — model-checked | <span class="status built">Built</span> |
| The end-user is never an authority; the consultant stays responsible | Structural — model-checked | <span class="status built">Built</span> |
| The managed host is honest about being non-attested | Operational — platform-operator trust | <span class="status planned">Planned</span> (host not live) |
| The inference provider is inside the trust boundary (it sees prompts + context) | Operational — current reality | <span class="status available">Available (current state)</span>; <span class="status planned">removing it — confidential inference — Planned</span> |
| A live per-visitor session actually runs | Operational — needs the managed host | <span class="status planned">Planned</span> |

!!! note "No per-OS sandbox caveat applies here"
    The kernel-enforced method-isolation sandbox (Linux/macOS
    <span class="status available">Available</span>; Windows
    <span class="status planned">Planned</span>) protects the consultant's *local*
    build loop. An embedded agent runs on the **managed host**, not the visitor's or
    the consultant's machine, so that per-OS sandbox is not what guards an embed — the
    scope/isolation/fail-closed boundary above is. See
    [How GaugeWright protects your work](../../concepts/protection.md).

---

## Known gaps (today)

Stated here, not only on the external trust site:

- **No usable embed path.** There is no published `embed.js`, no live managed host,
  and no deploy/monitor surface you can operate. The whole embed surface is
  <span class="status planned">Planned</span> for end-users.
- **The viewer and files panels are later than MVP.** MVP is the chat panel
  (anonymous + managed-auth) with my-chats. The
  [viewer](../../concepts/glossary.md#output) and files panels, BYO-OIDC + token
  pass-through, the claim flow, white-label, the attested-host ceiling, and the
  server-side secret key are all <span class="status planned">Planned</span> after
  MVP.
- **Inference is remote and inside the trust boundary.** Visitor prompts and in-scope
  context reach the third-party provider the consultant configured; confidential
  inference (removing the provider from the boundary) is
  <span class="status planned">Planned</span>.
- **The managed host is non-attested.** Attested hosting for embeds is
  <span class="status planned">Planned</span>; until it ships, the platform operator
  is inside the trust boundary of any embedded deployment.

---

## Where to go next

- **Build the agent you'll embed** → [Build an agent](../expert/build-an-agent.md)
- **The consultant-side deploy story in full** →
  [Package & deploy](../expert/package-and-deploy.md)
- **What protects the work, and who can see plaintext** →
  [How GaugeWright protects your work](../../concepts/protection.md)
- **Where the agent runs and who's involved** →
  [Deployment modes](../../concepts/deployment-modes.md)
- **The single status source** → [Roadmap & status](../../reference/status.md)
- **Terms used here** → [Glossary](../../concepts/glossary.md#audience)

# Troubleshooting runs &amp; deployment

<span class="status available">Available</span> (local desktop)

This page is for the **method-owner** when something a run *should* do is refused,
or a deployment won't go live. GaugeWright is **fail-closed by design**: when a
required grant is missing, stale, or uncertain, the action is **denied** rather
than allowed on doubt ([INV-20]). So most "errors" you'll hit are not bugs — they
are the boundary doing its job. The goal of this page is to tell you, for each
denial, *which grant is missing and how to restore it* in concrete steps.

!!! warning "The one counterintuitive truth — read this first"
    GaugeWright orchestrates **locally**, but it does **not** run inference
    locally. The agent's reasoning is performed by the **third-party LLM provider
    you configure**, so your prompts and the in-scope
    [context](../../concepts/glossary.md#context) are sent to that provider
    over the network. Many "the run failed" reports are really *provider* failures
    (auth, quota, blocked egress). See
    [Where your data goes](../../concepts/protection.md#where-your-data-goes).

## How to read a denial before you debug it

A [run](../../concepts/glossary.md#run) only reaches the `running` state after it
is **admitted**, and admission requires several independent things to be true at
once. A denial names *which* one was missing. The checks, in order:

| Gate | What it requires | Lifecycle source |
|---|---|---|
| **Boundary is active** | The [boundary](../../concepts/glossary.md#boundary) ceiling is declared and every participant has accepted. | `boundary` lifecycle |
| **Method access granted** | A current grant to read the method definition. ([INV-12]) | `resource-access` |
| **Context access granted** | A current grant for each in-scope context [resource](../../concepts/glossary.md#resource). ([INV-12]) | `resource-access` |
| **Run admitted** | The run was admitted against those grants — no ambient authority. ([INV-11]) | `run` lifecycle |
| **Entitlement (governed deploys only)** | An active deployment entitlement for this context. | `deployment-entitlement` |

!!! note "Structural vs. operational denials — don't conflate them"
    Some denials come from **structural, machine-checked invariants** (a handle
    isn't access; a run has no ambient authority; method edits are kernel-blocked).
    These are guarantees, not toggles — see
    [the two kinds of guarantee](../../concepts/protection.md#two-kinds-of-guarantee-dont-conflate-them).
    Others are **policy / operational** (an expired grant, an org policy, a missing
    provider key). You *fix* the operational ones; you *work with* the structural
    ones.

---

## 1. A run was denied: missing or stale grant (fail-closed)

This is the most common case. A run that ran yesterday now refuses, or a freshly
attached context isn't readable. Under fail-closed, a grant that is **requested,
denied, revoked, or expired** is treated exactly like no grant at all — the
boundary will not resolve the [handle](../../concepts/glossary.md#handle) from it.

**Diagnose, then fix:**

1. Open the **work chat** (the chat rooted on the
   [placement](../../concepts/glossary.md#placement)) where the run was denied. The
   denial states which resource/purpose was refused.
2. Find that [resource](../../concepts/glossary.md#resource) in the project. Check
   its access state:
    - **requested** — the grant was never approved. The required approver is the
      resource **owner** (for single-owner inputs) or **every stakeholder** whose
      payload the access would reveal (for derived [outputs](../../concepts/glossary.md#output)).
      Have them approve.
    - **revoked** — someone withdrew the grant. Revocation is **future-only**: it
      blocks new reads but never rewrites prior history ([INV-18]). Request a fresh
      grant; you cannot "un-revoke".
    - **expired / denied** — terminal. Request access again from scratch.
3. Re-request access (resource → boundary/runtime/use, by owner/stakeholder, for
   purpose) and have the required approver(s) consent. The grant moves to
   **granted**.
4. **Re-run.** A run is admitted against grants *at admission time*; a grant that
   became valid after the denial does not retroactively admit the old attempt.

??? question "Why did attaching the context not make it readable?"
    Attaching context records **intent**, not permission. The
    [boundary](../../concepts/glossary.md#boundary) decides what is actually
    revealed during a run, and it needs an explicit grant for *each* in-scope
    resource. A handle is never access — holding or transporting it conveys no read
    ([INV-10]). This is structural; there is no setting that makes attachment imply
    a read.

??? question "I retried a failed run and it was denied again. Why?"
    A retried run must be **re-admitted**, and its scope can only be a **subset** of
    the original — it can never widen ([INV-17]). If the retry asked for a resource
    or purpose the first attempt didn't have, it is correctly refused. Narrow the
    retry, or request the extra access explicitly first.

!!! warning "Fail-closed is not negotiable"
    There is no "allow on uncertainty" flag. If a grant is missing, stale, or
    uncertain, the run is denied ([INV-20], model `fail-closed.qnt`). The fix is
    always to restore the *grant*, never to relax the boundary.

---

## 2. Per-OS sandbox limits (method isolation)

Kernel-enforced **method isolation** makes the definition surface — `.pi/SYSTEM.md`,
`AGENTS.md`, `.agent-config.json` — a read-only root for a running agent, so even a
shell `bash` call inside a run cannot rewrite the agent's own method ([INV-24]).
A [work chat](../../concepts/glossary.md#chat) may *read* the definition but never
mutate it. This is enforced differently per platform.

=== "Linux / macOS"

    Kernel-enforced method isolation is <span class="status available">Available</span>.
    The sandbox makes the definition surface read-only at the kernel, so the
    protection holds even against a shell inside the run. If a run *tries* to write
    `SYSTEM.md`/`AGENTS.md`/`.agent-config.json`, the write is blocked by the OS,
    not by tool-gating — expect a permission/IO error from inside the run, which is
    correct behavior, not a bug.

=== "Windows"

    The kernel sandbox that enforces method isolation is
    <span class="status planned">Planned</span> — **not in force on Windows today.**
    Until it ships, the OS-level read-only guarantee described above does **not**
    apply. **Run untrusted or third-party methods on Linux or macOS**, where the
    sandbox is active. See
    [platform limitations](../../reference/limitations.md#platform-and-per-os-limitations).

!!! note "You meant to edit the method, and it was blocked"
    If you are *trying* to change instructions/skills/tools and the surface is
    read-only, you are in a **work chat**. Method edits are only allowed from an
    **edit chat** (rooted on the [archetype](../../concepts/glossary.md#archetype)).
    Switch to the edit chat and change `.pi/SYSTEM.md` / `AGENTS.md` /
    `.agent-config.json` there. See [Build an agent](build-an-agent.md).

---

## 3. Provider / credential failures (inference)

Because inference is remote, a large share of run failures originate at the
**LLM provider you configured**, not inside GaugeWright. The agent has **no ambient
authority**, and a run reaches the network only as allowed by the project's egress
posture — so a misconfigured provider, a missing credential, or a network-isolated
project surfaces as a failed run.

**Work through these in order:**

1. **Confirm a provider/model is set.** In the archetype's **edit chat**, open
   `.agent-config.json` and verify the **model/provider** field is set. A blank or
   unresolved model is the most common cause of an immediate run failure.
2. **Check the credential.** The provider API key is supplied as an **environment
   key** (your provider's API-key variable), referenced from `.agent-config.json` —
   never pasted into instruction files. A missing/expired/typo'd key shows as an
   auth/401 from the provider. Re-set the env key and re-run.
3. **Auth, quota, rate limits.** 401/403 = bad or revoked key; 429 / quota errors =
   you've hit the provider's rate or spend limit. These are governed by **your**
   account with the provider, not by GaugeWright — fix them in the provider's
   console.
4. **Egress was blocked by network isolation.** By default a project's network
   egress is **open** (a chat can reach the model out of the box), so this is rarely
   the cause. But if the project has **network isolation** enabled
   (`network_isolated`), the run cannot reach the network and the provider call
   fails. Turn isolation off for that project to allow the call — and note that,
   because a per-host model-endpoint allowlist proxy is not yet built
   (<span class="status planned">Planned</span>), isolation today means *no* egress,
   not filtered egress.

!!! warning "A provider failure means your prompt already left"
    By the time a provider returns an auth/quota error, your prompt and in-scope
    context have been **sent to that provider**. The provider's retention/training
    terms are the **provider's**, not GaugeWright's; with your own credentials it is
    **your** subprocessor. If your data may not leave for a third-party model, use a
    provider you have contracted, or wait for **confidential inference**
    <span class="status planned">Planned</span>. See
    [Where your data goes](../../concepts/protection.md#where-your-data-goes).

??? question "How do I change which provider gets my data?"
    Open the [archetype](../../concepts/glossary.md#archetype)'s **edit chat** and
    edit the model/provider in `.agent-config.json`. Never change it from a work
    chat — the definition surface is read-only there ([INV-24]).

---

## 4. Deployment won't go live (cross-party)

If you are trying to **deploy a packaged agent to a client** and it does not
become runnable, first check status: cross-party deployment is
<span class="status built">Built</span> <span class="status planned">live Planned</span> —
it is implemented and tested in the core but **not yet operationally deployed**, so
a fully live consultant→client handoff is not something the shipped desktop product
performs today. See [Package &amp; deploy](package-and-deploy.md) and the
[deployment modes](../../concepts/deployment-modes.md).

When the model *is* exercised (e.g. in federation), a governed run can still be
refused for reasons distinct from §1's grants:

1. **Package install is present but the run is denied.** Installing a
   [package](../../concepts/glossary.md#package) admits the *record*; it is **not**
   run admission and **not** method/context access. The target must still request
   and be granted method and context access separately (§1).
2. **Governed deployment needs an active entitlement.** For a governed deployment,
   the deployment context must have an **active deployment entitlement** in addition
   to an eligible install. If entitlement is **requested** (not yet activated),
   **suspended**, or **closed**, future governed runs are blocked — entitlement is
   *eligibility*, never execution.
3. **Billing/support receipts do not grant entitlement.** A recorded payment or
   support receipt never activates entitlement, grants access, admits a run, or
   releases output. If you expected a paid receipt to unlock runs, it won't — the
   responsible authority must **activate** the entitlement.
4. **Attested runs are gated at the attestation issuer.** Running in an
   [attested](../../concepts/glossary.md#attestation) (sealed confidential-VM)
   placement is gated: the issuer refuses to seal/quote a run without a valid,
   unexpired entitlement, fail-closed ([INV-20]). Note that **live** attested
   compute is <span class="status built">verifier Built</span>
   <span class="status planned">live Planned</span> — the SEV-SNP quote *verifier*
   is built and tested against genuine AMD material, but generating a fresh quote
   needs a confidential VM, so live attested runs are not available in the shipped
   product. Local/non-attested runs are **not** gated this way.

!!! note "Cross-party movement always takes two keys"
    Nothing crosses between you and a client without the **source** permitting it to
    leave **and** the **target** admitting it ([INV-13]). If a deployment seems
    "stuck", one of the two consents is usually missing. A federation relay never
    becomes a payload authority and never sees plaintext ([INV-14]).

---

## 5. The downloaded app won't launch (unsigned builds)

!!! warning "Desktop builds are unsigned today"
    GaugeWright desktop builds are **not code-signed or notarized**
    (<span class="status none">Not implemented</span>). On macOS and Windows your OS
    will warn that the publisher is unverified, and you must explicitly allow the app
    to run. **Verify you obtained the build from the official download page before
    allowing it.** See
    [supply-chain limitations](../../reference/limitations.md#supply-chain-and-build-limitations).

=== "macOS"

    Gatekeeper will refuse an unsigned/un-notarized app ("cannot be opened because
    the developer cannot be verified"). After confirming the source, allow it from
    **System Settings → Privacy &amp; Security → Open Anyway**, then relaunch.

=== "Windows"

    SmartScreen will show "Windows protected your PC" for an app from an unverified
    publisher. After confirming the source, choose **More info → Run anyway**.

=== "Linux"

    No OS publisher-signing prompt applies. Ensure the downloaded binary is
    executable and obtained from the official download page.

---

## When it's genuinely a bug, not a denial

A denial *names a missing grant or a status caveat*. If a run fails with no missing
grant, no provider error, and no status caveat above, it may be a genuine defect.
Note these honest gaps when reporting:

- There is **no production monitoring, alerting, or incident-response procedure** in
  the product today (<span class="status none">Not implemented</span>) — the shipped
  product is a local desktop app.
- The audit log is enforced **semantically** (an immutable append-only event log)
  and is exportable, but it is **not yet cryptographically** tamper-evident.

Every action — including denials — lands in the **append-only history**, so you can
reconstruct exactly what was attempted, what was refused, and why. Filter by actor
or action to build the timeline. See
[Audit and revoke](../../concepts/protection.md#4-audit-and-if-needed-revoke).

---

## Keep reading

- **[How GaugeWright protects your work](../../concepts/protection.md)** — the
  boundary contract these denials enforce, and where your data goes.
- **[Run &amp; review work](run-and-review.md)** · **[Build an agent](build-an-agent.md)**
  · **[Package &amp; deploy](package-and-deploy.md)** — the expert workflow.
- **[Known limitations &amp; gaps](../../reference/limitations.md)** — the in-docs
  list of what does not work today.
- **[Roadmap &amp; status](../../reference/status.md)** — the single source for every
  status badge.
- **[Deployment modes](../../concepts/deployment-modes.md)** ·
  **[Glossary](../../concepts/glossary.md)** — the vocabulary used here.

[INV-10]: ../../concepts/protection.md#the-structural-guarantees-machine-checked
[INV-11]: ../../concepts/protection.md#the-structural-guarantees-machine-checked
[INV-12]: ../../concepts/protection.md#the-structural-guarantees-machine-checked
[INV-13]: ../../concepts/protection.md#the-structural-guarantees-machine-checked
[INV-14]: ../../concepts/protection.md#the-structural-guarantees-machine-checked
[INV-17]: ../../concepts/protection.md#the-structural-guarantees-machine-checked
[INV-18]: ../../concepts/protection.md#the-structural-guarantees-machine-checked
[INV-20]: ../../concepts/protection.md#the-structural-guarantees-machine-checked
[INV-24]: ../../concepts/protection.md#the-structural-guarantees-machine-checked

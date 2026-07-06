# Review & release outputs

<span class="status built">Built</span> <span class="status planned">reviewer UI Planned</span>

You're the **context-owner**. When an agent produces an answer derived from your
private data, that answer carries *you* as a stakeholder — so it does not leave on
its own. The one guarantee on this page: **generating an
[output](../../concepts/glossary.md#output) is not releasing it.** Every output the
agent produces is **held** until you (or a designated reviewer) review it and
explicitly **release** it to the people allowed to see it. Release is the last word:
it is **terminal and irreversible**.

!!! warning "Where your data goes — read this first"
    GaugeWright orchestrates work locally, but the agent's *reasoning* runs on the
    **third-party LLM provider you configure**. Your prompt and the in-scope
    [context](../../concepts/glossary.md#context) are sent to that provider
    over the network for every [run](../../concepts/glossary.md#run); **there is no
    local-only inference.** An output then sits **held** on your machine until you
    release it. See
    [Where your data goes](../../concepts/protection.md#where-your-data-goes).

!!! note "Status — the lifecycle is real, the reviewer UI is being built"
    The review-and-release lifecycle is **implemented and modeled in the core** and
    its per-[resource](../../concepts/glossary.md#resource) `review` / `export`
    routes are wired (<span class="status built">Built</span>). What is **not yet
    shipped** is the **cross-party reviewer UI** — the held-output placeholder, the
    provenance-showing review queue, and the friction-reduction levers below
    (tracker **UX-11**, <span class="status planned">Planned</span>). The walkthrough
    describes how that surface behaves once it lands; the rules it enforces hold in
    the core today.

---

## What "held" means

When a run produces an output that was derived from protected data, the system
applies **conservative taint**: the output is marked with every party whose
resource the run *could have read* — its **stakeholders** — not just the parties it
provably revealed. An output with stakeholders other than the recipient is **held**.

- **Held is not the payload.** A held output is a *projection of state*, not the
  protected text — the core never exposes the payload to a party that isn't cleared.
  (INV-10) Once the reviewer UI lands, anyone who isn't yet cleared will see this as
  a placeholder — "withheld pending {your} release" — never the content itself.
- **Held by default.** Nothing is delivered, exported, or shown to the other party
  until release. Fail-closed: if it's uncertain whether someone may see it, they
  don't. (INV-20)
- **The recipient can ask, not take.** Once the reviewer UI lands, the party who
  wants the output (often the expert/consultant) will see the placeholder and a
  **propose-release** affordance. That *requests* a disclosure; it never reveals
  anything.

??? question "Why is an output held when it doesn't obviously contain my data?"
    Because taint is deliberately **over-approximated**. The system marks you as a
    stakeholder on anything the run *could* have read from your resources, even if
    the final text doesn't actually quote them. This is safe-by-construction: it can
    only ever ask for *more* consent than strictly necessary, never less. The cost
    is review volume — which is exactly what the
    [anti-rubber-stamping levers](#keeping-review-honest-anti-rubber-stamping-levers)
    are designed to manage.

---

## The review flow, step by step

!!! note "What's enforced vs. what you'll click"
    The state machine below — `proposed → cleared → released` / `withheld`, and the
    consent gating on each transition — is **implemented and machine-checked in the
    core** today (<span class="status built">Built</span>). The *review queue*,
    *propose-release affordance*, and *held-output placeholder* described in the
    steps are the **cross-party reviewer UI**, which is
    <span class="status planned">Planned</span> (tracker **UX-11**). The steps are
    written in the future tense to mark that: they describe how the surface will
    behave once it lands, over rules the reducer enforces now.

The unit you act on is a **disclosure**: one output proposed to one recipient. Each
disclosure moves through `proposed → cleared → released` (or terminates at
`withheld`).

1. **Hold (automatic).** A new output that carries stakeholders other than the
   recipient starts **held**. No action delivers or exports it. Once the UI lands,
   the recipient will see only a placeholder in their transcript.
2. **Propose (recipient).** The recipient who wants the output will issue
   **propose-release**, and a **disclosure** will appear in *your* review queue.
   Still nothing is revealed to them.
3. **Review with provenance (you).** When the reviewer UI lands, you will open the
   disclosure in your review queue. You will see, together (see
   [the provenance you see](#the-provenance-you-see)), so the decision is informed,
   not blind:
    - the **output** itself (and, where cheap, a preview/diff of the releasable text),
    - the **recipient** it would go to,
    - the **derived-from resources** — which admitted resources the run read,
    - the **stakeholders** still required to consent — whether you're the last sign-off.
4. **Consent or reject (you).** Per disclosure, you will choose **consent** or
   **reject**.
    - The required consenters are every stakeholder *except* the recipient — you
      need not consent to a party receiving its own asset, but every *other*
      stakeholder must.
    - When the last required consent lands, the disclosure becomes **cleared**:
      release is authorized but **not yet executed**.
    - A **reject** moves it to **withheld** (terminal); the recipient sees a
      withheld state, never a silent drop.
5. **Release (you) — terminal.** From `cleared`, you will press **release**. This is
   the separate, explicit, final act that delivers the output to the recipient and
   lets it cross the [boundary](../../concepts/glossary.md#boundary).

!!! danger "Release is irreversible"
    `released` is **absorbing and terminal**. Revocation is **future-only** — once
    an output is released it **cannot be recalled** (INV-18). Before you release,
    assume the recipient keeps it. The release action states this before it fires.

??? question "Can I change my mind before releasing?"
    Yes — until you press release. Before clearance you can **revoke a consent**
    (`revokeConsent`), which drops the disclosure back to `proposed`. After
    `released`, nothing recalls it. That asymmetry is the whole point: consent has
    weight precisely because release is final.

---

## The provenance you see

Provenance is the antidote to blind approval. For each held output the review queue
surfaces:

| You see | So you can decide |
|---|---|
| The **output** (and, where available, a preview/diff of the releasable text) | …what is actually leaving |
| The **recipient** the disclosure targets | …who would get it |
| The **derived-from resources** — which admitted resources the run read | …whether this really touches your sensitive data |
| The **stakeholders** still required to consent | …whether you're the last sign-off |

!!! note "The preview is your own data"
    A text preview of a held output is itself tainted content. You're entitled to
    see it — you're the owner reviewing your own asset before deciding it may leave.
    That's a legitimate read, not a leak.

---

## Keeping review honest (anti-rubber-stamping levers)

Conservative taint can over-ask. If every output demands a click, review degrades
into reflexive approval and the control becomes theater. These levers reduce
friction **without weakening the guarantee** — they may only *coarsen the UI*, never
*widen* what consent actually authorizes. They are part of UX-11 and are
<span class="status planned">Planned</span>.

=== "Risk-scoped batching"

    Group **low-provenance** disclosures — outputs derived only from your *own* or
    `public` resources — into a single one-action release, while **escalating
    high-classification** ones (e.g. `pii` / `regulated`) to individual review.

    Batching is **monotone**: it can only group items that already need the same
    consents; it can never lower the bar on a single item. The risk is mis-scoping
    from over-taint, so escalation always wins ties.

=== "Standing pre-authorization"

    Pre-consent a **scoped, expiring basis** in advance — e.g. *"release outputs to
    this expert, within this [engagement](../../concepts/glossary.md#engagement),
    that derive only from resources tagged `internal`."* Then matching outputs clear
    without a per-item click.

    This is **you exercising your own authority ahead of time**, not a policy grant
    handed to someone else. It stays inside the verified floor and is revocable
    **future-only** (INV-18) — revoking it stops future auto-clears, never recalls
    what already released.

=== "Provenance-first review"

    Always lead with *what the output derived from* and a preview of the releasable
    text. Seeing the basis is the strongest single defense against rubber-stamping;
    everything else is secondary to showing you the real input to your decision.

---

## What's guaranteed structurally vs. what's policy

GaugeWright keeps two kinds of claim separate so each stays defensible. See
[two kinds of guarantee](../../concepts/protection.md#two-kinds-of-guarantee-dont-conflate-them).

**Structural (implemented and machine-checked in the core):**

!!! note "Built means verified-in-core, not switch-on-able today"
    Each guarantee below is <span class="status built">Built</span>: implemented in
    the reducer and machine-checked in Quint. That is a claim about the **core**, not
    about an end-user feature you can use today — the cross-party UI that surfaces
    these guarantees is <span class="status planned">Planned</span> (see the status
    note at the top). This matches the canonical
    [status table](../../reference/status.md), which lists *Output review &amp;
    release lifecycle* as <span class="status built">Built</span>.

- **Safe release.** An output reaches `released` **only if every required
  stakeholder consented** — verified in Quint (`review-lifecycle.qnt`,
  `SAFE_RELEASE`), with an adversarial probe that breaks the property if release
  could bypass clearance. (INV-16) <span class="status built">Built</span>
- **No ghost consent.** Only a stakeholder may consent, and only for themselves; a
  retried command can never consent on another party's behalf. (`NO_GHOST_CONSENT`,
  INV-13, INV-17) <span class="status built">Built</span>
- **A held output is not the payload.** Until release, the recipient sees a
  projection, never the protected text. (INV-10)
  <span class="status built">Built</span>
- **Two-key crossing.** Cross-party release requires the crossing to be **permitted
  by the source** *and* **admitted by the target**. (INV-13)
  <span class="status built">Built</span>
- **Release is terminal.** Revocation is future-only; the past is not recalled.
  (INV-18) <span class="status built">Built</span>

**Policy / operational (settings and integrations, not invariants):**

- **Batching, standing pre-authorization, and classification-driven escalation** are
  the friction levers above — operational UI behaviors over the same invariants
  (<span class="status planned">Planned</span>, UX-11).
- **Per-actor audit of who released what to whom** records each release immutably,
  attributed to the authenticated actor. The audit log itself is
  <span class="status available">Available</span>; the cross-party release-attribution
  timeline rides on UX-11 (<span class="status planned">Planned</span>).

!!! note "Per-OS caveat"
    The review-and-release lifecycle is OS-independent. The related guarantee that a
    *running agent cannot rewrite its own method* depends on the kernel sandbox,
    which is <span class="status available">Available</span> on **Linux / macOS** and
    <span class="status planned">Planned</span> on **Windows** — see
    [protection](../../concepts/protection.md#the-structural-guarantees-machine-checked).

---

## Frequently asked

??? question "Who can see the output before I release it?"
    You can — it's your machine and your data. The recipient sees a **held
    placeholder** only. Your configured LLM provider already saw the prompt and
    in-scope context during the run (that's how inference works); release controls
    delivery to the *other party*, not to the provider. See
    [who can see plaintext, plainly](../../concepts/protection.md#where-your-data-goes).

??? question "What happens if I do nothing?"
    The output stays **held** indefinitely; nothing is delivered. A proposal may
    also **expire** to `withheld` if the [placement](../../concepts/glossary.md#placement)
    policy sets a timeout. Inaction is always the safe state.

??? question "Is releasing the same as 'keeping' a run's diff?"
    No. **Keep/discard** decides whether a run's proposed change applies to *your*
    work — see [Run & review work](../expert/run-and-review.md). **Release** decides
    whether an output crosses to *another party*. An output can be kept in your
    project and still held from the recipient.

---

## Related

- **[For clients](index.md)** — your role and guarantees in one page.
- **[How GaugeWright protects your work](../../concepts/protection.md)** — the
  boundary contract and where your data goes.
- **[Run & review work](../expert/run-and-review.md)** — producing the outputs you
  review here.
- **[Roadmap & status](../../reference/status.md)** — the single source for every
  status badge.
- **[Glossary](../../concepts/glossary.md)** — [output](../../concepts/glossary.md#output),
  [resource](../../concepts/glossary.md#resource),
  [boundary](../../concepts/glossary.md#boundary),
  [engagement](../../concepts/glossary.md#engagement).

# Verifying the security claims

This page is for the **reviewer**: the security engineer, IT admin, or auditor who
has read [How GaugeWright protects your work](../concepts/protection.md) and wants
to know *how to check it for yourself* rather than take it on faith.

GaugeWright's protection claims are not marketing copy. Most of them are
**structural** — built into how the system decides what a run may do — and the
strongest ones are **machine-checked**: stated as a formal invariant, modelled
against an adversary in [Quint](https://quint-lang.org/), exercised by a test that
deliberately tries to break them, and enforced on every push by CI. This page maps
each guarantee to that backing evidence so you can open the file and read it.

!!! note "Read this with the status table open"
    Every claim below carries a status badge. The single source of truth for
    status is [Roadmap &amp; status](status.md); where this page and that table
    ever disagree, the table wins. Today only the **local desktop workbench** is
    <span class="status available">Available</span>; cross-party deployment,
    attested compute, and enterprise identity are
    <span class="status built">Built</span> or <span class="status planned">Planned</span>.
    Before relying on any row, read [Deployment modes](../concepts/deployment-modes.md)
    — it states precisely what is usable end-to-end today versus what is only
    exercisable in CI (for example, federation runs only over a loopback /
    NAT-isolated test harness, not an operational deployment).

!!! warning "The one fact that changes the threat model"
    GaugeWright's *orchestration* is local, but its *inference is not*. The
    agent's reasoning is performed by the **third-party LLM provider you
    configure**, so your prompts and the in-scope [context](../concepts/glossary.md#context)
    are sent to that provider over the network. No claim on this page removes that
    provider from the trust boundary. See
    [Where your data goes](../concepts/protection.md#where-your-data-goes) before
    you rely on any of the below.

## Two kinds of guarantee — keep them apart

A claim is only as defensible as the thing backing it. GaugeWright makes two
distinct kinds of promise, and a reviewer should treat them differently.

| Kind | What backs it | How you verify it | Example |
|---|---|---|---|
| **Structural / machine-checked** | A formal invariant, a Quint model that discharges it, and a teeth probe that proves the check can fail | Read the model, run the checker, flip a teeth flag and watch it break | "A reference is not access"; "fail-closed"; "context never reaches the method owner" |
| **Policy / operational** | A configuration, a deployment practice, or a process | Read the config/spec and confirm the deployment matches | Which LLM provider is configured; KMS key custody; SIEM export wiring |

The rest of this page is mostly about the first kind, because that is where you
can get certainty rather than assurance. Known gaps in the second kind are listed
in [What is *not* yet proven](#what-is-not-yet-proven).

## The invariant → control idea

The chain is deliberately short, so there is no place for a claim to drift:

1. An **invariant** (`INV-n`) is stated in plain operational terms in
   `specs/principles.md`
   — e.g. *uncertainty denies; never permits.*
2. A **Quint model** under
   [`specs/models/`](https://github.com/jamesjscully/gaugedesk/tree/main/specs/models)
   encodes that invariant as a property over an adversarial state space and
   proves the property holds.
3. A **teeth probe** inside the same model is a deliberately injected bug behind a
   flag. Flipping it must make the invariant *fail* — proving the check has teeth
   and is not a tautology.
4. A **CI gate** runs both directions on every push: invariants must hold, and
   every teeth probe must bite.
5. Where the same rule lives in shipping code, a **Rust reducer** mirrors the
   model and a property test (`proptest`) checks the implementation against the
   same shape over random inputs.

If any link breaks, CI goes red and the change cannot land. That is the whole
mechanism. INV/ADR references below are optional deep links — you do not need them
to follow the page.

## The guarantee map

Each row names the claim, its backing model file, and the named property and teeth
probe inside it. All file links point at the canonical sources on GitHub.

### Boundary & egress

> *Method and context meet without either leaking; a run can only do what it was
> admitted to do.* <span class="status available">Available</span>

- **Model:** `boundary.qnt`
- **Properties that must hold:** `SAFE_EGRESS` (nothing leaves to a non-owner
  without an admitted basis), `CONTEXT_CONTAINED` (the client's context never
  reaches the method owner without a basis), `EGRESS_CONTAINED` (outsiders never
  obtain anything without a basis).
- **Teeth:** `BUGGY` injects one unmediated egress channel; the checker finds the
  leak. This is what proves the single-chokepoint design is load-bearing.
- **Honesty probe:** `METHOD_HIDDEN_FROM_B` is *expected to fail* at the local
  desktop placement — because whoever operates the host can read plaintext, the
  method is not secret from the client when the client hosts execution. It passes
  only under [attestation](#attested-compute). This is the asymmetry stated
  honestly in the model, not hidden.
- **End-to-end test:** the protection chain (grant → run → taint → review/export
  consent) is exercised over random interleavings in
  [`crates/app/tests/protection_chain_proptest.rs`](https://github.com/jamesjscully/gaugedesk/blob/main/crates/app/tests/protection_chain_proptest.rs).

!!! note "Confidentiality is an *emergent* goal, not a separate gate"
    The overall confidentiality guarantee — *protected payload never reaches a
    non-stakeholder without a basis* — is named `INV-22` in
    `specs/principles.md`.
    It is **not** a new check bolted on top: it is *discharged* by the existing
    protection models (`boundary.qnt`'s `SAFE_EGRESS`, `derived-output.qnt`'s
    `SOUND_RELEASE`, `engagement-taint.qnt`'s `SOUND`). The constitution states the
    end goal so the guarantee is legible; the teeth live in those models.

### Reference is not access

> *Holding a [handle](../concepts/glossary.md#handle) conveys no read of the
> payload.* <span class="status available">Available</span>

- **Models:** `resource-access.qnt`,
  `abac.qnt`
  (the resource-floor rules: a payload read requires an explicit grant evaluated
  at the boundary, with a restrict-only protection floor).
- **In code:** the resource store derives an output's stakeholders from the
  *persisted* read records, so a revoke after a read cannot launder the taint
  (see the proptest above).

### Fail-closed

> *If a required grant is missing, stale, or uncertain, the action is denied —
> never allowed on doubt.* <span class="status available">Available</span>

- **Model:** `fail-closed.qnt`
  (traces to `INV-20`).
- **Property:** `NO_ALLOW_ON_UNCERTAIN` — an action is performed only under a
  basis that is *definitely present*; absence, staleness, and indeterminacy all
  deny.
- **Teeth:** `FAIL_OPEN` makes the evaluator permit on uncertainty; the invariant
  breaks immediately.

### A running agent cannot rewrite its own method

> *The agent definition is editable only from an [edit chat](../concepts/glossary.md#chat),
> never from a [work chat](../concepts/glossary.md#chat).*
> <span class="status available">Available (Linux/macOS)</span>

- **Model:** `method-integrity.qnt`
  — discharges `METHOD_WRITE_REQUIRES_EDIT`.
- **Structural enforcement:** a work chat is rooted on a
  [placement](../concepts/glossary.md#placement) whose method is an installed,
  read-only version; only an edit chat rooted on the
  [archetype](../concepts/glossary.md#archetype) can write the method surface.
- **Per-OS caveat:** the kernel sandbox that stops even a shell inside a run from
  editing the method is enforced on **Linux and macOS**. The Windows
  method-isolation sandbox is <span class="status planned">Planned</span> — see
  [status](status.md).

### Append-only history

> *Every durable fact is an immutable event; corrections are new events.*
> <span class="status available">Available</span>

- **Model:** `projection.qnt`
  (a projection is derived, never authoritative); reinforced by
  `idempotency.qnt`
  and `revocation.qnt`.
- **What you get:** an append-only audit log, exportable to your SIEM.
  <span class="status available">Available</span>

### Federation (crossing between parties)

> *Nothing crosses without the source permitting it to leave and the target
> admitting it; relays route encrypted bytes but never read payload.*
> <span class="status built">Built</span>

- **Models:** `federation.qnt`,
  `federated-delivery.qnt`,
  `remote-call.qnt`,
  `handoff.qnt`.
- **End-to-end test:** [`crates/app/tests/federation_crossing.rs`](https://github.com/jamesjscully/gaugedesk/blob/main/crates/app/tests/federation_crossing.rs)
  drives two paired authorities through a real broker and asserts the adversarial
  cases — a crossing to an unpaired peer is refused, a shared output releases only
  after the remote stakeholder consents, and **a revoked device subkey can no
  longer cross**.
- **Status caveat:** federation is <span class="status built">Built</span> and
  tested, but it is exercised only over a **loopback / NAT-isolated CI harness** —
  it is **not operationally deployed** for cross-party use today. See
  [Deployment modes — Federation](../concepts/deployment-modes.md#federation) and
  [status](status.md).

### Enterprise identity & admin roles (RBAC)

> *Console capabilities are default-deny: a role holds a capability only if the
> fixed matrix lists it.* <span class="status built">Built</span>
> <span class="status planned">live Planned</span>

- **Model:** `rbac.qnt`
  (traces to `INV-20`, ADR 0043).
- **Properties:** `RBAC_FAIL_CLOSED` (an unrecognized role can do nothing),
  `OWNER_ADMIN_FULL`, `BILLING_ONLY_BILLING`, `MEMBER_VIEWER_NO_CONSOLE`.
- **Teeth:** `UNKNOWN_ALLOWED` (default-allow), `MEMBER_GETS_CONSOLE`,
  `BILLING_GETS_MEMBERS` (privilege escalation) — each widens exactly one cell and
  breaks a property.
- **End-to-end test:** [`ee/app/tests/rbac_enforcement.rs`](https://github.com/jamesjscully/gaugedesk/blob/main/ee/app/tests/rbac_enforcement.rs)
  confirms a `member` token is forbidden the admin routes and a viewer is denied
  export by policy (HTTP 403, the gate firing before admit).
- **Status caveat:** the enterprise identity layer (OIDC / SAML / SCIM / RBAC) is
  <span class="status built">Built</span> and tested but **not operationally
  deployed**; MFA enforcement is <span class="status none">Not implemented</span>.

### Attested compute

> *Cryptographic proof an agent ran inside a sealed, verified confidential VM, so
> the method and context stayed sealed even from the host operator.*
> <span class="status built">verifier Built</span>
> <span class="status planned">live Planned</span>

- **Why it matters here:** attestation is what lets `boundary.qnt`'s
  `METHOD_HIDDEN_FROM_B` *pass* — removing the host from the trusted set.
- **Tests:** [`crates/app/src/attestation_verifier.rs`](https://github.com/jamesjscully/gaugedesk/blob/main/crates/app/src/attestation_verifier.rs)
  and [`crates/core/src/attestation.rs`](https://github.com/jamesjscully/gaugedesk/blob/main/crates/core/src/attestation.rs)
  exercise the verifier against attestation vectors.
- **Status caveat:** the **verifier** is <span class="status built">Built</span>;
  running real workloads under attestation (live confidential VMs) is
  <span class="status planned">Planned</span>.

### Public hosting / embedded sessions

> *Each embedded session is isolated; a durable end-user chat is kept only for an
> authenticated principal.* <span class="status planned">Planned</span>

- **Model:** `public-session.qnt`.
- **Properties:** `DURABLE_CHAT_REQUIRES_IDENTITY` (no durable chat without an
  authenticated principal), `RETENTION_MATCHES_PRINCIPAL` (anonymous sessions
  discard at teardown), `RESUME_REQUIRES_IDENTITY`, `TERMINAL_RETAINS_TRANSCRIPT`.
- **Teeth:** `PERSIST_WITHOUT_IDENTITY`, `DISCARD_WITH_IDENTITY`,
  `RESUME_WITHOUT_IDENTITY`, `TEARDOWN_DROPS_TRANSCRIPT`.
- **Status caveat:** modelled and partially built, but public hosting / embed is
  <span class="status planned">Planned</span> — not usable today.

## How to run the checks yourself

You do not need GaugeWright installed to verify the structural claims — only the
[Quint](https://quint-lang.org/) checker and the repository. The exact commands CI
runs are in [`.github/workflows/ci.yml`](https://github.com/jamesjscully/gaugedesk/blob/main/.github/workflows/ci.yml)
and the project's green-bar contract.

=== "Verify the formal models"

    1. Clone the repo and install Quint:
       `npm install -g @informalsystems/quint`
    2. Typecheck every model:
       `quint typecheck specs/models/*.qnt`
    3. Confirm every invariant holds:
       `scripts/check-models.sh invariants`
    4. Confirm every teeth probe still bites (each injected bug breaks an
       invariant): `scripts/check-models.sh teeth`
    5. To convince yourself a check is real, open a model such as
       `specs/models/fail-closed.qnt`, change `pure val FAIL_OPEN: bool = false`
       to `true`, and re-run step 3 — it must now report a violation.

=== "Verify the implementation"

    1. Run the full Rust suite (includes the teeth/adversarial integration tests
       named above): `cargo test --workspace`
    2. The property tests under `crates/core/src/*.rs` (e.g. `rbac.rs`,
       `resource_export.rs`, `federation.rs`, `public_session.rs`) check each
       reducer against random inputs in the same shape as its Quint model.

=== "Verify the gate"

    The green-bar contract that CI enforces on every push to `main`:

    - `cargo test --workspace`
    - web client typecheck + unit tests (and `npm run e2e` when the control-plane
      contract changed)
    - `quint typecheck specs/models/*.qnt`
    - `python3 scripts/audit-gate.py` — the spec-audit coverage gate: a milestone
      cannot declare its gate closed while any coverage row is neither done nor
      deferred.

??? question "What stops a claim from quietly losing its teeth?"
    `scripts/check-models.sh teeth` flips each probe flag and asserts at least one
    invariant fails. If a probe is flipped and *nothing* breaks, the script reports
    `DULL` and CI fails — so a model that has degraded into a tautology cannot
    silently pass.

## What is *not* yet proven

Honesty is part of the claim. A machine-checked model verifies only what is
*modelled* — it catches logical gaps in the encoded channels; it cannot catch a
channel nobody encoded. So the following are real limits a reviewer should weigh:

- **Inference is out of the trust boundary.** Prompts and in-scope context go to
  the third-party LLM provider you configure. Confidential inference (provider
  removed from the boundary) is <span class="status planned">Planned</span>.
  See [Where your data goes](../concepts/protection.md#where-your-data-goes).
- **No third-party certifications yet.** SOC 2 Type II, a DPA, and a penetration
  test are committed but <span class="status planned">Planned</span>.
- **Supply-chain gaps.** No SBOM / dependency scanning, no production monitoring,
  and unsigned builds today — documented on the
  [Security &amp; trust](../security.md) page.
- **Built ≠ live.** Cross-party deployment, attested compute, and enterprise
  identity are implemented and tested but not operationally deployed. Read no
  row's badge as "usable today" unless it says
  <span class="status available">Available</span> in [status](status.md).
- **Operational/policy controls are your responsibility to confirm** — which
  provider is configured, KMS key custody for encryption at rest, and SIEM export
  wiring are deployment facts, not machine-checked invariants.

## Where to go next

- [How GaugeWright protects your work](../concepts/protection.md) — the
  plain-language guarantees these checks back.
- [Roadmap &amp; status](status.md) — the single source of truth for what is
  Available, Built, Planned, or Not implemented.
- [Security &amp; trust](../security.md) — reviewer-grade architecture, the
  invariant→control crosswalk (SOC 2 / ISO 27001 / NIST), and threat model.
- [For admins (IT)](../guides/admin/index.md) — the role guide for deploying and
  administering GaugeWright.
- [Glossary](../concepts/glossary.md) — the canonical vocabulary used above.

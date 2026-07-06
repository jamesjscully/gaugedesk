# For admins (IT)

<span class="status built">Built</span> <span class="status planned">live deployment Planned</span>

The **admin console** is the organization-facing layer of GaugeWright: the surfaces
your IT and security functions use to govern a deployment for an
**[organization](../../concepts/glossary.md#organization)** — who can sign in, what
each person may do, what gets recorded, and what the deployment will and won't allow.
It sits *beside* the workbench, not inside any one
[project](../../concepts/glossary.md#project).

!!! warning "Read this first — status of the admin layer"
    The whole organization layer (B10–B16 below) is
    <span class="status built">Built</span> <span class="status planned">live Planned</span>:
    the identity, access, audit, and policy substrate is **implemented and tested in
    the code** — OIDC id-token verification runs per-commit against a self-hosted
    Keycloak; SAML is behind a hardened sidecar with single-use replay defense; RBAC
    and the restrict-only policy law are modeled in Quint. But **live interop with
    specific IdP vendors and the admin-console UI are not operationally available
    yet.** Nothing on this page is something you can switch on in the shipped product
    today.

    The only thing **[Available](../../reference/status.md)** today is the **local
    desktop workbench**, which is single-party and needs no account or sign-in. The
    canonical capability status lives in the
    [roadmap &amp; status table](../../reference/status.md) — if any badge here ever
    disagrees with it, that table wins.

!!! note "The one counterintuitive truth — even with the org layer"
    GaugeWright orchestrates **locally**, but it **does not run the model locally**.
    The agent's reasoning is performed by the **third-party LLM provider your users
    configure**, so prompts and the in-scope
    [context](../../concepts/glossary.md#context) are sent to that provider over the
    network, in plaintext. SSO, RBAC, and audit govern *who can act and what is
    recorded* — they do **not** remove the provider from the trust boundary. Who can
    and cannot see plaintext is spelled out in
    [Where your data goes](../../concepts/protection.md#where-your-data-goes).

---

## What the admin console is for

A mid-market security reviewer wants three questions answered without filing a
ticket: *who can get in*, *what can they do once in*, and *what's on the record*.
The console answers all three from one place, and it is built so a standard security
questionnaire is mostly pre-answered.

Two properties shape everything below, and they are different *kinds* of promise:

- **Structural, machine-checked.** Default-deny RBAC, the restrict-only policy law,
  and references-only audit are invariants paired with adversarial tests that fail if
  the protection is removed. These hold by construction.
- **Policy / operational.** Live IdP interop, certifications, and the hosted control
  plane depend on configuration, infrastructure, and third parties. These are tracked
  honestly and are **not** invariants.

The [status page](../../reference/status.md#how-the-guarantees-are-backed) splits the
two site-wide; the [Security &amp; trust](../../security.md) page gives the reviewer-grade
crosswalk with optional `INV-*` deep links.

---

## Console surface reference (B10–B16)

Every surface is itself gated by the actor's role, evaluated by the **same** ABAC
machinery as everything else in the system — there is no separate "admin backdoor".
Rendering a row never implies access to the payload behind it (`INV-10`): the console
shows *references and operational state*, never protected content.

| Surface | What it governs | Status |
|---|---|---|
| **B10 — Org &amp; workspace settings** | Org profile, verified email domains, default placement / data-residency region | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| **B11 — Members &amp; roles** | People directory; invite (single + bulk), assign/change role, deactivate | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| **B12 — SSO configuration** | Connect an IdP (OIDC / SAML), test connection, enforce-SSO, claim mapping | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| **B13 — Provisioning (SCIM)** | Inbound directory sync; SCIM token, group→role mapping, offboarding chain | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| **B14 — Audit log viewer** | Per-actor / per-resource timeline, filter, CSV/JSON export | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| **B15 — Security policy** | MFA enforcement, session/idle timeout, allowed auth domains, residency default | <span class="status built">Built</span> <span class="status planned">live Planned</span> |
| **B16 — Billing &amp; seats** | Plan/tier, seat usage vs. entitlement, invoices, payment method | <span class="status built">Built</span> <span class="status planned">live Planned</span> |

### B10 — Organization &amp; workspace settings

The org's profile and defaults: display name, **verified email domains** (the basis
for domain-capture auto-join — verified-domain users auto-join, unverified domains are
refused), and the **default placement / data-residency region** that new projects
inherit. Domain-verification status shown before the verifying event is admitted is
**operational evidence**, not yet product truth (`INV-5`).

### B11 — Members &amp; roles

The people directory: list members with role and status (active / invited /
deprovisioned), **invite** (single and **bulk**), assign or change a role, and
deactivate. *Who may grant a role* is itself a governed act — the surface offers the
action to anyone, but admission enforces that only `admin`/`owner` may complete it.
Members managed by your IdP (B13) are shown **read-only** with a "managed by your IdP"
marker; their lifecycle is owned by SCIM, not edited here.

### B12 — SSO configuration

Connect an IdP over **OIDC or SAML 2.0** (Okta, Microsoft Entra ID, Google Workspace):
enter or upload metadata, run a **test connection**, and toggle **enforce-SSO** (require
every member to authenticate via the IdP). A failed test is operational state — it never
becomes an admitted "connected" fact. Enforce-SSO is **fail-safe**: it cannot lock out
the last break-glass `owner`.

The connection carries a **claim mapping** — which id-token claims feed the ABAC
attributes the verifier reads:

| Attribute | Default claim | Fallback | If unmapped |
|---|---|---|---|
| subject | `sub` | — | (always required) |
| roles | mapped | `GAUGEWRIGHT_OIDC_ROLES_CLAIM` | fail-closed |
| region | mapped | `GAUGEWRIGHT_OIDC_REGION_CLAIM` | fail-closed |
| tenant | mapped | `GAUGEWRIGHT_OIDC_TENANT_CLAIM` | fail-closed |

RBAC console gating reads the member's role from the **directory**, not the token. The
console's **Sign-in** affordance drives the OIDC auth-code flow: the browser goes to the
control plane's `/auth/login` → IdP → `/auth/callback`, which returns the verified
id-token; the client then carries it as the bearer on gated `/admin/*` calls. Signed-out
is the single-user local shape (admin ungated).

??? note "OIDC vs SAML status"
    OIDC id-token verification (signature via the IdP's JWKS, plus `iss`/`aud`/`exp`/`nbf`)
    is <span class="status built">Built</span> and verified per-commit against Keycloak.
    SAML is <span class="status built">Built</span> behind a hardened sidecar with
    single-use replay defense, **Planned** for the live vendor matrix. Live interop with
    Okta / Entra / Google is <span class="status planned">Planned</span>.

### B13 — Provisioning (SCIM)

Inbound directory sync (SCIM 2.0 Users): issue or **rotate a SCIM token**, view
last-sync status and errors, and map **IdP groups → workspace roles/teams**. The token
is stored **SHA-256 only** (shown in plaintext exactly once at creation). De-provisioning
is surfaced explicitly: a member removed in your IdP shows as **deprovisioned** here, and
the surface makes the **offboarding → access-revoked** chain visible — an inactive member
has no role and is denied. That is a security control, not a convenience. Sync
counts/latency are operational; a member's status becomes product truth only once the
membership event is admitted.

!!! note "Inbound only today"
    SCIM **inbound** (your IdP drives membership here) is <span class="status built">Built</span>.
    SCIM **outbound** sync is <span class="status planned">Planned</span>.

### B14 — Audit log viewer

A **per-actor, per-resource timeline** of governance-relevant actions — logins, role
changes, access grants/revocations, exports, config and policy changes — filterable by
actor / time / action, and **exportable** (see the export schema below). It reads the
audit-timeline projection over the append-only event stream, pivoted by authority/actor.

Two structural guarantees apply:

- **References only (`INV-10`).** The viewer shows only what the actor is authorized to
  see and **cannot reveal payloads behind handles**. An audit row says *what happened to
  which reference*, never the protected content.
- **No implied global order (`INV-7`).** Ordering is local time for readability; it is
  never an implied global total order across machines.

The timeline is the append-only event log itself, so facts are never rewritten —
revocation blocks **future** access but never rewrites history (`INV-18`).

!!! warning "Tamper-evidence is semantic, not yet cryptographic"
    The log is tamper-evident **semantically** (it is an immutable, append-only event
    log). There is **no signature or merkle chain** over entries yet, and cross-party
    log non-repudiation is a <span class="status planned">Planned</span> federation
    guarantee. See [Known limitations](../../reference/limitations.md#operational-limitations).

#### Audit event taxonomy

The recorded governance actions group as:

| Category | Example events |
|---|---|
| **Authentication** | sign-in, sign-out, enforce-SSO test, break-glass owner use |
| **Membership** | invite, accept, role grant/change, deactivate, SCIM create/update/deprovision |
| **Access** | access grant, access revoke, consent given/withdrawn for output release |
| **Export** | audit export (CSV/JSON), output/artifact release |
| **Configuration** | org settings revision, SSO connection change, SCIM token rotate |
| **Policy** | security-policy revision (MFA, session, residency, allowed domains) |

#### Export schema (CSV / JSON / SIEM)

Export is **authorization-scoped** — you can only export what your role may see — and
carries **references only, never payload** (`INV-10`). Each record has the same shape:

| Field | Meaning |
|---|---|
| `timestamp` | local-time occurrence (not a global order, `INV-7`) |
| `actor` | the authenticated actor the action is attributed to |
| `action` | the event from the taxonomy above |
| `resource_ref` | the handle/reference acted on — **not** its content |
| `outcome` | admitted / denied |
| `attributes` | the ABAC attributes evaluated (role, region, tenant) |

=== "CSV / JSON"

    CSV and JSON export of the scoped timeline are <span class="status built">Built</span>.

=== "SIEM streaming"

    A streaming sink is <span class="status built">Built</span>; a Splunk / Datadog /
    webhook exporter attaches behind it and is configured per deployment. Live SIEM
    streaming is <span class="status planned">live Planned</span>.

### B15 — Security policy

Org-wide controls: **MFA enforcement**, **session lifetime / idle timeout**, **allowed
authentication domains**, and the **data-residency region** default. These compose with
the protection floor under one law:

!!! warning "The restrict-only law (`ABAC_MONOTONE`)"
    An org policy can **only tighten** the verified protection floor — it can never widen
    it. Adding a rule (e.g. `viewer ⇒ no export`, or `regulated data ⇒ attested placement
    + same region`) can only **remove** permissions that were otherwise allowed; it can
    never **add** a permission the floor denies. This is proven monotone in Quint
    (`abac.qnt`, `ABAC_MONOTONE`). So a misconfigured policy can lock people out, but it
    can **never** open a hole below the structural floor.

!!! note "MFA enforcement status"
    Org-level require-MFA + session policy are <span class="status built">Built</span> as
    policy, but **MFA enforcement is <span class="status none">Not implemented</span>** in
    the product — under enforce-SSO the MFA *factor* is enforced by **your IdP**, not by
    GaugeWright. See [Known limitations](../../reference/limitations.md#identity-mfa-and-access-control).

### B16 — Billing &amp; seats

Plan/tier, **seat usage vs. entitlement**, invoices, and payment method. Billing state is
**operational, never authority**: a paid seat is not an access grant, and a lapsed invoice
does **not** retroactively rewrite history (`INV-18`) — it may only *gate future* seat
assignment. This surface is visible to the `billing` role and to `admin`/`owner`.

---

## Who sees what — RBAC role visibility

The console roles are **workspace-administration** roles (who may invite / assign /
configure / pay). They are **fixed** — the admin console ships roles, not a policy editor
(custom roles and a policy-authoring DSL are deferred upmarket). RBAC is **default-deny
and fail-closed**: an unrecognized role gets nothing. Modeled in Quint (`rbac.qnt`).

| Role | Console access | Manage members &amp; roles | Configure SSO / SCIM / policy | View audit | Billing |
|---|---|---|---|---|---|
| **owner** | full | yes | yes | yes | yes |
| **admin** | full | yes | yes | yes | yes |
| **billing** | B16 only | no | no | no | yes |
| **member** | none | no | no | no | no |
| **viewer** | none | no | no | no | no |

A `member` or `viewer` sees **no console at all**. The `billing` role sees **only B16**.
Team-scoped admins can administer **only their own team**. "Who may grant a role" is itself
governed (`admin`/`owner` only) — the surface may offer an action, but **admission enforces
it**, so a UI button is never the authority.

!!! note "Workspace roles ≠ product roles"
    These admin roles are orthogonal to the *product* roles a person plays in the work
    itself — the [expert](../expert/index.md) who builds an
    [archetype](../../concepts/glossary.md#archetype), the
    [client](../client/index.md) who owns the [context](../../concepts/glossary.md#context).
    An `owner` in the console is not automatically a participant in any project's
    [boundary](../../concepts/glossary.md#boundary).

---

## How-to

These steps describe the **Built** flows so you can evaluate them. They are not
operable in the shipped desktop product yet (the console UI and live IdP interop are
Planned) — track go-live on the [status table](../../reference/status.md). The concrete,
artifact-level recipe lives in the
[Identity &amp; provisioning runbook](identity-and-provisioning.md).

### Connect an IdP over OIDC (B12)

1. In the console, open **SSO configuration** and choose **OIDC**.
2. Enter your IdP's issuer URL and client credentials (the control plane fetches the
   IdP's **JWKS** to verify id-token signatures).
3. Set the **claim mapping**: leave `subject` as `sub`; map `roles`, `region`, and
   `tenant` if your tokens carry them, otherwise set the matching
   `GAUGEWRIGHT_OIDC_*_CLAIM` env knob — any attribute left unmapped is **fail-closed**.
4. Run **Test connection**. A failure is operational state only; nothing is recorded as
   "connected".
5. When the test passes and you're ready, toggle **enforce-SSO**. Confirm a break-glass
   `owner` exists first — enforce-SSO is fail-safe and will not lock out the last one.

### Provision users from your IdP over SCIM (B13)

1. Open **Provisioning (SCIM)** and **issue a SCIM token**. Copy it now — it is shown in
   plaintext **once** and stored only as a SHA-256 hash.
2. Paste the token (and the SCIM base URL) into your IdP's provisioning app.
3. Map **IdP groups → workspace roles/teams**.
4. Confirm a test user appears as a **read-only**, "managed by your IdP" member in B11.
5. Verify offboarding: deactivate the user in your IdP and confirm they show as
   **deprovisioned** with access revoked here.

### Export the audit log to your SIEM (B14)

1. Open the **Audit log viewer** and filter by actor / time / action as needed.
2. Choose **Export → CSV** or **Export → JSON** for a one-shot scoped export
   (<span class="status built">Built</span>). Every record is references-only.
3. For continuous streaming, configure the **SIEM sink** (Splunk / Datadog / webhook) per
   deployment (<span class="status planned">live Planned</span>).

### Tighten org security policy (B15)

1. Open **Security policy**.
2. Set session lifetime / idle timeout, allowed authentication domains, and the default
   **data-residency region**.
3. Add restrict-only rules (e.g. `viewer ⇒ no export`; `regulated ⇒ attested placement +
   same region`). Remember the monotone law: a rule can only **remove** permissions, never
   add one below the floor.
4. Save. The revision is recorded in the audit timeline as a policy event.

---

## Limitations you should know before procuring

Stated here, not only behind the external trust site:

- **Live deployment is not yet available.** The whole org layer is
  <span class="status built">Built</span> <span class="status planned">live Planned</span>;
  the shipped product is the single-party local desktop workbench, which needs no account.
- **MFA enforcement is <span class="status none">Not implemented</span>** — under
  enforce-SSO the factor is enforced by your IdP.
- **SAML and live IdP interop are <span class="status planned">Planned</span>**; SCIM
  **outbound** is <span class="status planned">Planned</span>.
- **Audit tamper-evidence is semantic, not cryptographic** — no signature/merkle chain
  over entries yet.
- **No third-party certifications yet** — SOC 2 Type II, DPA + subprocessor list, and a
  penetration test are all <span class="status planned">Planned</span>.
- **Inference is remote and inside the trust boundary** — SSO/RBAC/audit do not change
  that. See [Where your data goes](../../concepts/protection.md#where-your-data-goes).

The full, honest list is on [Known limitations &amp; gaps](../../reference/limitations.md).

---

## Where to go next

- **Concrete recipe** → [Identity &amp; provisioning runbook](identity-and-provisioning.md)
  — the artifact-level OIDC/SCIM/policy steps.
- **Honest gap list** → [Known limitations &amp; gaps](../../reference/limitations.md)
- **Check a claim yourself** → [Verifying claims](../../reference/verifying-claims.md)
- **What's usable today** → [Roadmap &amp; status](../../reference/status.md)
- **What protects the work, and who sees plaintext** →
  [How GaugeWright protects your work](../../concepts/protection.md)
- **Reviewer-grade detail** (data-flow, invariant→control crosswalk, threat model) →
  [Security &amp; trust](../../security.md)
- **Sibling role guides** → [Experts](../expert/index.md) · [Clients](../client/index.md)
  · [Embedded end-users](../embed/index.md)
- **Vocabulary** → [Concepts](../../concepts/index.md) · [Glossary](../../concepts/glossary.md)

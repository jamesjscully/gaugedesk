# Identity & provisioning runbooks (SSO, SCIM)

This is the operator runbook for wiring an **[organization](../../concepts/glossary.md#organization)**'s
identity provider into GaugeWright: connect Okta, Microsoft Entra ID, or Google
Workspace; map id-token claims onto the attributes the access evaluator reads;
turn on enforce-SSO without locking yourself out; and issue, rotate, and offboard
through SCIM. Every step below names the real surface, the real env knob, and the
real artifact you touch.

!!! warning "Status — Built, not live"
    <span class="status built">Built</span> <span class="status planned">vendor interop Planned</span>

    The enterprise identity layer — OIDC / SAML / SCIM / RBAC — is **implemented
    and tested in the codebase**. OIDC verification runs per-commit against
    Keycloak; SAML runs through a hardened sidecar with replay defense. It is
    **not operationally live**: the gate is live interop with each specific IdP
    vendor and the admin-console UI that fronts these surfaces. Until that ships,
    the only mode you can use end-to-end today is the **local desktop workbench**.
    The single source of truth for status is the
    [roadmap & status table](../../reference/status.md); if anything here and that
    table disagree, the table wins.

    Read these runbooks as *what the operator will do when the surface goes live*,
    and as documentation of the design and code behind it — **not** as something
    you can switch on today.

!!! warning "Before you connect anything: where your data goes"
    GaugeWright orchestrates **locally**, but it **does not run the model
    locally.** The agent's reasoning is performed by the **third-party LLM
    provider you configure**, so your members' prompts and the in-scope
    [context](../../concepts/glossary.md#context) for each
    [run](../../concepts/glossary.md#run) are sent to that provider over the
    network, in plaintext, today. Wiring SSO governs *who may sign in and what role
    they hold* — it does **not** change who can read plaintext. See
    [Where your data goes](../../concepts/protection.md#where-your-data-goes)
    before you onboard members.

---

## What this layer governs (and what it does not)

The admin surfaces named here are **workspace-administration** controls — who may
sign in, what role they hold, and what's on the audit record. They are orthogonal
to the per-project [authority](../../concepts/glossary.md#authority-scope) and
[boundary](../../concepts/glossary.md#boundary) decisions that protect the work
itself; those are evaluated the same way whether a member arrived by SSO or not.

Two kinds of guarantee live on this page, kept apart so claims stay defensible:

- **Structural, machine-checked.** Fail-closed admission, the break-glass guard,
  and the monotone-policy law are properties of how the code decides — each paired
  with an adversarial test that fails if the protection is removed (modeled in
  Quint: `rbac.qnt`, `abac.qnt`). These hold by construction.
- **Policy / operational.** IdP vendor interop, MFA enforcement, session lifetime,
  and your provider's retention terms depend on configuration or a third party.
  They are *not* invariants and are stated as such.

Roles are **fixed** — `owner` / `admin` / `member` / `viewer` / `billing` — and
**default-deny**: an unrecognized or absent role gets nothing. There is no policy
editor or custom-role surface; that is deliberately deferred. A `member` or
`viewer` sees no admin console at all; `billing` sees only billing;
`admin` / `owner` see everything.

---

## Runbook 1 — Connect an IdP over OIDC

OIDC covers all three target IdPs (Okta, Entra ID, Google Workspace) and is the
recommended path. SAML is <span class="status planned">Planned</span> for legacy
IdPs (see [Runbook 2](#runbook-2-connect-an-idp-over-saml)).

The connection lives on the **SSO configuration** surface (admin-console B12),
which reads the SSO-connection record and issues connection + enforcement commands.
A failed test connection is *operational state* — it never becomes an admitted
"connected" fact.

### Steps

1. **At your IdP**, register GaugeWright as an OIDC application (Web / confidential
   client). Set the redirect URI to your control plane's callback:
   `https://<your-control-plane>/auth/callback`. Note the **issuer URL**, the
   **client id**, and the **client secret**.
2. **In the admin console → SSO**, choose protocol **OIDC** and enter:
    - **Issuer** — the OIDC issuer URL (the value the id-token's `iss` must match).
    - **Audience(s)** — the client id(s) the id-token's `aud` must match.
    - **Metadata** — the OIDC discovery URL (`.../.well-known/openid-configuration`).
3. **Run the test connection.** GaugeWright fetches the issuer's JWKS and verifies
   it can validate a token's signature and the `iss` / `aud` / `exp` / `nbf`
   claims. A failure shows as operational state; nothing is recorded as connected.
4. **Map claims** (see [Runbook 3](#runbook-3-map-claims-to-abac-attributes)) so
   the id-token's roles / region / tenant feed the access evaluator. The subject
   defaults to `sub`.
5. **Save.** The connection record is admitted. Members can now sign in via the
   **Sign-in** affordance, which runs the auth-code flow: the browser is sent to
   `/auth/login` → your IdP → `/auth/callback`, which returns a verified id-token
   the client then carries as the bearer on gated `/admin/*` calls.
6. **Verify a real sign-in** end-to-end before you enforce SSO
   ([Runbook 4](#runbook-4-enforce-sso-with-a-break-glass-owner)).

!!! note "How verification actually works"
    The id-token signature is checked against the IdP's JWKS, and the verifier
    **self-refreshes** its signing keys from the issuer (with a refresh cooldown,
    so a flood of unknown-key tokens can't stampede your IdP). A token whose
    signing key is already loaded but still fails is treated as a bad token, not a
    stale key set.

=== "Okta"

    Use Okta's OIDC Web application. The issuer is your Okta org URL (or a custom
    authorization server). Roles typically ride in a **custom `roles` claim** you
    add via a claim/profile mapping — point `GAUGEWRIGHT_OIDC_ROLES_CLAIM` (or the
    connection's roles field) at it.

=== "Microsoft Entra ID"

    Register an app in Entra ID; the issuer is your tenant's v2.0 issuer. Entra
    emits group membership in the **`groups`** claim — use that for the roles
    mapping, or rely on SCIM group→role mapping
    ([Runbook 5](#runbook-5-issue-and-rotate-a-scim-token)) for the authoritative
    role. Set the tenant claim if you scope by tenant.

=== "Google Workspace"

    Create an OAuth client in Google Cloud for your Workspace org. Google's
    id-token carries standard claims; Workspace does not emit rich group/role
    claims by default, so prefer **SCIM group→role mapping** for roles and use the
    id-token for subject/region.

---

## Runbook 2 — Connect an IdP over SAML

<span class="status planned">Planned</span>

SAML 2.0 is implemented in code (a hardened sidecar with replay defense) but is
not a usable connect path yet — OIDC covers the three target IdPs today. When SAML
goes live, the B12 surface will accept the IdP **entityID** as the issuer and the
**raw SAML metadata** as the connection material, with the same test-connection and
claim-mapping steps. Use OIDC until then.

---

## Runbook 3 — Map claims to ABAC attributes

The verifier reads **attributes**, not raw tokens. The claim mapping decides which
id-token claims feed which attribute. Each field has three layers, resolved in
order: the **connection's claim mapping** (the per-connection home), then the
matching **`GAUGEWRIGHT_OIDC_*_CLAIM` env knob** (the operator path), then
**unmapped** — and unmapped is *fail-closed*: no attribute, because a missing
attribute is safer than a wrong one.

| Attribute | Connection field | Env knob | Default if unset |
|---|---|---|---|
| Subject | subject | `GAUGEWRIGHT_OIDC_SUBJECT_CLAIM` | `sub` |
| Roles | roles | `GAUGEWRIGHT_OIDC_ROLES_CLAIM` | unmapped (no roles attribute) |
| Region | region | `GAUGEWRIGHT_OIDC_REGION_CLAIM` | unmapped |
| Tenant | tenant | `GAUGEWRIGHT_OIDC_TENANT_CLAIM` | unmapped |

!!! warning "Roles claim ≠ console role"
    The roles claim feeds the **attribute** path that the access evaluator reads at
    the [boundary](../../concepts/glossary.md#boundary). It does **not** set a
    member's RBAC console role — that is read from the org directory, not the token.
    The authoritative way to set a member's role from your IdP is **SCIM
    group→role mapping** ([Runbook 5](#runbook-5-issue-and-rotate-a-scim-token)),
    not the id-token roles claim.

Other OIDC env knobs the operator path exposes, for completeness:

| Knob | Purpose |
|---|---|
| `GAUGEWRIGHT_OIDC_REDIRECT_URI` | Override the callback URI (the value registered at the IdP) |
| `GAUGEWRIGHT_OIDC_SCOPE` | Requested scopes (default `openid profile email`) |
| `GAUGEWRIGHT_OIDC_POST_LOGIN_URL` | Where to send the browser after a successful sign-in |

### Steps

1. Decide each attribute's source claim from your IdP's token (use the
   per-IdP tab in [Runbook 1](#runbook-1-connect-an-idp-over-oidc)).
2. Prefer the **connection's claim mapping** in B12 over env knobs — it is the
   admitted home and travels with the connection record. Use the env knobs only
   for headless/operator deployments without the UI.
3. Leave an attribute unmapped if you do not use it. Unmapped is fail-closed; do
   **not** point a field at a guessed claim "to be safe."
4. After saving, sign in with a real account and confirm the resolved attributes
   match the token you expect.

---

## Runbook 4 — Enforce SSO with a break-glass owner

Enforce-SSO requires **all** members to authenticate via the IdP. It is **fail-safe
by construction**: the guard that keeps the last break-glass `owner` reachable is
**structural** — enforced by the member routes themselves, independent of the
enforce-SSO flag — so turning enforcement on can never lock out your last `owner`.
This is a machine-checked property (`abac.qnt`: enforce-SSO can never remove the
last `owner`), not a convenience setting.

### Steps

1. **Designate a break-glass `owner`** whose access does not depend on the IdP — a
   real human who can still reach the org if the IdP is down or misconfigured. The
   structural guard protects the *last* active `owner`; you should still keep at
   least one deliberate break-glass account.
2. **Verify SSO sign-in works** end-to-end for normal members
   ([Runbook 1](#runbook-1-connect-an-idp-over-oidc), step 6) **before** enabling
   enforcement. Do not enforce against an untested connection.
3. **Toggle enforce-SSO** on the B12 surface. From here, members must sign in via
   the IdP; domain-capture and SCIM govern who exists.
4. **Test the break-glass path:** confirm the designated `owner` can still reach
   the console even with enforcement on. The guard means the toggle will refuse to
   strand your last `owner`, but you should observe it working.

!!! note "What enforce-SSO does and doesn't cover"
    Under enforce-SSO the **MFA factor is enforced by your IdP**, not by
    GaugeWright. GaugeWright's own MFA enforcement is
    <span class="status none">Not implemented</span> — set MFA as a sign-in
    requirement in your IdP. Session lifetime and idle-timeout are an org policy
    (admin-console B15) the session layer honors; they are
    <span class="status built">Built</span>.

---

## Runbook 5 — Issue and rotate a SCIM token

SCIM 2.0 lets your IdP create, update, and deactivate members directly. The
provisioning surface (admin-console B13) issues/rotates the token, shows last-sync
status and errors, and maps IdP groups to roles/teams. The token authenticates
your IdP's SCIM calls, which then drive membership commands.

!!! note "The token is shown once and stored as a hash"
    The SCIM bearer token's **plaintext is shown exactly once at issuance and is
    never persisted.** GaugeWright stores only its **hex-encoded SHA-256**
    (`SEC-5`: no secret at rest in plaintext). To check a presented token,
    GaugeWright hashes it and compares to the stored hash; with no token issued,
    nothing authenticates (fail-closed). This is an **operational** secret-handling
    property, not a machine-checked invariant.

### Issue

1. **In the admin console → Provisioning (SCIM)**, choose **Issue token**.
2. **Copy the plaintext token immediately** — it is shown once and never again.
   Store it in your IdP's SCIM configuration (the bearer credential).
3. In your IdP, point the SCIM **base URL** at your control plane's SCIM endpoint
   and paste the token as the bearer.
4. Run your IdP's SCIM test/sync. Confirm **last-sync status** shows success on the
   B13 surface (sync counts and latency are operational state, not product truth).

### Rotate

1. **In Provisioning (SCIM)**, choose **Rotate token**. Rotating **issues a new
   token and overwrites the stored hash**, so the **prior token stops
   authenticating immediately.**
2. **Copy the new plaintext token** (shown once) and update your IdP's SCIM
   configuration with it.
3. Re-run the IdP sync and confirm last-sync success. If sync now fails, the IdP is
   still presenting the old (now-rejected) token — finish step 2.

!!! warning "Rotation is a hard cutover"
    There is no grace window. The moment you rotate, anything still presenting the
    old token is rejected. Rotate during a maintenance window if your IdP's SCIM
    sync is on a long interval.

---

## Runbook 6 — Map IdP groups to roles, and verify offboarding revokes access

Group→role mapping is how your IdP authoritatively sets a member's workspace role.
A mapping keys an IdP **group name** to a role (and an optional team); when SCIM
provisions a user carrying that group, the member takes the mapped role/team
instead of the default `member`. The **first matching mapping wins** in stable
order; if no group matches, the member defaults to `member`.

SCIM-managed members appear **read-only** in the members directory (B11) with a
"managed by your IdP" marker — their lifecycle is owned by B13, not by hand.

### Map groups to roles

1. **In Provisioning (SCIM)**, add a mapping: **IdP group → workspace role**
   (optionally a team). Repeat per group you want to grant.
2. Map only to the fixed roles: `owner` / `admin` / `member` / `viewer` /
   `billing`. There are no custom roles.
3. Sync from the IdP and confirm members land on the expected role in the B11
   directory.

### Verify offboarding = access revoked

This is a **security control, not a convenience**, and the chain is made visible on
the B13 surface. De-provisioning is structurally fail-closed (`INV-20`): an
inactive member carries **no role**, so they are denied — there is no "deactivated
but still allowed" state.

1. **Deactivate the user in your IdP** (or remove them from the provisioning
   scope). Your IdP issues the SCIM deactivate.
2. **On B13**, confirm the member now shows **deprovisioned**. The surface makes
   the offboarding → access-revoked chain explicit.
3. **Confirm denial:** a deprovisioned member has no standing — `role_of` returns
   nothing for them, so any gated action fails closed. Verify they can no longer
   sign in to the console or reach gated `/admin/*` calls.
4. **History is untouched.** Revocation blocks *future* access; it never rewrites
   the audit record (`INV-18`). The member's past actions remain on the append-only
   [audit log](../../reference/status.md) (admin-console B14), attributed to them.

!!! note "Domain capture vs. SCIM"
    Separately from SCIM, **verified-email-domain** users can auto-join (B10): an
    address on a verified domain is captured; an unverified domain is refused
    (fail-closed). SCIM is the path for IdP-driven role assignment and offboarding;
    domain capture is the path for self-service join on a trusted domain.

---

## Known gaps & limitations (today)

Stated here, not only behind the trust site:

- **Vendor interop is the gate.** OIDC / SAML / SCIM / RBAC are
  <span class="status built">Built</span> and tested, but **not operationally
  live** with specific IdP vendors, and the admin-console UI is not yet available.
  Treat these runbooks as the go-live procedure, not a today-procedure.
- **SAML is <span class="status planned">Planned</span>.** Use OIDC.
- **MFA enforcement is <span class="status none">Not implemented</span>** in
  GaugeWright — enforce the factor in your IdP under enforce-SSO.
- **Encryption at rest is <span class="status built">Built</span>** (the
  AES-256-GCM seam) but awaits the **KMS-managed data key**; SIEM streaming has its
  sink built and awaits a per-deployment exporter.
- **No third-party certifications yet** — SOC 2 Type II, DPA, and a penetration
  test are <span class="status planned">Planned</span>.
- **Inference is remote.** SSO does not change this: prompts and in-scope context
  go to the third-party provider each member configures. See
  [Where your data goes](../../concepts/protection.md#where-your-data-goes).

---

## Where to go next

- **What protects the work, and who can see plaintext** →
  [How GaugeWright protects your work](../../concepts/protection.md)
- **Full capability status** →
  [Roadmap & status](../../reference/status.md) ·
  [Known limitations](../../reference/limitations.md)
- **Check the claims yourself** →
  [Verifying the security claims](../../reference/verifying-claims.md)
- **Reviewer-grade detail** (data-flow, invariant→control crosswalk, compliance
  posture) → [Security & trust](../../security.md)
- **Back to the admin layer** → [For admins (IT)](index.md)

*Deep links for the curious (optional): the surface contracts are admin-console
B12 (SSO) and B13 (SCIM); the structural guarantees are `INV-20` (fail-closed),
`INV-18` (append-only history), and the Quint models `rbac.qnt` / `abac.qnt`.*

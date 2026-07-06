/**
 * The enterprise **Admin Console** (M3, ADR 0043; surfaces B10–B16 of
 * [`admin-console.md`](../../../../specs/experience/admin-console.md)): the org-facing
 * surfaces a mid-market IT/security function uses — organization settings, the member
 * directory, SSO, SCIM provisioning, the audit timeline, security policy, and billing.
 *
 * A thin renderer over the `/admin/*` control-plane (projection-first, `INV-5`): it
 * reads projections and issues commands, never owns durable truth. On the loopback
 * desktop these routes are ungated (single authority); a hosted deployment role-gates
 * them server-side, so a forbidden surface simply fails its fetch.
 */

import { createResource, createSignal, For, Show, type JSX } from "solid-js";
import {
    authority,
    beginLogin,
    controlPlaneBase,
    signedIn,
    signOut,
} from "@gaugewright/control-plane-client";
import type {
    ArchetypeApprovalPolicy,
    Billing,
    EnterpriseAdminApi,
    Member,
    PlacementPolicy,
    SecurityPolicy,
    SsoClaimMapping,
    SsoConnection,
} from "@gaugewright/enterprise-client";
import { SsoWizard } from "./SsoWizard";

const ROLES = ["owner", "admin", "member", "viewer", "billing"];

export function AdminConsole(props: { api: EnterpriseAdminApi; onClose: () => void }): JSX.Element {
    const [tick, setTick] = createSignal(0);
    const refresh = () => setTick((t) => t + 1);
    const [status, setStatus] = createSignal("");
    const [wizardOpen, setWizardOpen] = createSignal(false);

    const act = async (verb: string, fn: () => Promise<unknown>) => {
        try {
            await fn();
            setStatus(`${verb} ✓`);
            refresh();
        } catch (e) {
            setStatus(`could not ${verb}: ${e instanceof Error ? e.message : String(e)}`);
        }
    };

    const [org] = createResource(tick, () => props.api.adminGetOrg());
    const [members] = createResource(tick, () => props.api.adminGetMembers());
    const [sessions] = createResource(tick, () => props.api.adminGetSessions());
    const [sso] = createResource(tick, () => props.api.adminGetSso());
    const [security] = createResource(tick, () => props.api.adminGetSecurity());
    const [placement] = createResource(tick, () => props.api.adminGetPlacementPolicy());
    const [approval] = createResource(tick, () => props.api.adminGetArchetypeApproval());
    const [billing] = createResource(tick, () => props.api.adminGetBilling());
    const [integration] = createResource(tick, () => props.api.adminIntegration());

    // --- B10: org settings ---
    const [orgName, setOrgName] = createSignal("");
    const [domains, setDomains] = createSignal("");
    const [region, setRegion] = createSignal("");
    const saveOrg = () =>
        act("save organization", () =>
            props.api.adminSetOrg({
                display_name: orgName() || org()?.display_name || "",
                verified_domains: (domains() || org()?.verified_domains?.join(",") || "")
                    .split(",")
                    .map((d) => d.trim())
                    .filter(Boolean),
                default_region: region() || org()?.default_region || null,
            }),
        );

    // --- ONB-5: DNS-TXT domain verification ---
    const [verifyDomain, setVerifyDomain] = createSignal("");
    const [txtRecord, setTxtRecord] = createSignal<{ record_name: string; value: string } | null>(null);
    const [verifyMsg, setVerifyMsg] = createSignal("");
    const getTxtRecord = () =>
        act("get TXT record", async () => {
            const r = await props.api.adminDomainVerifyToken(verifyDomain().trim());
            setTxtRecord({ record_name: r.record_name, value: r.value });
            setVerifyMsg("");
        });
    const verifyDomainNow = () =>
        act("verify domain", async () => {
            const r = await props.api.adminDomainVerify(verifyDomain().trim());
            setVerifyMsg(
                r.verified
                    ? `✓ ${r.domain} verified — it now powers auto-join`
                    : "✗ TXT record not found yet — publish it and retry (DNS can take a few minutes)",
            );
            refresh();
        });

    // --- B11: invite a member ---
    const [invAuthority, setInvAuthority] = createSignal("");
    const [invEmail, setInvEmail] = createSignal("");
    const [invRole, setInvRole] = createSignal("member");
    const invite = () =>
        act("invite member", async () => {
            await props.api.adminInvite({
                authority: invAuthority(),
                email: invEmail(),
                role: invRole(),
            });
            setInvAuthority("");
            setInvEmail("");
        });

    return (
        <div class="modal-overlay" onClick={() => props.onClose()}>
            <div
                class="modal admin-console"
                data-admin-console
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => e.key === "Escape" && props.onClose()}
            >
                <div class="modal-head">
                    <h3>Admin console</h3>
                    <button type="button" onClick={() => props.onClose()}>
                        close
                    </button>
                </div>

                {/* ID-3 — Sign in via OIDC. An admin configures the SSO connection
                    below; here a member signs in through it, and the console then
                    carries the verified id-token as the bearer on gated /admin/* calls. */}
                <section class="admin-section" data-admin-identity>
                    <h4>Sign-in</h4>
                    <Show
                        when={signedIn()}
                        fallback={
                            <div class="admin-signin">
                                <p class="muted">
                                    Single-user local — admin is ungated. Once an OIDC
                                    connection is configured below, sign in to administer
                                    as your directory identity.
                                </p>
                                <button
                                    type="button"
                                    class="tree-action"
                                    data-admin-signin
                                    onClick={() => beginLogin(controlPlaneBase())}
                                >
                                    Sign in with SSO
                                </button>
                            </div>
                        }
                    >
                        <div class="admin-signin">
                            <span>
                                Signed in as <strong>{authority()}</strong>
                            </span>
                            <button
                                type="button"
                                class="tree-action"
                                data-admin-signout
                                onClick={() => {
                                    signOut();
                                    setStatus("signed out ✓");
                                }}
                            >
                                Sign out
                            </button>
                        </div>
                    </Show>
                </section>

                {/* B10 — Organization */}
                <section class="admin-section">
                    <h4>Organization</h4>
                    <label>
                        Name
                        <input
                            value={orgName() || org()?.display_name || ""}
                            onInput={(e) => setOrgName(e.currentTarget.value)}
                            placeholder="Acme Inc."
                        />
                    </label>
                    <label>
                        Verified domains (comma-separated)
                        <input
                            data-admin-domains
                            value={domains() || org()?.verified_domains?.join(", ") || ""}
                            onInput={(e) => setDomains(e.currentTarget.value)}
                            placeholder="acme.com"
                        />
                    </label>
                    <label>
                        Default residency region
                        <input
                            value={region() || org()?.default_region || ""}
                            onInput={(e) => setRegion(e.currentTarget.value)}
                            placeholder="eu"
                        />
                    </label>
                    <button type="button" class="tree-action" onClick={saveOrg}>
                        Save organization
                    </button>

                    {/* ONB-5 — prove control of a domain via a DNS TXT record. */}
                    <div class="domain-verify" data-admin-domain-verify>
                        <input
                            data-admin-verify-domain
                            value={verifyDomain()}
                            onInput={(e) => setVerifyDomain(e.currentTarget.value)}
                            placeholder="domain to verify (acme.com)"
                        />
                        <button type="button" class="tree-action" onClick={getTxtRecord}>
                            Get TXT record
                        </button>
                        <button type="button" class="tree-action" onClick={verifyDomainNow}>
                            Verify
                        </button>
                    </div>
                    <Show when={txtRecord()}>
                        {(r) => (
                            <p class="muted" data-admin-txt-record>
                                Publish a TXT record at <code>{r().record_name}</code> with value{" "}
                                <code>{r().value}</code>, then click Verify.
                            </p>
                        )}
                    </Show>
                    <Show when={verifyMsg()}>
                        <p class="status" data-admin-verify-result>
                            {verifyMsg()}
                        </p>
                    </Show>
                </section>

                {/* B11 — Members & roles */}
                <section class="admin-section">
                    <h4>Members</h4>
                    <ul class="member-list">
                        <For each={members()} fallback={<li class="muted">No members yet.</li>}>
                            {(m: Member) => (
                                <li class="member-row" data-member={m.id}>
                                    <span class="member-id">{m.email || m.authority}</span>
                                    <select
                                        value={m.role}
                                        disabled={m.managed_by_scim}
                                        onChange={(e) =>
                                            act("change role", () =>
                                                props.api.adminSetRole(m.id, e.currentTarget.value),
                                            )
                                        }
                                    >
                                        <For each={ROLES}>
                                            {(r) => <option value={r}>{r}</option>}
                                        </For>
                                    </select>
                                    <span class="member-status">{m.status}</span>
                                    <Show when={m.managed_by_scim}>
                                        <span class="badge" title="managed by your IdP">
                                            SCIM
                                        </span>
                                    </Show>
                                    <button
                                        type="button"
                                        class="tree-action"
                                        disabled={m.status === "deprovisioned"}
                                        onClick={() =>
                                            act("deactivate", () => props.api.adminDeactivate(m.id))
                                        }
                                    >
                                        deactivate
                                    </button>
                                </li>
                            )}
                        </For>
                    </ul>
                    <div class="admin-invite">
                        <input
                            data-admin-invite-authority
                            value={invAuthority()}
                            onInput={(e) => setInvAuthority(e.currentTarget.value)}
                            placeholder="authority id"
                        />
                        <input
                            value={invEmail()}
                            onInput={(e) => setInvEmail(e.currentTarget.value)}
                            placeholder="email"
                        />
                        <select value={invRole()} onChange={(e) => setInvRole(e.currentTarget.value)}>
                            <For each={ROLES}>{(r) => <option value={r}>{r}</option>}</For>
                        </select>
                        <button type="button" class="tree-action" onClick={invite}>
                            invite
                        </button>
                    </div>
                </section>

                {/* ITGOV-2 — Live sessions: who is currently active on the data routes. */}
                <section class="admin-section" data-sessions>
                    <h4>Active sessions</h4>
                    <p class="muted">
                        Members currently active on the workspace (recorded at each request; a
                        session drops off once it goes idle past the session policy). No credential
                        is ever shown.
                    </p>
                    <ul class="member-list">
                        <For each={sessions()} fallback={<li class="muted">No active sessions.</li>}>
                            {(s) => (
                                <li class="member-row" data-session={s.authority}>
                                    <span class="member-id">{s.authority}</span>
                                    <span class="member-status">
                                        active {Math.round(s.age_ms / 1000)}s · idle{" "}
                                        {Math.round(s.idle_ms / 1000)}s
                                    </span>
                                </li>
                            )}
                        </For>
                    </ul>
                </section>

                {/* ONB-1 — Integration details: the SP-side values to paste into your
                    IdP. Shown above SSO config because you give these to the IdP first. */}
                <section class="admin-section" data-admin-integration>
                    <h4>Connect your IdP — paste these in</h4>
                    <button
                        type="button"
                        class="tree-action"
                        data-admin-sso-wizard
                        onClick={() => setWizardOpen(true)}
                    >
                        Set up SSO (guided) →
                    </button>
                    <Show when={integration()} fallback={<p class="muted">Loading…</p>}>
                        {(d) => (
                            <ul class="integration-list">
                                <CopyRow label="OIDC redirect URI" value={d().oidc.redirect_uri} />
                                <CopyRow label="SCIM base URL" value={d().scim.base_url} />
                                <CopyRow label="SAML SP entity ID" value={d().saml.sp_entity_id} />
                                <CopyRow label="SAML metadata URL" value={d().saml.metadata_url} />
                                <CopyRow label="SAML ACS URL" value={d().saml.acs_url} />
                                <li class="muted" data-saml-status>
                                    SAML: {d().saml.status}
                                </li>
                            </ul>
                        )}
                    </Show>
                </section>

                {/* B12 — SSO */}
                <SsoSection api={props.api} current={sso()} act={act} />

                {/* B13 — SCIM */}
                <ScimSection api={props.api} setStatus={setStatus} />

                {/* B15 — Security */}
                <SecuritySection api={props.api} current={security()} act={act} />
                <PlacementPolicySection api={props.api} current={placement()} act={act} />
                <ArchetypeApprovalSection api={props.api} current={approval()} act={act} />

                {/* B16 — Billing */}
                <BillingSection api={props.api} billing={billing()} act={act} />

                {/* B14 — Audit */}
                <AuditSection api={props.api} tick={tick()} />

                <p class="status" data-admin-status>
                    {status()}
                </p>

                <Show when={wizardOpen()}>
                    <SsoWizard
                        api={props.api}
                        onClose={() => {
                            setWizardOpen(false);
                            refresh();
                        }}
                    />
                </Show>
            </div>
        </div>
    );
}

type Act = (verb: string, fn: () => Promise<unknown>) => Promise<void>;

/** A read-only value (an SP endpoint to paste into the IdP) with a copy button (`ONB-1`). */
function CopyRow(props: { label: string; value: string }): JSX.Element {
    const [copied, setCopied] = createSignal(false);
    return (
        <li class="integration-row">
            <span class="integration-label">{props.label}</span>
            <code class="integration-value">{props.value}</code>
            <button
                type="button"
                class="tree-action"
                onClick={() => {
                    void navigator.clipboard?.writeText(props.value);
                    setCopied(true);
                    setTimeout(() => setCopied(false), 1200);
                }}
            >
                {copied() ? "copied ✓" : "copy"}
            </button>
        </li>
    );
}

function SsoSection(props: {
    api: EnterpriseAdminApi;
    current: SsoConnection | null | undefined;
    act: Act;
}): JSX.Element {
    const [protocol, setProtocol] = createSignal("oidc");
    const [issuer, setIssuer] = createSignal("");
    const [audiences, setAudiences] = createSignal("");
    const [rolesClaim, setRolesClaim] = createSignal("");
    const [regionClaim, setRegionClaim] = createSignal("");
    const [tenantClaim, setTenantClaim] = createSignal("");
    // A claim name is "set" only if non-empty; an empty input clears it (→ null, the
    // server's "fall back to env, else unmapped"). Trims so whitespace ≠ a mapping.
    const claimOrNull = (s: string) => (s.trim() ? s.trim() : null);
    const claimMapping = (): SsoClaimMapping => ({
        subject_claim: props.current?.claim_mapping?.subject_claim ?? null,
        roles_claim: claimOrNull(rolesClaim() || props.current?.claim_mapping?.roles_claim || ""),
        region_claim: claimOrNull(regionClaim() || props.current?.claim_mapping?.region_claim || ""),
        tenant_claim: claimOrNull(tenantClaim() || props.current?.claim_mapping?.tenant_claim || ""),
    });
    // The connection as the inputs currently describe it — shared by Save and Test.
    const buildConnection = (): SsoConnection => ({
        protocol: protocol(),
        issuer: issuer() || props.current?.issuer || "",
        audiences: (audiences() || props.current?.audiences?.join(",") || "")
            .split(",")
            .map((a) => a.trim())
            .filter(Boolean),
        metadata: props.current?.metadata || "",
        enforce_sso: props.current?.enforce_sso ?? false,
        claim_mapping: claimMapping(),
    });
    const [testMsg, setTestMsg] = createSignal("");
    return (
        <section class="admin-section">
            <h4>SSO</h4>
            <Show when={props.current}>
                {(c) => (
                    <p class="muted">
                        Connected: {c().protocol.toUpperCase()} · {c().issuer} ·{" "}
                        {c().enforce_sso ? "enforced" : "optional"}
                    </p>
                )}
            </Show>
            <label>
                Protocol
                <select value={protocol()} onChange={(e) => setProtocol(e.currentTarget.value)}>
                    <option value="oidc">OIDC</option>
                    <option value="saml">SAML</option>
                </select>
            </label>
            <label>
                Issuer
                <input
                    data-admin-sso-issuer
                    value={issuer() || props.current?.issuer || ""}
                    onInput={(e) => setIssuer(e.currentTarget.value)}
                    placeholder="https://idp.example.com"
                />
            </label>
            <label>
                Audiences (comma-separated)
                <input
                    value={audiences() || props.current?.audiences?.join(", ") || ""}
                    onInput={(e) => setAudiences(e.currentTarget.value)}
                    placeholder="client-id"
                />
            </label>
            {/* ID-3 — claim mapping: which id-token claims feed the ABAC attributes
                (subject defaults to `sub`). Blank → fall back to env, else unmapped. */}
            <label>
                Roles claim
                <input
                    data-admin-sso-roles-claim
                    value={rolesClaim() || props.current?.claim_mapping?.roles_claim || ""}
                    onInput={(e) => setRolesClaim(e.currentTarget.value)}
                    placeholder="roles / groups"
                />
            </label>
            <label>
                Region claim
                <input
                    value={regionClaim() || props.current?.claim_mapping?.region_claim || ""}
                    onInput={(e) => setRegionClaim(e.currentTarget.value)}
                    placeholder="region"
                />
            </label>
            <label>
                Tenant claim
                <input
                    value={tenantClaim() || props.current?.claim_mapping?.tenant_claim || ""}
                    onInput={(e) => setTenantClaim(e.currentTarget.value)}
                    placeholder="org / tid"
                />
            </label>
            <div class="admin-row">
                <button
                    type="button"
                    class="tree-action"
                    onClick={() => props.act("connect SSO", () => props.api.adminSetSso(buildConnection()))}
                >
                    Save connection
                </button>
                <button
                    type="button"
                    class="tree-action"
                    data-admin-sso-test
                    onClick={async () => {
                        setTestMsg("testing…");
                        try {
                            const r = await props.api.adminTestSso(buildConnection());
                            setTestMsg((r.ok ? "✓ " : "✗ ") + r.detail);
                        } catch (e) {
                            setTestMsg(`test failed: ${e instanceof Error ? e.message : String(e)}`);
                        }
                    }}
                >
                    Test connection
                </button>
                <button
                    type="button"
                    class="tree-action"
                    onClick={() =>
                        props.act(props.current?.enforce_sso ? "relax SSO" : "enforce SSO", () =>
                            props.api.adminSetSso({
                                protocol: props.current?.protocol || protocol(),
                                issuer: props.current?.issuer || issuer(),
                                audiences: props.current?.audiences || [],
                                metadata: props.current?.metadata || "",
                                enforce_sso: !(props.current?.enforce_sso ?? false),
                                claim_mapping: props.current?.claim_mapping,
                            }),
                        )
                    }
                >
                    {props.current?.enforce_sso ? "Disable enforce-SSO" : "Enforce SSO"}
                </button>
            </div>
            <Show when={testMsg()}>
                <p class="status" data-admin-sso-test-result>
                    {testMsg()}
                </p>
            </Show>
        </section>
    );
}

function ScimSection(props: {
    api: EnterpriseAdminApi;
    setStatus: (s: string) => void;
}): JSX.Element {
    const [token, setToken] = createSignal("");
    return (
        <section class="admin-section">
            <h4>Provisioning (SCIM)</h4>
            <p class="muted">
                Issue a bearer token for your IdP to provision members. Shown once — copy it now.
            </p>
            <Show when={token()}>
                <code class="scim-token" data-scim-token>
                    {token()}
                </code>
            </Show>
            <button
                type="button"
                class="tree-action"
                onClick={async () => {
                    try {
                        setToken(await props.api.adminIssueScimToken());
                        props.setStatus("SCIM token issued ✓ (copy it now)");
                    } catch (e) {
                        props.setStatus(`could not issue token: ${e instanceof Error ? e.message : e}`);
                    }
                }}
            >
                Issue / rotate token
            </button>
        </section>
    );
}

function SecuritySection(props: {
    api: EnterpriseAdminApi;
    current: SecurityPolicy | null | undefined;
    act: Act;
}): JSX.Element {
    const [mfa, setMfa] = createSignal<boolean | null>(null);
    const [lifetime, setLifetime] = createSignal("");
    const [idle, setIdle] = createSignal("");
    const [retention, setRetention] = createSignal("");
    const [autoUpg, setAutoUpg] = createSignal<boolean | null>(null);
    const mfaOn = () => (mfa() === null ? (props.current?.require_mfa ?? false) : (mfa() as boolean));
    // AUD-3: the published minimum-retention guarantee (days). 0/unset shows the default (365).
    const retentionDays = () => retention() || String(props.current?.audit_retention_min_days || 365);
    // UX-9: whether this org accepts auto-upgrades of the archetypes its placements use.
    const autoUpgOn = () => (autoUpg() === null ? (props.current?.allow_auto_upgrade ?? false) : (autoUpg() as boolean));
    return (
        <section class="admin-section">
            <h4>Security</h4>
            <label class="admin-check">
                <input type="checkbox" checked={mfaOn()} onChange={(e) => setMfa(e.currentTarget.checked)} />
                Require MFA
            </label>
            <label>
                Session lifetime (seconds)
                <input
                    type="number"
                    value={lifetime() || String(props.current?.session_lifetime_secs ?? 0)}
                    onInput={(e) => setLifetime(e.currentTarget.value)}
                />
            </label>
            <label>
                Idle timeout (seconds)
                <input
                    type="number"
                    value={idle() || String(props.current?.idle_timeout_secs ?? 0)}
                    onInput={(e) => setIdle(e.currentTarget.value)}
                />
            </label>
            <label>
                Audit retention guarantee (days)
                <input
                    type="number"
                    data-admin-retention
                    value={retentionDays()}
                    onInput={(e) => setRetention(e.currentTarget.value)}
                />
            </label>
            <p class="muted">
                The audit log is kept indefinitely; this is the minimum you guarantee buyers (one
                year by default), not a deletion policy.
            </p>
            <label class="admin-check">
                <input type="checkbox" data-admin-auto-upgrade checked={autoUpgOn()} onChange={(e) => setAutoUpg(e.currentTarget.checked)} />
                Accept auto-upgrades of archetypes used here
            </label>
            <p class="muted">
                When off (the default), a new archetype version never changes your placements on
                its own — you take each upgrade manually.
            </p>
            <button
                type="button"
                class="tree-action"
                onClick={() =>
                    props.act("save security policy", () =>
                        props.api.adminSetSecurity({
                            require_mfa: mfaOn(),
                            session_lifetime_secs: Number(
                                lifetime() || props.current?.session_lifetime_secs || 0,
                            ),
                            idle_timeout_secs: Number(idle() || props.current?.idle_timeout_secs || 0),
                            residency_region: props.current?.residency_region ?? null,
                            audit_retention_min_days: Number(
                                retention() || props.current?.audit_retention_min_days || 365,
                            ),
                            allow_auto_upgrade: autoUpgOn(),
                        }),
                    )
                }
            >
                Save security policy
            </button>
        </section>
    );
}

const OPERATORS = ["local", "counterparty", "neutral"] as const;

/** Deployment placement policy (DEPLOY-2): which `(operator, attested)` modes an engagement
 *  touching this org's data may use. Restrict-only — leaving all operators checked + attested
 *  off is the open policy. The pairing gate (DEPLOY-3) enforces it at accept. */
function PlacementPolicySection(props: {
    api: EnterpriseAdminApi;
    current: PlacementPolicy | undefined;
    act: Act;
}): JSX.Element {
    const [requireAttested, setRequireAttested] = createSignal<boolean | null>(null);
    const [ops, setOps] = createSignal<Set<string> | null>(null);
    const attestedOn = () =>
        requireAttested() === null ? (props.current?.require_attested ?? false) : (requireAttested() as boolean);
    // Empty allowed_operators means "all allowed"; render that as every box checked.
    const allowed = () => {
        if (ops() !== null) return ops() as Set<string>;
        const cur = props.current?.allowed_operators ?? [];
        return new Set(cur.length ? cur : OPERATORS);
    };
    const toggleOp = (op: string, on: boolean) => {
        const next = new Set(allowed());
        if (on) next.add(op);
        else next.delete(op);
        setOps(next);
    };
    return (
        <section class="admin-section" data-admin-placement-policy>
            <h4>Deployment placement policy</h4>
            <p class="muted">
                Which deployment modes an engagement touching this org's data may use. Restrict-only;
                enforced at pairing.
            </p>
            <label class="admin-check">
                <input
                    type="checkbox"
                    checked={attestedOn()}
                    onChange={(e) => setRequireAttested(e.currentTarget.checked)}
                />
                Require attested (host-blind) execution
            </label>
            <For each={OPERATORS}>
                {(op) => (
                    <label class="admin-check">
                        <input
                            type="checkbox"
                            checked={allowed().has(op)}
                            onChange={(e) => toggleOp(op, e.currentTarget.checked)}
                        />
                        Allow {op}-operated host
                    </label>
                )}
            </For>
            <button
                type="button"
                class="tree-action"
                data-save-placement-policy
                onClick={() => {
                    // All three checked ⇒ no operator narrowing (send empty = "all").
                    const chosen = OPERATORS.filter((o) => allowed().has(o));
                    const allowed_operators = chosen.length === OPERATORS.length ? [] : chosen;
                    props.act("save placement policy", () =>
                        props.api.adminSetPlacementPolicy({
                            require_attested: attestedOn(),
                            allowed_operators,
                        }),
                    );
                }}
            >
                Save placement policy
            </button>
        </section>
    );
}

/** Archetype-approval policy (ADR 0063): the org default for whether *adding* an archetype
 *  to a project requires the project/placement owner to approve it (the placement lands
 *  pending) or is frictionless (active at once). Distinct from the deployment placement
 *  policy above — this gates the `archetype · project` install, not the execution boundary. */
function ArchetypeApprovalSection(props: {
    api: EnterpriseAdminApi;
    current: ArchetypeApprovalPolicy | undefined;
    act: Act;
}): JSX.Element {
    const [require, setRequire] = createSignal<boolean | null>(null);
    const requireOn = () =>
        require() === null ? (props.current?.require_approval ?? false) : (require() as boolean);
    return (
        <section class="admin-section" data-admin-archetype-approval>
            <h4>Archetype approval</h4>
            <p class="muted">
                When on, adding an archetype to a project leaves it <strong>pending</strong> until
                the project owner approves it; chats can only use approved archetypes. Off (the
                default) means anyone can add an archetype with no friction.
            </p>
            <label class="admin-check">
                <input
                    type="checkbox"
                    data-require-archetype-approval
                    checked={requireOn()}
                    onChange={(e) => setRequire(e.currentTarget.checked)}
                />
                Require owner approval before an added archetype is usable
            </label>
            <button
                type="button"
                class="tree-action"
                data-save-archetype-approval
                onClick={() =>
                    props.act("save archetype-approval policy", () =>
                        props.api.adminSetArchetypeApproval({ require_approval: requireOn() }),
                    )
                }
            >
                Save archetype-approval policy
            </button>
        </section>
    );
}

function BillingSection(props: {
    api: EnterpriseAdminApi;
    billing: { billing: Billing | null; seats_used: number } | undefined;
    act: Act;
}): JSX.Element {
    const [plan, setPlan] = createSignal("");
    const [seats, setSeats] = createSignal("");
    return (
        <section class="admin-section">
            <h4>Billing</h4>
            <p class="muted" data-billing-usage>
                {props.billing?.seats_used ?? 0} / {props.billing?.billing?.seats ?? 0} seats used ·
                plan: {props.billing?.billing?.plan || "—"}
            </p>
            <label>
                Plan
                <input
                    value={plan() || props.billing?.billing?.plan || ""}
                    onInput={(e) => setPlan(e.currentTarget.value)}
                    placeholder="business"
                />
            </label>
            <label>
                Seats
                <input
                    type="number"
                    value={seats() || String(props.billing?.billing?.seats ?? 0)}
                    onInput={(e) => setSeats(e.currentTarget.value)}
                />
            </label>
            <button
                type="button"
                class="tree-action"
                onClick={() =>
                    props.act("save billing", () =>
                        props.api.adminSetBilling({
                            plan: plan() || props.billing?.billing?.plan || "",
                            seats: Number(seats() || props.billing?.billing?.seats || 0),
                        }),
                    )
                }
            >
                Save billing
            </button>
        </section>
    );
}

function AuditSection(props: { api: EnterpriseAdminApi; tick: number }): JSX.Element {
    const [entries] = createResource(
        () => props.tick,
        () => props.api.adminGetAudit(),
    );
    return (
        <section class="admin-section">
            <h4>Audit</h4>
            <ul class="audit-list" data-audit-list>
                <For
                    each={entries()?.slice(-20).reverse()}
                    fallback={<li class="muted">No governance actions yet.</li>}
                >
                    {(e) => (
                        <li class="audit-row">
                            <span class="audit-actor">{e.actor}</span>
                            <span class="audit-action">{e.action}</span>
                            <span class="audit-target">{e.target}</span>
                        </li>
                    )}
                </For>
            </ul>
        </section>
    );
}

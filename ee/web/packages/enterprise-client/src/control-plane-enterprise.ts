import type { RouteJson } from "@gaugewright/control-plane-client";

/** Org profile + defaults (B10). */
export interface OrgSettings {
    readonly display_name: string;
    readonly verified_domains: string[];
    readonly default_region?: string | null;
}
/** A directory member (B11). */
export interface Member {
    readonly id: string;
    readonly authority: string;
    readonly email: string;
    readonly role: string;
    readonly status: string;
    readonly managed_by_scim: boolean;
}
/** Which id-token claims carry the ABAC attributes the verifier maps (B12 / `ID-3`).
 *  All optional — unset falls back to the `GAUGEWRIGHT_OIDC_*_CLAIM` env knob, else
 *  unmapped (subject defaults to `sub`). */
export interface SsoClaimMapping {
    readonly subject_claim?: string | null;
    readonly roles_claim?: string | null;
    readonly region_claim?: string | null;
    readonly tenant_claim?: string | null;
}
/** The SP-side values an admin pastes into their IdP to connect us (`ONB-1`). */
export interface IntegrationDetails {
    readonly base_url: string;
    readonly oidc: { readonly redirect_uri: string; readonly login_url: string };
    readonly saml: {
        readonly sp_entity_id: string;
        readonly acs_url: string;
        readonly metadata_url: string;
        readonly status: string;
    };
    readonly scim: { readonly base_url: string };
}
/** SSO connection (B12). */
export interface SsoConnection {
    readonly protocol: string;
    readonly issuer: string;
    readonly audiences: string[];
    readonly metadata: string;
    readonly enforce_sso: boolean;
    /** How id-token claims map onto ABAC attributes (`ID-3`). */
    readonly claim_mapping?: SsoClaimMapping;
}
/** Security policy (B15). */
export interface SecurityPolicy {
    readonly require_mfa: boolean;
    readonly session_lifetime_secs: number;
    readonly idle_timeout_secs: number;
    readonly residency_region?: string | null;
    /** Minimum audit-retention guarantee in days (AUD-3); `0`/unset ⇒ the published default
     *  (365). A promise floor — the log is kept forever — not a delete policy. */
    readonly audit_retention_min_days?: number;
    /** Whether this org accepts auto-upgrades of archetypes its placements use (UX-9, ADR
     *  0062). Default false — an archetype owner's auto preference falls back to manual here. */
    readonly allow_auto_upgrade?: boolean;
}
/** The org's archetype-approval policy (ADR 0063): the org-level default projects inherit
 *  for whether adding an archetype requires owner approval. */
export interface ArchetypeApprovalPolicy {
    readonly require_approval: boolean;
}
/** Org deployment placement policy (DEPLOY-2): admissible `(operator, attested)` modes for
 *  engagements touching this org's data. Restrict-only; empty `allowed_operators` = all. */
export interface PlacementPolicy {
    readonly require_attested: boolean;
    readonly allowed_operators: ReadonlyArray<"local" | "counterparty" | "neutral">;
}
/** Billing/seat state (B16). */
export interface Billing {
    readonly plan: string;
    readonly seats: number;
}
/** One audit-timeline entry (B14). */
export interface AdminAuditEntry {
    readonly actor: string;
    readonly action: string;
    readonly target: string;
}

export interface EnterpriseAdminApi {
    adminGetOrg(): Promise<OrgSettings | null>;
    adminSetOrg(s: OrgSettings): Promise<void>;
    adminDomainVerifyToken(
        domain: string,
    ): Promise<{ domain: string; record_name: string; record_type: string; value: string }>;
    adminDomainVerify(
        domain: string,
    ): Promise<{ verified: boolean; domain?: string; expected?: { record_name: string; value: string } }>;
    adminGetMembers(): Promise<Member[]>;
    adminGetSessions(): Promise<Session[]>;
    adminInvite(m: { authority: string; email?: string; role: string }): Promise<void>;
    adminSetRole(id: string, role: string): Promise<void>;
    adminDeactivate(id: string): Promise<void>;
    adminIntegration(): Promise<IntegrationDetails>;
    adminGetSso(): Promise<SsoConnection | null>;
    adminSetSso(s: SsoConnection): Promise<void>;
    adminTestSso(s: SsoConnection): Promise<{ ok: boolean; detail: string }>;
    adminGetSecurity(): Promise<SecurityPolicy | null>;
    adminSetSecurity(s: SecurityPolicy): Promise<void>;
    adminGetArchetypeApproval(): Promise<ArchetypeApprovalPolicy>;
    adminSetArchetypeApproval(p: ArchetypeApprovalPolicy): Promise<void>;
    adminGetPlacementPolicy(): Promise<PlacementPolicy>;
    adminSetPlacementPolicy(p: PlacementPolicy): Promise<void>;
    adminGetBilling(): Promise<{ billing: Billing | null; seats_used: number }>;
    adminSetBilling(b: Billing): Promise<void>;
    adminGetAudit(): Promise<AdminAuditEntry[]>;
    adminIssueScimToken(): Promise<string>;
}

export async function adminGetOrg(json: RouteJson): Promise<OrgSettings | null> {
    const o = (await json("GET", "/admin/org")) as { org: OrgSettings | null };
    return o.org;
}

export async function adminSetOrg(json: RouteJson, s: OrgSettings): Promise<void> {
    await json("POST", "/admin/org", s);
}

/** The TXT record to publish to prove control of a domain (`ONB-5`). */
export async function adminDomainVerifyToken(
    json: RouteJson,
    domain: string,
): Promise<{ domain: string; record_name: string; record_type: string; value: string }> {
    return (await json("POST", "/admin/domains/verify-token", { domain })) as {
        domain: string;
        record_name: string;
        record_type: string;
        value: string;
    };
}

/** Verify a domain via its published TXT record (DoH); on success it joins the
 *  org's verified domains (powers auto-join/JIT). `ONB-5`. */
export async function adminDomainVerify(
    json: RouteJson,
    domain: string,
): Promise<{ verified: boolean; domain?: string; expected?: { record_name: string; value: string } }> {
    return (await json("POST", "/admin/domains/verify", { domain })) as {
        verified: boolean;
        domain?: string;
        expected?: { record_name: string; value: string };
    };
}

export async function adminGetMembers(json: RouteJson): Promise<Member[]> {
    const o = (await json("GET", "/admin/members")) as { members: Member[] };
    return o.members;
}

/** One live session in the IT roster (ITGOV-2): the active member's authority + how long
 *  since first-seen (`age_ms`) / last-seen (`idle_ms`). Never carries a bearer. */
export interface Session {
    readonly authority: string;
    readonly age_ms: number;
    readonly idle_ms: number;
}

export async function adminGetSessions(json: RouteJson): Promise<Session[]> {
    const o = (await json("GET", "/admin/sessions")) as { sessions: Session[] };
    return Array.isArray(o.sessions) ? o.sessions : [];
}

export async function adminInvite(
    json: RouteJson,
    m: { authority: string; email?: string; role: string },
): Promise<void> {
    await json("POST", "/admin/members", m);
}

export async function adminSetRole(json: RouteJson, id: string, role: string): Promise<void> {
    await json("POST", `/admin/members/${encodeURIComponent(id)}/role`, { role });
}

export async function adminDeactivate(json: RouteJson, id: string): Promise<void> {
    await json("POST", `/admin/members/${encodeURIComponent(id)}/deactivate`);
}

/** The SP-side integration values an admin pastes into their IdP (`ONB-1`). */
export async function adminIntegration(json: RouteJson): Promise<IntegrationDetails> {
    return (await json("GET", "/admin/integration")) as IntegrationDetails;
}

export async function adminGetSso(json: RouteJson): Promise<SsoConnection | null> {
    const o = (await json("GET", "/admin/sso")) as { sso: SsoConnection | null };
    return o.sso;
}

export async function adminSetSso(json: RouteJson, s: SsoConnection): Promise<void> {
    await json("POST", "/admin/sso", s);
}

/** Live OIDC discovery+JWKS reachability test of a connection (`ONB-3`); not stored. */
export async function adminTestSso(
    json: RouteJson,
    s: SsoConnection,
): Promise<{ ok: boolean; detail: string }> {
    return (await json("POST", "/admin/sso/test", s)) as { ok: boolean; detail: string };
}

export async function adminGetSecurity(json: RouteJson): Promise<SecurityPolicy | null> {
    const o = (await json("GET", "/admin/security")) as { security: SecurityPolicy | null };
    return o.security;
}

export async function adminSetSecurity(json: RouteJson, s: SecurityPolicy): Promise<void> {
    await json("POST", "/admin/security", s);
}

/** The org's archetype-approval policy (ADR 0063): whether adding an archetype to a
 *  project requires owner approval (pending) or is frictionless (active at once). */
export async function adminGetArchetypeApproval(
    json: RouteJson,
): Promise<ArchetypeApprovalPolicy> {
    return (await json("GET", "/admin/archetype-approval")) as ArchetypeApprovalPolicy;
}

export async function adminSetArchetypeApproval(
    json: RouteJson,
    p: ArchetypeApprovalPolicy,
): Promise<void> {
    await json("POST", "/admin/archetype-approval", p);
}

export async function adminGetPlacementPolicy(json: RouteJson): Promise<PlacementPolicy> {
    const o = (await json("GET", "/admin/placement-policy")) as {
        placement_policy: PlacementPolicy;
    };
    return o.placement_policy;
}

export async function adminSetPlacementPolicy(
    json: RouteJson,
    p: PlacementPolicy,
): Promise<void> {
    await json("POST", "/admin/placement-policy", p);
}

export async function adminGetBilling(
    json: RouteJson,
): Promise<{ billing: Billing | null; seats_used: number }> {
    return (await json("GET", "/admin/billing")) as {
        billing: Billing | null;
        seats_used: number;
    };
}

export async function adminSetBilling(json: RouteJson, b: Billing): Promise<void> {
    await json("POST", "/admin/billing", b);
}

export async function adminGetAudit(json: RouteJson): Promise<AdminAuditEntry[]> {
    const o = (await json("GET", "/admin/audit")) as { entries: AdminAuditEntry[] };
    return o.entries;
}

export async function adminIssueScimToken(json: RouteJson): Promise<string> {
    const o = (await json("POST", "/admin/scim/token")) as { token: string };
    return o.token;
}

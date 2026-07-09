import type { RouteJson } from "./control-plane-transport";

/** A facility the person (or a tenant) holds — the attach/revoke unit of hosted
 *  functionality (ADR 0077 §7). Provider-agnostic; `config` is opaque here. */
export interface AccountFacility {
    readonly id: string;
    /** `library_sync` | `cloud_backup` | `hosted_home_node` | `registered_host`. */
    readonly kind: string;
    /** `person` (account-level, follows you) | `tenant` (billed to the tenant). */
    readonly owner: string;
    /** `active` | `suspended` | `revoked` — only `active` opens a connection. */
    readonly status: string;
    readonly displayName: string;
}

/** A tenant in the person's switcher (ADR 0077 §9). `personal` marks the
 *  auto-provisioned tenant-of-one — the Console shows it as your own space, not "your org." */
export interface AccountTenant {
    readonly id: string;
    readonly displayName: string;
    readonly role: string;
    readonly personal: boolean;
}

/** Parse one facility record (total: degrades field-by-field, never throws). */
export function parseFacility(v: unknown): AccountFacility {
    const o = (v ?? {}) as Record<string, unknown>;
    return {
        id: typeof o.id === "string" ? o.id : "",
        kind: typeof o.kind === "string" ? o.kind : "library_sync",
        owner: typeof o.owner === "string" ? o.owner : "person",
        status: typeof o.status === "string" ? o.status : "active",
        displayName: typeof o.display_name === "string" ? o.display_name : "",
    };
}

/** Parse one tenant switcher entry (total; never throws). */
export function parseTenant(v: unknown): AccountTenant {
    const o = (v ?? {}) as Record<string, unknown>;
    return {
        id: typeof o.id === "string" ? o.id : "",
        displayName: typeof o.display_name === "string" ? o.display_name : "",
        role: typeof o.role === "string" ? o.role : "",
        personal: o.personal === true,
    };
}

/** The person's account-level facilities (`GET /account/facilities`). */
export async function accountFacilities(json: RouteJson): Promise<AccountFacility[]> {
    const o = (await json("GET", "/account/facilities")) as { facilities?: unknown[] };
    return Array.isArray(o?.facilities) ? o.facilities.map(parseFacility) : [];
}

/** What `accountAttachFacility` needs; `kind`/`displayName`/`config` are optional. */
export interface AttachFacilityInput {
    id: string;
    kind?: string;
    displayName?: string;
    config?: unknown;
}

/** Attach or update one account-level facility (`POST /account/facilities`). */
export async function accountAttachFacility(
    json: RouteJson,
    input: AttachFacilityInput,
): Promise<AccountFacility> {
    const body: Record<string, unknown> = { id: input.id };
    if (input.kind !== undefined) body.kind = input.kind;
    if (input.displayName !== undefined) body.display_name = input.displayName;
    if (input.config !== undefined) body.config = input.config;
    const o = (await json("POST", "/account/facilities", body)) as { facility?: unknown };
    return parseFacility(o?.facility);
}

/** Detach (revoke) one account-level facility (`DELETE /account/facilities/:id`). */
export async function accountDetachFacility(json: RouteJson, id: string): Promise<void> {
    await json("DELETE", `/account/facilities/${encodeURIComponent(id)}`);
}

/** The person's tenant switcher (`GET /account/tenants`). */
export async function accountTenants(json: RouteJson): Promise<AccountTenant[]> {
    const o = (await json("GET", "/account/tenants")) as { tenants?: unknown[] };
    return Array.isArray(o?.tenants) ? o.tenants.map(parseTenant) : [];
}

import type { RouteJson } from "./control-plane-transport";

/** A device in the account's trusted-devices registry (ACCT-1). */
export interface AccountDevice {
    readonly id: string;
    readonly label: string;
    readonly status: string;
}
/** A linked LLM provider — provider name only, never the token. */
export interface LinkedProvider {
    readonly provider: string;
    readonly linked: boolean;
}

export async function accountDevices(json: RouteJson): Promise<AccountDevice[]> {
    const o = (await json("GET", "/account/devices")) as { devices: AccountDevice[] };
    return o.devices;
}

export async function accountRevokeDevice(json: RouteJson, id: string): Promise<void> {
    await json("POST", `/account/devices/${encodeURIComponent(id)}/revoke`);
}

export async function accountSettings(json: RouteJson): Promise<Record<string, string>> {
    const o = (await json("GET", "/account/settings")) as { settings: Record<string, string> };
    return o.settings;
}

export async function accountSetSetting(
    json: RouteJson,
    key: string,
    value: string,
): Promise<void> {
    await json("PUT", `/account/settings/${encodeURIComponent(key)}`, { value });
}

export async function accountCredentials(json: RouteJson): Promise<LinkedProvider[]> {
    const o = (await json("GET", "/account/credentials")) as { credentials: LinkedProvider[] };
    return o.credentials;
}

export async function accountLinkCredential(
    json: RouteJson,
    provider: string,
    token: string,
): Promise<void> {
    await json("POST", "/account/credentials", { provider, token });
}

export async function accountUnlinkCredential(json: RouteJson, provider: string): Promise<void> {
    await json("DELETE", `/account/credentials/${encodeURIComponent(provider)}`);
}

/** The BYOK providers pinned in one project's coordination scope (LLM-2, ADR 0062) — a
 *  per-project override of the account default; provider names only, never the token. */
export async function projectCredentials(
    json: RouteJson,
    project: string,
): Promise<LinkedProvider[]> {
    const o = (await json("GET", `/projects/${encodeURIComponent(project)}/credentials`)) as {
        credentials: LinkedProvider[];
    };
    return o.credentials;
}

/** Pin a provider's BYOK token for one project (sealed server-side, SEC-4). */
export async function linkProjectCredential(
    json: RouteJson,
    project: string,
    provider: string,
    token: string,
): Promise<void> {
    await json("POST", `/projects/${encodeURIComponent(project)}/credentials`, { provider, token });
}

/** Drop a project's pin, so the project falls back to the account default again. */
export async function unlinkProjectCredential(
    json: RouteJson,
    project: string,
    provider: string,
): Promise<void> {
    await json(
        "DELETE",
        `/projects/${encodeURIComponent(project)}/credentials/${encodeURIComponent(provider)}`,
    );
}

/** Codex OAuth (LLM-1, ADR 0062): whether a codex credential is present in Pi's
 *  store and until when (the token itself is never returned). */
export async function codexStatus(
    json: RouteJson,
): Promise<{ linked: boolean; expires: number | null; expired: boolean }> {
    const o = (await json("GET", "/account/oauth/openai-codex")) as {
        linked?: boolean;
        expires?: number | null;
        expired?: boolean;
    };
    return { linked: Boolean(o.linked), expires: o.expires ?? null, expired: Boolean(o.expired) };
}

/** Whether a first-run user must connect an LLM credential before the runtime can
 *  run a turn (ADR 0075 Phase 0). False under the scripted fake agent (dev/e2e),
 *  so the first-run overlay never gates a no-credential test run. Defaults to
 *  `true` (gate on) if the call fails — fail toward showing the setup step. */
export async function onboardingStatus(json: RouteJson): Promise<{ credentialRequired: boolean }> {
    const o = (await json("GET", "/account/onboarding-status")) as { credential_required?: boolean };
    return { credentialRequired: o.credential_required !== false };
}

/** Start the codex OAuth link; returns the authorize URL to open in a browser. The
 *  server's helper runs the callback server and writes the credential on success —
 *  poll {@link codexStatus} to see it land. */
export async function codexLoginStart(json: RouteJson): Promise<{ url: string }> {
    return (await json("POST", "/account/oauth/openai-codex/start", {})) as { url: string };
}

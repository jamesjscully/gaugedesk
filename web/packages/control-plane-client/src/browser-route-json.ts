import { Rejected } from "./control-plane-domain";
import type { RouteJson } from "./control-plane-transport";

export interface BrowserRouteJsonOptions {
    readonly bearer?: () => string | null;
    readonly publishableKey?: () => string | null;
}

/** Build a route error that carries the server's own message when it sent one.
 *  Control-plane failures return `{ "error": "…" }` (e.g. a 502 whose body explains a
 *  unavailable runtime); without this the UI only ever saw the bare status code. Falls
 *  back to the raw body, then to just the status when the body is empty/unreadable. */
async function routeError(
    method: string,
    path: string,
    res: Response,
): Promise<string> {
    const prefix = `${method} ${path}: ${res.status}`;
    let detail = "";
    try {
        detail = await res.text();
    } catch {
        return prefix;
    }
    try {
        const parsed = JSON.parse(detail) as { error?: unknown };
        if (typeof parsed.error === "string" && parsed.error) return `${prefix} ${parsed.error}`;
    } catch {
        /* not JSON — fall through to the raw text */
    }
    return detail ? `${prefix} ${detail}` : prefix;
}

export function browserRouteJson(
    base: string,
    options: BrowserRouteJsonOptions = {},
): RouteJson {
    return async (method, path, body) => {
        const bearer = options.bearer?.();
        const publishableKey = options.publishableKey?.();
        const res = await fetch(base + path, {
            method,
            headers: {
                ...(body !== undefined ? { "content-type": "application/json" } : {}),
                ...(bearer ? { authorization: `Bearer ${bearer}` } : {}),
                ...(publishableKey ? { "x-gw-publishable-key": publishableKey } : {}),
            },
            // Send the shared `.gaugewright.com` session cookie cross-origin (the hosted Console at
            // app.gaugewright.com → the hub at auth.gaugewright.com), so a cookie session
            // authenticates without a JS-visible bearer (ADR 0077; the server allows credentials for
            // its pinned origin allowlist). Same-origin/desktop is unaffected.
            credentials: "include",
            body: body !== undefined ? JSON.stringify(body) : undefined,
        });
        if (res.status === 409) {
            const r = (await res.json()) as { rejected?: string };
            throw new Rejected(r.rejected ?? "unknown");
        }
        if (!res.ok) throw new Error(await routeError(method, path, res));
        return res.status === 204 ? null : res.json();
    };
}

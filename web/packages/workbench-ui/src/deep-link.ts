/**
 * Deep links — the client's own **address scheme** (`mobile-client.md`,
 * "Links and addressing", MOB-010). A link, a task jump, and a notification tap
 * are the *same operation*: each resolves to a *(environment, selection, pane)*
 * triple and slides the carousel there. Because the client is projection-first
 * (addressable view state → URL), that triple **is a URL**.
 *
 * This module is the **parse** stage only (the first box of the resolution
 * pipeline). It turns an opaque URL string into a typed, branded `DeepLink`; it
 * does *not* check bridge grants, freshness, or access basis, and it does *not*
 * choose a pane — that is the resolver's job (MOB-022,
 * `parse → identify env → ensure grant → check freshness → check access → route`).
 * Keeping parse pure and separate means the resolver's gates each get an
 * explicit outcome and the unsupported-link case is caught here, at the edge.
 *
 * Wire form (the app's own scheme, never a trust surface itself):
 *
 *     gaugewright://<environment>/<kind>/<target-id>[/<sub-target>]
 *
 * mirroring the spec sketch `environment › … › target-kind › target-id ›
 * sub-target` (e.g. `gaugewright://peach/chat/42/turn-7-diff`). A bare `<environment>`
 * with no kind is a navigation link to that environment's root.
 */

import { scopeId, type ScopeId } from "@gaugewright/control-plane-client";

declare const brand: unique symbol;
type Brand<T, B> = T & { readonly [brand]: B };

// ----- Branded environment + target ids --------------------------------------

/** A *bound environment* the link addresses (the active-bridge target). The
 *  resolver switches the active bridge to this environment before landing. */
export type EnvironmentId = Brand<string, "EnvironmentId">;

export function environmentId(raw: string): EnvironmentId {
    if (!raw) throw new Error("empty EnvironmentId");
    return raw as EnvironmentId;
}

/** A target identifier within an environment (a chat id, file path, run id, …),
 *  carried opaquely until the resolver binds it to a concrete projection. */
export type TargetId = Brand<string, "TargetId">;

function targetId(raw: string): TargetId {
    if (!raw) throw new Error("empty TargetId");
    return raw as TargetId;
}

// ----- The kinds (the whole set from the addressing table) -------------------

/** The link kinds the client implements (`mobile-client.md`, "Kinds"). Each
 *  lands somewhere specific; the resolver (MOB-022) maps kind → pane/flow. */
export type DeepLinkKind =
    /** A chat / file / turn-diff / run / review in the *same* environment. */
    | "navigation"
    /** A target in a *different* bound environment (switches the active bridge). */
    | "cross-environment"
    /** A push → deep landing (review-needed / approval / run-finished). */
    | "notification"
    /** A pairing ticket (device↔env) or project invite → the pairing flow. */
    | "pairing"
    /** A chat-embedded handle: payload only with an access basis (`INV-10`). */
    | "resource"
    /** A plain web URL in chat → system browser, never an in-app trust surface. */
    | "external";

const DEEP_LINK_KINDS: readonly DeepLinkKind[] = [
    "navigation",
    "cross-environment",
    "notification",
    "pairing",
    "resource",
    "external",
];

// ----- The parsed link -------------------------------------------------------

/** A parsed deep link: an addressed *(environment, target)* the resolver lands
 *  on. The `environment` is also exposed as a `ScopeId` because every gate and
 *  projection downstream is scope-keyed; `sub` is the optional finer landing
 *  (e.g. a turn-diff within a chat). `external` links carry the raw URL and are
 *  never resolved to an in-app pane. */
export interface DeepLink {
    readonly kind: DeepLinkKind;
    /** The environment the link addresses (the bridge target). */
    readonly environment: EnvironmentId;
    /** The environment as a scope id — what downstream gates/projections key on. */
    readonly scope: ScopeId;
    /** The addressed target within the environment, if any (kind-dependent). */
    readonly target: TargetId | null;
    /** A finer sub-target within `target` (e.g. a turn-diff), if any. */
    readonly sub: TargetId | null;
    /** For `external` links: the verbatim URL handed to the system browser. */
    readonly externalUrl: string | null;
}

/** The explicit failure of a parse — an `unsupported` link is a first-class
 *  outcome, never a silent dead-end (the 112 rule, mirrored in the resolver). */
export class DeepLinkParseError extends Error {
    constructor(message: string) {
        super(message);
        this.name = "DeepLinkParseError";
    }
}

// ----- Parse -----------------------------------------------------------------

const SCHEME = "gaugewright://";

function parseKind(raw: string): DeepLinkKind {
    if ((DEEP_LINK_KINDS as readonly string[]).includes(raw)) {
        return raw as DeepLinkKind;
    }
    throw new DeepLinkParseError(`unsupported deep-link kind: ${JSON.stringify(raw)}`);
}

/**
 * Parse a deep-link URL into a typed `DeepLink`. Pure: no I/O, no resolution.
 *
 * Accepts the app scheme `gaugewright://<environment>/<kind>/<target-id>[/<sub>]`.
 * A bare `gaugewright://<environment>` (no kind) is a `navigation` link to the
 * environment root. An `external` link carries a plain URL as its target and is
 * passed through verbatim for the system browser.
 *
 * Throws `DeepLinkParseError` on anything unrecognised — the caller (resolver)
 * surfaces that as the explicit `unsupported` outcome.
 */
export function parse_deep_link(url: string): DeepLink {
    if (typeof url !== "string" || url.length === 0) {
        throw new DeepLinkParseError("empty deep-link url");
    }
    if (!url.startsWith(SCHEME)) {
        throw new DeepLinkParseError(`not an gaugewright deep link: ${JSON.stringify(url)}`);
    }

    const segments = url
        .slice(SCHEME.length)
        .split("/")
        .filter((s) => s.length > 0)
        .map((s) => decodeURIComponent(s));

    const env = segments[0];
    if (!env) {
        throw new DeepLinkParseError("deep link has no environment");
    }
    const environment = environmentId(env);
    const scope = scopeId(env);

    // Bare environment → navigate to its root.
    if (segments.length === 1) {
        return {
            kind: "navigation",
            environment,
            scope,
            target: null,
            sub: null,
            externalUrl: null,
        };
    }

    const kind = parseKind(segments[1]);

    // `external` carries the verbatim URL (everything after the kind), not a target.
    if (kind === "external") {
        const rest = url.slice(url.indexOf(segments[1]) + segments[1].length + 1);
        if (!rest) {
            throw new DeepLinkParseError("external deep link has no url");
        }
        return {
            kind,
            environment,
            scope,
            target: null,
            sub: null,
            externalUrl: rest,
        };
    }

    const target = segments[2] ? targetId(segments[2]) : null;
    const sub = segments[3] ? targetId(segments[3]) : null;
    return { kind, environment, scope, target, sub, externalUrl: null };
}

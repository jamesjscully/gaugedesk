/**
 * The deep-link **resolver** (`mobile-client.md`, "Resolution pipeline",
 * MOB-022): the single function that turns a parsed {@link DeepLink} into a
 * routing decision. A link tap, a task jump, and a notification tap are the
 * *same operation* — each resolves to a *(environment, selection, pane)* triple
 * and slides the carousel there — so they all run through one resolver.
 *
 * Parse (MOB-010) is the first, pure box of the pipeline and lives in
 * `deep-link.ts`; this module is every box *after* it:
 *
 *     parse → identify env → ensure bridge grant active
 *           → check online/freshness → check access basis (INV-10)
 *           → set route (env, selection, pane) → render
 *
 * Each gate has an **explicit** outcome — never a silent dead-end (the 112 rule,
 * mirrored from the parser's `unsupported`). The resolver does *not* mutate
 * anything: it reuses the committed pure substrates — the connection state
 * machine's {@link deriveStatus} (MOB-018) for the grant/relay gate, and a
 * caller-supplied access-basis predicate for the `resource` gate (`INV-10`) —
 * and returns the decision for the shell to act on (switch the bridge, run the
 * pairing flow, open the system browser, or land the carousel).
 *
 * Why a predicate for the access basis rather than a lookup here: whether a
 * handle's payload may be shown is the core's decision (`resource_access`,
 * mirrored client-side as `AccessPhase`); the resolver only *gates* on it. The
 * shell passes a pure `(scope, target) => AccessPhase` view of what it knows,
 * and the resolver applies `payloadAccessible` — the same predicate the Content
 * pane uses (MOB-016). A `resource` link with no granted basis resolves to
 * `no-access` (the "request access" affordance), never silently to the body.
 */

import {
    deriveStatus,
    type ConnectionStatus,
} from "./connection";
import {
    payloadAccessible,
    type AccessPhase,
} from "./mobile-files";
import {
    parse_deep_link,
    DeepLinkParseError,
    type DeepLink,
    type EnvironmentId,
    type TargetId,
} from "./deep-link";
import type { ScopeId } from "@gaugewright/control-plane-client";
import type { LocalState } from "@gaugewright/control-plane-client";
import type { PaneKind } from "./mobile-layout";

// ----- The route a resolved link lands on ------------------------------------

/** The *(environment, selection, pane)* triple a navigation/notification link
 *  resolves to — the carousel slides here. The `pane` is the deepest pane the
 *  link addresses; the shell may show shallower panes alongside it (tablet),
 *  but this is where the carousel lands. */
export interface ResolvedRoute {
    /** The environment the carousel addresses after the hop. */
    readonly environment: EnvironmentId;
    /** That environment as a scope id — what the panes/projections key on. */
    readonly scope: ScopeId;
    /** The target selected within the environment (a chat / file / run), or
     *  `null` for a bare-environment navigation to its root. */
    readonly target: TargetId | null;
    /** A finer sub-target (e.g. a turn-diff within a chat), if the link named one. */
    readonly sub: TargetId | null;
    /** The carousel pane to land on (`mobile-layout.PaneKind`). */
    readonly pane: PaneKind;
}

// ----- The explicit resolution outcomes --------------------------------------

/** Every box of the pipeline produces an *explicit* outcome — the 112 rule: a
 *  link never silently dead-ends. The `kind` is the machine-readable outcome;
 *  the shell maps it to an affordance (land, pair, switch-and-pair, browser,
 *  "request access", or an error toast). */
export type DeepLinkResolution =
    /** The link resolved cleanly: land the carousel on {@link route}. For a
     *  `cross-environment` link this implies switching the active bridge first
     *  (the grant is already held — a silent switch, no confirm). */
    | { readonly kind: "route"; readonly route: ResolvedRoute }
    /** A `pairing` link, or a link to an environment this device holds no grant
     *  for: run the pairing / approval flow (MOB-026) for {@link environment}. */
    | { readonly kind: "needs-pairing"; readonly environment: EnvironmentId; readonly scope: ScopeId }
    /** A device-bound grant for the target environment exists but is revoked:
     *  delivery is broken, repair needs the owning authority. */
    | { readonly kind: "grant-revoked"; readonly environment: EnvironmentId; readonly scope: ScopeId }
    /** A usable grant exists but the relay is unreachable: the link may not be
     *  followed to a *standing* surface right now (cached reads stay available
     *  through the panes, but the resolver will not land a command path offline). */
    | { readonly kind: "offline"; readonly environment: EnvironmentId; readonly scope: ScopeId }
    /** A `resource` (handle) link whose payload has no granted access basis
     *  (`INV-10`): land the name with the "request access" affordance, never the
     *  body. Carries the route so the Files/Content panes can still show the
     *  named, locked handle. */
    | { readonly kind: "no-access"; readonly route: ResolvedRoute; readonly access: AccessPhase }
    /** An `external` link: hand {@link url} to the system browser — never an
     *  in-app trust surface. */
    | { readonly kind: "external"; readonly url: string }
    /** The link could not be parsed or names something the client cannot route
     *  (mirrors the parser's `unsupported`). */
    | { readonly kind: "unsupported"; readonly reason: string };

// ----- Kind → pane mapping ---------------------------------------------------

/** Which carousel pane a routable link kind lands on, *given whether it names a
 *  target*. The deepest addressed pane: a bare-environment navigation lands on
 *  `nav`; a navigation/notification naming a target lands in `chat` (the target
 *  is opened there); a `resource` (handle) link lands on `content`. The shell
 *  may render shallower panes alongside; this is the carousel's stop. */
function paneForKind(link: DeepLink): PaneKind {
    switch (link.kind) {
        case "resource":
            return "content";
        case "navigation":
        case "cross-environment":
        case "notification":
            return link.target === null ? "nav" : "chat";
        // `pairing` and `external` never reach paneForKind — they are handled as
        // their own outcomes before routing.
        case "pairing":
        case "external":
            return "nav";
    }
}

function routeOf(link: DeepLink): ResolvedRoute {
    return {
        environment: link.environment,
        scope: link.scope,
        target: link.target,
        sub: link.sub,
        pane: paneForKind(link),
    };
}

// ----- The grant gate (reuses the connection state machine, MOB-018) ---------

/** Map the connection status for the link's environment to the resolver's
 *  grant-gate outcome. `active` passes (land); every degraded status is an
 *  explicit refusal — the same vocabulary the offline banner reads, so the
 *  resolver never invents a state the connection machine cannot also report. */
function grantGate(
    status: ConnectionStatus,
): "pass" | "needs-pairing" | "grant-revoked" | "offline" {
    switch (status) {
        case "active":
            return "pass";
        case "offline":
            return "offline";
        case "revoked":
            return "grant-revoked";
        // `expired` is, for repair purposes, the re-issue path that pairing
        // covers; `unpaired`/`paired` mean no usable grant for this environment.
        case "expired":
        case "unpaired":
        case "paired":
            return "needs-pairing";
    }
}

// ----- The resolver ----------------------------------------------------------

/** A pure view of what payload-access basis the device holds for a handle, as
 *  the resolver's `resource` gate consults it (`INV-10`). Mirrors the Content
 *  pane's input: given the addressed scope and target, the phase of that
 *  handle's payload access. Absent any knowledge it returns `init` (name only). */
export type AccessBasisLookup = (scope: ScopeId, target: TargetId) => AccessPhase;

/**
 * Resolve a *parsed* deep link to a routing decision. Pure: same inputs ⇒ same
 * outcome; no I/O, no mutation.
 *
 * The pipeline, in order, so each gate's outcome is unambiguous:
 *
 *   1. `external` → hand off to the system browser (no env gate; never in-app).
 *   2. `pairing`  → run the pairing flow for the addressed environment.
 *   3. **grant gate** — reuse {@link deriveStatus} for the link's environment:
 *      `needs-pairing` / `grant-revoked` / `offline` short-circuit here; only an
 *      `active` grant passes. (This subsumes "identify env" + "ensure grant
 *      active" + "check online" — a cross-environment hop with a held grant is a
 *      silent switch, so it simply passes the gate for its target environment.)
 *   4. **access-basis gate** (`resource` only, `INV-10`) — a handle's payload is
 *      shown only with a granted basis; otherwise `no-access` (with the route, so
 *      the locked, *named* handle still lands).
 *   5. **route** — land the carousel on `(environment, selection, pane)`.
 */
export function resolveDeepLink(
    link: DeepLink,
    local: LocalState,
    relayReachable: boolean,
    now: number,
    accessBasis: AccessBasisLookup,
): DeepLinkResolution {
    // 1. External: straight to the system browser, never an env gate.
    if (link.kind === "external") {
        if (link.externalUrl === null) {
            return { kind: "unsupported", reason: "external link has no url" };
        }
        return { kind: "external", url: link.externalUrl };
    }

    // 2. Pairing: genuine authorization, not a routable surface — run the flow.
    if (link.kind === "pairing") {
        return { kind: "needs-pairing", environment: link.environment, scope: link.scope };
    }

    // 3. Grant gate: identify env → ensure grant active → check online, all via
    //    the committed connection state machine for the link's environment.
    const status = deriveStatus(local, link.environment, relayReachable, now);
    const gate = grantGate(status);
    if (gate === "needs-pairing") {
        return { kind: "needs-pairing", environment: link.environment, scope: link.scope };
    }
    if (gate === "grant-revoked") {
        return { kind: "grant-revoked", environment: link.environment, scope: link.scope };
    }
    if (gate === "offline") {
        return { kind: "offline", environment: link.environment, scope: link.scope };
    }

    const route = routeOf(link);

    // 4. Access-basis gate (INV-10): a resource link needs a granted payload
    //    basis for its target; otherwise it lands the named, locked handle.
    if (link.kind === "resource") {
        if (link.target === null) {
            return { kind: "unsupported", reason: "resource link has no target handle" };
        }
        const access = accessBasis(link.scope, link.target);
        if (!payloadAccessible(access)) {
            return { kind: "no-access", route, access };
        }
    }

    // 5. Route: land the carousel.
    return { kind: "route", route };
}

/**
 * Convenience: parse *and* resolve in one call (the whole pipeline from a raw
 * URL). A parse failure is surfaced as the resolver's `unsupported` outcome, so
 * an unrecognised URL and an unroutable parsed link share one explicit path
 * (the 112 rule). The shell uses this when handed an opaque URL (a tapped link,
 * a notification payload reference); callers that already hold a parsed
 * {@link DeepLink} call {@link resolveDeepLink} directly.
 */
export function resolveDeepLinkUrl(
    url: string,
    local: LocalState,
    relayReachable: boolean,
    now: number,
    accessBasis: AccessBasisLookup,
): DeepLinkResolution {
    let link: DeepLink;
    try {
        link = parse_deep_link(url);
    } catch (err) {
        if (err instanceof DeepLinkParseError) {
            return { kind: "unsupported", reason: err.message };
        }
        throw err;
    }
    return resolveDeepLink(link, local, relayReachable, now, accessBasis);
}

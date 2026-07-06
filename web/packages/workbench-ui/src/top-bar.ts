/**
 * The mobile **top bar's view-derived vocabulary** (`mobile-client.md`, "Top
 * bar", MOB-019): the small, pure projections the {@link TopBar} component needs
 * to paint its three header affordances — the *context header*, the
 * *freshness/connection dot*, and the *next-task badge* — plus a re-export of the
 * canonical pane toggle (MOB-014). Like {@link carousel-view} none of these are
 * navigation truth; they are pure functions of the inputs the shell already
 * holds (the {@link ConnectionStatus} from MOB-018, the projection's
 * {@link FreshnessMarker} from MOB-007, the addressed context, and the queue
 * depth), kept here so the component stays a thin renderer and this layer is
 * testable without a DOM.
 *
 *   ┌──────────────────────────────────────────────────┐
 *   │ ● live   price-leveling · Peach        ⌄ Next ③  │  context + dot + next-task
 *   │   nav    [ chat ]    files    content             │  pane toggle (current boxed)
 *   └──────────────────────────────────────────────────┘
 *
 * The law-bearing part is the **dot**: it folds the connection machine and the
 * projection's freshness into one indicator without ever letting a degraded
 * connection read as `live`. Connection trouble (offline / revoked / expired)
 * dominates the freshness marker — there is no point captioning a body "stale"
 * when the real story is "the bridge is gone" — and among the remaining
 * connected states only a `live` projection paints `live`; any non-live marker
 * surfaces as its own caveat rather than reading as current truth (ADR 0037,
 * mirroring `FreshnessMarker::is_current`). That keeps the top bar honest with
 * the same rule the Content pane's freshness caveat (MOB-016) already enforces.
 */

import { type ConnectionStatus } from "./connection";
import { toggleSegments, type ToggleSegment } from "./carousel-view";
import { type CarouselState } from "./mobile-layout";
import { type FreshnessMarker } from "@gaugewright/control-plane-client";

// ----- Context header --------------------------------------------------------

/** The passive-awareness header (`mobile-client.md`, "Context header"): which
 *  chat / project is active, and the environment the phone is currently bound to.
 *  Tapping it returns to Browse — that intent is the component's wiring, not a fact
 *  decided here. When nothing is addressed the header is idle and carries no
 *  label, so a freshly paired device shows no phantom context. */
export interface ContextHeader {
    /** The addressed chat / project, e.g. `price-leveling · Peach`, or `null`
     *  when the bridge is idle (no environment / chat addressed yet). */
    readonly label: string | null;
    /** Whether tapping the header is a meaningful return-to-Browse (only when a
     *  context is actually shown). */
    readonly returnsToNav: boolean;
}

/** Build the context header from the addressed chat title and its environment.
 *  The label reads `chat · environment` (deepest → broadest, matching the
 *  spec's `price-leveling · Peach`); with only an environment it shows that
 *  alone, and with neither it is idle. Inputs are pre-resolved by the shell —
 *  this only composes the label and decides whether the tap is live. */
export function contextHeader(
    chatTitle: string | null,
    environment: string | null,
): ContextHeader {
    const parts = [chatTitle, environment].filter((p): p is string => p != null && p !== "");
    const label = parts.length > 0 ? parts.join(" · ") : null;
    return { label, returnsToNav: label !== null };
}

// ----- Freshness / connection dot --------------------------------------------

/** The single indicator the dot paints (`mobile-client.md`: "live / stale /
 *  offline / grant-expired"). It is the *fold* of the connection machine and the
 *  projection's freshness marker, never an independent flag — so the dot can
 *  never claim `live` while the bridge is down or a non-live projection is on
 *  screen. */
export type DotState =
    /** Connected to the current basis and the projection is live — the only
     *  state painted as current truth. */
    | "live"
    /** Connected, but the projection on screen is behind / cannot refresh. */
    | "stale"
    /** Connected, but the projection is intentionally partial or policy-redacted
     *  (`INV-10`) or its currentness is undecidable — a non-live caveat, not a
     *  failure. */
    | "caveated"
    /** A usable grant is held but the relay is unreachable: cached reads only,
     *  no standing command. */
    | "offline"
    /** The grant binding this device to the environment was revoked — delivery
     *  is broken; repair needs the owning authority. */
    | "revoked"
    /** The grant's expiry has passed — it must be re-issued before any command. */
    | "expired"
    /** No grant binds this device to the addressed environment yet (first launch
     *  / not paired). */
    | "unpaired";

/** What the dot says and whether it reads as current truth. The `label` is the
 *  short caption beside the dot; `isCurrent` is the single predicate the rest of
 *  the shell reads to know whether on-screen state may be trusted as live (the
 *  same `live`-only rule as `FreshnessMarker::is_current`). */
export interface DotView {
    readonly state: DotState;
    readonly label: string;
    /** Only `live` is current; every other dot is an explicit caveat. */
    readonly isCurrent: boolean;
}

const DOT_LABEL: Record<DotState, string> = {
    live: "live",
    stale: "stale",
    caveated: "limited",
    offline: "offline",
    revoked: "revoked",
    expired: "grant expired",
    unpaired: "not paired",
};

/** Fold the connection status and the on-screen projection's freshness into one
 *  dot. **Connection trouble dominates**: a degraded bridge (offline / revoked /
 *  expired / unpaired) is reported as itself, ignoring the freshness marker —
 *  there is no honest way to caption a body "stale" when the real story is that
 *  the bridge is gone. Only when the bridge is connected (`active` / `paired`)
 *  does the freshness marker decide the dot, and there only a `live` marker
 *  paints `live`; every other marker is a non-live caveat. A `null` marker means
 *  no projection is on screen yet — a connected bridge with nothing to show is
 *  still `live` (there is no stale truth to mislead). */
export function dotState(
    status: ConnectionStatus,
    marker: FreshnessMarker | null,
): DotState {
    switch (status) {
        case "offline":
            return "offline";
        case "revoked":
            return "revoked";
        case "expired":
            return "expired";
        case "unpaired":
            return "unpaired";
        case "paired":
        case "active":
            return freshnessDot(marker);
    }
}

function freshnessDot(marker: FreshnessMarker | null): DotState {
    switch (marker) {
        case null:
        case "live":
            return "live";
        case "stale":
            return "stale";
        case "partial":
        case "redacted":
        case "indeterminate":
            return "caveated";
    }
}

/** The full dot view (state + caption + currentness) for the shell to paint. */
export function dotView(
    status: ConnectionStatus,
    marker: FreshnessMarker | null,
): DotView {
    const state = dotState(status, marker);
    return { state, label: DOT_LABEL[state], isCurrent: state === "live" };
}

// ----- Next-task badge -------------------------------------------------------

/** The `Next ③` affordance (`mobile-client.md`: the human task queue as a header
 *  affordance, **not** a carousel stop). The badge is the queue depth; tapping
 *  jumps to the current (first) task, pull-down opens the full queue — both are
 *  the component's wiring over this derived shape. The badge hides entirely at
 *  depth 0 so an empty queue shows no count. */
export interface NextTaskBadge {
    /** Number of human tasks waiting (the badge count). */
    readonly depth: number;
    /** Whether to show the badge at all (depth > 0). */
    readonly visible: boolean;
    /** Whether a tap can jump to a current task (a task exists to jump to). */
    readonly hasCurrent: boolean;
}

/** Derive the next-task badge from the queue depth. A negative or non-finite
 *  depth is clamped to 0 (an absent queue shows nothing rather than a bad
 *  count). */
export function nextTaskBadge(queueDepth: number): NextTaskBadge {
    const depth = Number.isFinite(queueDepth) && queueDepth > 0 ? Math.floor(queueDepth) : 0;
    return { depth, visible: depth > 0, hasCurrent: depth > 0 };
}

// ----- The composed top-bar view ---------------------------------------------

/** Everything the {@link TopBar} component paints, derived in one pass from the
 *  shell's inputs. Bundling it keeps the component a thin renderer — it reads
 *  fields off this view and never recomputes a fold itself. */
export interface TopBarView {
    readonly context: ContextHeader;
    readonly dot: DotView;
    readonly nextTask: NextTaskBadge;
    /** The canonical pane toggle (reused from MOB-014). */
    readonly toggle: readonly ToggleSegment[];
}

/** The shell's inputs to the top bar, each already resolved by the layers that
 *  own it: the carousel state (MOB-009), the connection status (MOB-018), the
 *  on-screen projection's freshness (MOB-007, `null` when nothing is shown), the
 *  addressed chat/environment, and the human-task queue depth. */
export interface TopBarInputs {
    readonly carousel: CarouselState;
    readonly status: ConnectionStatus;
    readonly freshness: FreshnessMarker | null;
    readonly chatTitle: string | null;
    readonly environment: string | null;
    readonly queueDepth: number;
}

/** Derive the whole top-bar view in one pure pass. */
export function topBarView(inputs: TopBarInputs): TopBarView {
    return {
        context: contextHeader(inputs.chatTitle, inputs.environment),
        dot: dotView(inputs.status, inputs.freshness),
        nextTask: nextTaskBadge(inputs.queueDepth),
        toggle: toggleSegments(inputs.carousel),
    };
}

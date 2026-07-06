/**
 * Desktop projection **freshness + retry** (RF-E4).
 *
 * The mobile client wraps every projection in a {@link ProjectionCarriage} with an
 * explicit {@link Freshness} marker and decays it on read (`projection-cache.ts`,
 * MOB-017) so a stale view never silently reads as current. The desktop shell, by
 * contrast, consumes projections bare over SSE/HTTP: a dropped fetch leaves the UI
 * stuck on whatever it last rendered, with no staleness signal and no way back.
 *
 * This is the *minimal, pure* freshness layer for that path. It does NOT adopt the
 * full carriage wire-format on desktop (the backend serves projections bare and
 * changing that is out of scope); it tracks one thing the shell can know locally —
 * **when each projection last loaded successfully, and whether the latest attempt
 * failed** — and folds those two facts into a single status the shell can render:
 *
 *   - `fresh`   — the last attempt succeeded (we hold a current basis);
 *   - `stale`   — the last *attempt* failed but a prior success is recent enough to
 *                 keep showing with a "couldn't refresh — retry" caveat;
 *   - `stuck`   — a failure with no success at all, or the last success has aged
 *                 past the hard limit, so the held view can no longer be vouched
 *                 for and only a retry will recover it.
 *
 * Like the rest of `state/` this is a pure reducer over its inputs (a clock is
 * supplied, never read), so it is unit-tested without a DOM. It mirrors the
 * confidence-only-ever-drops doctrine of the mobile decay: an `ok` raises
 * confidence to `fresh`; a `fail` can only lower it.
 */

// ----- Status ----------------------------------------------------------------

/** How current a desktop projection is, decided from its last success + last
 *  attempt. Only `fresh` may be presented as current truth; `stale`/`stuck` are
 *  caveats the shell must surface (a freshness pill + a retry control). */
export type FreshnessStatus = "fresh" | "stale" | "stuck";

/** Whether a status may be shown as current truth (mirrors `is_current`): only
 *  `fresh`. Every other status carries a caveat. */
export function isFresh(status: FreshnessStatus): boolean {
    return status === "fresh";
}

/** Whether the shell should offer a retry affordance for this status. A `fresh`
 *  view needs none; `stale`/`stuck` both failed to refresh, so both do. */
export function shouldOfferRetry(status: FreshnessStatus): boolean {
    return status !== "fresh";
}

// ----- Policy -----------------------------------------------------------------

/** How long a prior success keeps a failed refresh `stale` (rather than `stuck`).
 *  Shares the clock unit the shell stamps with (ms in the browser). */
export interface FreshnessPolicy {
    /** Age (in clock units) of the last success past which a failure is `stuck`
     *  rather than `stale` — the held view can no longer be vouched for. */
    readonly stuckAfter: number;
}

/** A conservative default: a failed refresh keeps the last good view `stale` for
 *  60s, after which it is `stuck` and only a successful retry recovers it. */
export const DEFAULT_FRESHNESS_POLICY: FreshnessPolicy = {
    stuckAfter: 60_000,
};

// ----- State ------------------------------------------------------------------

/** The per-projection freshness facts the shell tracks: when it last loaded
 *  successfully, and whether the most recent attempt failed. The {@link status}
 *  is *derived* from these against a clock — never set independently — so it can
 *  never drift from the loads that actually happened. */
export interface FreshnessState {
    /** The clock value of the last successful load, or `null` if none yet. */
    readonly lastSuccessAt: number | null;
    /** Whether the most recent attempt (success or failure) was a failure. */
    readonly lastAttemptFailed: boolean;
    /** A short caveat describing the last failure (or the server-stale caveat), or
     *  `null` when fresh. */
    readonly error: string | null;
    /** UX-13: the last successful load carried a server-declared **non-live** carriage
     *  marker (a `200 OK` the server marked `stale`/`partial`). We hold data, but the
     *  server says it is stale — so it derives to `stale`, not `fresh`. */
    readonly serverStale: boolean;
}

/** The starting state: nothing has loaded yet, no attempt has failed. Derives to
 *  `stuck` (we hold no basis) until the first success. */
export const initialFreshness: FreshnessState = {
    lastSuccessAt: null,
    lastAttemptFailed: false,
    error: null,
    serverStale: false,
};

// ----- Events -----------------------------------------------------------------

/** The two things that move the machine: a projection fetch succeeded at `now`,
 *  or it failed (carrying a short caveat for the shell to show). */
export type FreshnessEvent =
    | { readonly kind: "ok"; readonly now: number }
    | { readonly kind: "fail"; readonly error: string }
    // UX-13: the load succeeded but the carriage marker was non-live (server-declared).
    | { readonly kind: "server-stale"; readonly now: number; readonly caveat: string };

/**
 * UX-13: decide the freshness event for a projection that arrived in a server
 * {@link ProjectionCarriage} marker. A `live` marker is an ordinary success
 * (`ok`); **any** non-live marker (`stale`/`partial`/`redacted`/`indeterminate`)
 * is a server-declared caveat we record as `server-stale` — the data is held (we
 * have a basis) but the server says it is not current, so it must surface with a
 * caveat rather than read as fresh. Pure: same marker ⇒ same event.
 *
 * `marker` is the carriage's `freshness.marker`; `repairHint` its optional
 * affordance (used as the caveat when present). Kept primitive (not the whole
 * carriage) so `state/` stays decoupled from the transport edge.
 */
export function freshnessEventForMarker(
    marker: import("@gaugewright/control-plane-client").FreshnessMarker,
    repairHint: string | null,
    now: number,
): FreshnessEvent {
    if (marker === "live") return { kind: "ok", now };
    return { kind: "server-stale", now, caveat: repairHint ?? `server reports this view ${marker}` };
}

// ----- Derivation (the pure heart) --------------------------------------------

/**
 * Decide the status from the tracked facts against `now`. Pure: same facts +
 * clock ⇒ same status.
 *
 *   - the last attempt succeeded            ⇒ `fresh`;
 *   - it failed but a recent success is held ⇒ `stale` (show with a caveat);
 *   - it failed with no/aged success         ⇒ `stuck` (only a retry recovers).
 */
export function deriveFreshness(
    state: FreshnessState,
    now: number,
    policy: FreshnessPolicy = DEFAULT_FRESHNESS_POLICY,
): FreshnessStatus {
    if (!state.lastAttemptFailed) {
        // The latest attempt succeeded — but if we never actually succeeded yet
        // (initial state) there is no basis to call fresh.
        if (state.lastSuccessAt === null) return "stuck";
        // A server-declared non-live read holds data but is known stale (UX-13).
        return state.serverStale ? "stale" : "fresh";
    }
    // The latest attempt failed: a recent prior success keeps us stale, else stuck.
    if (state.lastSuccessAt === null) return "stuck";
    return now - state.lastSuccessAt <= policy.stuckAfter ? "stale" : "stuck";
}

// ----- The reducer ------------------------------------------------------------

/** Apply an event. An `ok` records the success time and clears the failure flag +
 *  caveat; a `fail` raises the flag and records the caveat but **never erases the
 *  last success** — that prior basis is what keeps a failed refresh `stale`
 *  rather than `stuck` (confidence only ever drops, mirroring the cache decay). */
export function reduceFreshness(state: FreshnessState, event: FreshnessEvent): FreshnessState {
    switch (event.kind) {
        case "ok":
            return { lastSuccessAt: event.now, lastAttemptFailed: false, error: null, serverStale: false };
        case "fail":
            return { ...state, lastAttemptFailed: true, error: event.error };
        case "server-stale":
            // The load worked (we have a basis), but the server marked it non-live: keep
            // the failure flag clear so it derives to `stale` (data shown with a caveat).
            return {
                lastSuccessAt: event.now,
                lastAttemptFailed: false,
                error: event.caveat,
                serverStale: true,
            };
    }
}

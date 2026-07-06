/**
 * The client's **connection state machine** (`mobile-client.md`, "Bridge And
 * Connection States", MOB-018): a *pure* reducer over the device's own pairing
 * facts ({@link LocalState} from `api/bridge.ts`) plus the two things only the
 * shell knows — whether the relay is currently reachable and the wall clock
 * `now`. Given those, it decides the single machine-readable connection status
 * for the environment the user is looking at.
 *
 * It is the substrate the offline banner (MOB-028), the freshness dot (MOB-019),
 * and the deep-link resolver's grant gate (MOB-022) read: each of those asks
 * "can this device issue a standing command to this environment right now?" and
 * the answer is exactly {@link canCommand} over the reduced status.
 *
 * The status is a *function of the inputs*, never an independently mutated flag —
 * so the machine cannot drift from the grants it actually holds. The reducer is a
 * thin event layer over that derivation: an event updates one input
 * (`grants`/`environment`/`relay`/`now`) and we re-derive. That keeps the laws
 * trivially true:
 *
 *   - **No standing command without a current basis** — only `active` permits a
 *     standing command ({@link canCommand}); every degraded state (offline,
 *     revoked, expired, unpaired) refuses it rather than hiding the path behind
 *     disabled local UI (the spec's explicit-outcome rule).
 *   - **A revoked or expired grant is never `active`** — even with the relay up,
 *     because validity is the same predicate the core uses
 *     (`bridgeGrantIsValid`, mirroring `BridgeGrant::is_valid`).
 *
 * The status vocabulary is a deliberate subset of the spec's full table
 * (`112-mobile-paired-controller`): the buildable-now client distinguishes the
 * states it can decide from the facts it holds; `relayUnavailable`,
 * `deviceUntrusted`, and `policyDenied` are folded into `offline`/`revoked` until
 * the relay surfaces their distinct signals (needs-infra).
 */

import {
    activeGrantFor,
    bridgeGrantIsValid,
    bridgeGrantBindsDevice,
    type BridgeGrant,
    type LocalState,
} from "@gaugewright/control-plane-client";

// ----- Status (machine-readable outcomes) ------------------------------------

/** The connection status for the *currently addressed environment*. A subset of
 *  the spec's bridge/connection table the client can decide locally; the names
 *  are the machine-readable outcomes, not the product-friendly UI labels. */
export type ConnectionStatus =
    /** No grant binds this device to the addressed environment — the first-launch
     *  / not-yet-paired state. Resolves into the pairing flow (MOB-026). */
    | "unpaired"
    /** A usable grant exists but the user has not addressed any environment yet
     *  (no active surface). The bridge is held but idle. */
    | "paired"
    /** A usable grant for the addressed environment *and* the relay is reachable:
     *  the only state that permits a new standing command. */
    | "active"
    /** A usable grant exists but the relay is unreachable — cached projections may
     *  be read with freshness warnings, but no standing command may be issued
     *  (`offline_stale` / `relay_unavailable`). */
    | "offline"
    /** The grant binding this device to the environment has been revoked
     *  (`active === false`): delivery is broken, repair needs the owning
     *  authority (`grant_revoked` / `device_untrusted`). */
    | "revoked"
    /** The grant exists and is active but its `expiry` has passed: it must be
     *  re-issued before any command (`grant_expired`). */
    | "expired";

/** Whether `status` permits issuing a *standing* Environment command. Only
 *  `active` does — every degraded state refuses by returning `false` so the UI
 *  surfaces an explicit outcome rather than a silently disabled control. */
export function canCommand(status: ConnectionStatus): boolean {
    return status === "active";
}

// ----- State -----------------------------------------------------------------

/** The reducer's inputs: the device's pairing facts, the environment it is
 *  currently addressing (if any), whether the relay is reachable, and the clock.
 *  The {@link ConnectionState.status} is *derived* from these — it is carried so
 *  consumers read it directly, but it is never set independently of the inputs. */
export interface ConnectionState {
    /** The device's own pairing facts (identity + held grants). */
    readonly local: LocalState;
    /** The environment the user is currently addressing, or `null` when none is
     *  selected (idle bridge). */
    readonly environment: string | null;
    /** Whether the federation relay is currently reachable. */
    readonly relayReachable: boolean;
    /** The clock value the most recent derivation used (drives expiry). */
    readonly now: number;
    /** The derived status — a pure function of the four inputs above. */
    readonly status: ConnectionStatus;
}

// ----- Events ----------------------------------------------------------------

/** The events that move the machine. Each updates exactly one input; the status
 *  is then re-derived. They mirror the only things that can change a connection:
 *  the grants the device holds, the environment it addresses, the relay coming
 *  up or down, and the clock advancing. */
export type ConnectionEvent =
    /** Replace the held grants (a pairing completed, a grant arrived/was dropped,
     *  or a revocation was observed). */
    | { readonly kind: "grants-changed"; readonly grants: readonly BridgeGrant[] }
    /** Address an environment (or `null` to go idle). */
    | { readonly kind: "address"; readonly environment: string | null }
    /** The relay became reachable or unreachable. */
    | { readonly kind: "relay"; readonly reachable: boolean }
    /** The clock advanced — re-evaluate expiry against `now`. */
    | { readonly kind: "tick"; readonly now: number };

// ----- Derivation (the pure heart) -------------------------------------------

/**
 * Decide the status from the four inputs. Pure: same inputs ⇒ same status.
 *
 * The order of checks is the law-bearing part. With no environment addressed the
 * machine is `unpaired` (no grant at all) or `paired` (a grant is held but idle).
 * With an environment addressed we look only at grants for *that* environment
 * that *bind this device*, and report the most-actionable failure first
 * (`revoked` before `expired`) so the UI's repair path is unambiguous; a valid
 * grant is then gated only by relay reachability (`offline` vs `active`).
 */
export function deriveStatus(
    local: LocalState,
    environment: string | null,
    relayReachable: boolean,
    now: number,
): ConnectionStatus {
    if (environment === null) {
        // Idle: paired iff *any* held grant is usable by this device right now.
        return hasAnyUsableGrant(local, now) ? "paired" : "unpaired";
    }

    // A usable, device-bound, unexpired grant for this environment ⇒ active/offline.
    if (activeGrantFor(local, environment, now) !== null) {
        return relayReachable ? "active" : "offline";
    }

    // No usable grant: report why, distinguishing the device-bound grants for this
    // environment that exist but are unusable (revoked / expired) from no grant.
    let sawRevoked = false;
    let sawExpired = false;
    for (const grant of local.grants) {
        if (grant.targetEnvironment !== environment) continue;
        if (!bridgeGrantBindsDevice(grant, local.identity)) continue;
        if (!grant.active) {
            sawRevoked = true;
        } else if (!bridgeGrantIsValid(grant, now)) {
            sawExpired = true;
        }
    }
    if (sawRevoked) return "revoked";
    if (sawExpired) return "expired";
    return "unpaired";
}

function hasAnyUsableGrant(local: LocalState, now: number): boolean {
    for (const grant of local.grants) {
        if (bridgeGrantBindsDevice(grant, local.identity) && bridgeGrantIsValid(grant, now)) {
            return true;
        }
    }
    return false;
}

// ----- The reducer -----------------------------------------------------------

/** Build the initial state for a freshly loaded device: its pairing facts, no
 *  environment addressed yet, relay assumed up until proven otherwise, at `now`.
 *  The status is derived, so a device that boots already holding a usable grant
 *  starts `paired`, and one with none starts `unpaired`. */
export function initialConnection(local: LocalState, now: number): ConnectionState {
    return {
        local,
        environment: null,
        relayReachable: true,
        now,
        status: deriveStatus(local, null, true, now),
    };
}

/** Apply an event: update the one input it carries, then re-derive the status.
 *  Pure — returns the same reference unchanged when the event is a no-op (no
 *  input actually moved), so consumers can cheaply diff. */
export function reduce(state: ConnectionState, event: ConnectionEvent): ConnectionState {
    const next = applyEvent(state, event);
    if (next === state) return state;
    const status = deriveStatus(next.local, next.environment, next.relayReachable, next.now);
    return status === next.status ? next : { ...next, status };
}

function applyEvent(state: ConnectionState, event: ConnectionEvent): ConnectionState {
    switch (event.kind) {
        case "grants-changed":
            if (event.grants === state.local.grants) return state;
            return { ...state, local: { ...state.local, grants: event.grants } };
        case "address":
            if (event.environment === state.environment) return state;
            return { ...state, environment: event.environment };
        case "relay":
            if (event.reachable === state.relayReachable) return state;
            return { ...state, relayReachable: event.reachable };
        case "tick":
            if (event.now === state.now) return state;
            return { ...state, now: event.now };
    }
}

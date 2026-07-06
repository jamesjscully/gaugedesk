/**
 * The **connection-state banner's** view-derived vocabulary (`mobile-client.md`,
 * "Bridge And Connection States", MOB-028): the small, pure projection the
 * {@link ConnectionBanner} island and the {@link MobileChat} composer read to
 * surface a *degraded* connection as an explicit outcome — and to refuse a
 * standing command while it lasts — instead of leaving a silently dead control.
 *
 * It folds the connection machine (MOB-018) into two decisions:
 *
 *   - **The banner** — when (and only when) the connection cannot carry a
 *     standing command, what to say and how loudly. A connection that *can*
 *     command ({@link canCommand}) shows no banner at all (the happy path is
 *     chromeless); every degraded status surfaces its own caption and repair
 *     hint, mirroring the dot's caveats (MOB-019) but as the persistent, in-flow
 *     notice the spec's offline-read state calls for.
 *   - **The send gate** — whether the composer may issue a send *right now*. This
 *     is exactly {@link canCommand} (`mobile-client.md`, "Projection-first
 *     compliance": a send is one of the few commands), so the composer disables
 *     send for the *same* reason the banner shows — never hiding the path behind
 *     a disabled local control without an explicit outcome (the spec's
 *     explicit-outcome rule, `INV-5`: the client cannot manufacture truth, so an
 *     offline send is refused, not optimistically swallowed).
 *
 * Like {@link top-bar} / {@link carousel-view} this is a thin, DOM-free pure
 * layer: every "is the connection degraded, what do I say, may I send?" decision
 * is a pure function here, and the islands only wire Solid signals onto it.
 */

import { canCommand, type ConnectionStatus } from "./connection";

// ----- Severity ---------------------------------------------------------------

/** How the banner reads. `info` is a transient, self-healing condition (the
 *  relay is down but a usable grant is held — reads still work from cache);
 *  `warning` is a standing condition that needs the user (or the owning
 *  authority) to act before commands resume (revoked / expired / unpaired). The
 *  island maps this onto styling and `role` (`status` vs `alert`). */
export type BannerSeverity = "info" | "warning";

/** The banner to paint when the connection cannot carry a standing command. It
 *  is *only* produced for a degraded status — a connection that can command
 *  yields no banner ({@link connectionBanner} returns `null`). */
export interface ConnectionBannerView {
    /** The degraded status this banner reports (drives `data-` attributes and the
     *  repair affordance the island wires). */
    readonly status: ConnectionStatus;
    readonly severity: BannerSeverity;
    /** The short, human caption shown in the banner body. */
    readonly message: string;
    /** A one-line hint at how to recover, or `null` when waiting is the only
     *  action (offline self-heals when the relay returns). */
    readonly repairHint: string | null;
    /** Whether reads still work from cache while this banner shows. `true` for
     *  `offline` (a usable grant is held, only the relay is gone); `false` once
     *  the grant itself is unusable (revoked / expired / unpaired). Lets the
     *  shell decide whether to keep painting cached projections beneath. */
    readonly cachedReadsAvailable: boolean;
}

interface BannerCopy {
    readonly severity: BannerSeverity;
    readonly message: string;
    readonly repairHint: string | null;
    readonly cachedReadsAvailable: boolean;
}

/** Per-degraded-status copy. `active` and `paired` are deliberately absent — a
 *  connection that can command (or is idle-but-usable) shows no banner, so the
 *  map is keyed only by the statuses {@link connectionBanner} surfaces. */
const BANNER_COPY: Record<Exclude<ConnectionStatus, "active" | "paired">, BannerCopy> = {
    offline: {
        severity: "info",
        message: "Offline — showing the last synced view.",
        repairHint: null,
        cachedReadsAvailable: true,
    },
    revoked: {
        severity: "warning",
        message: "This device's access was revoked.",
        repairHint: "Ask the owner to re-pair this device.",
        cachedReadsAvailable: false,
    },
    expired: {
        severity: "warning",
        message: "This device's access has expired.",
        repairHint: "Re-pair to restore access.",
        cachedReadsAvailable: false,
    },
    unpaired: {
        severity: "warning",
        message: "This device is not paired.",
        repairHint: "Pair this device to continue.",
        cachedReadsAvailable: false,
    },
};

/**
 * Derive the banner for a connection status, or `null` when no banner is due.
 *
 * The law-bearing rule is the *iff*: a banner is shown **exactly when** the
 * connection cannot carry a standing command ({@link canCommand} is `false`).
 * That ties the banner to the same predicate the composer's send gate reads, so
 * the two can never disagree — a degraded connection always both shows the
 * notice and refuses the send, and a healthy one does neither. Pure: same status
 * ⇒ same banner.
 */
export function connectionBanner(status: ConnectionStatus): ConnectionBannerView | null {
    // `active` can command (happy path); `paired` (idle bridge, no environment
    // addressed) cannot command but is not a degraded *connection* — there is
    // simply nothing addressed to command yet. Neither carries a banner; only the
    // failure statuses do. The guard also narrows the type for the lookup.
    if (status === "active" || status === "paired") return null;
    const copy = BANNER_COPY[status];
    return {
        status,
        severity: copy.severity,
        message: copy.message,
        repairHint: copy.repairHint,
        cachedReadsAvailable: copy.cachedReadsAvailable,
    };
}

// ----- The send gate ----------------------------------------------------------

/** Whether the composer may issue a send for this connection status. This is
 *  exactly {@link canCommand}: a send is a standing command, so it is permitted
 *  only when the connection is `active`. Surfaced under this name so the composer
 *  reads an intent ("may I send?") rather than re-deriving the connection rule,
 *  and so the send gate and the banner provably share one predicate. */
export function canSendOnConnection(status: ConnectionStatus): boolean {
    return canCommand(status);
}

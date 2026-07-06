/**
 * The mobile **Connection-state banner** (`mobile-client.md`, "Bridge And
 * Connection States", MOB-028): the persistent, in-flow notice shown above the
 * {@link Carousel} when the connection cannot carry a standing command — offline,
 * revoked, expired, or not yet paired. On the happy path (`active`) it renders
 * nothing, so the banner is chromeless until there is an actual outcome to report.
 *
 *   ┌──────────────────────────────────────────────────┐
 *   │ ⚠ Offline — showing the last synced view.        │
 *   └──────────────────────────────────────────────────┘
 *
 * Like {@link TopBar} / {@link MobileChat} this is a thin renderer: the "is the
 * connection degraded, what do I say, can I still read?" decision is the pure
 * {@link connectionBanner} fold in `connection-banner.ts`. The island only paints
 * its derived view and routes the (optional) repair tap back through the host —
 * it decides no connection truth itself. It pairs with the composer's send gate
 * ({@link canSendOnConnection}): the banner and the disabled send are the same
 * outcome, surfaced in two places, never an independent flag.
 */

import { Show, type JSX } from "solid-js";
import { connectionBanner } from "./connection-banner";
import { type ConnectionStatus } from "./connection";

export interface ConnectionBannerProps {
    /** The current connection status (from the MOB-018 machine). When it permits
     *  a standing command the banner renders nothing. */
    readonly status: ConnectionStatus;
    /** Begin the repair the banner suggests (re-pair / re-issue). The host routes
     *  it to the pairing flow (MOB-026); the banner only offers the affordance,
     *  and only when there is a repair hint to act on. */
    readonly onRepair?: () => void;
}

export function ConnectionBanner(props: ConnectionBannerProps): JSX.Element {
    const view = () => connectionBanner(props.status);

    return (
        <Show when={view()}>
            {(banner) => (
                <div
                    class="connection-banner"
                    data-connection-banner={banner().status}
                    data-severity={banner().severity}
                    // A warning needs attention now; an info notice is passive.
                    role={banner().severity === "warning" ? "alert" : "status"}
                >
                    <span class="connection-banner-mark" aria-hidden="true">
                        {banner().severity === "warning" ? "⚠" : "●"}
                    </span>
                    <span class="connection-banner-message">{banner().message}</span>

                    {/* The repair affordance is offered only when waiting is not
                        the recovery (offline self-heals, so it has no hint). */}
                    <Show when={banner().repairHint && props.onRepair}>
                        <button
                            type="button"
                            class="connection-banner-repair"
                            data-connection-repair
                            onClick={() => props.onRepair?.()}
                        >
                            {banner().repairHint}
                        </button>
                    </Show>
                </div>
            )}
        </Show>
    );
}

/**
 * The desktop **projection-freshness banner** (RF-E4): the in-flow notice the
 * workbench shows when a projection fetch fails, so a dropped call surfaces as an
 * explicit "couldn't refresh — retry" outcome instead of leaving the UI silently
 * stuck on a view it can no longer vouch for.
 *
 * It is the desktop sibling of the mobile {@link ConnectionBanner}: a thin
 * renderer over a pure fold ({@link deriveFreshness} / {@link shouldOfferRetry} in
 * `desktop-freshness.ts`). On the happy path (`fresh`) it renders nothing —
 * chromeless until there is an outcome to report. `stale` (a recent good view is
 * still shown) reads as a passive caveat; `stuck` (no basis to trust) reads as a
 * warning. Either way it offers a single retry that re-runs the failed fetch; the
 * host owns the actual re-fetch and the freshness truth.
 */

import { Show, type JSX } from "solid-js";
import {
    type FreshnessStatus,
    isFresh,
    shouldOfferRetry,
} from "./desktop-freshness";

export interface FreshnessBannerProps {
    /** The derived freshness of the projection(s) the host is showing. When
     *  `fresh` the banner renders nothing. */
    readonly status: FreshnessStatus;
    /** A short caveat describing the last failure, shown in place of the default
     *  copy when present. */
    readonly error?: string | null;
    /** Re-run the failed fetch. The host owns the re-fetch; the banner only offers
     *  the affordance, and only while a retry is due. */
    readonly onRetry: () => void;
}

const COPY: Record<Exclude<FreshnessStatus, "fresh">, string> = {
    stale: "Couldn't refresh — showing the last loaded view.",
    stuck: "Couldn't load the latest — nothing current to show.",
};

export function FreshnessBanner(props: FreshnessBannerProps): JSX.Element {
    return (
        <Show when={!isFresh(props.status)}>
            <div
                class="freshness-banner"
                data-freshness-banner={props.status}
                // `stuck` has no trustworthy view beneath it → demands attention;
                // `stale` is a passive caveat over a still-shown view.
                data-severity={props.status === "stuck" ? "warning" : "info"}
                role={props.status === "stuck" ? "alert" : "status"}
            >
                <span class="freshness-banner-mark" aria-hidden="true">
                    {props.status === "stuck" ? "⚠" : "●"}
                </span>
                <span class="freshness-banner-message">
                    {props.error || COPY[props.status as "stale" | "stuck"]}
                </span>
                <Show when={shouldOfferRetry(props.status)}>
                    <button
                        type="button"
                        class="freshness-banner-retry"
                        data-freshness-retry
                        onClick={() => props.onRetry()}
                    >
                        retry
                    </button>
                </Show>
            </div>
        </Show>
    );
}

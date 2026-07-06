/**
 * The mobile **access-request approval panel** (`mobile-client.md`, MOB-030): the
 * panel shown in the Content pane (MOB-016) when a handle's payload is withheld.
 * It is the *action* side of `INV-10` — the Files pane shows the name, the Content
 * pane refuses the body, and this panel is the affordance that turns that refusal
 * into a request the owner can approve, then surfaces the answer.
 *
 * Like {@link MobileContent} / {@link MobileFiles} / {@link Carousel} this is a
 * thin renderer: every "what may the user do, and what do we show?" decision is a
 * pure function in `access-request.ts` (`presentAccessRequest`), so the component
 * only wires Solid's `<Switch>` / `<Show>` onto the derived presentation and never
 * decides access itself (it never grants — that is the owner's, server-side).
 */

import { Show, Switch, Match, type JSX } from "solid-js";
import { presentAccessRequest, type AccessRequestState } from "./access-request";

export interface AccessRequestPanelProps {
    /** The flow state for the withheld handle this panel is about. */
    readonly state: AccessRequestState;
    /** Submit (or re-submit) a request for the payload. The host issues the
     *  `request payload access` command and feeds the result back as a `phase`
     *  event; this panel only invokes the intent. */
    readonly onRequest: (path: string) => void;
    /** Withdraw a still-pending request. */
    readonly onCancel: (path: string) => void;
}

export function AccessRequestPanel(props: AccessRequestPanelProps): JSX.Element {
    const view = () => presentAccessRequest(props.state);

    return (
        <div
            class="access-request"
            data-access-request
            data-phase={view().phase}
            role="status"
        >
            <div class="access-request-caption" data-access-request-label>
                {view().label}
            </div>

            <Switch>
                {/* Awaiting the owner — show the wait, offer a cancel. */}
                <Match when={view().waiting}>
                    <div class="access-request-waiting" data-access-request-waiting>
                        <span class="spinner" aria-hidden="true" />
                        <Show when={view().canCancel}>
                            <button
                                type="button"
                                class="cancel-request"
                                data-cancel-request
                                onClick={() => props.onCancel(view().path)}
                            >
                                cancel request
                            </button>
                        </Show>
                    </div>
                </Match>

                {/* Terminally denied — explain, no further affordance. */}
                <Match when={view().settled && !view().granted}>
                    <div class="access-request-denied" data-access-request-denied>
                        the owner declined this request
                    </div>
                </Match>

                {/* Requestable (init / revoked) — offer the ask. */}
                <Match when={view().canRequest}>
                    <button
                        type="button"
                        class="request-access"
                        data-request-access
                        onClick={() => props.onRequest(view().path)}
                    >
                        request access
                    </button>
                </Match>
            </Switch>
        </div>
    );
}

/**
 * The mobile **Content pane** (`mobile-client.md`, MOB-016): the diff/body of
 * the handle picked in the Files pane (MOB-015), shown at the `content`
 * carousel stop. It is the deep end of the broad→deep order, and it is where
 * `INV-10` becomes visible to the user: **a handle's name is shown, its payload
 * is gated**. So this pane never blanks — when the body is withheld it paints an
 * *explicit* access-denied panel with the affordance that fits (request, wait,
 * re-request, or a terminal explanation), and when the body *is* shown it
 * surfaces any freshness caveat (`stale`/`partial`/`redacted`/`indeterminate`)
 * over it rather than letting a non-current body read as truth (ADR 0037).
 *
 * Like {@link MobileFiles} / {@link Carousel} this is a thin renderer: every
 * "what may I show, and how do I caption it?" decision is a pure function in
 * `mobile-content.ts` (`presentContent`), so the component only wires Solid's
 * `<Switch>` / `<Show>` onto the derived presentation and never decides access
 * or freshness itself.
 */

import { Show, Suspense, Switch, Match, lazy, type JSX } from "solid-js";
import { presentContent, type SelectedHandle } from "./mobile-content";
import { initialAccessRequest } from "./access-request";
import { AccessRequestPanel } from "./AccessRequestPanel";

// The diff body pulls in @git-diff-view (+ highlight.js, ~350 KB); load that
// chunk only when a granted body is actually rendered, not on pane mount (same
// deferral as the desktop {@link ContentViewer}).
const DiffView = lazy(() => import("./DiffView").then((m) => ({ default: m.DiffView })));

export interface MobileContentProps {
    /** The handle selected in the Files pane, or `null` when none is. Carries
     *  its access phase and (only when granted) the payload carriage. */
    readonly selection: SelectedHandle | null;
    /** Ask for payload access on a withheld handle (the access-request flow,
     *  MOB-030). The host issues the `request payload access` command and feeds
     *  the answer back as a new access phase; this pane never grants. */
    readonly onRequestAccess: (path: string) => void;
    /** Withdraw a still-pending access request (MOB-030). The host raises and
     *  tracks the request lifecycle; the panel only invokes the intent. */
    readonly onCancelAccess: (path: string) => void;
}

export function MobileContent(props: MobileContentProps): JSX.Element {
    const view = () => presentContent({ selection: props.selection });

    return (
        <div class="mobile-content" data-pane="content">
            <Switch>
                {/* Nothing selected: invite a pick rather than show a bare blank. */}
                <Match when={view().kind === "empty"}>
                    <div class="status">select a file in the files pane ‹</div>
                </Match>

                {/* Access withheld: the explicit access-denied panel (MOB-016's
                    whole point — a withheld body is never a silent blank), with the
                    access-request approval flow (MOB-030) for the affordance that
                    fits the phase — request / wait+cancel / terminal explanation. */}
                <Match when={view().kind === "denied"}>
                    <div class="content-denied" data-access-denied>
                        <div class="content-head">{view().path}</div>
                        <Show when={view().path && view().denial}>
                            {(_present) => (
                                <AccessRequestPanel
                                    state={initialAccessRequest(
                                        view().path ?? "",
                                        view().denial?.phase ?? "init",
                                    )}
                                    onRequest={(p) => props.onRequestAccess(p)}
                                    onCancel={(p) => props.onCancelAccess(p)}
                                />
                            )}
                        </Show>
                    </div>
                </Match>

                {/* Granted body: render the diff, surfacing any freshness caveat
                    over it (a non-live body is never bare current truth). */}
                <Match when={view().kind === "body"}>
                    <div class="content-body" data-content-body>
                        <div class="content-head">{view().path}</div>
                        <Show when={view().freshnessCaveat}>
                            {(caveat) => (
                                <div
                                    class="freshness-caveat"
                                    data-freshness={caveat().marker}
                                    role="status"
                                >
                                    {caveat().label}
                                    <Show when={caveat().repairHint}>
                                        {(hint) => <span class="repair-hint"> · {hint()}</span>}
                                    </Show>
                                </div>
                            )}
                        </Show>
                        <Suspense fallback={<div class="status">loading diff…</div>}>
                            <DiffView diff={view().diff ?? ""} />
                        </Suspense>
                    </div>
                </Match>
            </Switch>
        </div>
    );
}

/**
 * The mobile **Top bar** (`mobile-client.md`, "Top bar", MOB-019): the fixed
 * header above the {@link Carousel} island that carries the three awareness
 * affordances and the canonical pane toggle.
 *
 *   ┌──────────────────────────────────────────────────┐
 *   │ ● live   price-leveling · Peach        ⌄ Next ③  │
 *   │   nav    [ chat ]    files    content             │
 *   └──────────────────────────────────────────────────┘
 *
 * - **Context header** — the active chat / project (and, passively, the bound
 *   environment); tapping it returns to Browse.
 * - **Freshness/connection dot** — folds the connection machine (MOB-018) and the
 *   on-screen projection's freshness (MOB-007) into one indicator that never
 *   reads `live` while the bridge is degraded or a non-live projection is shown.
 * - **Next-task badge** — the human task queue as a header affordance, *not* a
 *   carousel stop; tapping the count **jumps to the current task**, the `⌄`
 *   chevron **opens the full queue sheet** (current-first), per `mobile-client.md`.
 * - **Pane toggle** — the same canonical, labelled control the carousel offers,
 *   greyed where unreachable for the current selection. Suppressed (`showToggle`
 *   = false) when the host already renders the canonical toggle elsewhere (the
 *   carousel island carries it on the phone, so the bar would otherwise double it).
 *
 * Like {@link Carousel} / {@link MobileContent} this is a thin renderer: every
 * "what do I show, and how do I caption it?" decision is a pure function in
 * `top-bar.ts` ({@link topBarView}), so the component only wires Solid signals
 * and DOM events onto the derived view and decides no freshness or reachability
 * itself. It is *controlled* — the host owns the {@link CarouselState} signal and
 * routes toggle taps back through `onState`, exactly as the carousel does, so the
 * two header rows stay one navigation truth.
 */

import { createMemo, Show, type JSX } from "solid-js";
import { tapGesture } from "./carousel-view";
import { reduce } from "./carousel";
import { topBarView, type TopBarInputs } from "./top-bar";
import { type CarouselGesture, type CarouselState } from "./mobile-layout";

export interface TopBarProps extends Omit<TopBarInputs, "carousel"> {
    /** Current carousel state (drives the toggle); the host owns the signal. */
    readonly carousel: CarouselState;
    /** Apply a reduced carousel state (the host's setter) — toggle taps and the
     *  context "return to Browse" tap flow through here, never mutating truth here. */
    readonly onState: (next: CarouselState) => void;
    /** Jump to the current human task (the next-task tap). The host resolves the
     *  task to a route; the badge never navigates itself. */
    readonly onJumpToTask: () => void;
    /** Open the full queue sheet (the `⌄` pull-down). The host owns the sheet's
     *  open state; the chevron only signals the intent. */
    readonly onOpenQueue: () => void;
    /** Render the canonical pane toggle row. Defaults to `true`; the phone passes
     *  `false` because the carousel island below already carries the toggle (and
     *  its "new chat" smarts), so the bar would otherwise paint a second one. */
    readonly showToggle?: boolean;
}

export function TopBar(props: TopBarProps): JSX.Element {
    const view = createMemo(() =>
        topBarView({
            carousel: props.carousel,
            status: props.status,
            freshness: props.freshness,
            chatTitle: props.chatTitle,
            environment: props.environment,
            queueDepth: props.queueDepth,
        }),
    );

    // Route a gesture through the pure reducer; the host owns the resulting truth.
    const apply = (gesture: CarouselGesture) => props.onState(reduce(props.carousel, gesture));

    return (
        <div class="top-bar">
            <div class="top-bar-status">
                <span class="freshness-dot" data-dot={view().dot.state} role="status">
                    <span class="dot-mark" aria-hidden="true">
                        ●
                    </span>
                    {view().dot.label}
                </span>

                {/* Context header: tap returns to Browse (only when a context is shown). */}
                <button
                    type="button"
                    class="context-header"
                    data-context-header
                    aria-disabled={!view().context.returnsToNav}
                    disabled={!view().context.returnsToNav}
                    onClick={() => view().context.returnsToNav && apply(tapGesture("nav"))}
                >
                    {view().context.label ?? "—"}
                </button>

                {/* Next-task affordance: the `⌄` chevron opens the full queue
                    sheet; the count jumps to the current task. Both are explicit
                    controls (the spec's pull-down + tap), hidden when empty. */}
                <Show when={view().nextTask.visible}>
                    <span class="next-task" data-next-task>
                        <button
                            type="button"
                            class="next-task-open"
                            data-open-queue
                            aria-label="open the task queue"
                            onClick={() => props.onOpenQueue()}
                        >
                            ⌄
                        </button>
                        <button
                            type="button"
                            class="next-task-jump"
                            data-jump-task
                            onClick={() => view().nextTask.hasCurrent && props.onJumpToTask()}
                        >
                            Next <span class="next-task-badge">{view().nextTask.depth}</span>
                        </button>
                    </span>
                </Show>
            </div>

            {/* Canonical pane toggle — the same control the carousel offers. On the
                phone the carousel island below renders it, so we suppress it here. */}
            <Show when={props.showToggle ?? true}>
                <div class="top-bar-toggle" role="tablist" aria-label="panes">
                    {view().toggle.map((seg) => (
                        <button
                            type="button"
                            class="carousel-seg"
                            role="tab"
                            classList={{ current: seg.current, greyed: !seg.reachable }}
                            aria-selected={seg.current}
                            aria-disabled={!seg.reachable}
                            onClick={() => apply(tapGesture(seg.pane))}
                        >
                            {seg.label}
                        </button>
                    ))}
                </div>
            </Show>
        </div>
    );
}

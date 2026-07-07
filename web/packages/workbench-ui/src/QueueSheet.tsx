/**
 * The mobile **full queue sheet** (`mobile-client.md`, "Top bar" → *Next ③*): the
 * pull-down the `⌄` chevron opens, listing the whole human task queue
 * **current-first**. It is the phone's analog of the desktop {@link TaskBar} — the
 * place to *see* every review-needed item, not just the current one the badge
 * jumps to — re-flowed from a one-row bar into a top-anchored sheet so it works in
 * the one-pane carousel.
 *
 * A thin renderer (`INV-5`): it paints the `GET /tasks` projection the host already
 * holds and reports a tap back through `onJump`; it owns no truth and issues no
 * command itself. Tapping a row resolves to the same *(selection, pane)* the badge
 * jump does — opening that chat — so the sheet is a navigation surface, not a
 * second review-command path (consent/keep stay on the inline chat card, MOB-031).
 */

import { For, Show, type JSX } from "solid-js";
import { type EngagementId, type HumanTask } from "@gaugewright/control-plane-client";
import { displayChatTitle } from "./chat-title";

export interface QueueSheetProps {
    /** The human task queue, already current-first (the server orders it). */
    readonly tasks: readonly HumanTask[];
    /** Jump to a task — opens its chat (the host resolves the route + closes us). */
    readonly onJump: (id: EngagementId) => void;
    /** Dismiss the sheet without navigating (backdrop / close affordance). */
    readonly onClose: () => void;
}

export function QueueSheet(props: QueueSheetProps): JSX.Element {
    return (
        <div class="queue-sheet-scrim" data-queue-scrim onClick={() => props.onClose()}>
            {/* Stop the backdrop dismiss from firing on taps inside the sheet. */}
            <div
                class="queue-sheet"
                data-queue-sheet
                role="dialog"
                aria-label="task queue"
                onClick={(e) => e.stopPropagation()}
            >
                <div class="queue-sheet-head">
                    <span class="queue-sheet-title">tasks</span>
                    <button
                        type="button"
                        class="queue-sheet-close"
                        data-queue-close
                        aria-label="close the task queue"
                        onClick={() => props.onClose()}
                    >
                        ×
                    </button>
                </div>
                <ul class="queue-sheet-list">
                    <For
                        each={props.tasks}
                        fallback={<li class="status" data-queue-empty>no reviews pending</li>}
                    >
                        {(t: HumanTask, i) => (
                            <li>
                                {/* Onboarding issues (ADR 0075) carry a whip work-item
                                    id, not an engagement, so their row is informational
                                    — it doesn't jump to a chat. Review rows navigate. */}
                                <Show
                                    when={t.kind === "issue"}
                                    fallback={
                                        <button
                                            type="button"
                                            class="queue-sheet-item"
                                            classList={{ current: i() === 0 }}
                                            data-queue-task={t.id}
                                            onClick={() => props.onJump(t.id as EngagementId)}
                                        >
                                            <Show when={i() === 0}>
                                                <span class="queue-item-current">current</span>
                                            </Show>
                                            <span class="queue-item-kind">{t.kind}</span>
                                            <span class="queue-item-title">{displayChatTitle(t.title)}</span>
                                            <span class="queue-item-agent">{t.agent}</span>
                                        </button>
                                    }
                                >
                                    <div
                                        class="queue-sheet-item queue-sheet-issue"
                                        data-queue-task={t.id}
                                        data-queue-kind="issue"
                                    >
                                        <span class="queue-item-kind">onboarding</span>
                                        <span class="queue-item-title">{displayChatTitle(t.title)}</span>
                                    </div>
                                </Show>
                            </li>
                        )}
                    </For>
                </ul>
            </div>
        </div>
    );
}

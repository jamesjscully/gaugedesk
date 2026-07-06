/**
 * The human task queue (`navigation.md` B1, `15-task-queue`): the top bar surfaces
 * **review-needed** work, current-first. M0 sources it from our own merge
 * lifecycle — a finished turn awaiting keep/reject is a "Review: {chat}" task.
 * Click a tab to open that chat; ✓ on the active task **keeps** it (admit→advance),
 * clearing it from the queue. Milestones / focus input / delegate are M1+.
 *
 * A thin renderer (`INV-5`): it shows a projection (`GET /tasks`) and submits the
 * existing merge command; it owns no truth.
 */

import { createResource, createSignal, For, Show } from "solid-js";
import type { EngagementId, HumanTask } from "@gaugewright/control-plane-client";
import { displayChatTitle } from "./chat-title";

/** A stable accent colour for a task, derived from the agent it's pinned to (#22):
 *  tasks group visually by their agent. An unpinned task (no agent) stays neutral. */
function agentColor(agent: string | undefined): string | undefined {
    if (!agent) return undefined; // not pinned to an agent → neutral (the default border)
    let hue = 0;
    for (let i = 0; i < agent.length; i++) hue = (hue * 31 + agent.charCodeAt(i)) % 360;
    return `hsl(${hue} 55% 58%)`;
}

export interface TaskQueueApi {
    getTasks(): Promise<HumanTask[]>;
}

export function TaskBar(props: {
    api: TaskQueueApi;
    selected: EngagementId | null;
    refreshKey: unknown;
    /** True when the *selected* chat's pending change loosens the assistant's
     *  permissions: the bar must route it through the in-panel review/confirm
     *  rather than expose a one-click keep (#5 round-4). */
    selectedLoosening?: boolean;
    onSelect: (id: EngagementId) => void;
    onComplete: (id: EngagementId) => void;
}) {
    const [tasks] = createResource(
        () => props.refreshKey,
        () => props.api.getTasks(),
    );

    // Keeping a change merges it permanently into the shared copy — and there's no
    // un-merge once it's done. So the always-visible top-bar `✓ keep` is guarded by
    // a two-click confirm (round-6 #2), the same forgiveness pattern the context-menu
    // `delete` uses: the first click arms ("keep?"), the second commits. A change the
    // user hasn't even looked at can no longer be merged from the chrome in one click.
    const [arming, setArming] = createSignal<EngagementId | null>(null);

    return (
        <div class="taskbar" data-testid="taskbar">
            <span class="taskbar-label">tasks</span>
            <div class="task-tabs">
                <For
                    each={tasks() ?? []}
                    fallback={<span class="status">no reviews pending</span>}
                >
                    {(t: HumanTask) => {
                        const active = () => props.selected === t.id;
                        const color = agentColor(t.agent);
                        // One canonical title everywhere (#4): never leak the raw
                        // "new chat" placeholder — show the same "Untitled" the tree
                        // and chat header show, so the pill is recognisably the same chat.
                        const title = () => displayChatTitle(t.title);
                        return (
                            <span
                                class="task-tab"
                                classList={{ active: active() }}
                                data-task={t.id}
                                data-task-agent={t.agent}
                                // Keyboard/SR reachable (#4 round-5): the review pills
                                // were clickable spans with no role/tabindex, so the one
                                // always-visible queue of pending decisions was a wall for
                                // keyboard users. Make each pill a real focusable button.
                                role="button"
                                tabindex="0"
                                aria-label={`open review for ${title()}`}
                                title={`Open "${title()}" to review the agent's work — keep it or discard it`}
                                style={color ? { "border-left": `3px solid ${color}` } : undefined}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter" || e.key === " ") { e.preventDefault(); setArming(null); props.onSelect(t.id); }
                                }}
                                onClick={() => { setArming(null); props.onSelect(t.id); }}
                            >
                                <span class="task-kind">review</span>
                                <span class="task-title">{title()}</span>
                                <span class="task-agent">{t.agent}</span>
                                <Show when={active()}>
                                    {/* A change that loosens the assistant's permissions
                                        is never one-click-kept from the bar (#5): the pill
                                        opens the in-panel review, where the keep is gated
                                        behind a plain-language confirm. Safe changes keep
                                        the fast one-click path so `send → keep` stays low
                                        friction (#1). */}
                                    <Show
                                        when={props.selectedLoosening}
                                        fallback={
                                            <button
                                                class="task-keep"
                                                classList={{ confirming: arming() === t.id }}
                                                data-task-keep
                                                data-arming={arming() === t.id ? "1" : undefined}
                                                title={
                                                    arming() === t.id
                                                        ? "Click again to keep this into the shared copy — this can't be undone"
                                                        : "Keep this work into the shared copy"
                                                }
                                                onClick={(e) => {
                                                    e.stopPropagation();
                                                    if (arming() !== t.id) {
                                                        setArming(t.id);
                                                        return;
                                                    }
                                                    setArming(null);
                                                    props.onComplete(t.id);
                                                }}
                                            >
                                                {arming() === t.id ? "confirm keep" : "✓ keep"}
                                            </button>
                                        }
                                    >
                                        <button
                                            class="task-keep warn"
                                            data-task-review
                                            title="This changes the assistant's permissions — open the review to see what changed before keeping"
                                            onClick={(e) => {
                                                e.stopPropagation();
                                                props.onSelect(t.id);
                                            }}
                                        >
                                            review →
                                        </button>
                                    </Show>
                                </Show>
                            </span>
                        );
                    }}
                </For>
            </div>
        </div>
    );
}

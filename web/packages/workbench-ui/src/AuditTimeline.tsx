/**
 * The history "Timeline" tab (round-6 #1): a **plain-language activity list** of
 * what happened in this chat — "You asked: …", "Wrote agent-note.txt", "Kept into
 * the shared copy" — folded from the raw audit log by `toActivity`.
 *
 * The raw event-sourcing rows (internal type names, serialized JSON, resource
 * IDs) used to be dumped verbatim here; that is dev-console material, not a
 * user-facing history. The raw rows are still reachable behind a quiet "show the
 * raw event log" developer toggle for debugging — never the default.
 */

import { createResource, createSignal, For, Show } from "solid-js";
import type { AuditEvent, ScopeId } from "@gaugewright/control-plane-client";
import { toActivity } from "./audit-activity";
import { LoadError } from "./LoadError";

export interface AuditTimelineApi {
    getAudit(scope: ScopeId): Promise<AuditEvent[]>;
}

export function AuditTimeline(props: { api: AuditTimelineApi; scope: ScopeId; refreshKey?: unknown }) {
    const [events, { refetch }] = createResource(
        () => [props.scope, props.refreshKey] as const,
        ([s]) => props.api.getAudit(s),
    );
    const [showRaw, setShowRaw] = createSignal(false);
    const activity = () => toActivity(events() ?? []);

    return (
        <Show when={!events.error} fallback={<LoadError what="the history" onRetry={() => void refetch()} />}>
        <Show
            when={(events()?.length ?? 0) > 0}
            fallback={
                <div class="status">
                    Nothing recorded yet. This is the story of this chat — every time the
                    assistant works, the files it changes, and your keep/discard
                    decisions — in plain language, newest at the bottom.
                </div>
            }
        >
            <div class="activity" data-audit>
                <Show
                    when={activity().length > 0}
                    fallback={<div class="status">Nothing the assistant did yet.</div>}
                >
                    <For each={activity()}>
                        {(a) => (
                            <div class="activity-item" data-activity-tone={a.tone}>
                                <span class={`activity-dot ${a.tone}`} aria-hidden="true" />
                                <span class="activity-text">{a.text}</span>
                            </div>
                        )}
                    </For>
                </Show>

                {/* Developer escape hatch: the raw event-sourcing log, off by default
                    so a layperson never meets it (round-6 #1). */}
                <button
                    class="link-button raw-log-toggle"
                    data-raw-log-toggle
                    onClick={() => setShowRaw((v) => !v)}
                >
                    {showRaw() ? "hide the raw event log" : "show the raw event log"}
                </button>
                <Show when={showRaw()}>
                    <div class="timeline raw-log" data-raw-log>
                        <For each={events()}>
                            {(e) => (
                                <div class="event">
                                    <span class="pos">{e.position}</span>
                                    <span class="kind">{e.kind}</span>
                                    <span class="payload">{e.payload}</span>
                                </div>
                            )}
                        </For>
                    </div>
                </Show>
            </div>
        </Show>
        </Show>
    );
}

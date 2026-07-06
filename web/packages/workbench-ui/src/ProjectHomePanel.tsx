/**
 * **Project Home** (`UX-2`, `mvp-workbench.md` "Project Home"): the per-project summary
 * panel — recent runs, live outputs/reviews, and an audit rollup — all derived **from
 * data** server-side (`INV-5`; `GET /projects/:id/home`) and rendered here. Opened from a
 * project node's "project home…" menu; the id comes from context, never typed.
 *
 * A thin, total renderer over the parsed {@link ProjectHome}: a partial/empty rollup
 * degrades to empty lists + zero counts (the parser never throws), so the panel always
 * renders.
 */

import { createResource, For, Show, type JSX } from "solid-js";
import type { ProjectHome } from "@gaugewright/control-plane-client";

export interface ProjectHomeApi {
    projectHome(project: string): Promise<ProjectHome>;
}

export function ProjectHomePanel(props: {
    api: ProjectHomeApi;
    project: string;
    projectName: string;
    onOpenChat?: (chat: string) => void;
    onClose: () => void;
}): JSX.Element {
    const [home] = createResource(() => props.project, (p) => props.api.projectHome(p));

    return (
        <div class="modal-overlay" onClick={() => props.onClose()}>
            <div
                class="modal project-home-panel"
                data-project-home-panel={props.project}
                role="dialog"
                aria-label={`project home for ${props.projectName}`}
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => e.key === "Escape" && props.onClose()}
            >
                <div class="modal-head">
                    <h3>{props.projectName}</h3>
                    <button type="button" onClick={() => props.onClose()}>
                        ×
                    </button>
                </div>

                <section class="admin-section" data-project-home-audit>
                    <h4>At a glance</h4>
                    <ul class="member-list">
                        <li class="member-row">
                            <span class="member-id">placements</span>
                            <span class="badge" data-audit-placements>{home()?.audit.placements ?? 0}</span>
                        </li>
                        <li class="member-row">
                            <span class="member-id">chats</span>
                            <span class="badge" data-audit-chats>{home()?.audit.chats ?? 0}</span>
                        </li>
                        <li class="member-row">
                            <span class="member-id">events</span>
                            <span class="badge" data-audit-events>{home()?.audit.events ?? 0}</span>
                        </li>
                    </ul>
                </section>

                <section class="admin-section" data-project-home-runs>
                    <h4>Recent runs</h4>
                    <ul class="member-list">
                        <For
                            each={home()?.recentRuns ?? []}
                            fallback={<li class="muted">No runs in this project yet.</li>}
                        >
                            {(r) => (
                                <li
                                    class="member-row"
                                    data-run-chat={r.chat}
                                    onClick={() => props.onOpenChat?.(r.chat)}
                                >
                                    <span class="member-id">{r.title || "untitled chat"}</span>
                                    <span class="member-status">{r.phase}</span>
                                    <Show when={r.ran}>
                                        <span class="badge">ran</span>
                                    </Show>
                                </li>
                            )}
                        </For>
                    </ul>
                </section>

                <section class="admin-section" data-project-home-outputs>
                    <h4>Outputs under review</h4>
                    <ul class="member-list">
                        <For
                            each={home()?.outputs ?? []}
                            fallback={<li class="muted">No outputs awaiting review.</li>}
                        >
                            {(o) => (
                                <li
                                    class="member-row"
                                    data-output-chat={o.chat}
                                    onClick={() => props.onOpenChat?.(o.chat)}
                                >
                                    <span class="member-id">{o.title || "untitled chat"}</span>
                                    <span class="member-status">{o.phase}</span>
                                </li>
                            )}
                        </For>
                    </ul>
                </section>
            </div>
        </div>
    );
}

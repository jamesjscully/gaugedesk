/**
 * The history shelf (round-6 #1): opened as a right-side drawer from the chat
 * header. For a layperson it shows one thing — the plain-language **activity**
 * list of what happened in this chat (the Timeline tab).
 *
 * The raw review/export **state-machine** control panel (propose / consent A /
 * target admit / release) is engine-authoring material, not user-facing UI, so
 * it's gated behind developer mode (`?dev=1`) — off by default. When dev mode is
 * on, a second "Review" tab appears for debugging.
 */

import { createSignal, For, Show } from "solid-js";
import type { EngagementId, ScopeId } from "@gaugewright/control-plane-client";
import { AuditTimeline, type AuditTimelineApi } from "./AuditTimeline";
import { isDevMode } from "./dev-mode";
import { OutputCatalog, type OutputCatalogApi } from "./OutputCatalog";
import { ReviewShelf, type ReviewShelfApi } from "./ReviewShelf";

type Tab = "audit" | "outputs" | "review";
const TAB_LABEL: Record<Tab, string> = { audit: "Activity", outputs: "Outputs", review: "Review (dev)" };

export type ShelfApi = AuditTimelineApi & OutputCatalogApi & ReviewShelfApi;

export function Shelf(props: {
    api: ShelfApi;
    /** The chat's scope (scope-keyed surfaces: activity timeline, review driver). */
    scope: ScopeId;
    /** The same chat as an engagement id (the engagement-keyed outputs route). The
     *  owner (`App`) mints both brands; the shelf never launders one into the other. */
    id: EngagementId;
    onClose: () => void;
}) {
    const dev = isDevMode();
    const [tab, setTab] = createSignal<Tab>("audit");
    // Activity + Outputs are user-facing; Review stays gated behind dev mode.
    const tabs = (): Tab[] => (dev ? ["audit", "outputs", "review"] : ["audit", "outputs"]);
    return (
        // A right-side drawer (#6) rather than a thin bar floating mid-screen: it
        // sits against the chat lane it describes; the dimmed backdrop still closes it.
        <div class="drawer-overlay" data-history-overlay onClick={props.onClose}>
            <div class="drawer shelf-drawer" onClick={(e) => e.stopPropagation()}>
                <div class="modal-head">
                    <div class="tabs" style={{ border: "none", margin: 0 }}>
                        <For each={tabs()}>
                            {(t) => (
                                <span class="tab" data-tab={t} classList={{ active: tab() === t }} onClick={() => setTab(t)}>
                                    {TAB_LABEL[t]}
                                </span>
                            )}
                        </For>
                    </div>
                    <button onClick={props.onClose}>close</button>
                </div>
                <Show when={tab() === "audit"}>
                    <AuditTimeline api={props.api} scope={props.scope} />
                </Show>
                <Show when={tab() === "outputs"}>
                    <OutputCatalog api={props.api} id={props.id} />
                </Show>
                <Show when={dev && tab() === "review"}>
                    <ReviewShelf api={props.api} scope={props.scope} />
                </Show>
            </div>
        </div>
    );
}

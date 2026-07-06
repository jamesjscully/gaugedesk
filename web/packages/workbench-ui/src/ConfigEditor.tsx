/**
 * Archetype authoring (edit mode): edit the engagement's
 * `.agent-config.json`. The control plane parses-then-persists, so an invalid
 * config is rejected at the boundary (400) and surfaced here, never written.
 */

import { createResource, createSignal, Show } from "solid-js";
import type { EngagementId } from "@gaugewright/control-plane-client";

export interface ConfigEditorApi {
    getConfig(id: EngagementId): Promise<string>;
    putConfig(id: EngagementId, raw: string): Promise<void>;
}

export interface ConfigEditorProps {
    api: ConfigEditorApi;
    id: EngagementId;
    onClose: () => void;
}

export function ConfigEditor(props: ConfigEditorProps) {
    const [loaded] = createResource(
        () => props.id,
        (id) => props.api.getConfig(id),
    );
    const [draft, setDraft] = createSignal<string | null>(null);
    const [msg, setMsg] = createSignal("");
    const text = () => draft() ?? loaded() ?? "{}";

    async function save() {
        try {
            await props.api.putConfig(props.id, text());
            setMsg("saved");
        } catch (e) {
            setMsg(String(e));
        }
    }

    return (
        <div class="modal" data-config-editor>
            <div class="modal-head">
                <h3>Settings · {props.id}</h3>
                <button onClick={props.onClose}>close</button>
            </div>
            <Show when={loaded.state === "ready" || draft() !== null} fallback={<div class="status">loading...</div>}>
                <textarea
                    class="config-text"
                    data-config-text
                    spellcheck={false}
                    value={text()}
                    onInput={(e) => setDraft(e.currentTarget.value)}
                />
            </Show>
            <div class="bar">
                <button onClick={save}>save</button>
                <span class="status" data-config-status>{msg()}</span>
            </div>
        </div>
    );
}

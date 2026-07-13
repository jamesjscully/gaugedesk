/**
 * GaugeDesk-owned runtime selection for an archetype. Method behavior and tool
 * authority live in the authored WhippleScript package; this surface owns only
 * host/provider choices such as the preferred model.
 *
 * Round 5 (#5): the modal used to be a single raw `{}` JSON textarea labelled
 * "Advanced … leave it as {} to use the defaults" — so the one beginner
 * instruction the empty state gives ("set what this method does in settings")
 * dead-ended at a field that told the beginner not to touch it. There was nowhere
 * to express, in plain words, how the method should behave. We now lead with a
 * plain-language model field and demote raw provider settings to Advanced. The
 * package draft is edited in an edit chat and frozen by Publish.
 */

import { createMemo, createResource, createSignal, onCleanup, Show } from "solid-js";
import { type ArchetypeId } from "@gaugewright/control-plane-client";

/** Turn a raw parser error (often double-wrapped JSON with a line/column) into one
 *  plain sentence (#2). The raw JSON is only the Advanced surface now, so we tell
 *  the user *what's wrong* in their terms rather than leaking the parser's object. */
export function plainConfigError(raw: string): string {
    if (/package-owned|\.whipple\/draft/i.test(raw)) {
        return "Behavior and tools are package-owned — change them in an edit chat, then publish.";
    }
    if (/trailing characters|expected|EOF|column|invalid|parse/i.test(raw)) {
        return "That isn't valid settings text — check for a stray character or a missing comma, bracket, or quote.";
    }
    // An unexpected (non-parse) failure: keep it short, drop the "Error:" prefix.
    return raw.replace(/^Error:\s*/, "").trim() || "Couldn't save those settings.";
}

/** The GaugeDesk runtime setting exposed by the plain form. */
interface FormConfig {
    model: string;
}

/** Read the subset the form controls out of a parsed config object. Unknown/missing
 *  fields fall back to the safe defaults the boundary itself uses. */
export function readFormConfig(parsed: unknown): FormConfig {
    const o = (parsed ?? {}) as Record<string, unknown>;
    return {
        model: typeof o.model === "string" ? o.model : "",
    };
}

/** Fold the form values back into a config object, preserving any other keys that
 *  were already there (so the Advanced JSON and the form never fight). */
export function writeFormConfig(prev: unknown, form: FormConfig): Record<string, unknown> {
    const base = (typeof prev === "object" && prev ? { ...(prev as Record<string, unknown>) } : {}) as Record<string, unknown>;
    // Model: omit the key entirely when blank, so "default" stays the default.
    if (form.model.trim()) base.model = form.model.trim();
    else delete base.model;
    delete base.policy;
    delete base.tools;
    return base;
}

export interface AgentSettingsApi {
    getArchetypeConfig(id: ArchetypeId): Promise<string>;
    setArchetypeConfig(id: ArchetypeId, config: string): Promise<void>;
}

export interface AgentSettingsProps {
    api: AgentSettingsApi;
    id: ArchetypeId;
    name: string;
    onClose: () => void;
}

export function AgentSettings(props: AgentSettingsProps) {
    const [loaded] = createResource(
        () => props.id,
        (id) => props.api.getArchetypeConfig(id),
    );
    // The raw JSON the Advanced section edits. Until the user touches Advanced it
    // tracks the loaded config; the form edits flow through it too, so saving always
    // sends one coherent document.
    const [raw, setRaw] = createSignal<string | null>(null);
    const [msg, setMsg] = createSignal("");
    const [showAdvanced, setShowAdvanced] = createSignal(false);
    const text = () => raw() ?? loaded() ?? "{}";

    // Escape closes the modal (#6 round-5: it didn't, leaving the user to hunt for
    // "close" — a forgiveness/convention gap). A native listener: Solid delegates
    // events, so a synthetic keydown on the modal wouldn't catch a focused textarea.
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && props.onClose();
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));

    // Parse the current text for the form. If the raw JSON is mid-edit and invalid,
    // the form falls back to defaults (and we keep editing through Advanced).
    const parsed = createMemo<unknown>(() => {
        try {
            return JSON.parse(text());
        } catch {
            return null;
        }
    });
    const form = createMemo(() => readFormConfig(parsed()));
    const rawIsValid = () => parsed() !== null;

    function updateForm(patch: Partial<FormConfig>) {
        const next = writeFormConfig(parsed() ?? {}, { ...form(), ...patch });
        setRaw(JSON.stringify(next, null, 2));
        setMsg("");
    }

    async function save() {
        try {
            await props.api.setArchetypeConfig(props.id, text());
            setMsg("saved");
        } catch (e) {
            setMsg(plainConfigError(String(e)));
        }
    }

    return (
        <div class="modal" data-config-editor>
            <div class="modal-head">
                <h3>Settings · {props.name}</h3>
                <button onClick={props.onClose}>close</button>
            </div>
            <p class="status" style={{ margin: "0 0 10px" }}>
                GaugeDesk chooses the runtime here. Change behavior and tools in an edit chat, then publish the package.
            </p>

            <Show
                when={loaded.state === "ready" || raw() !== null}
                fallback={<div class="status">loading…</div>}
            >
                <div class="settings-form" data-settings-form>
                    <label class="settings-field">
                        <span class="settings-label">Preferred model</span>
                        <input
                            class="settings-input"
                            data-settings-model
                            placeholder="leave blank to use the default"
                            value={form().model}
                            onInput={(e) => updateForm({ model: e.currentTarget.value })}
                        />
                    </label>

                </div>

                {/* The raw JSON is now a collapsed power-user surface, not the only
                    way in (#5). It edits the same document the form does. */}
                <button
                    type="button"
                    class="settings-advanced-toggle"
                    data-settings-advanced-toggle
                    onClick={() => setShowAdvanced((v) => !v)}
                >
                    {showAdvanced() ? "▾" : "▸"} Advanced (raw settings)
                </button>
                <Show when={showAdvanced()}>
                    <p class="status" style={{ margin: "4px 0 6px" }}>
                        The exact settings text. Leave it as <code>{"{}"}</code> to use the defaults.
                    </p>
                    <textarea
                        class="config-text"
                        data-config-text
                        spellcheck={false}
                        value={text()}
                        onInput={(e) => { setRaw(e.currentTarget.value); setMsg(""); }}
                    />
                    <Show when={!rawIsValid()}>
                        <div class="status" data-config-status>That isn't valid settings text — check for a stray character or a missing comma, bracket, or quote.</div>
                    </Show>
                </Show>
            </Show>

            <div class="bar">
                <button data-settings-save onClick={save}>save</button>
                <span class="status" data-config-status>{msg()}</span>
            </div>
        </div>
    );
}

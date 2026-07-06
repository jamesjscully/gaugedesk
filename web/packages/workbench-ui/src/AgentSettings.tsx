/**
 * Archetype settings (ADR 0035): edit an **archetype's** `.agent-config.json` —
 * the method every chat rooted on it (and every placement of it) inherits. The
 * control plane parses-then-persists, so an invalid config is rejected at the
 * boundary (400) and surfaced here, never written.
 *
 * Round 5 (#5): the modal used to be a single raw `{}` JSON textarea labelled
 * "Advanced … leave it as {} to use the defaults" — so the one beginner
 * instruction the empty state gives ("set what this method does in settings")
 * dead-ended at a field that told the beginner not to touch it. There was nowhere
 * to express, in plain words, how the method should behave. We now lead with a
 * small **plain-language form** over the config keys the boundary actually
 * respects (model, network, what the assistant may run) — every control maps to a
 * real `.agent-config.json` field — and demote the raw JSON to a collapsed
 * "Advanced" section. (A free-text "what should this method do?" persona →
 * AGENTS.md is genuinely the right next step but needs an archetype-level
 * read/write route the backend doesn't expose yet; faking it through the config
 * JSON would be silently ignored by the boundary parser — see response.md.)
 */

import { createMemo, createResource, createSignal, onCleanup, Show } from "solid-js";
import { type ArchetypeId } from "@gaugewright/control-plane-client";

/** Turn a raw parser error (often double-wrapped JSON with a line/column) into one
 *  plain sentence (#2). The raw JSON is only the Advanced surface now, so we tell
 *  the user *what's wrong* in their terms rather than leaking the parser's object. */
export function plainConfigError(raw: string): string {
    if (/trailing characters|expected|EOF|column|invalid|parse/i.test(raw)) {
        return "That isn't valid settings text — check for a stray character or a missing comma, bracket, or quote.";
    }
    // An unexpected (non-parse) failure: keep it short, drop the "Error:" prefix.
    return raw.replace(/^Error:\s*/, "").trim() || "Couldn't save those settings.";
}

/** The shape we read/write through the plain form. We only touch the keys we show;
 *  anything else in the config (provider, extra policy fields) is preserved. */
interface FormConfig {
    model: string;
    posture: "ask" | "auto";
    allowNetwork: boolean;
    allowShell: boolean;
}

/** Read the subset the form controls out of a parsed config object. Unknown/missing
 *  fields fall back to the safe defaults the boundary itself uses. */
export function readFormConfig(parsed: unknown): FormConfig {
    const o = (parsed ?? {}) as Record<string, unknown>;
    const policy = (o.policy ?? {}) as Record<string, unknown>;
    // Map the boundary's posture enum (kebab-case: trust-by-default | prompt-on-risk
    // | policy-only-block) onto the form's binary toggle. trust-by-default — and the
    // unset default — reads as "go ahead on its own"; the prompting/blocking postures
    // read as "ask first".
    const posture = policy.posture === "prompt-on-risk" || policy.posture === "policy-only-block" ? "ask" : "auto";
    const block = Array.isArray(policy.block_tools) ? (policy.block_tools as string[]) : [];
    return {
        model: typeof o.model === "string" ? o.model : "",
        posture,
        allowNetwork: policy.allow_network === true,
        // The shell is "allowed" unless explicitly blocked — the one toggle whose
        // wrong setting is dangerous, so we surface it plainly.
        allowShell: !block.includes("bash") && !block.includes("shell"),
    };
}

/** Fold the form values back into a config object, preserving any other keys that
 *  were already there (so the Advanced JSON and the form never fight). */
export function writeFormConfig(prev: unknown, form: FormConfig): Record<string, unknown> {
    const base = (typeof prev === "object" && prev ? { ...(prev as Record<string, unknown>) } : {}) as Record<string, unknown>;
    const policy = { ...((base.policy as Record<string, unknown>) ?? {}) };
    // Model: omit the key entirely when blank, so "default" stays the default.
    if (form.model.trim()) base.model = form.model.trim();
    else delete base.model;
    // Write the boundary's enum, never the form's shorthand: the backend rejects
    // unknown variants, so "ask"/"auto" must become prompt-on-risk / trust-by-default
    // or every save 400s (the round-5 form never round-tripped against the real enum).
    policy.posture = form.posture === "auto" ? "trust-by-default" : "prompt-on-risk";
    policy.allow_network = form.allowNetwork;
    const block = new Set(
        (Array.isArray(policy.block_tools) ? (policy.block_tools as string[]) : []).filter(
            (t) => t !== "bash" && t !== "shell",
        ),
    );
    if (!form.allowShell) block.add("bash");
    if (block.size) policy.block_tools = [...block];
    else delete policy.block_tools;
    base.policy = policy;
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
                How this archetype behaves. Every chat using it inherits these settings.
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

                    <fieldset class="settings-field">
                        <span class="settings-label">Before doing something risky</span>
                        <label class="settings-radio">
                            <input
                                type="radio"
                                name="posture"
                                data-settings-posture="ask"
                                checked={form().posture === "ask"}
                                onChange={() => updateForm({ posture: "ask" })}
                            />
                            <span>Ask me first <span class="status">(recommended)</span></span>
                        </label>
                        <label class="settings-radio">
                            <input
                                type="radio"
                                name="posture"
                                data-settings-posture="auto"
                                checked={form().posture === "auto"}
                                onChange={() => updateForm({ posture: "auto" })}
                            />
                            <span>Go ahead on its own</span>
                        </label>
                    </fieldset>

                    <label class="settings-checkbox">
                        <input
                            type="checkbox"
                            data-settings-shell
                            checked={form().allowShell}
                            onChange={(e) => updateForm({ allowShell: e.currentTarget.checked })}
                        />
                        <span>Let it run commands on the computer <span class="status">(more powerful, less safe)</span></span>
                    </label>
                    <label class="settings-checkbox">
                        <input
                            type="checkbox"
                            data-settings-network
                            checked={form().allowNetwork}
                            onChange={(e) => updateForm({ allowNetwork: e.currentTarget.checked })}
                        />
                        <span>Let it use the internet</span>
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

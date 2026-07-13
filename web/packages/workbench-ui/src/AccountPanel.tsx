/**
 * **Your account** (ACCT-1, ADR 0053): the operator's own surface — linked LLM
 * provider accounts, the trusted-device registry, and settings. Distinct from the
 * org Admin Console (a company you administer) and from Devices (federation pairing).
 *
 * A thin renderer over `/account/*`. The linked credential's token is write-only: you
 * link a provider, but the token is never read back (sealed server-side, SEC-4).
 */

import { createResource, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import type {
    AccountDevice,
    LinkedProvider,
} from "@gaugewright/control-plane-client";
import {
    ADVANCEMENT_RULES_SETTING,
    parseAdvancementScopes,
    serializeAdvancementScopes,
} from "./advancement";
import {
    ATTENTION_RULES_SETTING,
    ATTENTION_SIGNALS,
    parseAttentionRules,
    serializeAttentionRules,
    type AttentionLevel,
    type AttentionSignal,
} from "./attention";
import { waitForCodexLink } from "./codex-link-poll";
import {
    defaultVisibleKeys,
    ENABLED_MODELS_SETTING,
    modelKey,
    parseEnabledModels,
    pickableModels,
    serializeEnabledModels,
} from "./model-picker";

const PROVIDERS = ["openai", "anthropic"];

export interface AccountPanelApi {
    accountCredentials(): Promise<LinkedProvider[]>;
    accountDevices(): Promise<AccountDevice[]>;
    codexStatus(): Promise<{ linked: boolean; expires: number | null; expired: boolean }>;
    codexLoginStart(): Promise<{ url: string }>;
    accountLinkCredential(provider: string, token: string): Promise<void>;
    accountUnlinkCredential(provider: string): Promise<void>;
    accountRevokeDevice(id: string): Promise<void>;
    accountSettings(): Promise<Record<string, string>>;
    accountSetSetting(key: string, value: string): Promise<void>;
}

export function AccountPanel(props: {
    api: AccountPanelApi;
    /** Whether this runtime can complete the local Codex OAuth helper flow. */
    codexLoginAvailable?: boolean;
    onClose: () => void;
}): JSX.Element {
    const [tick, setTick] = createSignal(0);
    const refresh = () => setTick((t) => t + 1);
    const [status, setStatus] = createSignal("");

    const [credentials] = createResource(tick, () => props.api.accountCredentials());
    const [devices] = createResource(tick, () => props.api.accountDevices());
    // Codex OAuth (LLM-1, ADR 0062): presence + expiry in GaugeDesk's sealed
    // account store. `authUrl` holds the link as a manual
    // fallback if the popup is blocked.
    const [codex] = createResource(tick, () => props.api.codexStatus());
    const [authUrl, setAuthUrl] = createSignal("");
    const codexExpiry = () => {
        const e = codex()?.expires;
        return e ? new Date(e).toLocaleDateString() : null;
    };
    // The sign-in finishes in an external tab and the credential lands server-side,
    // so after starting it we poll the status projection and update on our own —
    // the manual refresh link stays only as the fallback for a very slow flow.
    let disposed = false;
    onCleanup(() => {
        disposed = true;
    });
    let watchingLink = false;
    const linkCodex = async () => {
        setStatus("starting OpenAI sign-in…");
        setAuthUrl("");
        // A re-sign-in starts with the old credential still linked, so completion
        // is "the expiry changed", not "a credential exists" — baseline it here.
        const baselineExpires = codex()?.expires ?? null;
        try {
            const { url } = await props.api.codexLoginStart();
            setAuthUrl(url);
            window.open(url, "_blank", "noopener,noreferrer");
            setStatus("finish the OpenAI sign-in in your browser — this panel updates by itself");
        } catch (e) {
            setStatus(`could not start sign-in: ${e instanceof Error ? e.message : String(e)}`);
            return;
        }
        if (watchingLink) return; // a prior click's watcher is already at it
        watchingLink = true;
        try {
            const linked = await waitForCodexLink(() => props.api.codexStatus(), {
                baselineExpires,
                cancelled: () => disposed,
            });
            if (disposed) return;
            if (linked) {
                setAuthUrl("");
                setStatus("signed in ✓");
                refresh();
            } else {
                setStatus("couldn't confirm the sign-in — finish it in the browser, then use refresh");
            }
        } finally {
            watchingLink = false;
        }
    };

    const [provider, setProvider] = createSignal("openai");
    const [token, setToken] = createSignal("");

    const link = async () => {
        if (!token()) {
            setStatus("paste a token first");
            return;
        }
        try {
            await props.api.accountLinkCredential(provider(), token());
            setToken("");
            setStatus(`linked ${provider()} ✓`);
            refresh();
        } catch (e) {
            setStatus(`could not link: ${e instanceof Error ? e.message : String(e)}`);
        }
    };
    const unlink = async (p: string) => {
        try {
            await props.api.accountUnlinkCredential(p);
            setStatus(`unlinked ${p}`);
            refresh();
        } catch (e) {
            setStatus(`could not unlink: ${e instanceof Error ? e.message : String(e)}`);
        }
    };
    const revoke = async (id: string) => {
        try {
            await props.api.accountRevokeDevice(id);
            setStatus(`revoked ${id}`);
            refresh();
        } catch (e) {
            setStatus(`could not revoke: ${e instanceof Error ? e.message : String(e)}`);
        }
    };

    const isLinked = (p: string) => (credentials() ?? []).some((c) => c.provider === p);

    // --- which models appear in the composer picker (LLM-1) ---------------------------
    // The catalog gives every model the linked accounts can run; this section lets the
    // operator choose which of them show in the per-chat picker. The choice persists in
    // the account-settings KV; absent → the default-visible subset (so the checklist is
    // pre-ticked with what the picker shows today).
    const [modelSettings, { refetch: refetchModelSettings }] = createResource(tick, () => props.api.accountSettings());
    const linkedAccounts = () => {
        const ps = (credentials() ?? []).filter((c) => c.linked).map((c) => c.provider);
        if (codex()?.linked) ps.push("openai-codex");
        return ps;
    };
    const allModels = () => pickableModels(linkedAccounts());
    const effectiveEnabled = () =>
        parseEnabledModels(modelSettings()?.[ENABLED_MODELS_SETTING]) ?? defaultVisibleKeys(linkedAccounts());
    async function toggleModel(key: string, on: boolean) {
        const next = new Set(effectiveEnabled());
        if (on) next.add(key);
        else next.delete(key);
        try {
            await props.api.accountSetSetting(ENABLED_MODELS_SETTING, serializeEnabledModels(next));
            refetchModelSettings();
        } catch (e) {
            setStatus(`could not update models: ${e instanceof Error ? e.message : String(e)}`);
        }
    }

    // --- advancement (ATTN-3, ADR 0082 §4): what auto-keeps without review ------------
    // One comma-separated scopes field editing the single writes-within rule. Empty =
    // everything holds (fail-closed). The unwaivable guards (config touch, external
    // reads) live server-side and are stated, not configurable.
    const advancementScopes = () =>
        parseAdvancementScopes(modelSettings()?.[ADVANCEMENT_RULES_SETTING]).join(", ");
    const [scopesDraft, setScopesDraft] = createSignal<string | null>(null);
    async function saveAdvancementScopes() {
        const draft = scopesDraft();
        if (draft === null) return;
        const scopes = draft.split(",").map((s) => s.trim()).filter(Boolean);
        try {
            await props.api.accountSetSetting(
                ADVANCEMENT_RULES_SETTING,
                serializeAdvancementScopes(scopes),
            );
            setScopesDraft(null);
            refetchModelSettings();
            setStatus(scopes.length ? "auto-keep scopes saved" : "auto-keep off — everything holds for review");
        } catch (e) {
            setStatus(`could not update auto-keep — ${e instanceof Error ? e.message : String(e)}`);
        }
    }

    // --- attention (ATTN-2, ADR 0082 §3): when a chat should ask for you --------------
    // One row per signal, rendered from the shared schema; each change writes the
    // whole (shallow) rules doc back to the account-settings KV the server's
    // task-queue projection evaluates.
    const attention = () => parseAttentionRules(modelSettings()?.[ATTENTION_RULES_SETTING]);
    async function setAttention(signal: AttentionSignal, level: AttentionLevel) {
        const next = { ...attention(), [signal]: level };
        try {
            await props.api.accountSetSetting(ATTENTION_RULES_SETTING, serializeAttentionRules(next));
            refetchModelSettings();
        } catch (e) {
            setStatus(`could not update attention — ${e instanceof Error ? e.message : String(e)}`);
        }
    }

    return (
        <div class="modal-overlay" onClick={() => props.onClose()}>
            <div
                class="modal account-panel"
                data-account-panel
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => e.key === "Escape" && props.onClose()}
            >
                <div class="modal-head">
                    <h3>Your account</h3>
                    <button type="button" onClick={() => props.onClose()}>
                        close
                    </button>
                </div>

                {/* This one-click flow starts a callback listener on the local machine, so it
                    is only offered by runtimes that actually host that helper. */}
                <Show when={props.codexLoginAvailable ?? true}>
                    <section class="admin-section" data-codex-oauth>
                        <h4>OpenAI sign-in (codex)</h4>
                        <p class="muted">
                            Sign in with your OpenAI account so the agent can run on the codex
                            endpoint — the default. No API key needed.
                        </p>
                        <div class="member-row" data-codex-status={codex()?.linked ? (codex()?.expired ? "expired" : "linked") : "none"}>
                            <span>
                                {codex()?.linked
                                    ? codex()?.expired
                                        ? `signed in — expired${codexExpiry() ? ` (${codexExpiry()})` : ""}`
                                        : `signed in${codexExpiry() ? ` — valid to ${codexExpiry()}` : ""}`
                                    : "not signed in"}
                            </span>
                            <Show when={codex()?.linked && !codex()?.expired}>
                                <span class="badge">linked</span>
                            </Show>
                            <button type="button" class="tree-action" data-codex-signin onClick={linkCodex}>
                                {codex()?.linked ? "re-sign in" : "Sign in with OpenAI"}
                            </button>
                        </div>
                        <Show when={authUrl()}>
                            <p class="muted">
                                If the browser didn't open,{" "}
                                <a href={authUrl()} target="_blank" rel="noopener noreferrer">open the sign-in page</a>.
                                {" "}When done,{" "}
                                <button type="button" class="link-btn" data-codex-refresh onClick={refresh}>
                                    refresh
                                </button>{" "}
                                to confirm.
                            </p>
                        </Show>
                    </section>
                </Show>

                {/* Linked LLM accounts */}
                <section class="admin-section">
                    <h4>Linked AI accounts</h4>
                    <p class="muted">
                        Link a provider so your agents use your account. The token is sealed —
                        it is never shown again.
                    </p>
                    <ul class="member-list">
                        <For
                            each={credentials()}
                            fallback={<li class="muted">No accounts linked yet.</li>}
                        >
                            {(c) => (
                                <li class="member-row" data-linked={c.provider}>
                                    <span class="member-id">{c.provider}</span>
                                    <span class="badge">linked</span>
                                    <button
                                        type="button"
                                        class="tree-action"
                                        onClick={() => unlink(c.provider)}
                                    >
                                        unlink
                                    </button>
                                </li>
                            )}
                        </For>
                    </ul>
                    <div class="admin-invite">
                        <select value={provider()} onChange={(e) => setProvider(e.currentTarget.value)}>
                            <For each={PROVIDERS}>
                                {(p) => (
                                    <option value={p}>
                                        {p}
                                        {isLinked(p) ? " (linked)" : ""}
                                    </option>
                                )}
                            </For>
                        </select>
                        <input
                            data-account-token
                            type="password"
                            value={token()}
                            onInput={(e) => setToken(e.currentTarget.value)}
                            placeholder="paste API key / token"
                        />
                        <button type="button" class="tree-action" onClick={link}>
                            link
                        </button>
                    </div>
                </section>

                {/* Which models show in the composer picker (LLM-1) */}
                <section class="admin-section" data-model-settings>
                    <h4>Models in the picker</h4>
                    <Show
                        when={linkedAccounts().length}
                        fallback={<p class="muted">Link an account above to choose its models.</p>}
                    >
                        <p class="muted">
                            Choose which of your linked accounts' models appear in the per-chat
                            picker. Unchecked models stay available — they're just hidden from the
                            dropdown.
                        </p>
                        <ul class="member-list model-checklist">
                            <For each={allModels()}>
                                {(m) => {
                                    const key = modelKey(m);
                                    return (
                                        <li class="model-check-row" data-model-toggle={key}>
                                            <label class="admin-check model-check">
                                                <input
                                                    type="checkbox"
                                                    checked={effectiveEnabled().has(key)}
                                                    onChange={(e) => void toggleModel(key, e.currentTarget.checked)}
                                                />
                                                <span>{m.label}</span>
                                            </label>
                                            <Show when={!m.primary}>
                                                <span class="badge" title="Available through this account, beyond its native model set">extra</span>
                                            </Show>
                                        </li>
                                    );
                                }}
                            </For>
                        </ul>
                    </Show>
                </section>

                {/* Attention (ATTN-2): when a chat should ask for you */}
                <section class="admin-section" data-attention-settings>
                    <h4>Attention</h4>
                    <p class="muted">
                        When a chat should ask for you. <strong>Task bar</strong> queues it
                        up top; <strong>chat dot</strong> only marks the chat in the tree;
                        <strong> quiet</strong> keeps it in the transcript.
                    </p>
                    <ul class="member-list">
                        <For each={ATTENTION_SIGNALS}>
                            {(m) => (
                                <li class="member-row" data-attention-signal={m.signal}>
                                    <span class="member-id" title={m.hint}>
                                        {m.label}
                                    </span>
                                    <select
                                        value={attention()[m.signal]}
                                        onChange={(e) =>
                                            void setAttention(
                                                m.signal,
                                                e.currentTarget.value as AttentionLevel,
                                            )
                                        }
                                    >
                                        <option value="queue">task bar</option>
                                        <option value="badge">chat dot only</option>
                                        <option value="mute">quiet</option>
                                    </select>
                                </li>
                            )}
                        </For>
                    </ul>
                </section>

                {/* Advancement (ATTN-3): what auto-keeps without review */}
                <section class="admin-section" data-advancement-settings>
                    <h4>Auto-keep</h4>
                    <p class="muted">
                        Keep a turn's work into the shared copy automatically when it{" "}
                        <em>only</em> touches these paths (comma-separated:{" "}
                        <code>docs/**</code>, <code>*.md</code>, or an exact path). Empty =
                        everything waits for your review. A change to the assistant's
                        permissions, or a turn that read someone else's content, always
                        waits — no scope can override that.
                    </p>
                    <div class="admin-invite">
                        <input
                            data-advancement-scopes
                            placeholder="e.g. docs/**, *.md"
                            value={scopesDraft() ?? advancementScopes()}
                            onInput={(e) => setScopesDraft(e.currentTarget.value)}
                            onKeyDown={(e) => e.key === "Enter" && void saveAdvancementScopes()}
                        />
                        <button
                            type="button"
                            class="tree-action"
                            disabled={scopesDraft() === null}
                            onClick={() => void saveAdvancementScopes()}
                        >
                            save
                        </button>
                    </div>
                </section>

                {/* Trusted devices */}
                <section class="admin-section">
                    <h4>Trusted devices</h4>
                    <ul class="member-list">
                        <For
                            each={devices()}
                            fallback={<li class="muted">No devices enrolled yet.</li>}
                        >
                            {(d) => (
                                <li class="member-row" data-device={d.id}>
                                    <span class="member-id">{d.label || d.id}</span>
                                    <span class="member-status">{d.status}</span>
                                    <Show when={d.status === "active"}>
                                        <button
                                            type="button"
                                            class="tree-action"
                                            onClick={() => revoke(d.id)}
                                        >
                                            revoke
                                        </button>
                                    </Show>
                                </li>
                            )}
                        </For>
                    </ul>
                </section>

                <p class="status" data-account-status>
                    {status()}
                </p>
            </div>
        </div>
    );
}

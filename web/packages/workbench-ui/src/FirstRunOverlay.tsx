/**
 * **First-run credential flow** (ADR 0075 §6 / Phase 0): the *separate, minimal,
 * static* welcome shown before anything else when the account has no LLM
 * credential. It exists because nothing agent-driven — not even the whip
 * onboarding tracker — can run before a credential exists, so this one step
 * cannot itself be agent-driven.
 *
 * Deliberately self-contained, not the full {@link AccountPanel}: welcome →
 * connect a model → get out of the way. It links a credential directly over
 * `/account/*`, then calls `onConnected` so the host re-checks and dismisses.
 * A quiet "I'll do this later" escape hatch keeps it from trapping anyone if
 * detection is ever wrong.
 *
 * A thin renderer (INV-5): the token is write-only (sealed server-side, SEC-4);
 * it is never read back.
 */

import { createSignal, Show, type JSX } from "solid-js";

/** The slice of the control-plane API this flow needs. */
export interface FirstRunApi {
    codexLoginStart(): Promise<{ url: string }>;
    accountLinkCredential(provider: string, token: string): Promise<void>;
}

/** The providers a first-run user can paste a key for (mirrors AccountPanel). */
const KEY_PROVIDERS: readonly { id: string; label: string }[] = [
    { id: "anthropic", label: "Anthropic (Claude)" },
    { id: "openai", label: "OpenAI" },
];

export function FirstRunOverlay(props: {
    api: FirstRunApi;
    /** Product name to greet with (e.g. "GaugeBench"). */
    productName: string;
    /** A credential was just linked — the host refetches status, which dismisses us. */
    onConnected: () => void;
    /** "I'll do this later" — dismiss for the session without connecting. */
    onDismiss: () => void;
}): JSX.Element {
    const [provider, setProvider] = createSignal(KEY_PROVIDERS[0].id);
    const [token, setToken] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    const [status, setStatus] = createSignal("");
    const [authUrl, setAuthUrl] = createSignal("");

    const linkKey = async (e: Event) => {
        e.preventDefault();
        if (!token().trim() || busy()) return;
        setBusy(true);
        setStatus("connecting…");
        try {
            await props.api.accountLinkCredential(provider(), token().trim());
            setToken("");
            setStatus("connected");
            props.onConnected();
        } catch (err) {
            setStatus(`couldn't connect — ${String(err)}`);
        } finally {
            setBusy(false);
        }
    };

    const linkCodex = async () => {
        if (busy()) return;
        setBusy(true);
        setStatus("starting OpenAI sign-in…");
        setAuthUrl("");
        try {
            const { url } = await props.api.codexLoginStart();
            setAuthUrl(url);
            window.open(url, "_blank", "noopener,noreferrer");
            setStatus("finish signing in in the new tab, then choose Continue");
        } catch (err) {
            setStatus(`couldn't start sign-in — ${String(err)}`);
        } finally {
            setBusy(false);
        }
    };

    return (
        <div class="firstrun-scrim" data-firstrun role="dialog" aria-modal="true" aria-label="connect a model to get started">
            <div class="firstrun-card">
                <h1 class="firstrun-title">Welcome to {props.productName}</h1>
                <p class="firstrun-lede">
                    Connect a model so agents can run. You can add more or change this
                    later in account settings.
                </p>

                <form class="firstrun-key" onSubmit={linkKey}>
                    <label class="firstrun-field">
                        <span class="firstrun-label">Provider</span>
                        <select
                            class="firstrun-select"
                            data-firstrun-provider
                            value={provider()}
                            onChange={(e) => setProvider(e.currentTarget.value)}
                            disabled={busy()}
                        >
                            {KEY_PROVIDERS.map((p) => (
                                <option value={p.id}>{p.label}</option>
                            ))}
                        </select>
                    </label>
                    <label class="firstrun-field">
                        <span class="firstrun-label">API key</span>
                        <input
                            class="firstrun-input"
                            data-firstrun-token
                            type="password"
                            autocomplete="off"
                            placeholder="paste your API key"
                            value={token()}
                            onInput={(e) => setToken(e.currentTarget.value)}
                            disabled={busy()}
                        />
                    </label>
                    <button
                        class="firstrun-connect"
                        data-firstrun-connect
                        type="submit"
                        disabled={busy() || !token().trim()}
                    >
                        Connect
                    </button>
                </form>

                <div class="firstrun-or">or</div>

                <button
                    class="firstrun-codex"
                    data-firstrun-codex
                    type="button"
                    onClick={linkCodex}
                    disabled={busy()}
                >
                    Sign in with OpenAI
                </button>
                <Show when={authUrl()}>
                    <div class="firstrun-codex-follow">
                        <a href={authUrl()} target="_blank" rel="noopener noreferrer">
                            Open the sign-in page
                        </a>
                        <button
                            class="firstrun-codex-continue"
                            data-firstrun-codex-continue
                            type="button"
                            onClick={() => props.onConnected()}
                        >
                            Continue
                        </button>
                    </div>
                </Show>

                <Show when={status()}>
                    <p class="firstrun-status" data-firstrun-status>{status()}</p>
                </Show>

                <button
                    class="firstrun-later"
                    data-firstrun-later
                    type="button"
                    onClick={() => props.onDismiss()}
                >
                    I'll do this later
                </button>
            </div>
        </div>
    );
}

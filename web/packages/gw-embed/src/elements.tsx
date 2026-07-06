/**
 * The embed custom elements (EMBED-2): `<gw-session>` + `<gw-chat>` / `<gw-viewer>`
 * / `<gw-files>`. They let a consultant drop the workbench's panels into their own
 * page in any stack (ADR 0051 §3) — the delivery side of the context-portable
 * panels EMBED-1 built.
 *
 * Architecture:
 *  - `<gw-session>` holds a **scoped** control plane + a {@link createRemoteSession}
 *    bound to one engagement, exposed as the element's `.session` property. It is a
 *    logical provider (light DOM) — its panel children find it by DOM ancestry.
 *  - each panel element finds its Session (the ancestor `<gw-session>`'s `.session`,
 *    or its own `.session` set directly — the JS-handle escape hatch for detached
 *    layouts), attaches a **shadow root** for style isolation, adopts the workbench
 *    stylesheet, and renders the existing Solid panel against the Session.
 *
 * Solid context cannot cross separate render roots, so each panel re-provides the
 * shared Session into its own tree via {@link SessionProvider} — the panel code is
 * unchanged (`useSession()` works exactly as on the desktop).
 */
import { createResource, createSignal, For, Show, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { type EngagementId } from "@gaugewright/control-plane-client";
import { EmbedControlPlane, controlPlaneBase } from "./embed-control-plane";
import { createRemoteSession } from "./remote-session";
import { ContentViewer } from "@gaugewright/workbench-ui/ContentViewer";
import { SessionProvider, type Session } from "@gaugewright/workbench-ui/session-context";
import { TranscriptView } from "@gaugewright/workbench-ui/TranscriptView";
import { Workspace } from "@gaugewright/workbench-ui/Workspace";
import appCss from "@gaugewright/workbench-ui/styles.css?inline";

/**
 * The shadow-root theme bridge. The workbench palette is defined on `:root`
 * (`styles.css`), which does **not** apply inside a shadow tree — so we re-declare
 * the workbench's internal palette vars on `:host` (custom properties *do* inherit
 * across the shadow boundary), each sourced from a consultant-facing `--gw-*`
 * token with the workbench default as fallback. A consultant themes the embed by
 * setting any `--gw-*` on `<gw-session>` (or any ancestor); it cascades into every
 * panel's shadow root. Injected before `styles.css` so its `var(--bg)` etc. resolve.
 */
const EMBED_THEME_CSS = `
:host {
  --bg: var(--gw-bg, #0f1115);
  --panel: var(--gw-panel, #161922);
  --edge: var(--gw-edge, #262b36);
  --ink: var(--gw-ink, #d8dee9);
  --muted: var(--gw-muted, #7d869c);
  --accent: var(--gw-accent, #6aa3ff);
  --warn: var(--gw-warn, #e0a35a);
  --bad: var(--gw-bad, #e06a6a);
  --ui: var(--gw-font, "Figtree", ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif);
  --mono: var(--gw-mono, "Inconsolata", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace);
  display: block;
  height: 100%;
  background: var(--bg);
  color: var(--ink);
  font-family: var(--ui);
}
`;

/** `<gw-session cp="…" engagement="…">`: builds + owns the scoped remote Session. */
export class GwSessionElement extends HTMLElement {
    /** The Session its panel children render against (also settable directly). */
    session?: Session;
    private _teardown?: () => void;

    connectedCallback() {
        if (this.session || this._teardown) return; // already built, or injected via handle
        const engagement = this.getAttribute("engagement");
        const key = this.getAttribute("key");
        if (engagement) {
            // Direct binding: the consultant (or the in-desktop preview) supplies the
            // control-plane base + the engagement id.
            const base = this.getAttribute("cp") ?? this.getAttribute("base") ?? controlPlaneBase();
            this.bindSession(base, engagement, key, false);
        } else {
            // Snippet bootstrap (EMBED-3): `<gw-session host="…" key="pk_…">` — the only thing a
            // consultant drops on their page. Activate a hosted visitor session and bind to the
            // returned { cp, engagement }.
            const host = this.getAttribute("host");
            if (host && key) void this.bootstrap(host, key);
        }
    }

    /** Activate a hosted visitor session via the host's `POST /sessions` bootstrap (EMBED-3), then
     *  bind to the returned per-visitor control plane. Fail-closed: a non-admitted key/origin or an
     *  unreachable host leaves the panel unbound (no broken pane, `INV-20`). */
    private async bootstrap(host: string, key: string) {
        try {
            const base = /^https?:\/\//.test(host) ? host.replace(/\/$/, "") : `https://${host}`;
            const res = await fetch(`${base}/sessions`, {
                method: "POST",
                headers: { "x-gw-publishable-key": key },
            });
            if (!res.ok) return;
            const { cp, engagement } = (await res.json()) as { cp?: string; engagement?: string };
            if (!cp || !engagement) return;
            if (this.session || this._teardown || !this.isConnected) return; // disconnected meanwhile
            this.bindSession(cp, engagement, key, true);
        } catch {
            /* host unreachable → fail-closed (no session) */
        }
    }

    /** Build the scoped control plane + remote session and bind the panel children. */
    private bindSession(base: string, engagement: string, key: string | null, publicEmbed: boolean) {
        const api = new EmbedControlPlane(base);
        if (key) api.setPublishableKey(key);
        // Authenticated mode: carry the audience session token (EMBED-4) so the embed
        // calls (e.g. my-chats) are scoped to this end-user.
        const token = this.getAttribute("token");
        if (token) api.setBearer(token);
        const { session, dispose } = createRemoteSession({
            api,
            engagementId: engagement as EngagementId,
            publicEmbed,
        });
        this.session = session;
        this._teardown = dispose;
        // Nudge any panel children that connected before us (DOM normally connects
        // us first, but be order-independent).
        this.querySelectorAll<GwPanelElement>("gw-chat, gw-viewer, gw-files").forEach((p) => p.bind?.());
    }

    disconnectedCallback() {
        this._teardown?.();
        this._teardown = undefined;
        this.session = undefined;
    }
}

/** Base for a panel element: find the Session, isolate in a shadow root, render. */
abstract class GwPanelElement extends HTMLElement {
    /** The JS-handle escape hatch: set this to mount detached from a `<gw-session>`. */
    session?: Session;
    private _disposeRender?: () => void;

    /** The Solid view this element renders against the resolved Session. */
    protected abstract view(session: Session): JSX.Element;

    connectedCallback() {
        this.bind();
    }

    /** Resolve the Session (own handle → ancestor `<gw-session>`) and render once.
     *  A no-op if already rendered or no Session is reachable yet (the ancestor
     *  `<gw-session>` re-drives this once it is built). Public so a host can call it
     *  after setting `.session` directly. */
    bind() {
        if (this._disposeRender) return;
        const session = this.session ?? this.closest<GwSessionElement>("gw-session")?.session;
        if (!session) return;
        const root = this.shadowRoot ?? this.attachShadow({ mode: "open" });
        // Theme bridge first (defines the palette on :host), then the workbench
        // stylesheet (consumes it via var(--bg)… — its own :root block is inert here).
        const theme = document.createElement("style");
        theme.textContent = EMBED_THEME_CSS;
        root.appendChild(theme);
        const style = document.createElement("style");
        style.textContent = appCss;
        root.appendChild(style);
        this._disposeRender = render(() => <SessionProvider value={session}>{this.view(session)}</SessionProvider>, root);
    }

    disconnectedCallback() {
        this._disposeRender?.();
        this._disposeRender = undefined;
    }
}

/** The embedded chat: the shared transcript renderer + a minimal composer over the
 *  Session's `transcript`/`send`/`selectFile`. The desktop's queue/steer/gate are
 *  owner affordances and intentionally absent here. */
function EmbeddedChat(props: { session: Session }) {
    const [draft, setDraft] = createSignal("");
    const submit = () => {
        const text = draft().trim();
        if (!text) return;
        setDraft("");
        props.session.send(text);
    };
    return (
        <div class="embed-chat" data-embed-chat>
            <div class="transcript" data-embed-transcript>
                <TranscriptView lines={props.session.transcript().lines} onOpen={props.session.selectFile} />
            </div>
            <form
                class="composer embed-composer"
                onSubmit={(e) => {
                    e.preventDefault();
                    submit();
                }}
            >
                <input
                    data-embed-composer
                    value={draft()}
                    onInput={(e) => setDraft(e.currentTarget.value)}
                    placeholder="Type a message…"
                    aria-label="Message"
                />
                <button type="submit" data-embed-send>Send</button>
            </form>
        </div>
    );
}

export class GwChatElement extends GwPanelElement {
    protected view(session: Session): JSX.Element {
        return <EmbeddedChat session={session} />;
    }
}

export class GwViewerElement extends GwPanelElement {
    protected view(): JSX.Element {
        return <ContentViewer />;
    }
}

export class GwFilesElement extends GwPanelElement {
    protected view(): JSX.Element {
        return <Workspace />;
    }
}

/** The signed-in end-user's saved chats (EMBED-5): a fail-closed, audience-scoped
 *  listing over `GET /embed/my-chats` (the bearer set on `<gw-session token=…>`). */
function MyChatsList(props: { session: Session }) {
    const [chats] = createResource(() => props.session.api.embedMyChats());
    return (
        <div class="embed-mychats" data-embed-mychats>
            <Show when={chats()} fallback={<div class="status">loading…</div>}>
                <Show
                    when={chats()!.length}
                    fallback={<div class="status" data-mychats-empty>No saved chats yet.</div>}
                >
                    <For each={chats()}>
                        {(c) => (
                            <div class="my-chat" data-my-chat>
                                {c.title}
                            </div>
                        )}
                    </For>
                </Show>
            </Show>
        </div>
    );
}

export class GwChatsElement extends GwPanelElement {
    protected view(session: Session): JSX.Element {
        return <MyChatsList session={session} />;
    }
}

/** Register the embed custom elements (idempotent). */
export function registerEmbedElements() {
    if (typeof customElements === "undefined" || customElements.get("gw-session")) return;
    customElements.define("gw-session", GwSessionElement);
    customElements.define("gw-chat", GwChatElement);
    customElements.define("gw-viewer", GwViewerElement);
    customElements.define("gw-files", GwFilesElement);
    customElements.define("gw-chats", GwChatsElement);
}

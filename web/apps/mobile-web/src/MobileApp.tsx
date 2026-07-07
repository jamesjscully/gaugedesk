/**
 * The mobile **flow harness** (`mobile-client.md`; MOB-029): the one place that
 * composes the committed D-MOBILE islands into the device's actual user journey —
 * **pair → navigate → send (offline + online)** — over the *real* control plane.
 * It is the surface the mobile e2e (`web/e2e/features/mobile.feature`) drives.
 *
 * It owns no truth of its own: every decision is a committed pure reducer.
 *   - Pairing is the {@link PairingFlow} island over the {@link reducePairing}
 *     machine, fed by the real `POST /pairing-requests` → owner `POST
 *     /boundaries/:id/accept` → `GET /pairing-status/:id` handshake (MOB-027). In
 *     the single-authority loopback dev/e2e shell the owner *is* this process, so
 *     the harness drives the accept itself — the same boundary lifecycle the
 *     desktop owner would, no fake.
 *   - Once paired it mints the held {@link BridgeGrant} the boundary pinned and
 *     feeds the {@link ConnectionState} machine (MOB-018); the relay toggle moves
 *     the device between `active` and `offline` exactly as a dropped relay would.
 *   - Navigation is the {@link Carousel} island (MOB-014) over the pure carousel
 *     reducer (MOB-009); the chat stop is the {@link MobileChat} composer (MOB-020)
 *     whose send gate and the {@link ConnectionBanner} (MOB-028) read one
 *     connection fold, so a degraded link refuses a standing command in both places.
 *
 * It is reached at `?mobile=1` (or `#mobile`) so it composes with — rather than
 * replaces — the desktop workbench, and so the e2e can address it directly without
 * a native toolchain (the projection-client web build, ADR 0020 / `MOBILE-PROJECTION-1`).
 */

import { createEffect, createResource, createSignal, For, onCleanup, type JSX, Show } from "solid-js";
import {
    bridgeGrantId,
    clientRequestId,
    deviceId,
    type BridgeGrant,
    type DeviceIdentity,
    type EngagementId,
    type LocalState,
    publicKey,
} from "@gaugewright/control-plane-client";
import {
    applySelection,
    applySend,
    Carousel,
    ChatApprovalCard,
    type ChatApprovalState,
    ConnectionBanner,
    type ConnectionState,
    type ComposerState,
    emptyComposer,
    emptyTranscript,
    FacetBrowser,
    fromSnapshot,
    initialCarousel,
    initialConnection,
    initialPairing,
    MobileChat,
    PairingFlow,
    parsePairingStatus,
    parseTicket,
    type PairingState,
    QueueSheet,
    reduceCarousel,
    reduceChatApproval,
    reduceConnection,
    reducePairing,
    reduceTranscript,
    tapGesture,
    TopBar,
    type Transcript,
    type CarouselState,
    type PaneKind,
    type TurnPhase,
    TranscriptView,
} from "@gaugewright/workbench-ui";
import { MobileControlPlane, controlPlaneBase } from "./mobile-control-plane";

const BASE = controlPlaneBase();
const api = new MobileControlPlane(BASE);

/** Whether the harness is addressed — `?mobile=1` or `#mobile`. The desktop app
 *  delegates here on that flag so the two shells share one entry point. */
export function isMobileHarness(): boolean {
    if (typeof window === "undefined") return false;
    const p = new URLSearchParams(window.location.search);
    return p.get("mobile") === "1" || window.location.hash.replace(/^#/, "") === "mobile";
}

/** This device's stable identity for the harness run. A real device key is a
 *  native secure-storage concern (MOB-025, needs-infra); the web harness presents
 *  a deterministic public handle so the pairing round-trips and the held grant
 *  binds back to it (`bridgeGrantBindsDevice`). */
const DEVICE: DeviceIdentity = {
    id: deviceId("device:web-harness"),
    deviceKey: publicKey("devkey-web-harness"),
};

export function MobileApp(): JSX.Element {
    const [pairing, setPairing] = createSignal<PairingState>(initialPairing);
    const [connection, setConnection] = createSignal<ConnectionState>(
        initialConnection({ identity: DEVICE, grants: [] }, Date.now()),
    );
    const [carousel, setCarousel] = createSignal<CarouselState>(initialCarousel);
    const [composer, setComposer] = createSignal<ComposerState>(emptyComposer);
    const [turn, setTurn] = createSignal<TurnPhase>("idle");
    // An inline merge/review approval card threaded in the chat transcript (MOB-031),
    // or `null` when the turn surfaced nothing to approve. It folds the same review
    // lifecycle the desktop ReviewShelf drives, inline at the chat stop.
    const [approval, setApproval] = createSignal<ChatApprovalState | null>(null);
    const [engagement, setEngagement] = createSignal<EngagementId | null>(null);
    const [environment, setEnvironment] = createSignal<string | null>(null);
    const [log, setLog] = createSignal<string[]>([]);
    const append = (line: string) => setLog((l) => [...l, line]);

    // The transcript of the selected chat (MOB-F2): a fold of the durable snapshot
    // plus the live SSE, exactly the desktop chat pane's source — so the mobile
    // Chat stop shows what the agent actually did, not just the consent surface.
    const [transcript, setTranscript] = createSignal<Transcript>(emptyTranscript);
    let unsubscribe: (() => void) | null = null;
    onCleanup(() => unsubscribe?.());

    // The browse projection (MOB-F1): the host's chats, so the Browse pane lists real,
    // selectable chats instead of a single environment label. `wsKey` bumps to
    // refetch the nav (the shared FacetBrowser) after a chat is created or a turn
    // settles, so the device sees the same workspace the desktop does.
    const [wsKey, setWsKey] = createSignal(0);

    // The selected chat's worktree files (MOB-F: the Files pane mirrors the host's
    // worktree for the open chat, not a stub). `filesKey` bumps after a turn so a
    // change to the worktree is reflected. Only fetched once a chat is open.
    const [filesKey, setFilesKey] = createSignal(0);
    const [files] = createResource(
        () => {
            const id = engagement();
            return id ? ([id, filesKey()] as const) : undefined;
        },
        ([id]) => api.getTree(id),
    );

    // The human task queue (the `Next ③` header affordance, `mobile-client.md`):
    // the same `GET /tasks` projection the desktop TaskBar reads — review-needed
    // work, current-first — so the phone surfaces tasks at all (it previously had
    // no way to see them). Fetched only once paired; refetched on `wsKey` so a turn
    // settling, a chat created, or a desktop-side change re-derives the queue.
    const [tasks] = createResource(
        () => (pairing().step === "paired" ? wsKey() : undefined),
        () => api.getTasks(),
    );
    const queueDepth = () => (tasks() ?? []).length;
    // Whether the pull-down full-queue sheet is open (local view state, not truth).
    const [queueOpen, setQueueOpen] = createSignal(false);

    // Jump to a task: open its chat and dismiss the sheet — the badge tap and a sheet
    // row resolve to the same (selection, pane), the spec's "tap jumps to the task".
    function jumpToTask(id: EngagementId) {
        setQueueOpen(false);
        void selectEngagement(id);
    }
    // The badge tap jumps to the *current* (first) navigable task; a no-op on an
    // empty queue. Onboarding `issue` tasks (ADR 0075) carry a whip work-item id,
    // not an engagement, so they are skipped here — only `review` tasks jump.
    function jumpToCurrentTask() {
        const first = (tasks() ?? []).find((t) => t.kind === "review");
        if (first) jumpToTask(first.id as EngagementId);
    }

    // Load (or switch to) a chat: subscribe its transcript and jump to the Chat
    // pane. Routed through here from both the nav list (MOB-F1) and the post-pair
    // auto-create, so the transcript always tracks the selected chat.
    async function selectEngagement(id: EngagementId) {
        if (engagement() === id) {
            setCarousel((c) => reduceCarousel(c, tapGesture("chat")));
            return;
        }
        unsubscribe?.();
        setEngagement(id);
        setTranscript(emptyTranscript);
        setApproval(null);
        // A chat is selected → un-grey the chat/files panes and land on Chat.
        setCarousel((c) => applySelection(c, { chatSelected: true, fileSelected: false }));
        setCarousel((c) => reduceCarousel(c, tapGesture("chat")));
        try {
            setTranscript(fromSnapshot(await api.getTranscript(id)));
        } catch (e) {
            append(`transcript error: ${String(e)}`);
        }
        unsubscribe = api.subscribe(id, (ev) => setTranscript((t) => reduceTranscript(t, ev)));
    }

    // ---- pairing handshake (real MOB-027 endpoints) -------------------------
    // The user submits a ticket → POST /pairing-requests opens + binds the
    // boundary → the owner (this loopback process) POSTs the accept → we poll
    // /pairing-status until the boundary is Active (`paired`). Every step folds a
    // real server fact through the pure pairing reducer; the island only paints it.
    async function runPairing(raw: string) {
        const ticket = parseTicket(raw);
        setPairing((s) => reducePairing(s, { kind: "ticket-entered", ticket }));
        if (ticket === null) return;
        try {
            const opened = await api.openPairing(ticket.device, ticket.bridgeGrant);
            setPairing((s) =>
                reducePairing(s, { kind: "request-accepted", pairingId: opened.pairingId }),
            );

            // The owner accepts the pairing boundary (single-authority loopback:
            // the owner is this process). This drives the boundary Active, exactly
            // as the desktop owner's accept would (MOB-027 ties pairing to accept).
            await api.acceptBoundary(opened.pairingId, "local-user");

            // Poll the pairing status until the reducer settles it (paired/failed).
            for (let i = 0; i < 20; i++) {
                const status = parsePairingStatus(
                    (await api.pairingStatus(opened.pairingId)) as Parameters<typeof parsePairingStatus>[0],
                );
                setPairing((s) => reducePairing(s, { kind: "status", status }));
                if (status.paired) {
                    completePairing(ticket.environment, opened.bridgeGrant);
                    return;
                }
                await new Promise((r) => setTimeout(r, 50));
            }
        } catch (e) {
            setPairing((s) => reducePairing(s, { kind: "error", reason: String(e) }));
        }
    }

    // Mint the held grant the boundary pinned and feed the connection machine: the
    // device now holds an active, device-bound grant for the paired environment.
    function completePairing(env: string, grant: string) {
        const held: BridgeGrant = {
            id: bridgeGrantId(grant),
            sourceAuthorityRootPubkey: publicKey("owner-root"),
            sourceAuthorityKeyId: "owner-key-0",
            targetEnvironment: env,
            targetRoute: "projection",
            deviceKey: DEVICE.deviceKey,
            governanceScope: env,
            expiry: Date.now() + 60 * 60 * 1000,
            active: true,
        };
        const local: LocalState = { identity: DEVICE, grants: [held] };
        setConnection((c) =>
            reduceConnection(
                reduceConnection(c, { kind: "grants-changed", grants: local.grants }),
                { kind: "address", environment: env },
            ),
        );
        setEnvironment(env);
        append(`paired → ${env}`);
        // Land on the Browse pane mirroring the host's real chats — don't auto-create a
        // placeholder chat or jump into one. The user picks a chat from the nav, or
        // starts one from the Chat tab (`startNewChat` below, the carousel's
        // selection-gated "new chat" affordance).
        setWsKey((k) => k + 1);
    }

    // Start a fresh chat on demand (the Chat tab's "new chat" affordance when none
    // is open) and select it, so "send" has a real control-plane surface. This is
    // the SAME "just chat" quick-start the desktop uses (`POST /chats` → a **work**
    // chat on the hidden Personal placement) — not an edit chat — so a chat started
    // on the device is the normal kind and shows up identically on the desktop.
    async function startNewChat() {
        try {
            const eng = await api.createEngagement();
            await selectEngagement(eng.id);
            setWsKey((k) => k + 1); // refetch the nav so the new chat lists
        } catch (e) {
            append(`new chat error: ${String(e)}`);
        }
    }

    // ---- relay toggle (offline ⇄ online) ------------------------------------
    // Flip the relay reachability the connection machine reduces over. Offline is
    // the dropped-relay state: cached views still read, but `canCommand` is false,
    // so the banner shows and the composer's send gate refuses — one fold, two
    // surfaces (MOB-028). Online restores `active` and re-enables send.
    function setRelay(reachable: boolean) {
        setConnection((c) => reduceConnection(c, { kind: "relay", reachable }));
    }

    // ---- send (the one standing command the client may issue) ---------------
    async function send(text: string) {
        const id = engagement();
        if (id === null) return;
        const rid = clientRequestId(`req-${Date.now()}`);
        setComposer((s) => applySend(s, rid, turn()));
        setTurn("running");
        append(`send: ${text}`);
        try {
            await api.runTask(id, text);
            append("turn complete");
            setWsKey((k) => k + 1); // a first turn auto-titles the chat — refresh nav
            setFilesKey((k) => k + 1); // the turn may have changed the worktree
        } catch (e) {
            append(`turn error: ${String(e)}`);
        } finally {
            setTurn("idle");
            // Reconcile the optimistic entry now the environment answered.
            setComposer((s) => ({ draft: s.draft, pending: s.pending.filter((p) => p !== rid) }));
        }
    }

    async function stop() {
        const id = engagement();
        if (id === null) return;
        try {
            await api.stopTurn(id);
        } catch {
            /* best-effort abort */
        }
        setTurn("idle");
    }

    // ---- inline approval (consent / release on the chat-stop card) ----------
    // Consent and release are standing review commands (INV-5/INV-7): we record the
    // optimistic step under a fresh ClientRequestId through the pure reducer; the
    // host issues the real `review` command and the answering projection folds back
    // through a `review` event. (In the loopback harness the card stays local until
    // a turn surfaces a proposal; the affordance and its gating are what MOB-031 adds.)
    function consent(proposalId: string) {
        append(`consent: ${proposalId}`);
        const rid = clientRequestId(`req-${Date.now()}`);
        setApproval((a) => (a ? reduceChatApproval(a, { kind: "consent", requestId: rid }) : a));
    }
    function release(proposalId: string) {
        append(`release: ${proposalId}`);
        const rid = clientRequestId(`req-${Date.now()}`);
        setApproval((a) => (a ? reduceChatApproval(a, { kind: "release", requestId: rid }) : a));
    }

    // Keep the connection clock fresh so an expiring grant is re-derived; harmless
    // for the e2e (the harness grant is hours out), but keeps the machine honest.
    createEffect(() => {
        const t = setInterval(() => setConnection((c) => reduceConnection(c, { kind: "tick", now: Date.now() })), 5_000);
        onCleanup(() => clearInterval(t));
    });

    // Mirror the node *live* over the workspace event stream (the sibling of the
    // per-chat SSE): the server pushes a "changed" ping whenever the library mutates
    // — a chat created on the desktop appears here at once, no polling. Subscribed
    // once paired; torn down on cleanup.
    createEffect(() => {
        if (pairing().step !== "paired") return;
        const stop = api.subscribeWorkspace(() => setWsKey((k) => k + 1));
        onCleanup(stop);
    });

    const status = () => connection().status;

    // The four carousel panes. Chat is the committed MobileChat composer wired to a
    // real engagement; the others are light, real projections of the paired state —
    // the carousel only needs reachable panes to demonstrate navigation (MOB-014).
    const panes = (): Record<PaneKind, JSX.Element> => ({
        nav: (
            <div class="mobile-nav" data-pane="nav">
                {/* A small connection header (the e2e reads `data-paired-environment`). */}
                <div class="mobile-nav-head status" data-paired-environment>
                    paired: {environment() ?? "—"}
                </div>
                {/* The REAL workspace nav (MOB-F1): the same FacetBrowser the desktop
                    renders — Chats | Projects | Library, real chats, create/search —
                    so the device mirrors the host exactly and a chat opened or created
                    here is the same one the desktop shows. */}
                <FacetBrowser
                    api={api}
                    selected={engagement()}
                    onSelect={(id) => void selectEngagement(id)}
                    onOpenArchetypeSettings={() => undefined}
                    onOpenEngagement={() => undefined}
                    onOpenModelAccess={() => undefined}
                    onOpenProjectHome={() => undefined}
                    onOpenForkTree={() => undefined}
                    onChatDeleted={(id) => {
                        if (engagement() === id) {
                            unsubscribe?.();
                            setEngagement(null);
                            setTranscript(emptyTranscript);
                            setCarousel((c) =>
                                applySelection(c, { chatSelected: false, fileSelected: false }),
                            );
                        }
                    }}
                    onStatus={append}
                    refreshKey={wsKey()}
                />
            </div>
        ),
        chat: (
            <div class="mobile-chat" data-pane="chat">
                {/* The live transcript (MOB-F2): the same fold the desktop chat pane
                    renders, so the device sees what the agent did — not just the
                    consent surface below it. */}
                <div class="mobile-transcript" data-mobile-transcript>
                    <TranscriptView
                        lines={transcript().lines}
                        onOpen={() => undefined}
                        fallback={<div class="status">no activity yet</div>}
                    />
                </div>
                {/* An inline merge/review approval card threads into the transcript
                    when a turn surfaces a result needing approval (MOB-031). */}
                <Show when={approval()}>
                    {(a) => (
                        <ChatApprovalCard
                            state={a()}
                            connection={status()}
                            onConsent={consent}
                            onRelease={release}
                        />
                    )}
                </Show>
                <MobileChat
                    state={composer()}
                    phase={turn()}
                    connection={status()}
                    onState={setComposer}
                    onSend={send}
                    onStop={stop}
                />
            </div>
        ),
        files: (
            <div class="mobile-files" data-pane="files">
                {/* Mirror the host's worktree for the open chat (MOB-F): the real
                    file tree, not a stub. */}
                <Show
                    when={engagement()}
                    fallback={<div class="status">open a chat to see its files</div>}
                >
                    <ul class="mobile-file-list" data-mobile-file-list>
                        <For
                            each={files() ?? []}
                            fallback={<li class="status">no files in this chat yet</li>}
                        >
                            {(f) => (
                                <li class="mobile-file" classList={{ dir: f.isDir }} data-file={f.path}>
                                    {f.path}
                                    {f.isDir ? "/" : ""}
                                </li>
                            )}
                        </For>
                    </ul>
                </Show>
            </div>
        ),
        content: <div class="mobile-content" data-pane="content">no content selected</div>,
    });

    return (
        <div class="workbench mobile" data-mobile data-mobile-harness>
            <Show
                when={pairing().step === "paired"}
                fallback={
                    <div class="mobile-pairing-stage" data-mobile-stage="pairing">
                        <PairingFlow
                            state={pairing()}
                            onSubmitTicket={(raw) => void runPairing(raw)}
                            onRetry={() => setPairing(initialPairing)}
                            onDismiss={() => setPairing(initialPairing)}
                        />
                    </div>
                }
            >
                <div class="mobile-carousel-stage" data-mobile-stage="carousel">
                    {/* The top bar carries the context header, freshness dot, and the
                        `Next ③` task affordance (tap → current task, `⌄` → full queue
                        sheet). Its pane toggle is suppressed — the carousel island below
                        is the canonical toggle on the phone (and owns the "new chat"
                        smarts), so the bar would otherwise double it. */}
                    <TopBar
                        carousel={carousel()}
                        onState={setCarousel}
                        status={status()}
                        freshness={null}
                        chatTitle={null}
                        environment={environment()}
                        queueDepth={queueDepth()}
                        onJumpToTask={jumpToCurrentTask}
                        onOpenQueue={() => setQueueOpen(true)}
                        showToggle={false}
                    />
                    <Show when={queueOpen()}>
                        <QueueSheet
                            tasks={tasks() ?? []}
                            onJump={jumpToTask}
                            onClose={() => setQueueOpen(false)}
                        />
                    </Show>
                    <ConnectionBanner status={status()} />
                    {/* The relay toggle stands in for the platform's reachability
                        signal — the e2e flips it to drive offline ⇄ online. */}
                    <div class="mobile-relay" data-relay={status() === "active" ? "online" : "offline"}>
                        <button
                            type="button"
                            data-relay-online
                            onClick={() => setRelay(true)}
                        >
                            go online
                        </button>
                        <button
                            type="button"
                            data-relay-offline
                            onClick={() => setRelay(false)}
                        >
                            go offline
                        </button>
                    </div>
                    <Carousel
                        state={carousel()}
                        onState={setCarousel}
                        panes={panes()}
                        onNewChat={() => void startNewChat()}
                    />
                    <ul class="mobile-log" data-mobile-log>
                        <For each={log()}>{(line) => <li>{line}</li>}</For>
                    </ul>
                </div>
            </Show>
        </div>
    );
}

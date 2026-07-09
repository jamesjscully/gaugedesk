/**
 * The workbench shell (`navigation.md` B1): a four-panel layout under a top
 * human-task-queue bar. It is a thin renderer — it renders projections and
 * submits commands; it never owns a lifecycle transition (`INV-5`).
 *
 *   ┌ task bar: human task queue ──────────────────────────────────┐
 *   ├──────────┬───────────────┬───────────────────┬──────────────┤
 *   │ nav      │ chat lane     │ content viewer    │ WORKSPACE     │
 *   │ (facets) │ (two-tier B4) │ files / turn diff │ worktree files│
 *   └──────────┴───────────────┴───────────────────┴──────────────┘
 *
 * The content viewer's Diff is the **review surface for the merge lifecycle**:
 * the human admits (advance `main`) or rejects (isolate) the turn (D1).
 */

import { createEffect, createMemo, createResource, createSignal, For, type JSX, on, onCleanup, Show, untrack } from "solid-js";
import {
    bearer,
    clientRequestId,
    consumeCallbackToken,
    startSessionRefresh,
    type ArchetypeId,
    describeFailure,
    type EngagementId,
    type ProjectId,
    Rejected,
    scopeId,
    type MergeAction,
    type MergePhase,
    type MergeState,
    type ClientRequestId,
} from "@gaugewright/control-plane-client";
import { WorkbenchControlPlane, controlPlaneBase } from "./workbench-control-plane";
import {
    AgentSettings,
    type Attachment,
    buildOutgoing,
    Carousel,
    applySelection,
    classifyAttachment,
    changedUserFiles,
    chatIdFromSearch,
    ContentViewer,
    ContextPanel,
    deriveFreshness,
    displayChatTitle,
    emptyTranscript as empty,
    EngagementPane,
    ENABLED_MODELS_SETTING,
    fileFromSearch,
    fileToBase64,
    freshnessEventForMarker,
    FacetBrowser,
    forkSource,
    FreshnessBanner,
    fromSnapshot,
    type ImageRef,
    Icon,
    initialCarousel,
    initialFreshness,
    isPlaceholderTitle,
    loadTranscriptFilterPrefs as loadPrefs,
    modelAcceptsImages,
    modelKey,
    modelOptions,
    type ModelOption,
    ForkTreePanel,
    OpenSettingsMenu as SettingsMenu,
    ProjectHomePanel,
    ProjectModelAccessPanel,
    parseEnabledModels,
    readChatModel,
    readChatProvider,
    readChatThinking,
    readPolicyDiff,
    reduceFreshness,
    reduceCarousel,
    reduceTranscript as reduce,
    saveTranscriptFilterPrefs as savePrefs,
    searchWithChat,
    searchWithFile,
    SessionProvider,
    Shelf,
    FirstRunOverlay,
    TaskBar,
    tapGesture,
    thinkingLevelsFor,
    titleFromPrompt,
    type CarouselState,
    type ChatRunTone,
    type FreshnessState,
    type PaneKind,
    type Session,
    type Transcript,
    TranscriptFilterMenu,
    TranscriptView,
    Workspace,
    writeChatModelPin,
    writeChatThinking,
} from "@gaugewright/workbench-ui";
import { isMobileHarness, MobileApp } from "@gaugewright/mobile-web";

const api = new WorkbenchControlPlane(controlPlaneBase());
// OIDC login (ID-3): if we just returned from `/auth/callback`, capture the id-token
// from the URL fragment before the first request, then hand the bearer to the
// transport so gated `/admin/*` calls carry it. Signed-out / single-user local is the
// no-op default (no header sent).
consumeCallbackToken();
api.setBearer(bearer());
// Hosted Console (ADR 0077): keep the `.gaugewright.com` cookie session alive by pinging
// `/auth/refresh` on a timer under the id-token's ~1h life. No-op on the loopback desktop.
startSessionRefresh(controlPlaneBase());

/** A message the human typed while a turn was in flight. Queued messages stack on
 *  top of the composer and drain in order when each turn settles. The `id` is a
 *  stable client key so edits/reorders/removals don't churn the DOM. `images` are
 *  the message's native image blocks (text attachments are already folded into
 *  `text` by composeOutgoing). */
type QueuedMsg = { id: number; text: string; images: ImageRef[] };

/** True inside the Tauri desktop shell (v2 injects this), false in a plain
 *  browser / e2e build — gates native-only affordances like the folder picker. */
const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export function App() {
    // The mobile projection-client flow harness (MOB-029) is addressed directly at
    // `?mobile=1` / `#mobile`, composing the committed D-MOBILE islands (pairing,
    // carousel, composer, connection banner) over the real control plane. It is a
    // sibling entry to the desktop workbench, not a media-query variant of it.
    if (isMobileHarness()) return <MobileApp />;

    const [selected, setSelected] = createSignal<EngagementId | null>(null);
    const [status, setStatus] = createSignal("ready");
    // A pulse the in-chat "no model attached" action bumps to open the Account panel
    // (LLM-1): the transcript surfaces the refusal, this opens where you fix it.
    const [accountRequest, setAccountRequest] = createSignal(0);
    // FED-7: an OS-delivered `gaugewright://invite` deep link. The Tauri shell dispatches it as a
    // `gw-deep-link` DOM CustomEvent (plain event, not IPC), which we route into the Devices
    // consent flow. A browser build simply never receives one.
    const [inviteDeepLink, setInviteDeepLink] = createSignal("");
    if (typeof window !== "undefined") {
        const onDeepLink = (e: Event) => {
            const url = (e as CustomEvent).detail;
            if (typeof url === "string" && url.startsWith("gaugewright://invite")) {
                setInviteDeepLink(url);
            }
        };
        window.addEventListener("gw-deep-link", onDeepLink);
        onCleanup(() => window.removeEventListener("gw-deep-link", onDeepLink));
    }
    const [agentSettings, setAgentSettings] = createSignal<{ id: ArchetypeId; name: string } | null>(null);
    // The per-project Engagement pane (FED-7), opened from a project node.
    const [engagement, setEngagement] = createSignal<{ id: ProjectId; name: string } | null>(null);
    // LLM-2: the per-project model-access panel (pin a BYOK key at project scope).
    const [modelAccess, setModelAccess] = createSignal<{ id: ProjectId; name: string } | null>(null);
    // UX-2: the per-project home panel (recent runs, outputs under review, audit rollup).
    const [projectHome, setProjectHome] = createSignal<{ id: ProjectId; name: string } | null>(null);
    // UX-8: the fork-tree panel (chat fork lineage); holds the chat that opened it.
    const [forkTreeFor, setForkTreeFor] = createSignal<EngagementId | null>(null);

    // Mirror the workspace *live* across clients over the workspace event stream
    // (the sibling of the per-chat SSE): the server pushes a "changed" ping whenever
    // the library mutates — a chat created on another client (a paired mobile device)
    // appears here at once, no polling. `navTick` is the nav's refresh key.
    //
    // It is bumped by: the workspace event stream (below), a turn settling
    // (`runPrompt`), creating a chat (`startNewChat`), and keep/discard (`onMerge`).
    // It deliberately does NOT fold in `status`: `status` is transient action
    // feedback ("turn complete", "ingested 3 files"), not a workspace change, and
    // refetching the whole nav on every status message both wasted a `GET /workspace`
    // per keystroke-ish action and churned the tree (round-13 follow-up).
    const [navTick, setNavTick] = createSignal(0);
    const bumpNav = () => setNavTick((k) => k + 1);
    createEffect(() => {
        const stop = api.subscribeWorkspace(bumpNav);
        onCleanup(stop);
    });

    // Keep the transport's bearer in lock-step with the login session (ID-3), so a
    // sign-out (or sign-in without a full reload) takes effect on the next request.
    createEffect(() => api.setBearer(bearer()));
    const navRefresh = () => navTick();

    // Desktop projection freshness (RF-E4): the shell consumes projections bare
    // over HTTP/SSE, so a dropped fetch would otherwise leave the UI silently stuck
    // on a stale view. We fold each main projection load's success/failure into one
    // pure freshness reducer (workbench-ui/desktop-freshness.ts); when a refresh fails the
    // FreshnessBanner surfaces an explicit "couldn't refresh — retry" affordance
    // wired to re-run the failed loads. This layers ON TOP of the SSE path — the
    // live stream is untouched; this only tracks the request/response projections.
    const [freshness, setFreshness] = createSignal<FreshnessState>(initialFreshness);
    const [freshNow, setFreshNow] = createSignal(Date.now());
    const markLoadOk = () => setFreshness((s) => reduceFreshness(s, { kind: "ok", now: Date.now() }));
    const markLoadFail = (e: unknown) =>
        setFreshness((s) => reduceFreshness(s, { kind: "fail", error: `couldn't refresh — ${String(e)}` }));
    // Re-derive against a live clock so a `stale` view ages into `stuck` on its own.
    createEffect(() => {
        const t = setInterval(() => setFreshNow(Date.now()), 5_000);
        onCleanup(() => clearInterval(t));
    });
    const freshnessStatus = () => deriveFreshness(freshness(), freshNow());

    const [run, { refetch: refetchRun }] = createResource(selected, async (id) => {
        try {
            const r = await api.getRun(scopeId(id));
            markLoadOk();
            return r;
        } catch (e) {
            // The run projection still falls back to Init so the UI renders, but the
            // failure now feeds the freshness signal (it no longer reads as success).
            markLoadFail(e);
            return { phase: "Init" as const, admittedOnce: false };
        }
    });
    // diff/merge: on failure feed the freshness signal but keep the LAST good value
    // (createResource preserves the prior value when the fetcher returns rather than
    // throws). That is the whole point of RF-E4 — a dropped refresh must leave the UI
    // on its last-known view with a staleness caveat, not crash the render or blank
    // the panel. A `throw` here would put the resource into an error state and reads
    // like `merge()?.phase` would re-throw; returning the prior value avoids that.
    const [diff, { refetch: refetchDiff }] = createResource<string, EngagementId>(
        selected,
        async (id, info) => {
            try {
                const d = await api.engagementDiff(id);
                markLoadOk();
                return d;
            } catch (e) {
                markLoadFail(e);
                return info.value ?? "";
            }
        },
    );
    const [merge, { refetch: refetchMerge }] = createResource<MergeState | null, EngagementId>(
        selected,
        async (id, info) => {
            try {
                // UX-13: the merge review is the desktop's highest-stakes surface, so it
                // reads through the freshness carriage and honors a server-declared
                // non-live marker (stale/partial/redacted) as `server-stale` — held data
                // shown with a caveat — rather than rendering it as fresh.
                const c = await api.getMergeCarriage(id);
                setFreshness((s) =>
                    reduceFreshness(s, freshnessEventForMarker(c.freshness.marker, c.freshness.repairHint, Date.now())),
                );
                return c.value;
            } catch (e) {
                markLoadFail(e);
                return info.value ?? null;
            }
        },
    );
    // Retry the main projection loads (the freshness-banner affordance). A success
    // on any of them flips the reducer back to `fresh`; this re-runs them all.
    function retryProjections() {
        void Promise.allSettled([refetchRun(), refetchDiff(), refetchMerge(), refetchChatInfo()]);
    }

    // The selected chat's raw `.agent-config.json` — read so the composer model picker
    // (LLM-1, ADR 0062) can show the per-chat model and write a new one back without
    // clobbering the rest of the config. Not a freshness-fed projection (it is config,
    // not run truth), so it stays out of markLoadOk/Fail.
    const [chatConfig, { refetch: refetchChatConfig }] = createResource<string, EngagementId>(
        selected,
        async (id) => {
            try {
                return await api.getConfig(id);
            } catch {
                return "{}";
            }
        },
    );
    const chatModel = () => readChatModel(chatConfig() ?? "");
    const chatProvider = () => readChatProvider(chatConfig() ?? "");
    const chatThinking = () => readChatThinking(chatConfig() ?? "");
    // The model picker + reasoning-effort toggle offer only what the operator's linked
    // accounts actually provide (LLM-1, ADR 0062), derived from the Pi model catalog.
    // Keyed on `selected` so opening/switching a chat — the only time the picker shows —
    // re-reads the linked providers (a key linked or the codex OAuth signed in elsewhere
    // shows up on the next chat open). Failures degrade to "nothing linked" → just Default.
    const [linkedCreds] = createResource(selected, () => api.accountCredentials().catch(() => []));
    const [codexCred] = createResource(selected, () => api.codexStatus().catch(() => null));
    // First-run credential gate (ADR 0075 Phase 0): a startup-time (not selection-
    // keyed) read of whether *any* LLM credential is linked. `undefined` while
    // loading — we never flash the welcome before we know. `refetch*` re-check
    // after the overlay links one, which flips `hasAnyCredential` and dismisses it.
    const [startupCreds, { refetch: refetchStartupCreds }] = createResource(() =>
        api.accountCredentials().catch(() => []),
    );
    const [startupCodex, { refetch: refetchStartupCodex }] = createResource(() =>
        api.codexStatus().catch(() => null),
    );
    // Whether the default runtime actually needs a credential (server truth): off
    // under the scripted fake agent (dev/e2e), so the overlay never blocks a
    // no-credential test. Fail toward showing setup if the probe fails.
    const [credentialRequired] = createResource(() =>
        api.onboardingStatus().then((s) => s.credentialRequired).catch(() => true),
    );
    const [firstRunDismissed, setFirstRunDismissed] = createSignal(false);
    const hasAnyCredential = (): boolean | undefined => {
        const creds = startupCreds();
        const codex = startupCodex();
        if (creds === undefined || codex === undefined) return undefined; // still loading
        return creds.some((c) => c.linked) || Boolean(codex?.linked);
    };
    const showFirstRun = () =>
        credentialRequired() === true && hasAnyCredential() === false && !firstRunDismissed();
    // The operator's curated "which models show" preference (managed in the Account panel,
    // persisted in the account-settings KV). `null` = never curated → default-visible subset.
    const [acctSettings] = createResource(selected, () => api.accountSettings().catch((): Record<string, string> => ({})));
    const linkedAccounts = createMemo(() => {
        const ps = (linkedCreds() ?? []).filter((c) => c.linked).map((c) => c.provider);
        if (codexCred()?.linked) ps.push("openai-codex");
        return ps;
    });
    const enabledModels = createMemo(() => parseEnabledModels(acctSettings()?.[ENABLED_MODELS_SETTING]));
    const modelChoices = createMemo<ModelOption[]>(() =>
        modelOptions(linkedAccounts(), enabledModels(), { id: chatModel(), provider: chatProvider() }),
    );
    // The current pin as the `<select>` value: `provider:id`, or "" for Default.
    const modelValue = () => (chatModel() ? modelKey({ id: chatModel(), provider: chatProvider() }) : "");
    // The reasoning-effort options follow the pinned model; the toggle only shows when the
    // model supports thinking (more than just "off"). "" = the model's own default effort.
    const effortLevels = createMemo(() => thinkingLevelsFor(linkedAccounts(), chatModel(), chatProvider()));
    const showEffort = createMemo(() => effortLevels().some((l) => l !== "off"));

    // Pin a model for this chat: the picker's option value is `provider:id` (empty =
    // Default → clear the override). Writes `model`+`provider`, preserving every other key.
    async function pickModel(value: string) {
        const id = selected();
        if (!id) return;
        const i = value.indexOf(":");
        const pin = value === "" ? { id: "", provider: "" } : { id: value.slice(i + 1), provider: value.slice(0, i) };
        try {
            await api.putConfig(id, writeChatModelPin(chatConfig() ?? "{}", pin));
            await refetchChatConfig();
        } catch (e) {
            setStatus(`couldn't set the model — ${String(e)}`);
        }
    }
    // Pin the reasoning effort (Pi `--thinking`) for this chat; "" clears it (model default).
    async function pickThinking(level: string) {
        const id = selected();
        if (!id) return;
        try {
            await api.putConfig(id, writeChatThinking(chatConfig() ?? "{}", level));
            await refetchChatConfig();
        } catch (e) {
            setStatus(`couldn't set reasoning effort — ${String(e)}`);
        }
    }
    // Whether the selected chat's pending change *loosens* the assistant's
    // permissions (#5 round-4). The in-panel keep already gates this behind a
    // confirm; the always-visible TASKS-bar pill must not be a one-click bypass for
    // the same change. App owns the diff, so it can tell the pill to route a
    // loosening change through the review instead of committing it from the bar.
    const selectedLoosening = createMemo(() =>
        readPolicyDiff(diff() ?? "").notes.some((n) => n.direction === "loosen"),
    );
    // The files a turn actually changed, read from the diff (round-7 #3). The
    // Changes tab auto-renders the diff; View should not sit empty with a "pick a
    // file" hint when the turn touched exactly one file. When it did, and nothing
    // is selected yet, auto-open that file in View — one populated panel, not one
    // populated and one empty for the same just-modified file. We only ever fill an
    // *empty* selection, never override a file the user picked themselves.
    // The selected chat's lineage + kind (ADR 0035). The chat's KIND is its root,
    // fixed at creation: rooted on an archetype ⇒ `edit`, on a placement ⇒ `work`.
    // We resolve it (and the displayed lineage) from the workspace projection: for
    // a work chat the lineage is `archetype · project`; for an edit chat it is
    // `archetype · Library`.
    // The open chat's header facts (title, lineage, kind, project) are a **library
    // projection**, so they must track the workspace event stream — not just the
    // selection. Keying on `navTick` too means a rename (or any library change to
    // this chat, incl. from another client) re-resolves the header live, the same
    // way the nav does. Guarded so no selection ⇒ no fetch.
    const [chatInfo, { refetch: refetchChatInfo }] = createResource(
        () => (selected() ? ([selected()!, navTick()] as const) : false),
        async ([id]) => {
        const ws = await api.getWorkspace();
        // Work chats live under a project's placement → lineage is archetype · project.
        for (const p of ws.projects) {
            for (const pl of p.placements) {
                const c = pl.chats.find((c) => c.id === id);
                if (c) {
                    return {
                        kind: c.kind,
                        lineage: `${pl.archetypeName} · ${p.name}`,
                        title: c.title,
                        // The project this chat lives in drives the network-egress
                        // bar (RF-B3): only a project-rooted (work) chat has one.
                        project: { id: p.id, name: p.name, networkIsolated: p.networkIsolated },
                    };
                }
            }
        }
        // Edit chats live under an archetype → lineage is archetype · Library.
        for (const a of ws.archetypes) {
            const c = a.chats.find((c) => c.id === id);
            if (c) {
                return { kind: c.kind, lineage: `${a.name} · Library`, title: c.title };
            }
        }
        // Fallback to the flat recent list (archetype name only).
        const r = ws.recent.find((c) => c.id === id);
        return {
            kind: r?.kind ?? "work",
            lineage: r?.archetype ?? "",
            title: r?.title ?? "",
        };
        },
    );
    const chatKind = () => chatInfo()?.kind ?? "work";
    const lineage = () => chatInfo()?.lineage ?? "";
    // The chat's own name for the header (round-6 #6): two chats under one method
    // share a lineage (`Email helper · Marketing`), so the header must show the
    // chat's title to tell them apart. Reuse the shared placeholder rule so a
    // still-unnamed chat reads "Untitled" rather than the raw "new chat" token.
    const chatTitle = () => displayChatTitle(chatInfo()?.title ?? "");
    // Fork lineage (#3): the source this chat was copied from (from the "(fork)"
    // suffix), so the empty state can explain what a fork is and what carried over.
    const forkOf = () => forkSource(chatInfo()?.title ?? "");
    // The method name driving this chat (round-6 #6): the lineage is
    // `method · project` (work) or `method · Library` (edit), so the part before
    // the "·" is the method — used to name it in the composer caption instead of
    // the hardcoded "assistant".
    const methodName = () => (lineage().split("·")[0] ?? "").trim();

    // The project the open chat lives in (RF-B3): drives the bottom-left network
    // bar. Only a project-rooted work chat has one — an edit chat or the hidden
    // Personal default resolves to `null`, so the bar reads-only there.
    const currentProject = () => chatInfo()?.project ?? null;
    const [networkBusy, setNetworkBusy] = createSignal(false);
    async function toggleNetworkIsolated() {
        const p = currentProject();
        if (!p || networkBusy()) return;
        setNetworkBusy(true);
        try {
            await api.setProjectNetworkIsolated(p.id, !p.networkIsolated);
            await refetchChatInfo();
        } finally {
            setNetworkBusy(false);
        }
    }

    // Auto-title (#4): a brand-new chat is created with a generic placeholder
    // ("new chat", "edit chat", …). The first thing the user types is the obvious
    // title, so on the first message of a still-unnamed, still-empty chat we adopt
    // a trimmed version of it — All chats stops filling with identical "new chat"
    // rows. We only ever overwrite a known placeholder, never a user-chosen name.
    // (PLACEHOLDER_TITLES / isPlaceholderTitle / titleFromPrompt are shared with the
    // nav + task bar via state/chat-title.)
    async function maybeAutoTitle(id: EngagementId, prompt: string) {
        const current = (chatInfo()?.title ?? "").trim();
        // "First message" must be judged from the DURABLE snapshot, not the live
        // transcript: runPrompt echoes the user's line into `live` optimistically
        // (round-7 #6) *before* calling this, so transcript().lines is already 1.
        // Reading the snapshot (admitted records only) keeps the first-turn check
        // honest — otherwise auto-title never fires and chats stay "Untitled".
        const isFirst = snapshot().lines.length === 0;
        if (!isFirst || !isPlaceholderTitle(current)) return;
        const title = titleFromPrompt(prompt);
        if (!title) return;
        try {
            await api.renameChat(id, title);
            // The header title comes from chatInfo (keyed on `selected`, which didn't
            // change), so refetch it or the header keeps showing the old placeholder.
            await refetchChatInfo();
        } catch {
            /* titling is best-effort; a failed rename must never block the turn */
        }
    }

    // The transcript is a projection of durable truth: a **snapshot** of admitted
    // records (refetched per engagement, survives reloads) concatenated with the
    // **live** SSE reduction of the in-progress turn. No client-only history.
    const [snapshot, setSnapshot] = createSignal<Transcript>(empty);
    const [live, setLive] = createSignal<Transcript>(empty);
    const transcript = (): Transcript => ({
        lines: [...snapshot().lines, ...live().lines],
        openText: live().openText,
    });
    async function loadSnapshot(id: EngagementId) {
        try {
            setSnapshot(fromSnapshot(await api.getTranscript(id)));
            setLive(empty);
        } catch {
            /* a fresh engagement has no snapshot */
        }
    }
    // The optimistic send, modeled as an **explicit pending command** (app-stack.md,
    // "Optimistic UI models pending commands explicitly"): when a turn starts we echo
    // the user's line into the live transcript at once and record a `clientRequestId`
    // for the in-flight command. Reconciliation is the transcript's snapshot-repair
    // model (the doctrine's named transcript mechanism): on settle OR rejection we
    // re-read the durable snapshot, which retires the optimistic echo — a kept turn
    // shows the admitted line, a failed/rejected turn drops it (no dangling echo).
    const [pendingSend, setPendingSend] = createSignal<{ id: EngagementId; rid: ClientRequestId } | null>(null);
    let nextRid = 1;
    const retireSend = (rid: ClientRequestId) =>
        setPendingSend((p) => (p?.rid === rid ? null : p));
    const [draft, setDraft] = createSignal("");
    // Per-chat agent run state (round-13): one global `busy` can't represent
    // concurrent agents. `runTones` is the *live, local* state of turns this client
    // started (working / error). The needs-review state is server truth from the
    // tasks projection, so it survives reload and reflects other clients too. They
    // fold in `runToneOf`. `busy()` (the selected chat's gate) is derived from it.
    const [runTones, setRunTones] = createSignal<Record<string, "working" | "error">>({});
    const setRunTone = (id: EngagementId, tone: "working" | "error" | null) =>
        setRunTones((s) => {
            const next = { ...s };
            if (tone) next[String(id)] = tone;
            else delete next[String(id)];
            return next;
        });
    const [tasks] = createResource(navRefresh, () => api.getTasks().catch(() => []));
    const reviewSet = () => new Set((tasks() ?? []).map((t) => String(t.id)));
    const runToneOf = (id: EngagementId | null): ChatRunTone | undefined => {
        if (!id) return undefined;
        const local = runTones()[String(id)];
        if (local) return local; // working / error — a live turn is the most current fact
        if (reviewSet().has(String(id))) return "review";
        return undefined;
    };
    const busy = () => runToneOf(selected()) === "working";
    // Client-only send queue: messages typed while a turn runs. It is per-engagement
    // (cleared on switch) and never durable — the durable transcript only ever
    // records what actually ran (run-chat.md). `nextQid` mints stable list keys.
    const [queue, setQueue] = createSignal<QueuedMsg[]>([]);
    let nextQid = 1;
    // The queue **gate** (#24): when gated, typed messages *stage* in the thread
    // without draining into turns — the user can line up several, then release them.
    // Default open (ungated) = the prior immediate/queue-and-run behaviour.
    const [gated, setGated] = createSignal(false);
    // Message attachments (UX-14): file(s) the user clips to the message being
    // composed. Their text is read client-side and inlined into the turn's prompt
    // (see composeOutgoing) — message-scoped, cleared on send, never workspace
    // context. Uniform across browser and the Tauri webview; no backend.
    const [attachments, setAttachments] = createSignal<Attachment[]>([]);
    let attachInput: HTMLInputElement | undefined;
    const [activity, setActivity] = createSignal(""); // what the agent is doing now
    // The agent's pending approvals from the last turn (UX-3): an `extension_ui_request`
    // the runtime surfaced but did not auto-confirm — answered out-of-band via the
    // resource-access / export grant flow. Shown as an inline confirm notice.
    const [pendingApprovals, setPendingApprovals] = createSignal<string[]>([]);
    // Pending approvals are per-chat: clear them when the selected chat changes so one
    // chat's held requests never bleed into another's view.
    createEffect(() => {
        selected();
        setPendingApprovals([]);
    });
    const [streamReady, setStreamReady] = createSignal(false);
    // Transcript view prefs (which event types show, what opens expanded): pure
    // view state. Toggles apply live for the session but are NOT auto-persisted —
    // the filter is a working state. "Save as default" (below) is what writes the
    // current filter to localStorage as the user's persistent default; on load we
    // seed from that saved default (else the factory defaults).
    const filterStorage = typeof window === "undefined" ? null : window.localStorage;
    const [filterPrefs, setFilterPrefs] = createSignal(loadPrefs(filterStorage));
    // Persist the current filter as the user's default (the "Save as default"
    // action). Reverting to it is just re-seeding from storage.
    const saveFilterDefault = () => savePrefs(filterStorage, filterPrefs());
    const [selectedFile, setSelectedFile] = createSignal<string | null>(null);
    const [showShelf, setShowShelf] = createSignal(false);
    const [showContext, setShowContext] = createSignal(false);
    const [contextPath, setContextPath] = createSignal("");
    // The context-sources panel (RF-E1 / O-1): lists what context the chat holds.
    const [showSources, setShowSources] = createSignal(false);

    // On selecting an engagement, load its durable transcript snapshot, then
    // subscribe to live SSE (appending the in-progress turn). Switching back or
    // reloading rebuilds from the snapshot — the chat is durable, not client-only.
    createEffect(() => {
        const id = selected();
        if (!id) return;
        setStreamReady(false);
        setSelectedFile(null);
        setSnapshot(empty);
        setLive(empty);
        setQueue([]); // the queue is per-engagement and client-only
        setGated(false); // the stage-gate resets with the thread
        void loadSnapshot(id);
        const unsubscribe = api.subscribe(
            id,
            (ev) => {
                setLive((t) => reduce(t, ev));
                // Reflect what the agent is doing, live, from the operational stream.
                if (ev.type === "text") setActivity("writing…");
                else if (ev.type === "tool") setActivity("using a tool…");
                else if (ev.type === "blocked") setActivity("effect blocked by the membrane");
            },
            () => setStreamReady(true),
        );
        onCleanup(unsubscribe);
    });

    // Auto-open the single changed file in View (round-7 #3). A one-way side effect
    // keyed explicitly on the diff (`on`, not an ambient effect): when the diff
    // settles and the turn touched exactly one user-facing file, and the user hasn't
    // picked a file of their own (read untracked, so this never re-fires on the
    // user's own selection), select it so View and Changes agree.
    createEffect(
        on(diff, (d) => {
            const changed = changedUserFiles(d ?? "");
            if (changed.length === 1 && untrack(selectedFile) === null) setSelectedFile(changed[0]);
        }),
    );

    // Opening a chat is ONE behavior, wherever it's triggered from (any nav facet,
    // the task bar, or creating a new chat): select it and put the cursor in the
    // composer. Every entry point calls this — there is no per-facet variation.
    // (Microtask-deferred so the composer, mounted by <Show> on first selection,
    // exists before we focus it.)
    let composerEl: HTMLInputElement | undefined;
    function openChat(id: EngagementId) {
        setSelected(id);
        // UX-4: mirror the selection into the URL (`?chat=<id>`) so it's deep-linkable.
        if (typeof window !== "undefined") {
            window.history.replaceState(null, "", searchWithChat(window.location.search, id));
        }
        setChatOff(false); // a folded chat reopens when you open a chat — no dead click
        // On the mobile carousel, opening a chat must bring the chat pane on-screen.
        // Selecting it alone strands the user on Browse: Browse stays reachable, so the
        // selection-repair effect never moves them, and the top toggle's Chat/Files
        // segments only un-grey — they don't navigate on their own. Mirror the mobile
        // harness: mark the chat reachable, then jump straight to it.
        if (isMobile()) {
            setCarousel((c) =>
                reduceCarousel(
                    applySelection(c, { chatSelected: true, fileSelected: false }),
                    tapGesture("chat"),
                ),
            );
        }
        queueMicrotask(() => composerEl?.focus());
    }

    // UX-4: mirror the in-chat file selection into the URL (`?chat=<id>&file=<path>`) so a
    // file open within a chat is deep-linkable too. A separate effect from the chat mirror
    // (searchWithChat/searchWithFile each preserve the other's param), so it composes cleanly
    // and clears `file` whenever no file is open (including the reset on a chat switch).
    if (typeof window !== "undefined") {
        createEffect(() => {
            const path = selectedFile();
            window.history.replaceState(null, "", searchWithFile(window.location.search, path));
        });
    }

    // UX-4: restore a deep-linked chat (`?chat=<id>`) and the file open within it
    // (`&file=<path>`) on first load — best-effort, after sync setup. A stale chat id just
    // yields an empty transcript (handled); the file is restored after the chat's snapshot
    // load resets the per-chat selection, so a stale path simply resolves to no open file.
    if (typeof window !== "undefined") {
        const urlChat = chatIdFromSearch(window.location.search);
        const urlFile = fileFromSearch(window.location.search);
        if (urlChat)
            queueMicrotask(() => {
                openChat(urlChat as EngagementId);
                if (urlFile) queueMicrotask(() => setSelectedFile(urlFile));
            });
    }

    // Start a fresh chat on the hidden Personal default placement (ADR 0036) — the
    // same "just start typing" path as the nav's "+ new chat". Wired to the mobile
    // carousel's Chat tab so it's never a dead control when no chat is open yet.
    async function startNewChat() {
        try {
            const eng = await api.createEngagement();
            setStatus("new chat");
            bumpNav(); // surface the new chat in the nav (the event stream also will)
            openChat(eng.id); // selects it and (on mobile) brings the chat pane on-screen
        } catch (e) {
            setStatus(`couldn't start a chat — ${String(e)}`);
        }
    }

    // Stop a running turn (run-chat.md): aborts the agent's Pi child; the in-flight
    // task POST then resolves as failed and the composer re-enables.
    async function stopTurn() {
        const id = selected();
        if (!id) return;
        setActivity("stopping…");
        try {
            await api.stopTurn(id);
        } catch (e) {
            setStatus(`stop error: ${String(e)}`);
        }
    }

    // Run exactly one prompt as a turn. The single primitive behind both an
    // immediate send and a queue drain. On settle it swaps the live operational
    // stream for the now-durable transcript, then pumps the queue so the next
    // queued message runs automatically — turns chain without a round-trip to the human.
    async function runPrompt(id: EngagementId, prompt: string, images: ImageRef[] = []) {
        // This turn may run in the background while the user looks at a different
        // chat, so every *display-mutating* side effect is gated on `isCurrent()` —
        // the running chat's transcript/diff/status must never clobber whatever chat
        // is on screen (round-13). The per-chat run tone is set regardless, so the
        // chat's dot in Browse reflects its own state.
        const isCurrent = () => selected() === id;
        const rid = clientRequestId(`${id}:${nextRid++}`);
        setRunTone(id, "working");
        if (isCurrent()) {
            setActivity("thinking…");
            setStatus("tasking agent…");
            // Immediate feedback (round-7 #6): echo the user's message into the
            // transcript the instant the turn starts — only when this chat is the one
            // on screen, else we'd inject it into the displayed chat's transcript. The
            // echo is an optimistic pending command, retired by snapshot-repair below.
            setPendingSend({ id, rid });
            setLive((t) => reduce(t, { type: "user", text: prompt }));
        }
        // Adopt the first message as the chat's title before the transcript fills.
        await maybeAutoTitle(id, prompt);
        setPendingApprovals([]); // a fresh turn clears the prior turn's pending approvals
        try {
            const res = (await api.runTask(id, prompt, images)) as {
                pending_approvals?: string[];
                run_phase?: string;
                error?: string;
            };
            if (isCurrent()) setPendingApprovals(res?.pending_approvals ?? []);
            // A turn can return 200 yet have *failed* (e.g. the model rejected an
            // attached image): the runtime error rides `run_phase`/`error`, not an HTTP
            // status. Report it honestly instead of a blanket "turn complete" — the
            // reason itself is also a durable transcript line (loadSnapshot shows it).
            const failed = res?.run_phase === "Failed";
            setRunTone(id, failed ? "error" : null);
            bumpNav(); // refresh nav + tasks (auto-title, review dot)
            if (isCurrent()) {
                setStatus(failed ? `turn failed${res?.error ? ` — ${res.error}` : ""}` : "turn complete");
                await Promise.all([loadSnapshot(id), refetchRun(), refetchDiff(), refetchMerge()]);
            }
        } catch (e) {
            setRunTone(id, "error");
            if (isCurrent()) {
                // A rejection (INV-2) surfaces its reason; either way repair from the
                // durable snapshot so a failed turn leaves no dangling optimistic echo.
                setStatus(describeFailure("run that turn", e));
                await loadSnapshot(id);
            }
        } finally {
            retireSend(rid);
            if (isCurrent()) {
                setActivity("");
                pump();
            }
        }
    }

    // Drain the head of the queue if idle. A no-op while a turn is in flight or the
    // queue is empty; runPrompt() calls it again on settle, so the queue chains itself.
    function pump() {
        if (busy() || gated()) return; // a gated queue stages; it does not drain
        const id = selected();
        if (!id) return;
        const next = queue()[0];
        if (!next) return;
        setQueue((q) => q.slice(1));
        void runPrompt(id, next.text, next.images);
    }

    // Toggle the stage-gate (#24). Opening it releases whatever's staged into the
    // running queue (pump drains the front, and each settle chains the rest).
    function toggleGate() {
        const opening = gated();
        setGated((g) => !g);
        if (opening) pump();
    }

    // The composer's primary action. Idle → send now; busy → append to the queue
    // (it runs when the current turn settles). One path either way: enqueue + pump.
    // The send *primitive* (the Session's `send`, EMBED-1): enqueue a message and
    // pump the queue — start a turn on the current engagement. An embed composer
    // rides this directly; the desktop's draft/queue/steer controls layer on top.
    function sendText(text: string, images: ImageRef[] = []) {
        const id = selected();
        const t = text.trim();
        if (!id || (!t && images.length === 0)) return;
        setQueue((q) => [...q, { id: nextQid++, text: t, images }]);
        pump();
    }

    // There's something to send when the draft has text OR a file is attached.
    const hasOutgoing = () => draft().trim().length > 0 || attachments().length > 0;

    // Build the outgoing turn from the draft + pending attachments, then clear them
    // (attachments are message-scoped). Text files inline into the prompt as delimited
    // blocks; images become native `images[]` (sent to Pi as image content blocks, not
    // inlined) with only a byte-free `[attached image: …]` note left in the text so the
    // durable transcript honestly shows one rode along (run-chat.md "Message
    // attachments"; base64 never enters the log — INV-10).
    function composeOutgoing(): { message: string; images: ImageRef[] } {
        const out = buildOutgoing(draft(), attachments());
        setAttachments([]); // attachments are message-scoped — cleared on send
        return out;
    }

    function submitDraft() {
        if (!selected() || !hasOutgoing()) return;
        const { message, images } = composeOutgoing();
        setDraft("");
        sendText(message, images);
    }

    // Steer: redirect the agent *now*. With one blocking turn at a time, "now" means
    // jump the queue and abort the in-flight turn — the steer message is the next
    // thing the agent runs (the stop settles → pump() drains the front). Idle steer
    // is just an immediate send.
    function steerDraft() {
        const id = selected();
        if (!id || !hasOutgoing()) return;
        const { message, images } = composeOutgoing();
        setDraft("");
        setQueue((q) => [{ id: nextQid++, text: message, images }, ...q]);
        if (busy()) void stopTurn();
        else pump();
    }

    // Paperclip attach (UX-14): a plain <input type=file> (works in the browser and
    // the Tauri webview — no plugin, no backend). Images go to Pi as native image
    // blocks; text files inline into the prompt. PDF/Office aren't supported yet — the
    // classify() seam is where future client-side extraction (text + images) lands.
    async function onAttachInput(e: Event) {
        const input = e.currentTarget as HTMLInputElement;
        const picked = Array.from(input.files ?? []);
        input.value = ""; // let the same file be picked again later
        const next: Attachment[] = [];
        const rejected: string[] = [];
        // UX-14 vision pre-check: block an image attach up front on a KNOWN non-vision
        // model (rather than letting the turn fail at send). The default/unknown model
        // stays permissive — we take the runtime's word.
        const canSeeImages = modelAcceptsImages({ id: chatModel(), provider: chatProvider() });
        const visionBlocked: string[] = [];
        for (const f of picked) {
            const kind = classifyAttachment(f);
            if (kind === "image") {
                if (!canSeeImages) {
                    visionBlocked.push(f.name);
                    continue;
                }
                next.push({ kind: "image", name: f.name, mimeType: f.type, data: await fileToBase64(f) });
            } else if (kind === "text") {
                next.push({ kind: "text", name: f.name, text: await f.text() });
            } else {
                rejected.push(f.name);
            }
        }
        if (next.length) setAttachments((a) => [...a, ...next]);
        if (visionBlocked.length) {
            setStatus(`this chat's model can't read images — pick a vision-capable model to attach ${visionBlocked.join(", ")}`);
        } else if (rejected.length) {
            setStatus(`can't attach ${rejected.join(", ")} yet — images and text files only (PDF/Office coming soon)`);
        }
    }
    const removeAttachment = (i: number) => setAttachments((a) => a.filter((_, n) => n !== i));

    // Queue edits: reorder (drag), edit-in-place (empty text removes), and cancel.
    function reorderQueue(from: number, to: number) {
        setQueue((q) => {
            const next = q.slice();
            const [m] = next.splice(from, 1);
            next.splice(to, 0, m);
            return next;
        });
    }
    function editQueued(qid: number, text: string) {
        const t = text.trim();
        setQueue((q) =>
            t ? q.map((m) => (m.id === qid ? { ...m, text: t } : m)) : q.filter((m) => m.id !== qid),
        );
    }
    function removeQueued(qid: number) {
        setQueue((q) => q.filter((m) => m.id !== qid));
    }
    // Run one queued message NOW, jumping ahead of the rest and overriding a hold
    // for just that message. Idle (incl. held) → run it immediately; mid-turn →
    // move it to the front and interrupt, so it runs next (steer semantics). The
    // remaining queue keeps its order and hold.
    function sendNowQueued(qid: number) {
        const id = selected();
        if (!id) return;
        const item = queue().find((m) => m.id === qid);
        if (!item) return;
        if (busy()) {
            setQueue((q) => [item, ...q.filter((m) => m.id !== qid)]);
            void stopTurn(); // on settle, pump drains the front (this message)
        } else {
            setQueue((q) => q.filter((m) => m.id !== qid));
            void runPrompt(id, item.text, item.images);
        }
    }

    async function onMerge(action: MergeAction, target?: EngagementId) {
        const id = target ?? selected();
        if (!id) return;
        try {
            const m = await api.mergeCommand(id, action);
            // Plain, consistent status (#1): one verb per outcome, never the raw
            // phase token ("review: Rejected") or a "you couldn't" framing.
            const statusFor: Partial<Record<MergePhase, string>> = {
                Advanced: "kept into the shared copy",
                Integrated: "kept into the shared copy",
                // A `Rejected` merge is a conflict (couldn't be merged) or a user discard —
                // don't call a conflict a "discard" (UX-7).
                Rejected:
                    m.git_outcome === "Conflict"
                        ? "conflicted — repair it in the changes view"
                        : "discarded — nothing was kept",
                Clean: "ready to review",
                Repairing: "fixing up a conflict…",
            };
            setStatus(statusFor[m.phase] ?? "ready to review");
            bumpNav(); // keep/discard changes the task queue → refresh nav + review dots
            await Promise.all([refetchMerge(), refetchDiff(), loadSnapshot(id)]);
        } catch (e) {
            setStatus(e instanceof Rejected ? `couldn't do that — ${e.reason}` : `something went wrong — ${String(e)}`);
        }
    }

    async function ingestPath(path: string) {
        const id = selected();
        if (!id || !path) return;
        try {
            const c = await api.ingestContext(id, path);
            setStatus(`ingested ${c} file(s)`);
            setShowContext(false);
            setContextPath("");
            await Promise.all([refetchDiff(), refetchMerge()]);
        } catch (e) {
            setStatus(`context error: ${String(e)}`);
        }
    }

    // Browser/test build: attach the path typed into the fallback box.
    const attachContext = () => ingestPath(contextPath().trim());

    // "add files": in the desktop shell, open the native OS folder picker; in a
    // plain browser (incl. e2e) fall back to the paste-a-path box. The backend
    // ingests by absolute filesystem path (it copies the folder recursively),
    // which is exactly what the native picker returns.
    async function addFiles() {
        if (!isTauri()) {
            setShowContext((v) => !v);
            return;
        }
        const { open } = await import("@tauri-apps/plugin-dialog");
        const dir = await open({
            directory: true,
            multiple: false,
            title: "Add a folder of files for this chat",
        });
        if (typeof dir === "string") await ingestPath(dir);
    }

    // "add a file" (UX-1): single-file native picker in the desktop shell; the browser
    // falls back to the same paste-a-path box. The backend ingests a file path directly.
    async function addFile() {
        if (!isTauri()) {
            setShowContext((v) => !v);
            return;
        }
        const { open } = await import("@tauri-apps/plugin-dialog");
        const file = await open({
            directory: false,
            multiple: false,
            title: "Add a single file for this chat",
        });
        if (typeof file === "string") await ingestPath(file);
    }

    const phase = createMemo(() => run()?.phase ?? "—");

    // The chat-status badge (#4): a real, coloured badge separated from the
    // breadcrumb — not a small-caps word camouflaged inside the title. While a turn
    // is in flight it shows "Working" (animated); a finished turn with work to keep
    // shows "Needs review"; otherwise plain "Ready". `tone` drives the colour.
    type StatusTone = "working" | "review" | "ready" | "conflict";
    const statusBadge = createMemo<{ tone: StatusTone; label: string }>(() => {
        // The selected chat's own state: a conflict is the most urgent — it is a
        // decision the human must make (WS-H c) — then working / needs-review / ready.
        const t = runToneOf(selected());
        if (merge()?.phase === "Repairing") return { tone: "conflict", label: "Conflict" };
        if (t === "working") return { tone: "working", label: "Working" };
        if (t === "review" || merge()?.phase === "Clean") return { tone: "review", label: "Needs review" };
        return { tone: "ready", label: "Ready" };
    });

    // --- resizable panels ---------------------------------------------------
    // Sidebars (nav, workspace) are draggable px widths; the two middle panels
    // (run, content) share the remaining space via a draggable fraction. Widths
    // persist across reloads. The grid columns are: nav | ↔ | run | ↔ | content | ↔ | workspace.
    const stored = (k: string, fallback: number) => {
        const v = Number(localStorage.getItem(k));
        return Number.isFinite(v) && v > 0 ? v : fallback;
    };
    const [navW, setNavW] = createSignal(stored("ui.navW", 190));
    const [wsW, setWsW] = createSignal(stored("ui.wsW", 230));
    const [runFr, setRunFr] = createSignal(stored("ui.runFr", 0.5));
    createEffect(() => localStorage.setItem("ui.navW", String(navW())));
    createEffect(() => localStorage.setItem("ui.wsW", String(wsW())));
    createEffect(() => localStorage.setItem("ui.runFr", String(runFr())));

    // Collapse: every panel — including the chat (`run`) — folds to a thin rail to
    // give the rest of the workbench more room; a collapsed panel donates its space
    // to its neighbours. (The chat's collapse button lives in its own header, so
    // opening a chat re-expands it — see openChat — to avoid a dead click.)
    //
    // Each panel's collapse is a single browser-local boolean (persisted, §4) and
    // is the ONLY thing a chevron/rail click changes — it is purely the user's
    // intent, fully independent per panel. There is deliberately NO width-driven
    // auto-folding: that conflated "the user hid this" with "this didn't fit",
    // recomputed globally, so collapsing one panel could silently reopen another.
    // Instead, when the window is too narrow to hold the four-panel grid the shell
    // hands off wholesale to the mobile carousel (see `mobileQuery`) — one reflow,
    // never spooky action at a distance.
    const RAIL = 30; // a folded panel's rail width, px
    const storedCollapsed = (k: string) => localStorage.getItem(k) === "collapsed";
    const [navOff, setNavOff] = createSignal(storedCollapsed("ui.navPanel"));
    const [chatOff, setChatOff] = createSignal(storedCollapsed("ui.chatPanel"));
    const [contentOff, setContentOff] = createSignal(storedCollapsed("ui.contentPanel"));
    const [wsOff, setWsOff] = createSignal(storedCollapsed("ui.filesPanel"));
    createEffect(() => localStorage.setItem("ui.navPanel", navOff() ? "collapsed" : "open"));
    createEffect(() => localStorage.setItem("ui.chatPanel", chatOff() ? "collapsed" : "open"));
    createEffect(() => localStorage.setItem("ui.contentPanel", contentOff() ? "collapsed" : "open"));
    createEffect(() => localStorage.setItem("ui.filesPanel", wsOff() ? "collapsed" : "open"));
    // A chevron collapses (want=true); a rail click reopens (want=false). Each
    // call touches exactly one panel — no neighbour is ever affected.
    const pinPanel = (set: (v: boolean) => void, want: boolean) => set(want);

    const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
    const RSZ = 5; // resizer column width, px
    // The *effective* sidebar widths: a folded panel is a rail, not its stored
    // width — the resize math and the grid both reason from these.
    const effNavW = () => (navOff() ? RAIL : navW());
    const effWsW = () => (wsOff() ? RAIL : wsW());
    let shellEl: HTMLDivElement | undefined;
    // Capture the grid element so the resizer drag math (onResize) can read its
    // live geometry. The desktop grid only mounts above the mobile breakpoint.
    const observeShell = (el: HTMLDivElement) => {
        shellEl = el;
    };
    function onResize(which: "nav" | "mid" | "ws", clientX: number) {
        if (!shellEl) return;
        const r = shellEl.getBoundingClientRect();
        if (which === "nav") setNavW(clamp(clientX - r.left, 120, r.width - effWsW() - 240));
        else if (which === "ws") setWsW(clamp(r.right - clientX, 150, r.width - effNavW() - 240));
        else {
            const midLeft = r.left + effNavW() + RSZ;
            const afterContent = wsOff() ? RAIL : wsW() + RSZ;
            const midRight = r.right - afterContent;
            setRunFr(clamp((clientX - midLeft) / (midRight - midLeft), 0.15, 0.85));
        }
    }
    // The chat lane carries the primary action (send) and the content panel the
    // review diff, so neither may be squeezed below a usable width (round-7 #1).
    // Give each middle column a hard floor; if the window is too narrow to honour
    // both, the whole shell scrolls horizontally rather than clipping `send`.
    //
    // Content always stays between chat and files, even as a folded rail.
    //
    // Exactly one open track must carry a full `1fr` so the grid always fills the
    // window: a lone fr factor < 1 (CSS Grid spec) would leave an empty gap. When
    // chat AND content are both open they SPLIT the middle via `runFr` (factors sum
    // to 1 — fills); otherwise the highest-priority open panel (chat → content →
    // files → nav) is the "filler" and takes the slack.
    const cols = () => {
        const bothMiddleOpen = !chatOff() && !contentOff();

        const nav = navOff()
            ? `${RAIL}px`
            : chatOff() && contentOff() && wsOff() // only nav left open → it fills
                ? `minmax(${navW()}px,1fr)`
                : `${navW()}px`;
        const r1 = navOff() ? "0px" : `${RSZ}px`;

        const run = chatOff()
            ? `${RAIL}px`
            : bothMiddleOpen ? `minmax(280px,${runFr()}fr)` : "minmax(280px,1fr)";

        // Each right-hand panel is "rail" OR "leading resizer + track". The track
        // counts/order match the rightPanels() children one-for-one.
        const content = contentOff()
            ? `${RAIL}px`
            : bothMiddleOpen
                ? `${RSZ}px minmax(240px,${1 - runFr()}fr)`
                : `${RSZ}px minmax(240px,1fr)`; // chat folded → content fills
        const files = wsOff()
            ? `${RAIL}px`
            : chatOff() && contentOff() // both middle folded → files fills
                ? `${RSZ}px minmax(${wsW()}px,1fr)`
                : `${RSZ}px ${wsW()}px`;

        return `${nav} ${r1} ${run} ${content} ${files}`;
    };

    // --- mobile variant (MOB-021) -------------------------------------------
    // On a narrow viewport the same four projections render through the mobile
    // Carousel island (MOB-014), one pane at a time, reusing the *identical* pane
    // bodies the desktop grid renders — the shell is a thin renderer either way,
    // so the only difference is layout. The media query is a reactive signal so a
    // rotate / resize swaps shells live; the `matchMedia` guard keeps it SSR- and
    // test-safe (a plain non-DOM environment renders the desktop grid).
    //
    // The breakpoint is the four-panel grid's comfortable floor, not a phone width:
    // nav + chat + content + files at their minimums need ~955px, so below ~1024px
    // the desktop grid would clip or scroll. Since panels no longer auto-fold to
    // absorb that pressure, the carousel IS the narrow-window layout — we hand off
    // to it well before things get cramped rather than railing panels behind the
    // user's back. (Was 720px, which left a broken auto-folding band above it.)
    const mobileQuery = "(max-width: 1024px)";
    const matchMobile = () =>
        typeof window !== "undefined" && typeof window.matchMedia === "function"
            ? window.matchMedia(mobileQuery).matches
            : false;
    const [isMobile, setIsMobile] = createSignal(matchMobile());
    createEffect(() => {
        if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
        const mql = window.matchMedia(mobileQuery);
        const onChange = () => setIsMobile(mql.matches);
        onChange();
        // Listen to BOTH the media-query change and a plain window resize. The
        // matchMedia "change" event alone proved unreliable across DevTools device
        // emulation / programmatic resizes (it didn't fire, so the shell never
        // flipped between the desktop grid and the mobile carousel until a reload).
        // `resize` always fires on a viewport change; re-reading `mql.matches` on it
        // keeps `isMobile` honest. Both funnel through the same idempotent setter.
        mql.addEventListener("change", onChange);
        window.addEventListener("resize", onChange);
        onCleanup(() => {
            mql.removeEventListener("change", onChange);
            window.removeEventListener("resize", onChange);
        });
    });

    // The carousel's pure navigation state (MOB-009): which pane is on screen and
    // the selection that gates reachability. The island owns no truth — it routes
    // every gesture back through `setCarousel`. We keep the gating selection in
    // lockstep with `selected` / `selectedFile` so a chat opened or a file picked
    // (from any pane) repairs the carousel onto a still-reachable pane rather than
    // stranding the user on an empty stop.
    const [carousel, setCarousel] = createSignal<CarouselState>(initialCarousel);
    createEffect(() => {
        const selection = { chatSelected: selected() !== null, fileSelected: selectedFile() !== null };
        setCarousel((c) => applySelection(c, selection));
    });

    // The four pane bodies, defined once and reused by both shells (the desktop
    // grid and the mobile Carousel). They are plain accessors so the island stays
    // projection-agnostic: it receives ready-rendered panes and only decides which
    // one is on screen.
    const navPane = () => (
        <FacetBrowser
            api={api}
            selected={selected()}
            onSelect={openChat}
            onOpenArchetypeSettings={(id, name) => setAgentSettings({ id, name })}
            onOpenEngagement={(id, name) => setEngagement({ id, name })}
            onOpenModelAccess={(id, name) => setModelAccess({ id, name })}
            onOpenProjectHome={(id, name) => setProjectHome({ id, name })}
            onOpenForkTree={(chat) => setForkTreeFor(chat)}
            onChatDeleted={(id) => selected() === id && setSelected(null)}
            onStatus={setStatus}
            runToneOf={runToneOf}
            refreshKey={navRefresh()}
        />
    );

    // The network-egress bar (RF-B3), pinned bottom-left of the nav column. It
    // reflects the open chat's project posture: open (the app default — the agent
    // can reach the model, and with no per-host proxy yet, any host) or isolated
    // (fail-closed). Clicking toggles it for that project. With no project-rooted
    // chat open (an edit chat / the Personal default) there's nothing to manage, so
    // it shows a muted, read-only hint.
    const networkBar = () => (
        <div class="network-bar" classList={{ isolated: !!currentProject()?.networkIsolated }}>
            <Show
                when={currentProject()}
                fallback={
                    <span class="network-bar-label" title="Open a project's work chat to manage its network egress.">
                        <span class="network-bar-dot" /> Network · open
                    </span>
                }
            >
                {(p) => (
                    <button
                        type="button"
                        class="network-bar-toggle"
                        data-testid="network-toggle"
                        disabled={networkBusy()}
                        title={
                            p().networkIsolated
                                ? `“${p().name}” is network-isolated — the agent can't reach the model. Click to open egress.`
                                : `“${p().name}” has open network egress — the agent can reach any host. Click to isolate (fail-closed).`
                        }
                        onClick={toggleNetworkIsolated}
                    >
                        <span class="network-bar-dot" />
                        {p().networkIsolated ? "Network · isolated" : "Network · open"}
                    </button>
                )}
            </Show>
            {/* Settings (FED-7): a gear to the right of the network toggle; opens the
                settings menu → Devices, the single device-management modal. */}
            <SettingsMenu api={api} openAccount={accountRequest} openInvite={inviteDeepLink} />
        </div>
    );

    const contentPane = () => (
        <>
            <h2>Content</h2>
            <Show when={selected()} fallback={<div class="status">Open a chat to view its files and changes here.</div>}>
                <ContentViewer />
            </Show>
        </>
    );

    const filesPane = () => (
        <>
            <h2>
                Files
                <Show when={selected()}>
                    {/* Durable context upload lives here (UX-14), not the chat
                        composer: a folder or single file is copied into this chat's
                        workspace and persists. The composer's paperclip is the
                        separate, message-scoped attach. */}
                    <span class="header-actions">
                        <button
                            class="icon-btn"
                            aria-label="Add files"
                            title="Add a folder of files for the agent to work with (copied into this chat's workspace)"
                            onClick={addFiles}
                        >
                            <Icon name="add-files" />
                        </button>
                        <button
                            class="icon-btn"
                            aria-label="Add a file"
                            title="Add a single file for the agent to work with (copied into this chat's workspace)"
                            onClick={addFile}
                        >
                            <Icon name="add-files" />
                        </button>
                    </span>
                </Show>
            </h2>
            <Show when={selected()} fallback={<div class="status">Open a chat to see the files it's working with.</div>}>
                <Show when={showContext()}>
                    {/* In the desktop app "add files" opens a real OS folder picker
                        (see addFiles); this typed path is the browser fallback only. */}
                    <div class="add-files-fallback" style={{ "margin-bottom": "8px" }}>
                        <div class="bar">
                            <input
                                data-context-path
                                placeholder="paste a folder location…"
                                value={contextPath()}
                                onInput={(e) => setContextPath(e.currentTarget.value)}
                                onKeyDown={(e) => e.key === "Enter" && attachContext()}
                            />
                            <button data-context-attach onClick={attachContext}>attach</button>
                        </div>
                        <span class="status">In the desktop app this opens a folder picker — no typing needed.</span>
                    </div>
                </Show>
                <Workspace />
            </Show>
        </>
    );

    const chatPane = () => (
        <>
            <h2>
                {/* The chat's own collapse control (desktop only): folds the chat
                    panel to a rail, like every other panel. Floated into the chat
                    panel's top-right corner (CSS) so it lines up with the nav /
                    content / files chevrons. `data-collapse="run"` is the same
                    hook the other panels expose; the rail reopens it. */}
                <Show when={!isMobile()}>
                    <button
                        class="panel-collapse left"
                        data-collapse="run"
                        title="Hide Chat"
                        aria-label="Hide Chat"
                        onClick={() => setChatOff(true)}
                    >
                        ‹
                    </button>
                </Show>
                {/* The chat's own name leads the header (round-6 #6) so two chats
                    under one method are distinguishable; "Chat" is the fallback
                    when nothing is selected. */}
                <Show when={selected()} fallback="Chat">
                    <span class="chat-title" data-chat-title>{chatTitle()}</span>
                </Show>
                {/* The chat's lineage (ADR 0035): `archetype · project` for a work
                    chat, `archetype · Library` for an edit chat — secondary to the name. */}
                <Show when={selected() && lineage()}>
                    <span class="chat-lineage" data-chat-lineage data-kind={chatKind()} title={`what this chat is working on: ${lineage()}`}>
                        {lineage()}
                    </span>
                </Show>
                <Show when={selected()}>
                    {/* A real status badge (#4), not a word wedged in the title. */}
                    <span
                        class="status-badge"
                        data-testid="run-phase"
                        data-status={statusBadge().tone}
                        data-run-phase={phase()}
                        title={
                            statusBadge().tone === "conflict"
                                ? "This chat hit a sync conflict — resolve it in the Changes view"
                                : statusBadge().tone === "working"
                                    ? "The agent is working on your request now"
                                    : statusBadge().tone === "review"
                                        ? "The agent finished — review what changed and keep or discard it"
                                        : "Ready for your next request"
                        }
                    >
                        <Show when={statusBadge().tone === "working"}><span class="status-dot" /></Show>
                        {statusBadge().label}
                    </span>
                </Show>
                <Show when={selected()}>
                    {/* Icon-driven action cluster: each button is icon-only with its
                        label carried by aria-label + a title tooltip. */}
                    <span class="header-actions">
                        <TranscriptFilterMenu
                            prefs={filterPrefs()}
                            onChange={setFilterPrefs}
                            onSaveDefault={saveFilterDefault}
                        />
                        <button
                            class="icon-btn"
                            data-open-sources
                            aria-label="Sources"
                            title="See the context this chat is working with — attached files and its archetype"
                            onClick={() => setShowSources(true)}
                        >
                            <Icon name="sources" />
                        </button>
                        <button
                            class="icon-btn"
                            aria-label="History"
                            title="A timeline of everything that's happened in this chat, plus the review surface"
                            onClick={() => setShowShelf(true)}
                        >
                            <Icon name="history" />
                        </button>
                    </span>
                </Show>
            </h2>
            <Show when={selected()}>
                {/* WS-H: the chat header carries no permanent "private draft" caption
                    and no manual pull/discard buttons — a chat targets one shared line
                    (default mainline = workstream of one) and co-rooted sync is greedy
                    and automatic (WS-D). Keep/discard is the review surface (Changes
                    tab); shared-line status surfaces only when named/shared or in
                    conflict (WS-H b,c). The header is quiet by default. */}
                <Show when={pendingApprovals().length > 0}>
                    <div class="approval-notice" data-pending-approvals role="status">
                        <strong>Waiting for your approval</strong>
                        <ul>
                            <For each={pendingApprovals()}>{(a) => <li>{a}</li>}</For>
                        </ul>
                        <span class="muted">
                            The agent asked to do the above and is holding until you grant it
                            (in the sources/access panel) — nothing happens without your OK.
                        </span>
                    </div>
                </Show>
            </Show>
            <Show when={selected()} fallback={<div class="status">Open a chat to get started.</div>}>
                {/* Desktop projection-freshness notice (RF-E4): chromeless while
                    fresh; on a failed refresh it surfaces the staleness + a retry
                    that re-runs the failed loads. */}
                <FreshnessBanner
                    status={freshnessStatus()}
                    error={freshness().error}
                    onRetry={retryProjections}
                />
                <Show when={streamReady()}>
                    <span data-testid="stream-ready" style={{ display: "none" }} />
                </Show>
                {/* Fork first-view note (#3): on the fork's empty transcript, explain
                    the copy-semantics established round 1 #2 — files came along, the
                    conversation started fresh — so the lineage is legible at a glance. */}
                <Show when={forkOf() && transcript().lines.length === 0}>
                    <div class="fork-note" data-fork-note>
                        Started as a copy of <strong>{forkOf()}</strong> — its files came along, the conversation starts fresh here.
                    </div>
                </Show>
                <div
                    class="transcript"
                    data-pending-send={pendingSend()?.id === selected() ? pendingSend()?.rid : undefined}
                >
                    <TranscriptView
                        lines={transcript().lines}
                        onOpen={setSelectedFile}
                        prefs={filterPrefs()}
                        onResolveCredential={() => setAccountRequest((n) => n + 1)}
                    />
                    <Show when={busy()}>
                        <div class="working" data-testid="agent-working">
                            <span class="pulse" />
                            agent working{activity() ? ` — ${activity()}` : ""}
                            <button class="stop-btn" data-testid="stop-turn" onClick={stopTurn}>
                                stop
                            </button>
                        </div>
                    </Show>
                </div>
                {/* The composer dock (run-chat.md B4): one bottom unit — the
                    chat-kind indicator, the stack of queued messages, and the
                    input. A chat's kind is fixed by its root (ADR 0035): an edit
                    chat improves the archetype, a work chat does the job. Shown
                    read-only — there is no toggle. */}
                <div class="composer-dock">
                    {/* The chat's kind (edit vs work) is fixed by its root (ADR 0035)
                        and surfaced read-only in the lineage header — no separate
                        composer caption (it was static filler). */}

                    {/* Queued messages stack directly on top of the box, next-to-run
                        first. Each is draggable to reorder, click-to-edit, cancellable. */}
                    <Show when={queue().length > 0}>
                        <QueueStack
                            items={queue()}
                            onReorder={reorderQueue}
                            onEdit={editQueued}
                            onRemove={removeQueued}
                            onSendNow={sendNowQueued}
                        />
                    </Show>

                    {/* Pending message attachments (UX-14): chips above the box, each
                        removable before send. Cleared once the message is sent. */}
                    <Show when={attachments().length > 0}>
                        <div class="composer-attachments" data-attachments>
                            <For each={attachments()}>
                                {(a, i) => (
                                    <span class="attachment-chip" data-attachment data-kind={a.kind}>
                                        {a.kind === "image" ? (
                                            <img class="chip-thumb" src={`data:${a.mimeType};base64,${a.data}`} alt="" />
                                        ) : (
                                            <Icon name="paperclip" />
                                        )}
                                        {a.name}
                                        <button
                                            class="chip-x"
                                            type="button"
                                            aria-label={`Remove ${a.name}`}
                                            onClick={() => removeAttachment(i())}
                                        >
                                            ×
                                        </button>
                                    </span>
                                )}
                            </For>
                        </div>
                    </Show>

                    {/* Paperclip attach (UX-14): a plain file input (works in the browser
                        and the Tauri webview), kept OUTSIDE `.composer` so the existing
                        `.composer input` steps still match only the text box. */}
                    <input
                        ref={attachInput}
                        type="file"
                        multiple
                        data-attach-input
                        style={{ display: "none" }}
                        onChange={onAttachInput}
                    />
                    {/* The per-chat model picker + reasoning-effort toggle (LLM-1, ADR 0062):
                        a thin toolbar above the input. The picker lists the models the
                        linked accounts provide (from the Pi catalog); the effort toggle
                        appears only for models that support reasoning, and its options are
                        that model's `--thinking` levels. Both write the chat's config
                        (model+provider / thinking) — the global→archetype→chat axis; the
                        empty model choice clears the override so the default resolves. */}
                    <div class="composer-toolbar">
                        <label class="model-picker" title="Model for this chat — overrides the archetype's default for this conversation only">
                            <span class="model-picker-tag" aria-hidden="true">model</span>
                            <select
                                data-model-picker
                                aria-label="Model for this chat"
                                value={modelValue()}
                                onChange={(e) => void pickModel(e.currentTarget.value)}
                            >
                                <For each={modelChoices()}>
                                    {(m) => <option value={m.id ? modelKey(m) : ""}>{m.label}</option>}
                                </For>
                            </select>
                        </label>
                        <Show when={showEffort()}>
                            <label class="model-picker effort-picker" title="Reasoning effort for this chat — higher is more deliberate (slower, costlier); Default uses the model's own setting">
                                <span class="model-picker-tag" aria-hidden="true">effort</span>
                                <select
                                    data-effort-picker
                                    aria-label="Reasoning effort for this chat"
                                    value={chatThinking()}
                                    onChange={(e) => void pickThinking(e.currentTarget.value)}
                                >
                                    <option value="">Default</option>
                                    <For each={effortLevels()}>
                                        {(lvl) => <option value={lvl}>{lvl}</option>}
                                    </For>
                                </select>
                            </label>
                        </Show>
                    </div>
                    <div class="composer">
                        <input
                            ref={composerEl}
                            placeholder={chatKind() === "edit" ? `Describe what to change about ${methodName() || "this archetype"}…` : "task the agent…"}
                            value={draft()}
                            onInput={(e) => setDraft(e.currentTarget.value)}
                            onKeyDown={(e) => e.key === "Enter" && submitDraft()}
                        />
                        {/* The paperclip sits left of the stage-gate; it opens the file
                            input above. No backend — the file's text rides the prompt. */}
                        <button
                            class="icon-btn attach-btn"
                            type="button"
                            data-attach
                            aria-label="Attach files"
                            title="Attach file(s) to this message — their text rides along with the agent (not saved to the workspace)"
                            onClick={() => attachInput?.click()}
                        >
                            <Icon name="paperclip" />
                        </button>
                        {/* The stage-gate (#24) lives right of the input, left of the
                            primary action: ⏸ hold lines messages up without running them;
                            ▶ release·N drains them in order. While held, `send` reads
                            `queue` so it's honest that nothing runs until release. Hidden
                            mid-turn — staging is an idle affordance. */}
                        <Show when={!busy()}>
                            <button
                                class="queue-gate"
                                classList={{ gated: gated() }}
                                data-queue-gate
                                title={
                                    gated()
                                        ? "Release held messages — they run in order"
                                        : "Hold messages: line several up, then release them to run in order"
                                }
                                onClick={toggleGate}
                            >
                                {gated() ? `▶ release${queue().length ? `·${queue().length}` : ""}` : "⏸ hold"}
                            </button>
                        </Show>
                        <Show
                            when={busy()}
                            fallback={
                                <button class="send-btn" onClick={submitDraft}>
                                    <Icon name={gated() ? "queue" : "send"} />
                                    {gated() ? "queue" : "send"}
                                </button>
                            }
                        >
                            <button
                                class="steer-btn"
                                data-testid="steer-turn"
                                title="Send now — interrupts the running turn and redirects the agent"
                                disabled={!hasOutgoing()}
                                onClick={steerDraft}
                            >
                                steer
                            </button>
                            <button
                                class="queue-btn"
                                data-testid="queue-msg"
                                title="Queue this message — runs after the current turn finishes"
                                disabled={!hasOutgoing()}
                                onClick={submitDraft}
                            >
                                <Icon name="queue" />
                                queue ⏎
                            </button>
                        </Show>
                    </div>
                </div>
            </Show>
        </>
    );

    // The four panes keyed for the Carousel island. The chat body is the same
    // chatPane() the desktop grid renders.
    const carouselPanes = (): Record<PaneKind, JSX.Element> => ({
        nav: navPane(),
        chat: chatPane(),
        files: filesPane(),
        content: contentPane(),
    });

    // The shared overlays (archetype settings, history shelf) sit above whichever
    // shell is active, so both the desktop grid and the mobile carousel mount them.
    const overlays = () => (
        <>
            <Show when={agentSettings()}>
                {(a) => (
                    <div class="modal-overlay" onClick={() => setAgentSettings(null)}>
                        <div onClick={(e) => e.stopPropagation()}>
                            <AgentSettings
                                api={api}
                                id={a().id}
                                name={a().name}
                                onClose={() => setAgentSettings(null)}
                            />
                        </div>
                    </div>
                )}
            </Show>

            <Show when={engagement()}>
                {(e) => (
                    <EngagementPane
                        api={api}
                        project={e().id}
                        projectName={e().name}
                        onClose={() => setEngagement(null)}
                    />
                )}
            </Show>

            <Show when={modelAccess()}>
                {(e) => (
                    <ProjectModelAccessPanel
                        api={api}
                        project={e().id}
                        projectName={e().name}
                        onClose={() => setModelAccess(null)}
                    />
                )}
            </Show>

            <Show when={projectHome()}>
                {(e) => (
                    <ProjectHomePanel
                        api={api}
                        project={e().id}
                        projectName={e().name}
                        onOpenChat={(chat) => {
                            setProjectHome(null);
                            openChat(chat as EngagementId);
                        }}
                        onClose={() => setProjectHome(null)}
                    />
                )}
            </Show>

            <Show when={forkTreeFor()}>
                {(chat) => (
                    <ForkTreePanel
                        api={api}
                        highlight={chat()}
                        onOpenChat={(id) => {
                            setForkTreeFor(null);
                            openChat(id as EngagementId);
                        }}
                        onClose={() => setForkTreeFor(null)}
                    />
                )}
            </Show>

            <Show when={selected() && showShelf()}>
                <Shelf api={api} scope={scopeId(selected()!)} id={selected()!} onClose={() => setShowShelf(false)} />
            </Show>

            <Show when={selected() && showSources()}>
                <ContextPanel
                    api={api}
                    id={selected()!}
                    refreshKey={status()}
                    onClose={() => setShowSources(false)}
                />
            </Show>
        </>
    );

    // The mobile shell: the Carousel island reused as-is, fed the same pane bodies.
    // The island owns no navigation truth — it reduces gestures into `carousel` and
    // we render only the current pane, with the shared overlays above it.
    const MobileShell = () => (
        <div class="workbench mobile" data-mobile>
            <Carousel
                state={carousel()}
                onState={setCarousel}
                panes={carouselPanes()}
                onNewChat={() => void startNewChat()}
            />
            {overlays()}
        </div>
    );

    const rightPanels = () => (
        <>
            <Show when={!contentOff()}>
                <Resizer onMove={(x) => onResize("mid", x)} />
                <CollapsiblePanel
                    cls="content"
                    fold="right"
                    title="Content"
                    collapsed={false}
                    onToggle={(v) => pinPanel(setContentOff, v)}
                >
                    {contentPane()}
                </CollapsiblePanel>
            </Show>

            <Show when={contentOff()}>
                <CollapsiblePanel
                    cls="content"
                    fold="right"
                    title="Content"
                    collapsed={true}
                    onToggle={(v) => pinPanel(setContentOff, v)}
                >
                    {contentPane()}
                </CollapsiblePanel>
            </Show>

            <Show when={!wsOff()}>
                <Resizer onMove={(x) => onResize("ws", x)} />
                <CollapsiblePanel
                    cls="workspace"
                    fold="right"
                    title="Files"
                    collapsed={false}
                    onToggle={(v) => pinPanel(setWsOff, v)}
                >
                    {filesPane()}
                </CollapsiblePanel>
            </Show>

            <Show when={wsOff()}>
                <CollapsiblePanel
                    cls="workspace"
                    fold="right"
                    title="Files"
                    collapsed={true}
                    onToggle={(v) => pinPanel(setWsOff, v)}
                >
                    {filesPane()}
                </CollapsiblePanel>
            </Show>
        </>
    );

    // The desktop grid: the four panels under the task bar (the original layout),
    // each rendering the same pane body the carousel reuses.
    const DesktopShell = () => (
        <div class="workbench" ref={observeShell} style={{ "grid-template-columns": cols() }}>
            <footer class="tasks">
                {/* Tasks are a projection over chats (incl. their titles), so the
                    queue tracks the workspace event stream (navRefresh), not just
                    local turn status — a rename re-titles the task live. */}
                <TaskBar
                    api={api}
                    selected={selected()}
                    refreshKey={navRefresh()}
                    selectedLoosening={selectedLoosening()}
                    onSelect={openChat}
                    onComplete={(id) => onMerge("admit", id)}
                />
            </footer>

            <CollapsiblePanel
                cls="nav"
                fold="left"
                title="Browse"
                collapsed={navOff()}
                onToggle={(v) => pinPanel(setNavOff, v)}
            >
                <div class="nav-stack">
                    <div class="nav-scroll">{navPane()}</div>
                    {networkBar()}
                </div>
            </CollapsiblePanel>

            <Resizer onMove={(x) => onResize("nav", x)} />

            {/* The chat lane. Its collapse button lives in its own header (chatPane),
                so when open it is a plain section; when folded it becomes a rail that
                reopens it — matching the other panels' rail behaviour. */}
            <Show
                when={!chatOff()}
                fallback={
                    <div
                        class="panel run rail"
                        data-rail="run"
                        role="button"
                        tabindex="0"
                        title="Show Chat"
                        onClick={() => setChatOff(false)}
                        onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && setChatOff(false)}
                    >
                        <span class="rail-chevron">›</span>
                        <span class="rail-label">Chat</span>
                    </div>
                }
            >
                <section class="panel run">{chatPane()}</section>
            </Show>

            {rightPanels()}

            {overlays()}
        </div>
    );

    // The desktop Session (EMBED-1): the panel-facing seam built over the shell's
    // own signals. Panels read addressing/projections/commands from here, never
    // from these globals directly, so the same panel renders unchanged inside a
    // remote, scoped embedded session. Grows as each panel is migrated.
    const desktopSession: Session = {
        api,
        engagementId: selected,
        worktreeRev: status,
        selectedFile,
        selectFile: (path) => setSelectedFile(path),
        diff: () => diff() ?? "",
        mergePhase: () => merge()?.phase ?? null,
        mergeConflicted: () => merge()?.phase === "Rejected" && merge()?.git_outcome === "Conflict",
        chatKind,
        methodName,
        transcript,
        merge: (action) => void onMerge(action),
        onContentSaved: () => void Promise.all([refetchDiff(), refetchMerge()]),
        send: sendText,
    };

    // One shell at a time: `<Show>` lazily mounts only the active branch, so the
    // inactive layout never builds (no double-mounted FacetBrowser / Workspace).
    return (
        <SessionProvider value={desktopSession}>
            {/* First-run credential gate (ADR 0075 Phase 0): overlays both shells
                until a model is connected, then dismisses itself. */}
            <Show when={showFirstRun()}>
                <FirstRunOverlay
                    api={api}
                    productName="GaugeDesk"
                    onConnected={() => {
                        void refetchStartupCreds();
                        void refetchStartupCodex();
                    }}
                    onDismiss={() => setFirstRunDismissed(true)}
                />
            </Show>
            <Show when={isMobile()} fallback={<DesktopShell />}>
                <MobileShell />
            </Show>
        </SessionProvider>
    );
}

/** The stack of queued messages sitting on top of the composer (run-chat.md B4).
 *  The top card is the next to run when the current turn settles. Each card is
 *  HTML5-draggable to reorder, click-to-edit in place (clearing it cancels), and
 *  removable. Drag is disabled on the card being edited so text selection works. */
function QueueStack(props: {
    items: QueuedMsg[];
    onReorder: (from: number, to: number) => void;
    onEdit: (id: number, text: string) => void;
    onRemove: (id: number) => void;
    onSendNow: (id: number) => void;
}) {
    const [dragIdx, setDragIdx] = createSignal<number | null>(null);
    const [overIdx, setOverIdx] = createSignal<number | null>(null);
    const [editId, setEditId] = createSignal<number | null>(null);

    function commitDrop() {
        const from = dragIdx();
        const to = overIdx();
        if (from !== null && to !== null && from !== to) props.onReorder(from, to);
        setDragIdx(null);
        setOverIdx(null);
    }

    return (
        <div class="queue-stack" data-testid="queue-stack">
            <span class="queue-cap">queued · runs after this turn</span>
            <For each={props.items}>
                {(m, i) => (
                    <div
                        class="queue-item"
                        data-testid="queue-item"
                        classList={{
                            dragging: dragIdx() === i(),
                            over: overIdx() === i() && dragIdx() !== null && dragIdx() !== i(),
                        }}
                        draggable={editId() !== m.id}
                        onDragStart={(e) => {
                            setDragIdx(i());
                            e.dataTransfer!.effectAllowed = "move";
                            e.dataTransfer!.setData("text/plain", String(m.id)); // Firefox needs payload
                        }}
                        onDragOver={(e) => {
                            e.preventDefault();
                            setOverIdx(i());
                        }}
                        onDrop={(e) => {
                            e.preventDefault();
                            commitDrop();
                        }}
                        onDragEnd={commitDrop}
                    >
                        <span class="queue-grip" title="Drag to reorder">⠿</span>
                        <span class="queue-pos">{i() + 1}</span>
                        <Show
                            when={editId() === m.id}
                            fallback={
                                <span
                                    class="queue-text"
                                    title="Click to edit"
                                    onClick={() => setEditId(m.id)}
                                >
                                    {m.text}
                                </span>
                            }
                        >
                            <input
                                class="queue-edit"
                                value={m.text}
                                ref={(el) => queueMicrotask(() => el.focus())}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter") {
                                        props.onEdit(m.id, e.currentTarget.value);
                                        setEditId(null);
                                    } else if (e.key === "Escape") {
                                        setEditId(null);
                                    }
                                }}
                                onBlur={(e) => {
                                    props.onEdit(m.id, e.currentTarget.value);
                                    setEditId(null);
                                }}
                            />
                        </Show>
                        <button
                            class="queue-send-now"
                            data-testid="queue-send-now"
                            title="Send this one now — runs immediately, ahead of the rest"
                            onClick={() => props.onSendNow(m.id)}
                        >
                            ▶
                        </button>
                        <button
                            class="queue-remove"
                            title="Cancel this queued message"
                            onClick={() => props.onRemove(m.id)}
                        >
                            ✕
                        </button>
                    </div>
                )}
            </For>
        </div>
    );
}

/** One workbench panel that can fold to a thin rail (legacy `13-chrome-ui.md` §6).
 *  Expanded, a single collapse chevron floats in the panel's top inner edge (the
 *  side facing chat), over the body's heading — it no longer occupies a row of its
 *  own. Folded, the panel becomes a clickable rail with a vertical label and an
 *  outward chevron that restores it. `fold` is the screen edge the panel tucks
 *  toward; the chevrons point that way (collapse) and back (expand). Chat is never
 *  wrapped in this — it does not collapse. */
function CollapsiblePanel(props: {
    cls: string; // "nav" | "content" | "workspace" — also the panel's grid class
    fold: "left" | "right";
    title: string;
    collapsed: boolean;
    onToggle: (v: boolean) => void;
    children: JSX.Element;
}) {
    const collapseGlyph = () => (props.fold === "left" ? "‹" : "›");
    const expandGlyph = () => (props.fold === "left" ? "›" : "‹");
    return (
        <Show
            when={!props.collapsed}
            fallback={
                <div
                    class={`panel ${props.cls} rail`}
                    data-rail={props.cls}
                    role="button"
                    tabindex="0"
                    title={`Show ${props.title}`}
                    onClick={() => props.onToggle(false)}
                    onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && props.onToggle(false)}
                >
                    <span class="rail-chevron">{expandGlyph()}</span>
                    <span class="rail-label">{props.title}</span>
                </div>
            }
        >
            <div class={`panel ${props.cls} collapsible`}>
                {/* The collapse chevron no longer owns a row of its own — it floats
                    in the panel's top inner corner (over the body's heading), so the
                    panel body starts at the very top. `fold` puts it on the inner
                    edge (right for a left-folding nav, left for right-folding panels). */}
                <button
                    class={`panel-collapse ${props.fold}`}
                    data-collapse={props.cls}
                    title={`Hide ${props.title}`}
                    aria-label={`Hide ${props.title}`}
                    onClick={() => props.onToggle(true)}
                >
                    {collapseGlyph()}
                </button>
                <div class="panel-body">{props.children}</div>
            </div>
        </Show>
    );
}

/** A vertical drag handle between two panels. Reports the live pointer x so the
 *  parent recomputes widths from geometry (no accumulation drift). */
function Resizer(props: { onMove: (clientX: number) => void }) {
    const [dragging, setDragging] = createSignal(false);
    function down(e: PointerEvent) {
        e.preventDefault();
        setDragging(true);
        document.body.style.cursor = "col-resize";
        document.body.style.userSelect = "none";
        const move = (m: PointerEvent) => props.onMove(m.clientX);
        const up = () => {
            setDragging(false);
            document.body.style.cursor = "";
            document.body.style.userSelect = "";
            window.removeEventListener("pointermove", move);
            window.removeEventListener("pointerup", up);
        };
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", up);
    }
    return (
        <div
            class="resizer"
            classList={{ dragging: dragging() }}
            role="separator"
            aria-orientation="vertical"
            onPointerDown={down}
        />
    );
}

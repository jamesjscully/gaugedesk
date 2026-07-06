/**
 * A {@link Session} implemented over a **remote, scoped** control plane for a
 * single engagement (EMBED-2) — the *second* producer of the EMBED-1 Session
 * contract, and the thing that makes the panels demonstrably context-portable:
 * the desktop's Session is built inline in `App.tsx` over its many signals; an
 * embed's is built here over a scoped embed control plane + one fixed engagement.
 *
 * It owns its OWN engagement state — the transcript snapshot+live, the run-stream
 * subscription, and the diff/merge projections — exactly the per-engagement
 * machinery the desktop keeps in its shell, condensed to one chat. Projection-first
 * (`INV-5`): everything exposed is a view or a scoped command, never authority.
 *
 * Must be created inside a Solid reactive root (the `<gw-session>` element's
 * render owns it). The returned `dispose` closes the live stream.
 */
import { createResource, createSignal } from "solid-js";
import { type EngagementId, type MergeAction } from "@gaugewright/control-plane-client";
import {
    empty,
    fromSnapshot,
    reduce,
    type Transcript,
} from "@gaugewright/workbench-ui/transcript";
import { type Session } from "@gaugewright/workbench-ui/session-context";
import { type EmbedSessionApi } from "./embed-control-plane";

export interface RemoteSessionOptions {
    /** The transport, scoped to the embed's backend (a deployment's control plane). */
    readonly api: EmbedSessionApi;
    /** The single engagement this embedded session is bound to. */
    readonly engagementId: EngagementId;
    /** Drive turns through the **public embed** route (`/embed/sessions/:id/turn`, SERVE-1) —
     *  the admitted, fail-closed, Use-mode path with lazy session activation. Default for a hosted
     *  visitor session. When `false`, turns use the desktop chat route (`/chats/:id/task`). */
    readonly publicEmbed?: boolean;
}

export function createRemoteSession(opts: RemoteSessionOptions): { session: Session; dispose: () => void } {
    const { api } = opts;
    const id = opts.engagementId;
    const [engagementId] = createSignal<EngagementId | null>(id);
    const [selectedFile, setSelectedFile] = createSignal<string | null>(null);
    const [worktreeRev, setWorktreeRev] = createSignal(0);
    const bumpWorktree = () => setWorktreeRev((n) => n + 1);

    // Transcript: durable snapshot of admitted records + the live SSE reduction of
    // the in-progress turn (`transcript.ts`) — repairable from the snapshot.
    const [snapshot, setSnapshot] = createSignal<Transcript>(empty);
    const [live, setLive] = createSignal<Transcript>(empty);
    const transcript = (): Transcript => ({
        lines: [...snapshot().lines, ...live().lines],
        openText: live().openText,
    });
    async function loadSnapshot() {
        try {
            setSnapshot(fromSnapshot(await api.getTranscript(id)));
            setLive(empty);
        } catch {
            /* a fresh engagement has no snapshot yet */
        }
    }
    void loadSnapshot();
    const unsubscribe = api.subscribe(id, (ev) => setLive((t) => reduce(t, ev)));

    // Engagement-scoped read projections (the desktop's `createResource`s, here for
    // one fixed engagement).
    const [diff, { refetch: refetchDiff }] = createResource(engagementId, (eid) => api.engagementDiff(eid));
    const [merge, { refetch: refetchMerge }] = createResource(engagementId, (eid) => api.getMerge(eid));

    // On a turn/merge settling, re-read durable truth (retiring the optimistic echo)
    // and bump the worktree rev so the files panel refetches.
    async function settle() {
        await Promise.allSettled([loadSnapshot(), refetchDiff(), refetchMerge()]);
        bumpWorktree();
    }

    function send(text: string) {
        const t = text.trim();
        if (!t) return;
        // Optimistic echo: show the user's line the instant the turn starts; the
        // snapshot re-read in settle() retires it (a failed turn drops it).
        setLive((tr) => reduce(tr, { type: "user", text: t }));
        // A hosted visitor session drives the admitted public-embed route (activate + Use-mode +
        // fail-closed); the live SSE subscription streams the tokens while this blocks.
        const turn = opts.publicEmbed ? api.runEmbedTurn(id, t) : api.runTask(id, t);
        void turn.then(settle).catch(() => void loadSnapshot());
    }

    const session: Session = {
        api,
        engagementId,
        worktreeRev,
        selectedFile,
        selectFile: (path) => setSelectedFile(path),
        diff: () => diff() ?? "",
        mergePhase: () => merge()?.phase ?? null,
        mergeConflicted: () => merge()?.phase === "Rejected" && merge()?.git_outcome === "Conflict",
        // An embedded end-user chat is a work chat; method-edit vocabulary/lineage is
        // a desktop-owner affordance, not exposed to the audience (kept as a default).
        chatKind: () => "work",
        methodName: () => "",
        transcript,
        merge: (action: MergeAction) => void api.mergeCommand(id, action).then(() => settle()),
        onContentSaved: () => void Promise.allSettled([refetchDiff(), refetchMerge()]),
        send,
    };
    return { session, dispose: unsubscribe };
}

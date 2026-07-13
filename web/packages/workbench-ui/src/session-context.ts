/**
 * Session: the scoped context a panel renders against — the seam that makes the
 * workbench panels **context-portable** (EMBED-1; [ADR 0051] §3, "Context-portable
 * panels"). A panel reads its addressing, projections, and commands from the
 * injected Session, never from a desktop global, so the *same* panel mounts
 * unchanged against either the local desktop workspace (App.tsx builds a Session
 * over its signals) or a remote, scoped embedded session (a future `<gw-session>`
 * builds one over a scoped control plane).
 *
 * Projection-first (`INV-5`): a Session exposes *projections* (accessors) and a
 * scoped *transport* (commands), never authority. Nothing here writes truth; the
 * panels remain thin renderers.
 *
 * The interface grows one accessor at a time as each panel is migrated off
 * App.tsx props — it carries exactly what a context-portable panel needs, no more.
 */
import { createContext, useContext, type Accessor } from "solid-js";
import {
    type EngagementId,
    type FileEntry,
    type MergeAction,
    type MergePhase,
    type MergePreviewResult,
    type RegionResolution,
    type SaveBase,
    type SaveFileResult,
} from "@gaugewright/control-plane-client";
import { type Transcript } from "./transcript";

export interface SessionApi {
    getFile(id: EngagementId, path: string): Promise<string>;
    /** `getFile` plus the cut the read serves (SUB-6 §12) — the base a
     *  cut-carrying save sends back. Optional: sessions without it fall
     *  back to content-based bases. */
    getFileWithCut?(
        id: EngagementId,
        path: string,
    ): Promise<{ content: string; cut: string | null }>;
    putFile(id: EngagementId, path: string, content: string): Promise<void>;
    /** Base-carrying save (SUB-6): merges concurrent changes through whip's
     *  engine; a real divergence resolves to a structured conflict payload.
     *  Fold-settled `resolutions` mint durable region memory. Optional —
     *  sessions without it fall back to the legacy putFile. */
    saveFile?(
        id: EngagementId,
        path: string,
        content: string,
        base: SaveBase,
        resolutions?: RegionResolution[],
    ): Promise<SaveFileResult>;
    /** Read-only save preview (the live fold, §12.3). Optional. */
    previewMerge?(
        id: EngagementId,
        path: string,
        draft: string,
        baseCut: string,
    ): Promise<MergePreviewResult>;
    getTree(id: EngagementId): Promise<FileEntry[]>;
    embedMyChats(): Promise<{ chat: string; title: string }[]>;
    /** The deployment's public embed config (EMBED-7 white-label). Optional: only a
     *  scoped embed session serves `/embed/config`; desktop sessions omit it. */
    embedGetConfig?(): Promise<{ white_label: boolean }>;
}

export interface Session {
    /** The control-plane transport, already scoped to this session's backend. */
    readonly api: SessionApi;
    /** The engagement (chat) this session is bound to; null when none is open. */
    readonly engagementId: Accessor<EngagementId | null>;
    /** An opaque key that changes whenever this session's worktree mutates — the
     *  refetch trigger for worktree-derived projections (the desktop wires it to
     *  the per-turn `status` signal). */
    readonly worktreeRev: Accessor<unknown>;
    /** Cross-panel file selection within this session (the content-viewer target). */
    readonly selectedFile: Accessor<string | null>;
    readonly selectFile: (path: string | null) => void;

    // Engagement-scoped read projections (`INV-5`). The desktop exposes its existing
    // resources here; an embed exposes the same shapes over its scoped session.
    /** The engagement-vs-`main` diff (the merge-review surface), "" when none. */
    readonly diff: Accessor<string>;
    /** The merge lifecycle's current phase for this engagement, null when settled/none. */
    readonly mergePhase: Accessor<MergePhase | null>;
    /** Whether a `Rejected` merge isolated because of a git **conflict** (couldn't be merged)
     *  rather than a user discard — so the UI says "conflicted", not "you discarded". */
    readonly mergeConflicted: Accessor<boolean>;
    /** Whether this chat edits a reusable method ("edit") or does work ("work") —
     *  drives keep/kept vocabulary. */
    readonly chatKind: Accessor<"work" | "edit">;
    /** The method/archetype name behind this chat, for edit-chat phrasing ("" when unknown). */
    readonly methodName: Accessor<string>;

    /** This engagement's transcript projection: the durable snapshot of admitted
     *  records concatenated with the live SSE reduction of the in-progress turn
     *  (`transcript.ts`). A view, not truth (`INV-5`). */
    readonly transcript: Accessor<Transcript>;

    // Scoped commands + refetch (the panel issues; the session admits/refetches).
    /** Drive the merge lifecycle for this engagement (keep / discard / …). */
    readonly merge: (action: MergeAction) => void;
    /** Re-read content-derived projections after an in-panel save (diff + merge). */
    readonly onContentSaved: () => void;
    /** Send a message — start a turn on this engagement. The primitive a composer
     *  rides; the desktop layers its draft/queue/steer controls on top. */
    readonly send: (text: string) => void;
}

const SessionContext = createContext<Session>();

/**
 * Read the ambient Session. Fail-closed (`INV-20`): a panel mounted outside a
 * provider is a wiring bug, not a silent degrade — we throw rather than let a
 * panel fall back to a desktop global.
 */
export function useSession(): Session {
    const session = useContext(SessionContext);
    if (!session) {
        throw new Error(
            "useSession: no Session in context — a panel must be mounted under <SessionProvider>",
        );
    }
    return session;
}

/** Provide a Session to a panel subtree. */
export const SessionProvider = SessionContext.Provider;

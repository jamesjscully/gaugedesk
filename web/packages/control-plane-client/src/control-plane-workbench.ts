import { type ProjectionCarriage, parseProjectionCarriage } from "./projection-carriage";
import { type ProjectHome, parseProjectHome } from "./project-home";
import {
    engagementId,
    isWorkspaceRecord,
    parseAuditEvent,
    parseExportState,
    parseMergeState,
    parseResourceView,
    parseReviewState,
    parseRunState,
    parseWorkspace,
    parseWorkstream,
    type StreamEvent,
    workstreamId,
} from "./control-plane-domain";
import type {
    ArchetypeId,
    AuditEvent,
    Engagement,
    EngagementId,
    ExportCommand,
    ExportState,
    FileEntry,
    HumanTask,
    MergeAction,
    MergeState,
    PlacementId,
    ProjectId,
    ResourceView,
    ReviewCommand,
    ReviewState,
    RunCommand,
    RunState,
    ScopeId,
    SearchHit,
    WorkstreamId,
    WorkstreamNode,
    Workspace,
    WorkspaceChange,
} from "./control-plane-domain";
import type { RouteJson } from "./control-plane-transport";

export interface WorkbenchTransport {
    readonly base: string;
    readonly json: RouteJson;
}

export async function getRun(transport: WorkbenchTransport, scope: ScopeId): Promise<RunState> {
    return parseRunState(await transport.json("GET", `/scopes/${scope}/run`));
}

export async function listEngagements(transport: WorkbenchTransport): Promise<EngagementId[]> {
    const o = (await transport.json("GET", "/chats")) as { engagements: string[] };
    return o.engagements.map(engagementId);
}

/** The whole nav tree: archetypes, projects, recent chats, and workstreams. */
export async function getWorkspace(transport: WorkbenchTransport): Promise<Workspace> {
    return parseWorkspace(await transport.json("GET", "/workspace"));
}

/** The workspace in its freshness carriage (ADR 0037). */
export async function getWorkspaceCarriage(
    transport: WorkbenchTransport,
): Promise<ProjectionCarriage<Workspace>> {
    const raw = (await transport.json(
        "GET",
        "/projections/library/workspace?freshness=live",
    )) as {
        value: unknown;
        freshness: { marker?: unknown; generated_at?: unknown; repair_hint?: unknown };
        client_request_id?: unknown;
    };
    return parseProjectionCarriage(raw, (v) => parseWorkspace(v));
}

/** The human task queue (top bar): onboarding issues + ask-typed chat tasks
 *  (ADR 0082 §2 — `review`/`answer`/`repair`). Chat-ask ids are engagement ids;
 *  `issue` ids are whip work-item ids (`WS-N`), so we keep `id` a raw string
 *  here and narrow on `kind` at the use site. An unknown kind degrades to
 *  `review` (the always-safe ask: it just opens the chat). */
export async function getTasks(transport: WorkbenchTransport): Promise<HumanTask[]> {
    const o = (await transport.json("GET", "/tasks")) as {
        tasks: { id: string; title: string; agent: string; kind: string; assignee?: string }[];
    };
    const kinds = new Set(["review", "answer", "repair", "reply", "issue"]);
    return o.tasks.map((t) => ({
        id: t.id,
        title: t.title,
        agent: t.agent,
        kind: (kinds.has(t.kind) ? t.kind : "review") as HumanTask["kind"],
        assignee: t.assignee,
    }));
}

/** Content search across chat transcripts (SEARCH-1) and worktree files (SEARCH-2).
 *  The server ranks log hits before file hits and emits one hit per chat via its
 *  strongest tier; the tree renders each hit's snippet in place. */
export async function search(transport: WorkbenchTransport, query: string): Promise<SearchHit[]> {
    if (!query.trim()) return [];
    const o = (await transport.json("GET", `/search?q=${encodeURIComponent(query)}`)) as {
        hits?: { id: string; title: string; snippet: string; tier?: "log" | "file"; path?: string }[];
    };
    return (o.hits ?? []).map((h) => ({
        id: engagementId(h.id),
        title: h.title,
        snippet: h.snippet,
        tier: h.tier ?? "log",
        path: h.path,
    }));
}

export async function createArchetype(
    transport: WorkbenchTransport,
    name: string,
): Promise<ArchetypeId> {
    const o = (await transport.json("POST", "/archetypes", { name })) as { id: string };
    return o.id as ArchetypeId;
}

export async function renameArchetype(
    transport: WorkbenchTransport,
    id: ArchetypeId,
    name: string,
): Promise<void> {
    await transport.json("PUT", `/archetypes/${id}`, { name });
}

export async function getArchetypeConfig(
    transport: WorkbenchTransport,
    id: ArchetypeId,
): Promise<string> {
    const o = (await transport.json("GET", `/archetypes/${id}`)) as { config: string };
    return o.config;
}

/** Save an archetype's config; a 400 means it failed boundary parse. */
export async function setArchetypeConfig(
    transport: WorkbenchTransport,
    id: ArchetypeId,
    config: string,
): Promise<void> {
    const res = await fetch(`${transport.base}/archetypes/${id}`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ config }),
    });
    if (res.status === 400) throw new Error(`invalid config: ${await res.text()}`);
    if (!res.ok) throw new Error(`PUT archetype: ${res.status}`);
}

export async function deleteArchetype(
    transport: WorkbenchTransport,
    id: ArchetypeId,
): Promise<void> {
    await transport.json("DELETE", `/archetypes/${id}`);
}

export async function forkArchetype(
    transport: WorkbenchTransport,
    id: ArchetypeId,
    name?: string,
): Promise<ArchetypeId> {
    const o = (await transport.json("POST", `/archetypes/${id}/fork`, name ? { name } : {})) as {
        id: string;
    };
    return o.id as ArchetypeId;
}

export async function pullFromSource(
    transport: WorkbenchTransport,
    id: ArchetypeId,
): Promise<void> {
    await transport.json("POST", `/archetypes/${id}/pull-from-source`, {});
}

export async function publishArchetype(
    transport: WorkbenchTransport,
    id: ArchetypeId,
    autoUpgrade?: boolean,
): Promise<{ version: number; autoUpgraded: number }> {
    const o = (await transport.json(
        "POST",
        `/archetypes/${id}/publish`,
        autoUpgrade === undefined ? {} : { auto_upgrade: autoUpgrade },
    )) as { version: number; auto_upgraded: number };
    return { version: o.version, autoUpgraded: o.auto_upgraded };
}

export async function upgradePlacement(
    transport: WorkbenchTransport,
    placementId: PlacementId,
): Promise<number> {
    const o = (await transport.json("POST", `/placements/${placementId}/upgrade`, {})) as { version: number };
    return o.version;
}

/** Accept a pending placement (APPROVE-1, ADR 0064): the owner's second act, flipping it
 *  Pending → Active so it can host work chats. */
export async function acceptPlacement(
    transport: WorkbenchTransport,
    placementId: PlacementId,
): Promise<void> {
    await transport.json("POST", `/placements/${placementId}/accept`, {});
}

export async function getPlacementConfig(
    transport: WorkbenchTransport,
    placementId: PlacementId,
): Promise<{ config: string; notes: string }> {
    const s = (await transport.json("GET", `/placements/${placementId}`)) as {
        local_config?: string | null;
        notes?: string | null;
    };
    return { config: s.local_config ?? "", notes: s.notes ?? "" };
}

export async function setPlacementConfig(
    transport: WorkbenchTransport,
    placementId: PlacementId,
    config: string,
    notes: string,
): Promise<void> {
    await transport.json("POST", `/placements/${placementId}/command`, { SetLocalConfig: { config, notes } });
}

export async function forkChat(
    transport: WorkbenchTransport,
    id: EngagementId,
): Promise<EngagementId> {
    const o = (await transport.json("POST", `/chats/${id}/fork`, {})) as { id: string };
    return o.id as EngagementId;
}

export async function revertChat(transport: WorkbenchTransport, id: EngagementId): Promise<void> {
    await transport.json("POST", `/chats/${id}/revert`, {});
}

export async function createProject(
    transport: WorkbenchTransport,
    name: string,
): Promise<ProjectId> {
    const o = (await transport.json("POST", "/projects", { name })) as { id: string };
    return o.id as ProjectId;
}

export async function renameProject(
    transport: WorkbenchTransport,
    id: ProjectId,
    name: string,
): Promise<void> {
    await transport.json("PUT", `/projects/${id}`, { name });
}

export async function setProjectNetworkIsolated(
    transport: WorkbenchTransport,
    id: ProjectId,
    isolated: boolean,
): Promise<void> {
    await transport.json("PUT", `/projects/${id}`, { network_isolated: isolated });
}

export async function deleteProject(transport: WorkbenchTransport, id: ProjectId): Promise<void> {
    await transport.json("DELETE", `/projects/${id}`);
}

export async function projectHome(transport: WorkbenchTransport, id: ProjectId): Promise<ProjectHome> {
    return parseProjectHome(await transport.json("GET", `/projects/${id}/home`));
}

export async function placeArchetype(
    transport: WorkbenchTransport,
    pid: ProjectId,
    archetypeId: ArchetypeId,
): Promise<PlacementId> {
    const o = (await transport.json("POST", `/projects/${pid}/placements`, { agent_id: archetypeId })) as {
        instance_id: string;
    };
    return o.instance_id as PlacementId;
}

export async function removePlacement(
    transport: WorkbenchTransport,
    pid: ProjectId,
    placementId: PlacementId,
): Promise<void> {
    await transport.json("DELETE", `/projects/${pid}/placements/${placementId}`);
}

export async function createChatUnderArchetype(
    transport: WorkbenchTransport,
    archetypeId: ArchetypeId,
    title: string,
): Promise<EngagementId> {
    const o = (await transport.json("POST", `/archetypes/${archetypeId}/chats`, {
        title,
    })) as { id: string };
    return engagementId(o.id);
}

export async function useArchetype(
    transport: WorkbenchTransport,
    archetypeId: ArchetypeId,
    title: string,
): Promise<EngagementId> {
    const o = (await transport.json("POST", `/archetypes/${archetypeId}/use`, {
        title,
    })) as { id: string };
    return engagementId(o.id);
}

export async function createChatUnderPlacement(
    transport: WorkbenchTransport,
    pid: ProjectId,
    placementId: PlacementId,
    title: string,
): Promise<EngagementId> {
    const o = (await transport.json("POST", `/projects/${pid}/placements/${placementId}/chats`, {
        title,
    })) as { id: string };
    return engagementId(o.id);
}

export async function renameChat(
    transport: WorkbenchTransport,
    id: EngagementId,
    title: string,
): Promise<void> {
    await transport.json("PUT", `/chats/${id}/title`, { title });
}

export async function deleteChat(transport: WorkbenchTransport, id: EngagementId): Promise<void> {
    await transport.json("DELETE", `/chats/${id}`);
}

export async function engagementDiff(transport: WorkbenchTransport, id: EngagementId): Promise<string> {
    const o = (await transport.json("GET", `/chats/${id}/diff`)) as { diff: string };
    return o.diff;
}

export async function submitRunCommand(
    transport: WorkbenchTransport,
    scope: ScopeId,
    command: RunCommand,
): Promise<RunState> {
    return parseRunState(await transport.json("POST", `/scopes/${scope}/run/command`, command));
}

export async function createEngagement(
    transport: WorkbenchTransport,
    id?: EngagementId,
): Promise<Engagement> {
    const o = (await transport.json("POST", "/chats", id ? { id } : {})) as {
        id: string;
        branch: string;
        path: string;
    };
    return { id: engagementId(o.id), branch: o.branch, path: o.path };
}

export async function runTask(
    transport: WorkbenchTransport,
    id: EngagementId,
    prompt: string,
    images: { data: string; mimeType: string }[] = [],
): Promise<unknown> {
    const body = images.length ? { prompt, images } : { prompt };
    return transport.json("POST", `/chats/${id}/task`, body);
}

export async function stopTurn(
    transport: WorkbenchTransport,
    id: EngagementId,
): Promise<{ stopped: boolean }> {
    return (await transport.json("POST", `/chats/${id}/stop`)) as { stopped: boolean };
}

export async function syncFromMain(
    transport: WorkbenchTransport,
    id: EngagementId,
): Promise<{ synced: boolean; conflict: boolean }> {
    return (await transport.json("POST", `/chats/${id}/sync`)) as { synced: boolean; conflict: boolean };
}

export async function createWorkstream(
    transport: WorkbenchTransport,
    placementId: PlacementId,
    name: string,
): Promise<WorkstreamNode> {
    const o = (await transport.json("POST", `/placements/${placementId}/workstreams`, { name })) as {
        id: string;
        name: string;
    };
    return { id: workstreamId(o.id), name: o.name, placementId, status: "active", members: [] };
}

export async function listWorkstreams(
    transport: WorkbenchTransport,
    placementId: PlacementId,
): Promise<WorkstreamNode[]> {
    const o = (await transport.json("GET", `/placements/${placementId}/workstreams`)) as {
        workstreams?: { id: string; name: string; placement_id: string; status?: string; members?: string[] }[];
    };
    return (o.workstreams ?? []).map(parseWorkstream);
}

export async function joinWorkstream(
    transport: WorkbenchTransport,
    ws: WorkstreamId,
    chat: EngagementId,
): Promise<void> {
    await transport.json("POST", `/workstreams/${ws}/join`, { chat });
}

export async function leaveWorkstream(
    transport: WorkbenchTransport,
    ws: WorkstreamId,
    chat: EngagementId,
): Promise<void> {
    await transport.json("POST", `/workstreams/${ws}/leave`, { chat });
}

export async function archiveWorkstream(
    transport: WorkbenchTransport,
    ws: WorkstreamId,
): Promise<void> {
    await transport.json("POST", `/workstreams/${ws}/archive`);
}

export async function promoteWorkstream(
    transport: WorkbenchTransport,
    ws: WorkstreamId,
): Promise<void> {
    await transport.json("POST", `/workstreams/${ws}/promote`);
}

export async function getMerge(transport: WorkbenchTransport, id: EngagementId): Promise<MergeState> {
    return parseMergeState(await transport.json("GET", `/chats/${id}/merge`));
}

export async function getMergeCarriage(
    transport: WorkbenchTransport,
    id: EngagementId,
): Promise<ProjectionCarriage<MergeState>> {
    const raw = (await transport.json("GET", `/projections/${id}/merge?freshness=live`)) as {
        value: unknown;
        freshness: { marker?: unknown; generated_at?: unknown; repair_hint?: unknown };
        client_request_id?: unknown;
    };
    return parseProjectionCarriage(raw, (v) => parseMergeState(v));
}

export async function mergeCommand(
    transport: WorkbenchTransport,
    id: EngagementId,
    action: MergeAction,
): Promise<MergeState> {
    return parseMergeState(await transport.json("POST", `/chats/${id}/merge/command`, { action }));
}

export function subscribe(
    transport: WorkbenchTransport,
    id: EngagementId,
    onEvent: (ev: StreamEvent) => void,
    onOpen?: () => void,
): () => void {
    // `withCredentials` sends the shared `.gaugewright.com` session cookie on the cross-origin
    // stream (the Console → hub), so SSE authenticates like the fetch routes (ADR 0077).
    const es = new EventSource(`${transport.base}/chats/${id}/events`, { withCredentials: true });
    if (onOpen) es.onopen = onOpen;
    es.onmessage = (m) => {
        try {
            onEvent(JSON.parse(m.data) as StreamEvent);
        } catch {
            /* ignore malformed frames */
        }
    };
    return () => es.close();
}

export function subscribeWorkspace(
    transport: WorkbenchTransport,
    onChange: (change: WorkspaceChange) => void,
): () => void {
    const es = new EventSource(`${transport.base}/workspace/events`, { withCredentials: true });
    es.onmessage = (m) => {
        try {
            const ev = JSON.parse(m.data) as {
                type?: string;
                record?: string;
                id?: string;
                op?: string;
            };
            if (ev.type === "workspacechanged" && isWorkspaceRecord(ev.record)) {
                onChange({
                    record: ev.record,
                    id: ev.id ?? "",
                    op: ev.op === "tombstone" ? "tombstone" : "upsert",
                });
            }
        } catch {
            /* ignore malformed frames */
        }
    };
    return () => es.close();
}

export async function getReview(transport: WorkbenchTransport, scope: ScopeId): Promise<ReviewState> {
    return parseReviewState(await transport.json("GET", `/scopes/${scope}/review`));
}

export async function reviewCommand(
    transport: WorkbenchTransport,
    scope: ScopeId,
    command: ReviewCommand,
): Promise<ReviewState> {
    return parseReviewState(await transport.json("POST", `/scopes/${scope}/review/command`, command));
}

export async function getExport(transport: WorkbenchTransport, scope: ScopeId): Promise<ExportState> {
    return parseExportState(await transport.json("GET", `/scopes/${scope}/export`));
}

export async function exportCommand(
    transport: WorkbenchTransport,
    scope: ScopeId,
    command: ExportCommand,
): Promise<ExportState> {
    return parseExportState(await transport.json("POST", `/scopes/${scope}/export/command`, command));
}

export async function getAudit(transport: WorkbenchTransport, scope: ScopeId): Promise<AuditEvent[]> {
    const o = (await transport.json("GET", `/scopes/${scope}/audit`)) as { events?: unknown };
    return (Array.isArray(o.events) ? o.events : []).map(parseAuditEvent);
}

export async function getResources(
    transport: WorkbenchTransport,
    id: EngagementId,
): Promise<ResourceView[]> {
    const o = (await transport.json("GET", `/chats/${id}/resources`)) as unknown[];
    return (Array.isArray(o) ? o : []).map(parseResourceView);
}

export async function getTranscript(
    transport: WorkbenchTransport,
    id: EngagementId,
): Promise<StreamEvent[]> {
    return (await transport.json("GET", `/chats/${id}/transcript`)) as StreamEvent[];
}

export async function getTree(transport: WorkbenchTransport, id: EngagementId): Promise<FileEntry[]> {
    const o = (await transport.json("GET", `/chats/${id}/tree`)) as {
        files: { path: string; is_dir: boolean }[];
    };
    return o.files.map((f) => ({ path: f.path, isDir: f.is_dir }));
}

export async function getFile(
    transport: WorkbenchTransport,
    id: EngagementId,
    path: string,
): Promise<string> {
    const res = await fetch(`${transport.base}/chats/${id}/file?path=${encodeURIComponent(path)}`);
    if (!res.ok) throw new Error(`read ${path}: ${res.status}`);
    return res.text();
}

/**
 * `getFile`, also reading the `x-workspace-cut` header — the recorded state
 * this read serves (SUB-6 §12). A cut-carrying save sends it back as the
 * three-way base; `cut` is null against servers that predate it.
 */
export async function getFileWithCut(
    transport: WorkbenchTransport,
    id: EngagementId,
    path: string,
): Promise<{ content: string; cut: string | null }> {
    const res = await fetch(`${transport.base}/chats/${id}/file?path=${encodeURIComponent(path)}`);
    if (!res.ok) throw new Error(`read ${path}: ${res.status}`);
    return { content: await res.text(), cut: res.headers.get("x-workspace-cut") };
}

export async function putFile(
    transport: WorkbenchTransport,
    id: EngagementId,
    path: string,
    content: string,
): Promise<void> {
    const res = await fetch(`${transport.base}/chats/${id}/file?path=${encodeURIComponent(path)}`, {
        method: "PUT",
        body: content,
    });
    if (!res.ok) throw new Error(`write ${path}: ${res.status}`);
}

/** One span of a merged/conflicted save (whip's text-merge piece surface). */
export type MergePiece =
    | { kind: "merged"; text: string; provenance: "base" | "ours" | "theirs" | "both" | "resolved" }
    | { kind: "conflict"; base_text: string; ours_text: string; theirs_text: string };

/** One fold-settled region riding a resolve re-save (§12.2): the exact
 *  three texts the user saw plus the text they chose or authored. The
 *  server records it as durable region-resolution memory. */
export interface RegionResolution {
    base_text: string;
    ours_text: string;
    theirs_text: string;
    resolution_text: string;
}

/** The editor's save base: the cut it loaded (preferred; the GET header),
 *  or the content it loaded (pre-cut fallback, resolved server-side). */
export type SaveBase = { cut: string } | { content: string };

export type SaveFileResult =
    | { kind: "saved"; cut: string | null }
    | { kind: "merged"; cut: string | null; content: string; pieces: MergePiece[] }
    | { kind: "conflict"; current: string; currentCut: string | null; pieces: MergePiece[] };

/**
 * Base-carrying save (SUB-6): `base` names the state the editor loaded.
 * Concurrent changes merge through whip's token-level engine server-side
 * (region memory applies); a real divergence resolves to `conflict` with
 * the structured regions and the file's current body + cut (the re-save
 * base) — nothing written. `resolutions` are fold-settled regions riding
 * a resolve re-save: they mint durable memory that pays forward.
 */
/** The JSON body a base-carrying save PUTs (shared by every transport). */
export function saveFileBody(
    content: string,
    base: SaveBase,
    resolutions: RegionResolution[] = [],
): string {
    const body: Record<string, unknown> = { content };
    if ("cut" in base) body.base_cut = base.cut;
    else body.base_content = base.content;
    if (resolutions.length) body.resolutions = resolutions;
    return JSON.stringify(body);
}

/** Decode a base-carrying save response (200 or 409) — shared by every
 *  transport so desktop, embed, and remote agree on the wire shape. */
export function decodeSaveFileResponse(status: number, payload: unknown): SaveFileResult {
    if (status === 409) {
        const conflict = payload as {
            current: string;
            current_cut?: string | null;
            pieces: MergePiece[];
        };
        return {
            kind: "conflict",
            current: conflict.current,
            currentCut: conflict.current_cut ?? null,
            pieces: conflict.pieces,
        };
    }
    const saved = payload as {
        merged?: boolean;
        cut?: string | null;
        content?: string;
        pieces?: MergePiece[];
    };
    if (saved.merged && typeof saved.content === "string") {
        return {
            kind: "merged",
            cut: saved.cut ?? null,
            content: saved.content,
            pieces: saved.pieces ?? [],
        };
    }
    return { kind: "saved", cut: saved.cut ?? null };
}

export async function saveFile(
    transport: WorkbenchTransport,
    id: EngagementId,
    path: string,
    content: string,
    base: SaveBase,
    resolutions: RegionResolution[] = [],
): Promise<SaveFileResult> {
    const res = await fetch(`${transport.base}/chats/${id}/file?path=${encodeURIComponent(path)}`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: saveFileBody(content, base, resolutions),
    });
    if (!res.ok && res.status !== 409) throw new Error(`write ${path}: ${res.status}`);
    return decodeSaveFileResponse(res.status, await res.json());
}

/** The read-only preview of a base-carrying save (the live fold, §12.3). */
export type MergePreviewResult =
    | { knownBase: false }
    | {
          knownBase: true;
          clean: boolean;
          merged: string | null;
          currentCut: string | null;
          pieces: MergePiece[];
      };

/**
 * What WOULD this draft do against the file as it stands? Nothing moves;
 * region memory applies exactly as a save would apply it. `knownBase:
 * false` means the base cut isn't recorded there (stale tab — reload).
 */
/** Decode a merge-preview response — shared by every transport. */
export function decodePreviewResponse(payload: unknown): MergePreviewResult {
    const preview = payload as {
        known_base: boolean;
        clean?: boolean;
        merged?: string | null;
        current_cut?: string | null;
        pieces?: MergePiece[];
    };
    if (!preview.known_base) return { knownBase: false };
    return {
        knownBase: true,
        clean: preview.clean ?? false,
        merged: preview.merged ?? null,
        currentCut: preview.current_cut ?? null,
        pieces: preview.pieces ?? [],
    };
}

export async function previewMerge(
    transport: WorkbenchTransport,
    id: EngagementId,
    path: string,
    draft: string,
    baseCut: string,
): Promise<MergePreviewResult> {
    const res = await fetch(`${transport.base}/chats/${id}/merge-preview`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ path, draft, base_cut: baseCut }),
    });
    if (!res.ok) throw new Error(`preview ${path}: ${res.status}`);
    return decodePreviewResponse(await res.json());
}

export async function getConfig(transport: WorkbenchTransport, id: EngagementId): Promise<string> {
    const res = await fetch(`${transport.base}/chats/${id}/config`);
    if (!res.ok) throw new Error(`GET config: ${res.status}`);
    return res.text();
}

export async function putConfig(
    transport: WorkbenchTransport,
    id: EngagementId,
    raw: string,
): Promise<void> {
    const res = await fetch(`${transport.base}/chats/${id}/config`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: raw,
    });
    if (res.status === 400) throw new Error(`invalid config: ${await res.text()}`);
    if (!res.ok) throw new Error(`PUT config: ${res.status}`);
}

export async function ingestContext(
    transport: WorkbenchTransport,
    id: EngagementId,
    path: string,
): Promise<number> {
    const o = (await transport.json("POST", `/chats/${id}/context`, { path })) as {
        ingested: number;
    };
    return o.ingested;
}

/** One uploaded context file: a name and its text content (`ENTSEC-5`). */
export interface UploadContextFile {
    name: string;
    content: string;
}

/**
 * Upload context from the client's own files rather than a server-local path
 * (`POST /chats/:id/context/upload`, `ENTSEC-5`). This is the browser's path-free
 * ingest: a native picker hands us `File`s, we read their text and upload it —
 * no absolute filesystem path (which browsers hide) is involved. Works in both
 * solo and enterprise modes; enterprise *requires* it, since the server-path
 * ingest is disabled there.
 */
export async function ingestContextUpload(
    transport: WorkbenchTransport,
    id: EngagementId,
    files: UploadContextFile[],
): Promise<number> {
    const o = (await transport.json("POST", `/chats/${id}/context/upload`, { files })) as {
        ingested: number;
    };
    return o.ingested;
}

export async function openPairing(
    transport: WorkbenchTransport,
    device: string,
    bridgeGrant: string | null,
): Promise<{ pairingId: string; bridgeGrant: string }> {
    const o = (await transport.json("POST", "/pairing-requests", {
        device,
        bridge_grant: bridgeGrant,
    })) as { pairing_id?: unknown; bridge_grant?: unknown };
    if (typeof o.pairing_id !== "string") throw new Error("pairing-requests: missing pairing_id");
    return {
        pairingId: o.pairing_id,
        bridgeGrant: typeof o.bridge_grant === "string" ? o.bridge_grant : "",
    };
}

export async function acceptBoundary(
    transport: WorkbenchTransport,
    boundaryId: string,
    participant: string,
): Promise<void> {
    await transport.json("POST", `/boundaries/${boundaryId}/accept`, { participant });
}

export async function pairingStatus(
    transport: WorkbenchTransport,
    boundaryId: string,
): Promise<unknown> {
    return transport.json("GET", `/pairing-status/${boundaryId}`);
}

import * as embedClient from "@gaugewright/control-plane-client";
import * as workbenchClient from "@gaugewright/control-plane-client";
import type {
    Engagement,
    EngagementId,
    FileEntry,
    MergeAction,
    MergeState,
    StreamEvent,
} from "@gaugewright/control-plane-client";
import {
    browserRouteJson,
    controlPlaneBase,
    type RouteJson,
} from "@gaugewright/control-plane-client";

export { controlPlaneBase };

export interface EmbedSessionApi {
    getTranscript(id: EngagementId): Promise<StreamEvent[]>;
    subscribe(id: EngagementId, onEvent: (ev: StreamEvent) => void, onOpen?: () => void): () => void;
    engagementDiff(id: EngagementId): Promise<string>;
    getMerge(id: EngagementId): Promise<MergeState>;
    runEmbedTurn(id: EngagementId, prompt: string, images?: { data: string; mimeType: string }[]): Promise<unknown>;
    runTask(id: EngagementId, prompt: string, images?: { data: string; mimeType: string }[]): Promise<unknown>;
    mergeCommand(id: EngagementId, action: MergeAction): Promise<MergeState>;
    getFile(id: EngagementId, path: string): Promise<string>;
    /** `getFile` plus the cut the read serves (SUB-6 §12) — the base a
     *  cut-carrying save sends back. Optional so test fakes and older
     *  hosts stay valid; panels fall back to content-named bases. */
    getFileWithCut?(
        id: EngagementId,
        path: string,
    ): Promise<{ content: string; cut: string | null }>;
    putFile(id: EngagementId, path: string, content: string): Promise<void>;
    /** Base-carrying save (SUB-6); fold resolutions mint region memory. */
    saveFile?(
        id: EngagementId,
        path: string,
        content: string,
        base: workbenchClient.SaveBase,
        resolutions?: workbenchClient.RegionResolution[],
    ): Promise<workbenchClient.SaveFileResult>;
    /** Read-only save preview (the live fold, §12.3). */
    previewMerge?(
        id: EngagementId,
        path: string,
        draft: string,
        baseCut: string,
    ): Promise<workbenchClient.MergePreviewResult>;
    getTree(id: EngagementId): Promise<FileEntry[]>;
    embedMyChats(): Promise<{ chat: string; title: string }[]>;
    embedGetConfig(): Promise<{ white_label: boolean }>;
}

/** Package-owned control-plane edge for the public embed bundle. */
export class EmbedControlPlane implements EmbedSessionApi {
    private bearer: string | null = null;
    private publishableKey: string | null = null;
    private readonly base: string;
    private readonly route: RouteJson;

    constructor(base = controlPlaneBase()) {
        // Normalize away any trailing slash: the panel prepends leading-slash paths
        // ("/embed/config", "/chats/:id/…"), so a trailing-slash base yields a double
        // slash ("…//embed/config") → 404. The hosted cp (exposePort) returns one, so
        // be robust here rather than trusting every base to be clean.
        this.base = base.replace(/\/+$/, "");
        this.route = browserRouteJson(this.base, {
            bearer: () => this.bearer,
            publishableKey: () => this.publishableKey,
        });
    }

    setBearer(token: string | null): void {
        this.bearer = token;
    }

    setPublishableKey(key: string | null): void {
        this.publishableKey = key;
    }

    private routeJson(): RouteJson {
        return this.route;
    }

    private workbenchTransport(): workbenchClient.WorkbenchTransport {
        return { base: this.base, json: this.routeJson() };
    }

    embedSignin(email: string): Promise<{ token: string; audience: string }> {
        return embedClient.embedSignin(this.routeJson(), email);
    }

    embedCreateChat(title: string): Promise<{ chat: string }> {
        return embedClient.embedCreateChat(this.routeJson(), title);
    }

    embedMyChats(): Promise<{ chat: string; title: string }[]> {
        return embedClient.embedMyChats(this.routeJson());
    }

    embedGetConfig(): Promise<{ white_label: boolean }> {
        return embedClient.embedGetConfig(this.routeJson());
    }

    runEmbedTurn(
        id: EngagementId,
        prompt: string,
        images: { data: string; mimeType: string }[] = [],
    ): Promise<unknown> {
        return embedClient.runEmbedTurn(this.routeJson(), id, prompt, images);
    }

    createEngagement(): Promise<Engagement> {
        return workbenchClient.createEngagement(this.workbenchTransport());
    }

    getTranscript(id: EngagementId): Promise<StreamEvent[]> {
        return workbenchClient.getTranscript(this.workbenchTransport(), id);
    }

    subscribe(id: EngagementId, onEvent: (ev: StreamEvent) => void, onOpen?: () => void): () => void {
        return workbenchClient.subscribe(this.workbenchTransport(), id, onEvent, onOpen);
    }

    engagementDiff(id: EngagementId): Promise<string> {
        return workbenchClient.engagementDiff(this.workbenchTransport(), id);
    }

    getMerge(id: EngagementId): Promise<MergeState> {
        return workbenchClient.getMerge(this.workbenchTransport(), id);
    }

    runTask(
        id: EngagementId,
        prompt: string,
        images: { data: string; mimeType: string }[] = [],
    ): Promise<unknown> {
        return workbenchClient.runTask(this.workbenchTransport(), id, prompt, images);
    }

    mergeCommand(id: EngagementId, action: MergeAction): Promise<MergeState> {
        return workbenchClient.mergeCommand(this.workbenchTransport(), id, action);
    }

    getFile(id: EngagementId, path: string): Promise<string> {
        return workbenchClient.getFile(this.workbenchTransport(), id, path);
    }

    getFileWithCut(
        id: EngagementId,
        path: string,
    ): Promise<{ content: string; cut: string | null }> {
        return workbenchClient.getFileWithCut(this.workbenchTransport(), id, path);
    }

    putFile(id: EngagementId, path: string, content: string): Promise<void> {
        return workbenchClient.putFile(this.workbenchTransport(), id, path, content);
    }

    saveFile(
        id: EngagementId,
        path: string,
        content: string,
        base: workbenchClient.SaveBase,
        resolutions?: workbenchClient.RegionResolution[],
    ): Promise<workbenchClient.SaveFileResult> {
        return workbenchClient.saveFile(
            this.workbenchTransport(),
            id,
            path,
            content,
            base,
            resolutions,
        );
    }

    previewMerge(
        id: EngagementId,
        path: string,
        draft: string,
        baseCut: string,
    ): Promise<workbenchClient.MergePreviewResult> {
        return workbenchClient.previewMerge(this.workbenchTransport(), id, path, draft, baseCut);
    }

    getTree(id: EngagementId): Promise<FileEntry[]> {
        return workbenchClient.getTree(this.workbenchTransport(), id);
    }
}

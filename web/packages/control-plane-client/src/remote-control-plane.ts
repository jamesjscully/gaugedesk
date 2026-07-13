import {
    browserRouteJson,
    type BrowserRouteJsonOptions,
} from "./browser-route-json";
import type {
    Engagement,
    EngagementId,
    FileEntry,
    StreamEvent,
} from "./control-plane-domain";
import type { RouteJson } from "./control-plane-transport";
import * as workbench from "./control-plane-workbench";

/** The placement-neutral product transport shared by desktop, managed web,
 * embeds, and future in-page environments (ADR 0076). */
export interface ControlPlane {
    setBearer(token: string | null): void;
    listEngagements(): Promise<EngagementId[]>;
    createEngagement(id?: EngagementId): Promise<Engagement>;
    deleteChat(id: EngagementId): Promise<void>;
    getTranscript(id: EngagementId): Promise<StreamEvent[]>;
    subscribe(
        id: EngagementId,
        onEvent: (event: StreamEvent) => void,
        onOpen?: () => void,
    ): () => void;
    runTask(
        id: EngagementId,
        prompt: string,
        images?: { data: string; mimeType: string }[],
    ): Promise<unknown>;
    stopTurn(id: EngagementId): Promise<{ stopped: boolean }>;
    getTree(id: EngagementId): Promise<FileEntry[]>;
    getFile(id: EngagementId, path: string): Promise<string>;
    /** `getFile` plus the cut the read serves (SUB-6 §12) — the base a
     *  cut-carrying save sends back. */
    getFileWithCut(
        id: EngagementId,
        path: string,
    ): Promise<{ content: string; cut: string | null }>;
    putFile(id: EngagementId, path: string, content: string): Promise<void>;
    /** Base-carrying save (SUB-6): concurrent changes merge server-side;
     *  divergence resolves to a structured conflict; fold-settled
     *  `resolutions` mint durable region memory. */
    saveFile(
        id: EngagementId,
        path: string,
        content: string,
        base: workbench.SaveBase,
        resolutions?: workbench.RegionResolution[],
    ): Promise<workbench.SaveFileResult>;
    /** Read-only save preview (the live fold, §12.3). */
    previewMerge(
        id: EngagementId,
        path: string,
        draft: string,
        baseCut: string,
    ): Promise<workbench.MergePreviewResult>;
    getConfig(id: EngagementId): Promise<string>;
    putConfig(id: EngagementId, raw: string): Promise<void>;
}

export interface RemoteControlPlaneOptions {
    readonly bearer?: string | null | (() => string | null);
    /** Test/server injection. Production uses credentialed browser fetch. */
    readonly route?: RouteJson;
}

/** HTTP + SSE adapter for a remotely hosted GaugeDesk placement. It carries
 * product commands/projections only; WhippleScript policy and evidence remain
 * behind the server-side runtime boundary. */
export class RemoteControlPlane implements ControlPlane {
    private bearer: string | null;
    private readonly bearerProvider?: () => string | null;
    private readonly route: RouteJson;

    constructor(
        private readonly base: string,
        options: RemoteControlPlaneOptions = {},
    ) {
        this.base = base.replace(/\/+$/, "");
        this.bearerProvider =
            typeof options.bearer === "function" ? options.bearer : undefined;
        this.bearer = typeof options.bearer === "string" ? options.bearer : null;
        const auth: BrowserRouteJsonOptions = { bearer: () => this.currentBearer() };
        this.route = options.route ?? browserRouteJson(this.base, auth);
    }

    setBearer(token: string | null): void {
        this.bearer = token;
    }

    private currentBearer(): string | null {
        return this.bearerProvider?.() ?? this.bearer;
    }

    protected routeJson(): RouteJson {
        return this.route;
    }

    protected transport(): workbench.WorkbenchTransport {
        return { base: this.base, json: this.route };
    }

    private async raw(
        method: string,
        path: string,
        body?: string,
        contentType?: string,
        allowStatuses: number[] = [],
    ): Promise<Response> {
        const response = await fetch(this.base + path, {
            method,
            headers: {
                ...(this.currentBearer()
                    ? { authorization: `Bearer ${this.currentBearer()}` }
                    : {}),
                ...(contentType ? { "content-type": contentType } : {}),
            },
            credentials: "include",
            body,
        });
        if (!response.ok && !allowStatuses.includes(response.status)) {
            throw new Error(`${method} ${path}: ${response.status}`);
        }
        return response;
    }

    listEngagements() {
        return workbench.listEngagements(this.transport());
    }

    createEngagement(id?: EngagementId) {
        return workbench.createEngagement(this.transport(), id);
    }

    deleteChat(id: EngagementId) {
        return workbench.deleteChat(this.transport(), id);
    }

    getTranscript(id: EngagementId) {
        return workbench.getTranscript(this.transport(), id);
    }

    subscribe(
        id: EngagementId,
        onEvent: (event: StreamEvent) => void,
        onOpen?: () => void,
    ) {
        return workbench.subscribe(this.transport(), id, onEvent, onOpen);
    }

    runTask(
        id: EngagementId,
        prompt: string,
        images: { data: string; mimeType: string }[] = [],
    ) {
        return workbench.runTask(this.transport(), id, prompt, images);
    }

    stopTurn(id: EngagementId) {
        return workbench.stopTurn(this.transport(), id);
    }

    getTree(id: EngagementId) {
        return workbench.getTree(this.transport(), id);
    }

    async getFile(id: EngagementId, path: string) {
        const response = await this.raw(
            "GET",
            `/chats/${id}/file?path=${encodeURIComponent(path)}`,
        );
        return response.text();
    }

    async putFile(id: EngagementId, path: string, content: string) {
        await this.raw(
            "PUT",
            `/chats/${id}/file?path=${encodeURIComponent(path)}`,
            content,
        );
    }

    async getFileWithCut(id: EngagementId, path: string) {
        const response = await this.raw(
            "GET",
            `/chats/${id}/file?path=${encodeURIComponent(path)}`,
        );
        return {
            content: await response.text(),
            cut: response.headers.get("x-workspace-cut"),
        };
    }

    async saveFile(
        id: EngagementId,
        path: string,
        content: string,
        base: workbench.SaveBase,
        resolutions?: workbench.RegionResolution[],
    ) {
        const response = await this.raw(
            "PUT",
            `/chats/${id}/file?path=${encodeURIComponent(path)}`,
            workbench.saveFileBody(content, base, resolutions),
            "application/json",
            [409],
        );
        return workbench.decodeSaveFileResponse(response.status, await response.json());
    }

    async previewMerge(id: EngagementId, path: string, draft: string, baseCut: string) {
        const response = await this.raw(
            "POST",
            `/chats/${id}/merge-preview`,
            JSON.stringify({ path, draft, base_cut: baseCut }),
            "application/json",
        );
        return workbench.decodePreviewResponse(await response.json());
    }

    async getConfig(id: EngagementId) {
        const response = await this.raw("GET", `/chats/${id}/config`);
        return response.text();
    }

    async putConfig(id: EngagementId, raw: string) {
        await this.raw("PUT", `/chats/${id}/config`, raw, "application/json");
    }
}

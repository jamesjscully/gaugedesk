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
    putFile(id: EngagementId, path: string, content: string): Promise<void>;
    getTree(id: EngagementId): Promise<FileEntry[]>;
    embedMyChats(): Promise<{ chat: string; title: string }[]>;
}

/** Package-owned control-plane edge for the public embed bundle. */
export class EmbedControlPlane implements EmbedSessionApi {
    private bearer: string | null = null;
    private publishableKey: string | null = null;
    private readonly route: RouteJson;

    constructor(private readonly base = controlPlaneBase()) {
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

    putFile(id: EngagementId, path: string, content: string): Promise<void> {
        return workbenchClient.putFile(this.workbenchTransport(), id, path, content);
    }

    getTree(id: EngagementId): Promise<FileEntry[]> {
        return workbenchClient.getTree(this.workbenchTransport(), id);
    }
}

import * as workbenchClient from "@gaugewright/control-plane-client";
import type {
    ArchetypeId,
    Engagement,
    EngagementId,
    FileEntry,
    HumanTask,
    PlacementId,
    ProjectId,
    SearchHit,
    StreamEvent,
    WorkstreamId,
    WorkstreamNode,
    Workspace,
    WorkspaceChange,
    ProjectionCarriage,
} from "@gaugewright/control-plane-client";
import {
    browserRouteJson,
    controlPlaneBase,
    type RouteJson,
} from "@gaugewright/control-plane-client";
import type { FacetBrowserApi } from "@gaugewright/workbench-ui";

export { controlPlaneBase };

/** App-owned control-plane edge for the mobile web harness. */
export class MobileControlPlane implements FacetBrowserApi {
    private readonly route: RouteJson;

    constructor(private readonly base = controlPlaneBase()) {
        this.route = browserRouteJson(this.base);
    }

    private routeJson(): RouteJson {
        return this.route;
    }

    private workbenchTransport(): workbenchClient.WorkbenchTransport {
        return { base: this.base, json: this.routeJson() };
    }

    getWorkspaceCarriage(): Promise<ProjectionCarriage<Workspace>> {
        return workbenchClient.getWorkspaceCarriage(this.workbenchTransport());
    }

    getTasks(): Promise<HumanTask[]> {
        return workbenchClient.getTasks(this.workbenchTransport());
    }

    search(query: string): Promise<SearchHit[]> {
        return workbenchClient.search(this.workbenchTransport(), query);
    }

    getPlacementConfig(placementId: PlacementId): Promise<{ config: string; notes: string }> {
        return workbenchClient.getPlacementConfig(this.workbenchTransport(), placementId);
    }

    setPlacementConfig(placementId: PlacementId, config: string, notes: string): Promise<void> {
        return workbenchClient.setPlacementConfig(this.workbenchTransport(), placementId, config, notes);
    }

    createArchetype(name: string): Promise<ArchetypeId> {
        return workbenchClient.createArchetype(this.workbenchTransport(), name);
    }

    renameArchetype(id: ArchetypeId, name: string): Promise<void> {
        return workbenchClient.renameArchetype(this.workbenchTransport(), id, name);
    }

    deleteArchetype(id: ArchetypeId): Promise<void> {
        return workbenchClient.deleteArchetype(this.workbenchTransport(), id);
    }

    forkArchetype(id: ArchetypeId, name?: string): Promise<ArchetypeId> {
        return workbenchClient.forkArchetype(this.workbenchTransport(), id, name);
    }

    pullFromSource(id: ArchetypeId): Promise<void> {
        return workbenchClient.pullFromSource(this.workbenchTransport(), id);
    }

    publishArchetype(
        id: ArchetypeId,
        autoUpgrade?: boolean,
    ): Promise<{ version: number; autoUpgraded: number }> {
        return workbenchClient.publishArchetype(this.workbenchTransport(), id, autoUpgrade);
    }

    upgradePlacement(placementId: PlacementId): Promise<number> {
        return workbenchClient.upgradePlacement(this.workbenchTransport(), placementId);
    }

    acceptPlacement(placementId: PlacementId): Promise<void> {
        return workbenchClient.acceptPlacement(this.workbenchTransport(), placementId);
    }

    createProject(name: string): Promise<ProjectId> {
        return workbenchClient.createProject(this.workbenchTransport(), name);
    }

    renameProject(id: ProjectId, name: string): Promise<void> {
        return workbenchClient.renameProject(this.workbenchTransport(), id, name);
    }

    deleteProject(id: ProjectId): Promise<void> {
        return workbenchClient.deleteProject(this.workbenchTransport(), id);
    }

    placeArchetype(pid: ProjectId, archetypeId: ArchetypeId): Promise<PlacementId> {
        return workbenchClient.placeArchetype(this.workbenchTransport(), pid, archetypeId);
    }

    removePlacement(pid: ProjectId, placementId: PlacementId): Promise<void> {
        return workbenchClient.removePlacement(this.workbenchTransport(), pid, placementId);
    }

    createChatUnderArchetype(archetypeId: ArchetypeId, title: string): Promise<EngagementId> {
        return workbenchClient.createChatUnderArchetype(this.workbenchTransport(), archetypeId, title);
    }

    createChatUnderPlacement(
        pid: ProjectId,
        placementId: PlacementId,
        title: string,
    ): Promise<EngagementId> {
        return workbenchClient.createChatUnderPlacement(this.workbenchTransport(), pid, placementId, title);
    }

    useArchetype(archetypeId: ArchetypeId, title: string): Promise<EngagementId> {
        return workbenchClient.useArchetype(this.workbenchTransport(), archetypeId, title);
    }

    createEngagement(): Promise<Engagement> {
        return workbenchClient.createEngagement(this.workbenchTransport());
    }

    forkChat(id: EngagementId): Promise<EngagementId> {
        return workbenchClient.forkChat(this.workbenchTransport(), id);
    }

    renameChat(id: EngagementId, title: string): Promise<void> {
        return workbenchClient.renameChat(this.workbenchTransport(), id, title);
    }

    deleteChat(id: EngagementId): Promise<void> {
        return workbenchClient.deleteChat(this.workbenchTransport(), id);
    }

    createWorkstream(placementId: PlacementId, name: string): Promise<WorkstreamNode> {
        return workbenchClient.createWorkstream(this.workbenchTransport(), placementId, name);
    }

    joinWorkstream(ws: WorkstreamId, chat: EngagementId): Promise<void> {
        return workbenchClient.joinWorkstream(this.workbenchTransport(), ws, chat);
    }

    leaveWorkstream(ws: WorkstreamId, chat: EngagementId): Promise<void> {
        return workbenchClient.leaveWorkstream(this.workbenchTransport(), ws, chat);
    }

    promoteWorkstream(ws: WorkstreamId): Promise<void> {
        return workbenchClient.promoteWorkstream(this.workbenchTransport(), ws);
    }

    archiveWorkstream(ws: WorkstreamId): Promise<void> {
        return workbenchClient.archiveWorkstream(this.workbenchTransport(), ws);
    }

    runTask(
        id: EngagementId,
        prompt: string,
        images: { data: string; mimeType: string }[] = [],
    ): Promise<unknown> {
        return workbenchClient.runTask(this.workbenchTransport(), id, prompt, images);
    }

    stopTurn(id: EngagementId): Promise<{ stopped: boolean }> {
        return workbenchClient.stopTurn(this.workbenchTransport(), id);
    }

    getTranscript(id: EngagementId): Promise<StreamEvent[]> {
        return workbenchClient.getTranscript(this.workbenchTransport(), id);
    }

    getTree(id: EngagementId): Promise<FileEntry[]> {
        return workbenchClient.getTree(this.workbenchTransport(), id);
    }

    subscribe(id: EngagementId, onEvent: (ev: StreamEvent) => void, onOpen?: () => void): () => void {
        return workbenchClient.subscribe(this.workbenchTransport(), id, onEvent, onOpen);
    }

    subscribeWorkspace(onChange: (change: WorkspaceChange) => void): () => void {
        return workbenchClient.subscribeWorkspace(this.workbenchTransport(), onChange);
    }

    openPairing(device: string, bridgeGrant: string | null): Promise<{ pairingId: string; bridgeGrant: string }> {
        return workbenchClient.openPairing(this.workbenchTransport(), device, bridgeGrant);
    }

    acceptBoundary(boundaryId: string, participant: string): Promise<void> {
        return workbenchClient.acceptBoundary(this.workbenchTransport(), boundaryId, participant);
    }

    pairingStatus(boundaryId: string): Promise<unknown> {
        return workbenchClient.pairingStatus(this.workbenchTransport(), boundaryId);
    }
}

import * as accountClient from "@gaugewright/control-plane-client";
import * as embedClient from "@gaugewright/control-plane-client";
import * as federationClient from "@gaugewright/control-plane-client";
import * as workbenchClient from "@gaugewright/control-plane-client";
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
    StreamEvent,
    WorkstreamId,
    WorkstreamNode,
    Workspace,
    WorkspaceChange,
    ProjectionCarriage,
    ProjectHome,
} from "@gaugewright/control-plane-client";
import {
    browserRouteJson,
    controlPlaneBase,
    type RouteJson,
} from "@gaugewright/control-plane-client";

export { controlPlaneBase };

/** App-owned control-plane edge for the open workbench shell. */
export class WorkbenchControlPlane {
    private bearer: string | null = null;
    private readonly route: RouteJson;

    constructor(private readonly base = controlPlaneBase()) {
        this.route = browserRouteJson(this.base, { bearer: () => this.bearer });
    }

    setBearer(token: string | null): void {
        this.bearer = token;
    }

    private routeJson(): RouteJson {
        return this.route;
    }

    private workbenchTransport(): workbenchClient.WorkbenchTransport {
        return { base: this.base, json: this.routeJson() };
    }

    getRun(scope: ScopeId): Promise<RunState> {
        return workbenchClient.getRun(this.workbenchTransport(), scope);
    }

    listEngagements(): Promise<EngagementId[]> {
        return workbenchClient.listEngagements(this.workbenchTransport());
    }

    getWorkspace(): Promise<Workspace> {
        return workbenchClient.getWorkspace(this.workbenchTransport());
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

    createArchetype(name: string): Promise<ArchetypeId> {
        return workbenchClient.createArchetype(this.workbenchTransport(), name);
    }

    renameArchetype(id: ArchetypeId, name: string): Promise<void> {
        return workbenchClient.renameArchetype(this.workbenchTransport(), id, name);
    }

    getArchetypeConfig(id: ArchetypeId): Promise<string> {
        return workbenchClient.getArchetypeConfig(this.workbenchTransport(), id);
    }

    setArchetypeConfig(id: ArchetypeId, config: string): Promise<void> {
        return workbenchClient.setArchetypeConfig(this.workbenchTransport(), id, config);
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

    publishArchetype(id: ArchetypeId, autoUpgrade?: boolean): Promise<{ version: number; autoUpgraded: number }> {
        return workbenchClient.publishArchetype(this.workbenchTransport(), id, autoUpgrade);
    }

    upgradePlacement(placementId: PlacementId): Promise<number> {
        return workbenchClient.upgradePlacement(this.workbenchTransport(), placementId);
    }

    acceptPlacement(placementId: PlacementId): Promise<void> {
        return workbenchClient.acceptPlacement(this.workbenchTransport(), placementId);
    }

    getPlacementConfig(placementId: PlacementId): Promise<{ config: string; notes: string }> {
        return workbenchClient.getPlacementConfig(this.workbenchTransport(), placementId);
    }

    setPlacementConfig(placementId: PlacementId, config: string, notes: string): Promise<void> {
        return workbenchClient.setPlacementConfig(this.workbenchTransport(), placementId, config, notes);
    }

    forkChat(id: EngagementId): Promise<EngagementId> {
        return workbenchClient.forkChat(this.workbenchTransport(), id);
    }

    revertChat(id: EngagementId): Promise<void> {
        return workbenchClient.revertChat(this.workbenchTransport(), id);
    }

    createProject(name: string): Promise<ProjectId> {
        return workbenchClient.createProject(this.workbenchTransport(), name);
    }

    renameProject(id: ProjectId, name: string): Promise<void> {
        return workbenchClient.renameProject(this.workbenchTransport(), id, name);
    }

    setProjectNetworkIsolated(id: ProjectId, isolated: boolean): Promise<void> {
        return workbenchClient.setProjectNetworkIsolated(this.workbenchTransport(), id, isolated);
    }

    deleteProject(id: ProjectId): Promise<void> {
        return workbenchClient.deleteProject(this.workbenchTransport(), id);
    }

    projectHome(id: ProjectId): Promise<ProjectHome> {
        return workbenchClient.projectHome(this.workbenchTransport(), id);
    }

    forkTree(): Promise<import("@gaugewright/control-plane-client").ForkNode[]> {
        return accountClient.forkTree(this.routeJson());
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

    useArchetype(archetypeId: ArchetypeId, title: string): Promise<EngagementId> {
        return workbenchClient.useArchetype(this.workbenchTransport(), archetypeId, title);
    }

    createChatUnderPlacement(pid: ProjectId, placementId: PlacementId, title: string): Promise<EngagementId> {
        return workbenchClient.createChatUnderPlacement(this.workbenchTransport(), pid, placementId, title);
    }

    renameChat(id: EngagementId, title: string): Promise<void> {
        return workbenchClient.renameChat(this.workbenchTransport(), id, title);
    }

    deleteChat(id: EngagementId): Promise<void> {
        return workbenchClient.deleteChat(this.workbenchTransport(), id);
    }

    engagementDiff(id: EngagementId): Promise<string> {
        return workbenchClient.engagementDiff(this.workbenchTransport(), id);
    }

    submitRunCommand(scope: ScopeId, command: RunCommand): Promise<RunState> {
        return workbenchClient.submitRunCommand(this.workbenchTransport(), scope, command);
    }

    createEngagement(id?: EngagementId): Promise<Engagement> {
        return workbenchClient.createEngagement(this.workbenchTransport(), id);
    }

    runTask(id: EngagementId, prompt: string, images: { data: string; mimeType: string }[] = []): Promise<unknown> {
        return workbenchClient.runTask(this.workbenchTransport(), id, prompt, images);
    }

    stopTurn(id: EngagementId): Promise<{ stopped: boolean }> {
        return workbenchClient.stopTurn(this.workbenchTransport(), id);
    }

    syncFromMain(id: EngagementId): Promise<{ synced: boolean; conflict: boolean }> {
        return workbenchClient.syncFromMain(this.workbenchTransport(), id);
    }

    createWorkstream(placementId: PlacementId, name: string): Promise<WorkstreamNode> {
        return workbenchClient.createWorkstream(this.workbenchTransport(), placementId, name);
    }

    listWorkstreams(placementId: PlacementId): Promise<WorkstreamNode[]> {
        return workbenchClient.listWorkstreams(this.workbenchTransport(), placementId);
    }

    joinWorkstream(ws: WorkstreamId, chat: EngagementId): Promise<void> {
        return workbenchClient.joinWorkstream(this.workbenchTransport(), ws, chat);
    }

    leaveWorkstream(ws: WorkstreamId, chat: EngagementId): Promise<void> {
        return workbenchClient.leaveWorkstream(this.workbenchTransport(), ws, chat);
    }

    archiveWorkstream(ws: WorkstreamId): Promise<void> {
        return workbenchClient.archiveWorkstream(this.workbenchTransport(), ws);
    }

    promoteWorkstream(ws: WorkstreamId): Promise<void> {
        return workbenchClient.promoteWorkstream(this.workbenchTransport(), ws);
    }

    getMerge(id: EngagementId): Promise<MergeState> {
        return workbenchClient.getMerge(this.workbenchTransport(), id);
    }

    getMergeCarriage(id: EngagementId): Promise<ProjectionCarriage<MergeState>> {
        return workbenchClient.getMergeCarriage(this.workbenchTransport(), id);
    }

    mergeCommand(id: EngagementId, action: MergeAction): Promise<MergeState> {
        return workbenchClient.mergeCommand(this.workbenchTransport(), id, action);
    }

    subscribe(id: EngagementId, onEvent: (ev: StreamEvent) => void, onOpen?: () => void): () => void {
        return workbenchClient.subscribe(this.workbenchTransport(), id, onEvent, onOpen);
    }

    subscribeWorkspace(onChange: (change: WorkspaceChange) => void): () => void {
        return workbenchClient.subscribeWorkspace(this.workbenchTransport(), onChange);
    }

    getReview(scope: ScopeId): Promise<ReviewState> {
        return workbenchClient.getReview(this.workbenchTransport(), scope);
    }

    reviewCommand(scope: ScopeId, command: ReviewCommand): Promise<ReviewState> {
        return workbenchClient.reviewCommand(this.workbenchTransport(), scope, command);
    }

    getExport(scope: ScopeId): Promise<ExportState> {
        return workbenchClient.getExport(this.workbenchTransport(), scope);
    }

    exportCommand(scope: ScopeId, command: ExportCommand): Promise<ExportState> {
        return workbenchClient.exportCommand(this.workbenchTransport(), scope, command);
    }

    getAudit(scope: ScopeId): Promise<AuditEvent[]> {
        return workbenchClient.getAudit(this.workbenchTransport(), scope);
    }

    getResources(id: EngagementId): Promise<ResourceView[]> {
        return workbenchClient.getResources(this.workbenchTransport(), id);
    }

    getTranscript(id: EngagementId): Promise<StreamEvent[]> {
        return workbenchClient.getTranscript(this.workbenchTransport(), id);
    }

    getTree(id: EngagementId): Promise<FileEntry[]> {
        return workbenchClient.getTree(this.workbenchTransport(), id);
    }

    getFile(id: EngagementId, path: string): Promise<string> {
        return workbenchClient.getFile(this.workbenchTransport(), id, path);
    }

    putFile(id: EngagementId, path: string, content: string): Promise<void> {
        return workbenchClient.putFile(this.workbenchTransport(), id, path, content);
    }

    getConfig(id: EngagementId): Promise<string> {
        return workbenchClient.getConfig(this.workbenchTransport(), id);
    }

    putConfig(id: EngagementId, raw: string): Promise<void> {
        return workbenchClient.putConfig(this.workbenchTransport(), id, raw);
    }

    ingestContext(id: EngagementId, path: string): Promise<number> {
        return workbenchClient.ingestContext(this.workbenchTransport(), id, path);
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

    embedMyChats(): Promise<{ chat: string; title: string }[]> {
        return embedClient.embedMyChats(this.routeJson());
    }

    mintPairingTicket(): Promise<federationClient.PairingTicket> {
        return federationClient.mintPairingTicket(this.routeJson());
    }

    pair(ticket: federationClient.PairingTicket): Promise<federationClient.FederationPeer> {
        return federationClient.pair(this.routeJson(), ticket);
    }

    listPeers(): Promise<federationClient.FederationPeer[]> {
        return federationClient.listPeers(this.routeJson());
    }

    cross(peer: string, handle: string, correlation: string): Promise<boolean> {
        return federationClient.cross(this.routeJson(), peer, handle, correlation);
    }

    remoteRun(peer: string, runScope: string, prompt: string): Promise<federationClient.RemoteRunResult> {
        return federationClient.remoteRun(this.routeJson(), peer, runScope, prompt);
    }

    federationConsent(owner: string, reviewScope: string): Promise<unknown> {
        return federationClient.federationConsent(this.routeJson(), owner, reviewScope);
    }

    federationInbox(): Promise<federationClient.FederatedFact[]> {
        return federationClient.federationInbox(this.routeJson());
    }

    handoffOffer(project: ProjectId): Promise<federationClient.HandoffStatus> {
        return federationClient.handoffOffer(this.routeJson(), project);
    }

    handoffSync(project: ProjectId): Promise<federationClient.HandoffStatus> {
        return federationClient.handoffSync(this.routeJson(), project);
    }

    handoffCommit(project: ProjectId): Promise<federationClient.HandoffStatus> {
        return federationClient.handoffCommit(this.routeJson(), project);
    }

    handoffAbort(project: ProjectId): Promise<federationClient.HandoffStatus> {
        return federationClient.handoffAbort(this.routeJson(), project);
    }

    handoffStatus(project: ProjectId): Promise<federationClient.HandoffStatus> {
        return federationClient.handoffStatus(this.routeJson(), project);
    }

    handoffRelocate(project: ProjectId, peer: string): Promise<federationClient.HandoffStatus> {
        return federationClient.handoffRelocate(this.routeJson(), project, peer);
    }

    placeRun(
        peer: string,
        project: ProjectId,
        archetype: string,
        dataHandle: string,
        prompt: string,
    ): Promise<federationClient.PlacedRun> {
        return federationClient.placeRun(this.routeJson(), peer, project, archetype, dataHandle, prompt);
    }

    runQueue(): Promise<federationClient.QueuedRun[]> {
        return federationClient.runQueue(this.routeJson());
    }

    allowRuns(project: ProjectId, operator: string, allow = true): Promise<void> {
        return federationClient.allowRuns(this.routeJson(), project, operator, allow);
    }

    denyRun(correlation: string): Promise<void> {
        return federationClient.denyRun(this.routeJson(), correlation);
    }

    admitRunOnce(correlation: string): Promise<void> {
        return federationClient.admitRunOnce(this.routeJson(), correlation);
    }

    runResult(correlation: string): Promise<federationClient.RunResult> {
        return federationClient.runResult(this.routeJson(), correlation);
    }

    invite(project: ProjectId): Promise<federationClient.EngagementInvite> {
        return federationClient.invite(this.routeJson(), project);
    }

    inviteAccept(invite: string): Promise<federationClient.InviteAcceptResult> {
        return federationClient.inviteAccept(this.routeJson(), invite);
    }

    inviteStatus(inviteId: string): Promise<federationClient.InviteStatus> {
        return federationClient.inviteStatus(this.routeJson(), inviteId);
    }

    handoffIncoming(): Promise<federationClient.IncomingHandoff[]> {
        return federationClient.handoffIncoming(this.routeJson());
    }

    handoffAccept(project: string, source: string): Promise<federationClient.HandoffStatus> {
        return federationClient.handoffAccept(this.routeJson(), project, source);
    }

    handoffDecline(project: string, source: string): Promise<void> {
        return federationClient.handoffDecline(this.routeJson(), project, source);
    }

    handoffAcceptAll(): Promise<string[]> {
        return federationClient.handoffAcceptAll(this.routeJson());
    }

    handoffPreauth(peer: string, allow = true): Promise<void> {
        return federationClient.handoffPreauth(this.routeJson(), peer, allow);
    }

    handoffParticipants(project: ProjectId): Promise<federationClient.Participant[]> {
        return federationClient.handoffParticipants(this.routeJson(), project);
    }

    handoffRevoke(project: ProjectId, authority: string, owns: string): Promise<void> {
        return federationClient.handoffRevoke(this.routeJson(), project, authority, owns);
    }

    handoffConnectData(project: ProjectId, handle: string, label?: string): Promise<void> {
        return federationClient.handoffConnectData(this.routeJson(), project, handle, label);
    }

    handoffData(project: ProjectId): Promise<federationClient.ConnectedData[]> {
        return federationClient.handoffData(this.routeJson(), project);
    }

    accountDevices(): Promise<accountClient.AccountDevice[]> {
        return accountClient.accountDevices(this.routeJson());
    }

    accountRevokeDevice(id: string): Promise<void> {
        return accountClient.accountRevokeDevice(this.routeJson(), id);
    }

    accountSettings(): Promise<Record<string, string>> {
        return accountClient.accountSettings(this.routeJson());
    }

    accountSetSetting(key: string, value: string): Promise<void> {
        return accountClient.accountSetSetting(this.routeJson(), key, value);
    }

    accountCredentials(): Promise<accountClient.LinkedProvider[]> {
        return accountClient.accountCredentials(this.routeJson());
    }

    accountLinkCredential(provider: string, token: string): Promise<void> {
        return accountClient.accountLinkCredential(this.routeJson(), provider, token);
    }

    accountUnlinkCredential(provider: string): Promise<void> {
        return accountClient.accountUnlinkCredential(this.routeJson(), provider);
    }

    projectCredentials(project: string): Promise<accountClient.LinkedProvider[]> {
        return accountClient.projectCredentials(this.routeJson(), project);
    }

    linkProjectCredential(project: string, provider: string, token: string): Promise<void> {
        return accountClient.linkProjectCredential(this.routeJson(), project, provider, token);
    }

    unlinkProjectCredential(project: string, provider: string): Promise<void> {
        return accountClient.unlinkProjectCredential(this.routeJson(), project, provider);
    }

    codexStatus(): Promise<{ linked: boolean; expires: number | null; expired: boolean }> {
        return accountClient.codexStatus(this.routeJson());
    }

    codexLoginStart(): Promise<{ url: string }> {
        return accountClient.codexLoginStart(this.routeJson());
    }
}

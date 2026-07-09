declare const brand: unique symbol;
type Brand<T, B> = T & { readonly [brand]: B };

export type ScopeId = Brand<string, "ScopeId">;
export type EngagementId = Brand<string, "EngagementId">;
/** An **archetype** — the reusable method (ADR 0035; the old "agent definition"). */
export type ArchetypeId = Brand<string, "ArchetypeId">;
export type ProjectId = Brand<string, "ProjectId">;
/** A **placement** — an archetype installed on a project (ADR 0035; the old
 *  "using instance"). Identity is `archetype · project`. */
export type PlacementId = Brand<string, "PlacementId">;
/** A **workstream** — a named shared auto-sync line within a placement (WS-F). The
 *  UI keys and compares on this id (membership, join/leave), so it is branded like
 *  the other domain ids rather than left a bare `string`. */
export type WorkstreamId = Brand<string, "WorkstreamId">;

export function scopeId(raw: string): ScopeId {
    if (!raw) throw new Error("empty ScopeId");
    return raw as ScopeId;
}
export function engagementId(raw: string): EngagementId {
    if (!raw) throw new Error("empty EngagementId");
    return raw as EngagementId;
}
export function workstreamId(raw: string): WorkstreamId {
    if (!raw) throw new Error("empty WorkstreamId");
    return raw as WorkstreamId;
}

// ----- The library facet tree (ADR 0035/0036 data model) -----

/** A chat's **kind** is its ROOT (ADR 0035), fixed at creation, never toggled:
 *  rooted on an archetype ⇒ `edit` (improve the method); rooted on a placement ⇒
 *  `work` (do the job). This replaces the old `use`/`edit` ChatMode toggle. */
export type ChatKind = "edit" | "work";

/** A chat (engagement) leaf in the nav tree. */
export interface ChatNode {
    readonly id: EngagementId;
    readonly title: string;
    readonly kind: ChatKind;
    /** The id of the workstream this chat is homed to, or `null` for the placement
     *  mainline (the default). Drives the nav membership badge (WS-F). */
    readonly workstream: WorkstreamId | null;
    /** The chat's placement (its authoring/work instance). Lets a workstream be created
     *  from this chat row, resolving the placement to the chat's own home (WS-H). */
    readonly placement: PlacementId | null;
    /** Per-chat status for the nav gem (WS-H b/c), folded from the chat's merge scope.
     *  `changes` = a finished turn's diff awaits the human's keep; `conflict` = an
     *  auto-sync / merge hit a conflict being repaired. */
    readonly changes: boolean;
    readonly conflict: boolean;
}

/** A **workstream** (WS-E): a named shared auto-sync line within a placement. Member
 *  chats greedily sync into its main; promotion to the mainline is explicit. */
export interface WorkstreamNode {
    readonly id: WorkstreamId;
    readonly name: string;
    readonly placementId: PlacementId;
    readonly status: "active" | "archived";
    /** The chat ids currently homed to this workstream. */
    readonly members: EngagementId[];
}
/** An **archetype** (library method) with its edit chats (ADR 0035). Its edit chats can
 *  collaborate on the method in a workstream too (WS-F). */
export interface ArchetypeNode {
    readonly id: ArchetypeId;
    readonly name: string;
    /** The archetype's authoring instance — the root a workstream over its edit chats is
     *  created on (WS-F). */
    readonly instanceId: PlacementId;
    readonly isDefault: boolean;
    /** The source this archetype was forked from (ADR 0038), or null for an original.
     *  A fork shares its source's history, so it can pull upstream improvements. */
    readonly forkedFrom: ArchetypeId | null;
    readonly forkedFromName: string | null;
    readonly chats: ChatNode[];
    readonly workstreams: WorkstreamNode[];
}
/** A **placement**: an archetype installed on a project, with its work chats. Its
 *  lineage (`archetypeName`) is always visible — a placement is never an orphan. */
export interface PlacementNode {
    readonly placementId: PlacementId;
    readonly archetypeId: ArchetypeId;
    readonly archetypeName: string;
    /** The project's built-in **general** placement (project-tied default): the nav hides
     *  it as a node and shows its chats directly under the project (WS-H / project.md). */
    readonly isDefault: boolean;
    /** Whether this placement carries a config-only customization (config overlay or
     *  notes) — the nav badges it so a customized client placement is legible. */
    readonly hasConfig: boolean;
    /** The pinned (installed, read-only) method version, or `null` if unpinned. */
    readonly pinnedVersion: string | null;
    /** The archetype version this placement runs (UX-9). */
    readonly version: number;
    /** The archetype's current published version. */
    readonly currentVersion: number;
    /** Whether a newer archetype version is available to upgrade to (UX-9). */
    readonly upgradeAvailable: boolean;
    /** Whether this placement is **pending approval** (APPROVE-1, ADR 0064): approved-but-
     *  not-yet-accepted under an approval-required policy. It can't host work chats until
     *  the owner accepts it; the nav flags it. Frictionless placements are never pending. */
    readonly pending: boolean;
    readonly chats: ChatNode[];
    /** The named workstreams (shared auto-sync lines) in this placement (WS-F). */
    readonly workstreams: WorkstreamNode[];
}
/** A **project** — a trust/data boundary (ADR 0036) holding its placements. The
 *  hidden default "Personal" project is filtered out by the backend. */
export interface ProjectNode {
    readonly id: ProjectId;
    readonly name: string;
    /** Network egress posture (RF-B3): `true` isolates this project's chats from
     *  the network (fail-closed); `false` (the default) lets them reach the model. */
    readonly networkIsolated: boolean;
    readonly placements: PlacementNode[];
}
/** A flat "all chats" row, tagged with the archetype it came from + its kind, and the
 *  workstream it is homed to (if any) so the Chats facet can group by workstream. */
export interface RecentChat {
    readonly id: EngagementId;
    readonly title: string;
    readonly archetype: string;
    readonly kind: ChatKind;
    readonly workstream: WorkstreamId | null;
    /** The chat's placement (its home instance), so a workstream can be created from this
     *  row in the cross-cutting Chats facet without picking a root (WS-H). */
    readonly placement: PlacementId | null;
    /** Per-chat nav-gem status (WS-H b/c); see {@link ChatNode}. */
    readonly changes: boolean;
    readonly conflict: boolean;
}
/** The whole facet tree the nav renders: a projection over the library records. The
 *  flat `workstreams` list lets the Chats facet label cross-root workstream groups. */
export interface Workspace {
    readonly archetypes: ArchetypeNode[];
    readonly projects: ProjectNode[];
    readonly recent: RecentChat[];
    readonly workstreams: WorkstreamNode[];
    /** The hidden Personal placement personal chats root on — the target for a "+ workstream"
     *  in the cross-cutting Chats facet (WS-H). */
    readonly personalPlacement: PlacementId | null;
}

/** One chat-content search hit: a chat whose **log** (SEARCH-1) or **worktree file**
 *  (SEARCH-2) matched the query, with a one-line snippet of the match. These are the
 *  server's content relevance tiers (`navigation.md`); the title tier is filtered
 *  client-side over the tree. `tier` tells log from file (log ranks above file, and the
 *  server emits at most one hit per chat via its strongest tier); `path` is the matching
 *  file for a `file` hit (the snippet already leads with it), absent for a `log` hit. */
export interface SearchHit {
    readonly id: EngagementId;
    readonly title: string;
    readonly snippet: string;
    readonly tier: "log" | "file";
    readonly path?: string;
}

/** A workspace-change **reference** pushed on the event stream (ADR 0037): what
 *  library record changed and how — never its content. The nav resolves the
 *  projection on receipt. */
export interface WorkspaceChange {
    readonly record: "archetype" | "project" | "placement" | "chat";
    readonly id: string;
    readonly op: "upsert" | "tombstone";
}

/** An event from the control-plane stream that clients reduce into a transcript. */
export type StreamEvent =
    | { type: "user"; text: string }
    | { type: "assistant"; text: string }
    | { type: "text"; delta: string }
    | { type: "tool"; tool: string; mediated: boolean; call_id?: string; target?: string; args?: string }
    | { type: "toolresult"; call_id: string; ok: boolean; result?: string }
    | { type: "blocked"; tool: string; reason: string }
    | { type: "error"; reason: string; code?: string }
    | { type: "admitted"; kind: string; text: string };

const WORKSPACE_RECORDS: readonly WorkspaceChange["record"][] = ["archetype", "project", "placement", "chat"];
/** Narrow a raw event `record` to the closed {@link WorkspaceChange} set. */
export function isWorkspaceRecord(v: unknown): v is WorkspaceChange["record"] {
    return typeof v === "string" && (WORKSPACE_RECORDS as readonly string[]).includes(v);
}

/** The kinds of task the top bar surfaces (ADR 0075 §5): a clean-merge chat
 *  awaiting keep/reject (`review`), or an onboarding checklist item from the
 *  per-boundary whip tracker (`issue`). */
export type TaskKind = "review" | "issue";

/** One item in the human task queue (the top bar). `review` tasks come from
 *  chats awaiting a keep/reject; `issue` tasks come from the account-global whip
 *  tracker (onboarding). Note `id` is an {@link EngagementId} for `review` tasks
 *  but a whip work-item id (`WS-N`) for `issue` tasks — narrow on `kind` before
 *  treating it as an engagement. */
export interface HumanTask {
    readonly id: string;
    readonly title: string;
    readonly agent: string;
    readonly kind: TaskKind;
    /** The authority this task is assigned to — v1: the acting/owner authority.
     *  Undefined = unassigned / visible to the boundary owner (ADR 0075 §4). */
    readonly assignee?: string;
}

const parseChat = (c: { id: string; title: string; kind?: ChatKind; workstream?: string | null; placement?: string | null; changes?: boolean; conflict?: boolean }): ChatNode => ({
    id: engagementId(c.id),
    title: c.title,
    kind: c.kind === "edit" ? "edit" : "work",
    workstream: c.workstream ? workstreamId(c.workstream) : null,
    placement: c.placement ? (c.placement as PlacementId) : null,
    changes: c.changes ?? false,
    conflict: c.conflict ?? false,
});

export const parseWorkstream = (w: {
    id: string;
    name: string;
    placement_id: string;
    status?: string;
    members?: string[];
}): WorkstreamNode => ({
    id: workstreamId(w.id),
    name: w.name,
    placementId: w.placement_id as PlacementId,
    status: w.status === "archived" ? "archived" : "active",
    members: (w.members ?? []).map(engagementId),
});

/** Parse the raw workspace tree (the same wire shape from `GET /workspace` and the
 *  `/projections/library/workspace` carriage value) into the branded {@link Workspace}. */
export function parseWorkspace(raw: unknown): Workspace {
    const o = (raw ?? {}) as {
        archetypes?: { id: string; name: string; instance_id?: string; is_default: boolean; forked_from?: string | null; forked_from_name?: string | null; chats: { id: string; title: string; kind?: ChatKind; workstream?: string | null }[]; workstreams?: { id: string; name: string; placement_id: string; status?: string; members?: string[] }[] }[];
        projects?: {
            id: string;
            name: string;
            network_isolated?: boolean;
            placements: {
                placement_id: string;
                archetype_id: string;
                archetype_name: string;
                is_default?: boolean;
                has_config?: boolean;
                pinned_version: string | null;
                version?: number;
                current_version?: number;
                upgrade_available?: boolean;
                pending?: boolean;
                chats: { id: string; title: string; kind?: ChatKind; workstream?: string | null }[];
                workstreams?: { id: string; name: string; placement_id: string; status?: string; members?: string[] }[];
            }[];
        }[];
        recent?: { id: string; title: string; archetype: string; kind?: ChatKind; workstream?: string | null; placement?: string | null; changes?: boolean; conflict?: boolean }[];
        workstreams?: { id: string; name: string; placement_id: string; status?: string; members?: string[] }[];
        personal_placement?: string | null;
    };
    return {
        archetypes: (o.archetypes ?? []).map((a) => ({
            id: a.id as ArchetypeId,
            name: a.name,
            instanceId: (a.instance_id ?? "") as PlacementId,
            isDefault: a.is_default,
            forkedFrom: a.forked_from ? (a.forked_from as ArchetypeId) : null,
            forkedFromName: a.forked_from_name ?? null,
            chats: a.chats.map(parseChat),
            workstreams: (a.workstreams ?? []).map(parseWorkstream),
        })),
        projects: (o.projects ?? []).map((p) => ({
            id: p.id as ProjectId,
            name: p.name,
            networkIsolated: p.network_isolated ?? false,
            placements: p.placements.map((pl) => ({
                placementId: pl.placement_id as PlacementId,
                archetypeId: pl.archetype_id as ArchetypeId,
                archetypeName: pl.archetype_name,
                isDefault: pl.is_default ?? false,
                hasConfig: pl.has_config ?? false,
                pinnedVersion: pl.pinned_version ?? null,
                version: pl.version ?? 1,
                currentVersion: pl.current_version ?? 1,
                upgradeAvailable: pl.upgrade_available ?? false,
                pending: pl.pending ?? false,
                chats: pl.chats.map(parseChat),
                workstreams: (pl.workstreams ?? []).map(parseWorkstream),
            })),
        })),
        recent: (o.recent ?? []).map((c) => ({
            id: engagementId(c.id),
            title: c.title,
            archetype: c.archetype,
            kind: c.kind === "edit" ? "edit" : "work",
            workstream: c.workstream ? workstreamId(c.workstream) : null,
            placement: c.placement ? (c.placement as PlacementId) : null,
            changes: c.changes ?? false,
            conflict: c.conflict ?? false,
        })),
        workstreams: (o.workstreams ?? []).map(parseWorkstream),
        personalPlacement: o.personal_placement ? (o.personal_placement as PlacementId) : null,
    };
}

// ----- Run lifecycle projection (mirrors gaugewright_core::run) -----

export type RunPhase =
    | "Init"
    | "Requested"
    | "Admitted"
    | "Running"
    | "Completed"
    | "Failed"
    | "Canceled";

export interface RunState {
    readonly phase: RunPhase;
    readonly admittedOnce: boolean;
}

/** Run commands the client may submit (it requests; the server decides). */
export type RunCommand =
    | "RequestRun"
    | "AdmitRun"
    | "StartRun"
    | "CompleteRun"
    | "FailRun"
    | "CancelRun"
    | "RetryRun";

export interface Engagement {
    readonly id: EngagementId;
    readonly branch: string;
    readonly path: string;
}

/** A rejected command is a receipt, not a fact (`INV-2`). */
export class Rejected extends Error {
    constructor(public readonly reason: string) {
        super(`rejected: ${reason}`);
    }
}

/** Phrase a failed command for the user, keeping the `INV-2` distinction the
 *  `Rejected` receipt models: a rejection is the *expected* "the authority
 *  declined, and here is why" outcome (surface the reason), while anything else is
 *  an unexpected transport/internal failure. `action` is an imperative phrase,
 *  e.g. `"hand off"` → `"couldn't hand off — already home"`. */
export function describeFailure(action: string, e: unknown): string {
    return e instanceof Rejected
        ? `couldn't ${action} — ${e.reason}`
        : `${action} failed — something went wrong`;
}

// ----- Review / export projections (mirror gaugewright_core::review / resource_export) -----

export type ReviewPhase = "Init" | "Proposed" | "Cleared" | "Released" | "Withheld";
export interface ReviewState {
    readonly phase: ReviewPhase;
    readonly required: string[];
    readonly consented: string[];
}
/** Review commands (externally-tagged, matching serde). */
export type ReviewCommand =
    | { Propose: { required: string[] } }
    | { Consent: string }
    | { Reject: string }
    | { Revoke: string }
    | "Release"
    | "Cancel";

export type ExportPhase = "Init" | "Requested" | "Cleared" | "Exported";
export interface ExportState {
    readonly phase: ExportPhase;
    readonly source_required: string[];
    readonly source_consented: string[];
    readonly target_admitted: boolean;
}
export type ExportCommand =
    | { ProposeExport: { source_required: string[] } }
    | { SourceConsent: string }
    | { Revoke: string }
    | { Reject: string }
    | "TargetAdmit"
    | "Export"
    | "Cancel"
    | "Expire";

/** One row of the audit timeline (`INV-6`). */
export interface AuditEvent {
    readonly position: number;
    readonly kind: string;
    readonly payload: string;
}

// ----- Durable resources projection (mirrors gaugewright_core::resource / resource_access) -----

/** A resource's **kind** — `method | context | output`, an *open* set (the core
 *  treats it as a string, `INV-12`). The UI keys its panels on the three known
 *  kinds and passes any other through verbatim. */
export type ResourceKind = "method" | "context" | "output" | (string & {});

/** The access lifecycle phase for a resource handle (mirrors `AccessPhase`): a
 *  handle conveys no payload access until `Granted` (`INV-10`). A **closed** set —
 *  the core enum has exactly these five variants — so an unknown wire value is a
 *  parse error, not a silently-rendered phase. */
export type AccessPhase = "Init" | "Requested" | "Granted" | "Revoked" | "Denied";

const ACCESS_PHASES: readonly AccessPhase[] = ["Init", "Requested", "Granted", "Revoked", "Denied"];
/** Validate a wire `access` value against the closed lifecycle (mirrors core
 *  `AccessPhase`); an unknown value throws at the edge rather than reading as a
 *  benign `Init`. */
function parseAccessPhase(v: unknown): AccessPhase {
    if (typeof v === "string" && (ACCESS_PHASES as readonly string[]).includes(v)) return v as AccessPhase;
    throw new Error(`bad access phase: ${JSON.stringify(v)}`);
}

/** One durable resource as the `GET /chats/:id/resources` projection emits it:
 *  handle + metadata only — never the payload (`INV-10`). */
export interface ResourceView {
    readonly id: string;
    readonly kind: ResourceKind;
    readonly owner: string;
    readonly stakeholders: readonly string[];
    /** The access lifecycle phase; only `Granted` resolves the payload. */
    readonly access: AccessPhase;
    /** Whether the payload was erased via the tombstone lifecycle (`INV-18`). */
    readonly tombstoned: boolean;
}

/** Parse one raw resource row (snake-ish wire shape) into the branded view. The
 *  identity (`id`) and `kind` are required strings and `access` a valid phase — a
 *  malformed row throws at the edge rather than rendering as a plausible-but-wrong
 *  resource (`id:""`, `kind:"context"`, `access:"Init"`). */
export function parseResourceView(raw: unknown): ResourceView {
    const o = (raw ?? {}) as Record<string, unknown>;
    if (typeof o.id !== "string") throw new Error("resource: expected string id");
    if (typeof o.kind !== "string") throw new Error(`resource ${o.id}: expected string kind`);
    return {
        id: o.id,
        kind: o.kind,
        owner: typeof o.owner === "string" ? o.owner : "",
        stakeholders: Array.isArray(o.stakeholders) ? o.stakeholders.map(String) : [],
        access: parseAccessPhase(o.access),
        tombstoned: Boolean(o.tombstoned),
    };
}

// ----- Merge lifecycle (the diff review surface) -----

export type MergePhase =
    | "Idle"
    | "Merging"
    | "Clean"
    | "Rejected"
    | "Repairing"
    | "Advanced"
    | "Integrated";
/** Why a `Rejected` merge isolated: a git **Conflict** (couldn't be merged) vs a user
 *  discard (`Success`/`Unknown`). Lets the UI tell "this conflicted" from "you discarded". */
export type GitOutcome = "Unknown" | "Success" | "Conflict";
export interface MergeState {
    readonly phase: MergePhase;
    readonly thread_state: string;
    readonly git_outcome: GitOutcome;
}
export type MergeAction = "admit" | "reject" | "repair" | "retry" | "integrate";

const isMergePhase = (v: unknown): v is MergePhase =>
    typeof v === "string" &&
    ["Idle", "Merging", "Clean", "Rejected", "Repairing", "Advanced", "Integrated"].includes(v);

/** Parse a raw control-plane payload into the branded {@link MergeState}. Validates
 *  the lifecycle phase at the edge (cf. {@link parseRunState}) so an out-of-set
 *  phase is a parse error here, not a silent mis-render downstream. */
export function parseMergeState(raw: unknown): MergeState {
    const o = (raw ?? {}) as Record<string, unknown>;
    if (!isMergePhase(o.phase)) throw new Error(`bad merge phase: ${JSON.stringify(raw)}`);
    const git_outcome: GitOutcome =
        o.git_outcome === "Conflict" || o.git_outcome === "Success" ? o.git_outcome : "Unknown";
    return {
        phase: o.phase,
        thread_state: typeof o.thread_state === "string" ? o.thread_state : "",
        git_outcome,
    };
}

const isReviewPhase = (v: unknown): v is ReviewPhase =>
    typeof v === "string" && ["Init", "Proposed", "Cleared", "Released", "Withheld"].includes(v);

/** Parse a raw payload into the branded {@link ReviewState} (phase-guarded edge). */
export function parseReviewState(raw: unknown): ReviewState {
    const o = (raw ?? {}) as Record<string, unknown>;
    if (!isReviewPhase(o.phase)) throw new Error(`bad review phase: ${JSON.stringify(raw)}`);
    return {
        phase: o.phase,
        required: Array.isArray(o.required) ? o.required.map(String) : [],
        consented: Array.isArray(o.consented) ? o.consented.map(String) : [],
    };
}

const isExportPhase = (v: unknown): v is ExportPhase =>
    typeof v === "string" && ["Init", "Requested", "Cleared", "Exported"].includes(v);

/** Parse a raw payload into the branded {@link ExportState} (phase-guarded edge). */
export function parseExportState(raw: unknown): ExportState {
    const o = (raw ?? {}) as Record<string, unknown>;
    if (!isExportPhase(o.phase)) throw new Error(`bad export phase: ${JSON.stringify(raw)}`);
    return {
        phase: o.phase,
        source_required: Array.isArray(o.source_required) ? o.source_required.map(String) : [],
        source_consented: Array.isArray(o.source_consented) ? o.source_consented.map(String) : [],
        target_admitted: Boolean(o.target_admitted),
    };
}

/** Parse one raw audit row into the branded {@link AuditEvent}. The timeline is
 *  `INV-6` product truth, so it is validated at the edge rather than passed raw. */
export function parseAuditEvent(raw: unknown): AuditEvent {
    const o = (raw ?? {}) as Record<string, unknown>;
    if (typeof o.position !== "number") throw new Error(`bad audit position: ${JSON.stringify(raw)}`);
    return {
        position: o.position,
        kind: typeof o.kind === "string" ? o.kind : "",
        payload: typeof o.payload === "string" ? o.payload : "",
    };
}

/** One worktree file-tree entry. */
export interface FileEntry {
    readonly path: string;
    readonly isDir: boolean;
}

const isPhase = (v: unknown): v is RunPhase =>
    typeof v === "string" &&
    ["Init", "Requested", "Admitted", "Running", "Completed", "Failed", "Canceled"].includes(v);

/** Parse a raw control-plane payload into the branded `RunState`. */
export function parseRunState(raw: unknown): RunState {
    const o = raw as Record<string, unknown>;
    if (!isPhase(o?.phase)) throw new Error(`bad run phase: ${JSON.stringify(raw)}`);
    return { phase: o.phase, admittedOnce: Boolean(o.admitted_once) };
}

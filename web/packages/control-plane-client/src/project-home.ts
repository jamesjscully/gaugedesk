/**
 * Project-home rollup (UX-2) — the typed, parsed shape of `GET /projects/:id/home`
 * (`mvp-workbench.md` "Project Home"): the per-project summary derived **from data**
 * (`INV-5`) — recent runs, output/review summaries, and an audit summary across the
 * project's placements. Parsed at the transport edge into a branded domain shape; UI code
 * consumes this, never the raw JSON (`principles.md`, "Contracts at the boundary").
 */

/** One work chat's recent-run summary (from its `RunState`). */
export interface RunSummary {
    readonly chat: string;
    readonly title: string;
    /** The run lifecycle phase (e.g. `Init`, `Running`, `Completed`, `Failed`). */
    readonly phase: string;
    /** Whether the chat has ever admitted a run (`INV-11` history bit). */
    readonly ran: boolean;
}

/** One work chat's output/review summary (from its `MergeState`) — only chats with a live
 *  (non-idle, non-clean) merge appear. */
export interface OutputSummary {
    readonly chat: string;
    readonly title: string;
    readonly phase: string;
}

/** The audit summary — counts across the project's placements/chats (references only,
 *  `INV-10`). */
export interface AuditRollup {
    readonly placements: number;
    readonly chats: number;
    readonly events: number;
}

/** The whole project-home rollup. */
export interface ProjectHome {
    readonly projectId: string;
    readonly recentRuns: readonly RunSummary[];
    readonly outputs: readonly OutputSummary[];
    readonly audit: AuditRollup;
}

function str(v: unknown): string {
    return typeof v === "string" ? v : "";
}
function num(v: unknown): number {
    return typeof v === "number" ? v : 0;
}

function parseRun(raw: unknown): RunSummary {
    const o = (raw ?? {}) as Record<string, unknown>;
    return { chat: str(o.chat), title: str(o.title), phase: str(o.phase), ran: o.ran === true };
}

function parseOutput(raw: unknown): OutputSummary {
    const o = (raw ?? {}) as Record<string, unknown>;
    return { chat: str(o.chat), title: str(o.title), phase: str(o.phase) };
}

/** Parse the raw `/projects/:id/home` envelope into the branded {@link ProjectHome}. Total
 *  (never throws): missing/odd fields degrade to empty lists / zero counts, so a partial
 *  server response renders rather than crashes the panel. */
export function parseProjectHome(raw: unknown): ProjectHome {
    const o = (raw ?? {}) as Record<string, unknown>;
    const audit = (o.audit ?? {}) as Record<string, unknown>;
    const runs = Array.isArray(o.recent_runs) ? o.recent_runs : [];
    const outputs = Array.isArray(o.outputs) ? o.outputs : [];
    return {
        projectId: str(o.project_id),
        recentRuns: runs.map(parseRun),
        outputs: outputs.map(parseOutput),
        audit: {
            placements: num(audit.placements),
            chats: num(audit.chats),
            events: num(audit.events),
        },
    };
}

/**
 * Plain-language activity log (round-6 #1).
 *
 * The raw audit timeline is the engine's event-sourcing log: numbered rows whose
 * `kind`/`payload` are internal type names and serialized JSON
 * (`run "RunRequested"`, `transcript {"type":"tool","tool":"write",…}`,
 * `resource {"resource":{"id":"out-chat-…"}}`). That is written for the engine's
 * authors, not for a layperson opening "history" to see *what the assistant did*.
 *
 * This pure reducer folds those raw rows into a short, human-readable activity
 * list — "Wrote agent-note.txt", "Kept into the shared copy" — dropping the
 * bookkeeping events (admitted/observation markers, resource registrations) that
 * carry no meaning for the user. It is deliberately conservative: anything it
 * can't confidently translate is dropped rather than shown as raw JSON, so the
 * activity list never leaks implementation-speak. The raw rows remain available
 * behind the developer "raw log" toggle for debugging.
 */

import { friendlyToolVerb } from "./tool-verb";

export interface RawAuditRow {
    readonly position: number;
    readonly kind: string;
    readonly payload: string;
}

export interface ActivityItem {
    /** Stable key for rendering (the source row position). */
    readonly key: number;
    /** The plain-language line shown to the user. */
    readonly text: string;
    /** A coarse category for styling (lifecycle vs. file action vs. message). */
    readonly tone: "you" | "did" | "kept" | "discarded";
}

/** Best-effort parse of an audit payload, which is either a JSON-quoted string
 *  (`"RunStarted"`) or a serialized JSON object (`{"type":"tool",…}`). */
function parsePayload(payload: string): unknown {
    try {
        return JSON.parse(payload);
    } catch {
        return payload;
    }
}

/** An activity item plus the internal ordering metadata used to re-sequence the
 *  list into causal order (stripped before it leaves {@link toActivity}). */
interface RankedItem extends ActivityItem {
    /** Coarse phase: 0 = turn start, 1 = the work, 2 = turn end. The raw log writes
     *  the transcript (the work) *after* the RunCompleted marker, so write-order is
     *  not causal — we re-sequence by phase, then position within a phase. */
    readonly _rank: 0 | 1 | 2;
    /** What produced the item — lets us drop the assistant's restatement of a tool
     *  action it already performed (a non-adjacent echo). */
    readonly _source: "user" | "tool" | "assistant" | "lifecycle";
    readonly _pos: number;
}

/** Translate one raw audit row into a friendly activity line, or `null` to drop
 *  it (pure bookkeeping the user shouldn't see). */
function translate(row: RawAuditRow): RankedItem | null {
    const key = row.position;
    const _pos = row.position;
    const data = parsePayload(row.payload);

    // Lifecycle markers (run "RunStarted", merge "MergeStarted", …) arrive as a
    // bare quoted string. Surface only the few that mean something to the user.
    if (typeof data === "string") {
        const tag = data.replace(/"/g, "");
        switch (tag) {
            case "RunStarted":
                return { key, text: "The assistant started working", tone: "did", _rank: 0, _source: "lifecycle", _pos };
            case "RunCompleted":
                return { key, text: "Finished this turn", tone: "did", _rank: 2, _source: "lifecycle", _pos };
            case "MergeCompleted":
            case "Integrated":
                return { key, text: "Kept into the shared copy", tone: "kept", _rank: 2, _source: "lifecycle", _pos };
            case "Rejected":
            case "MergeRejected":
                return { key, text: "Discarded — nothing was kept", tone: "discarded", _rank: 2, _source: "lifecycle", _pos };
            default:
                // RunRequested / RunAdmitted / MergeStarted / GitCleaned / … are
                // internal bookkeeping — drop rather than show the raw type name.
                return null;
        }
    }

    if (data && typeof data === "object") {
        const o = data as Record<string, unknown>;
        const type = typeof o.type === "string" ? (o.type as string) : undefined;

        // Transcript records carry the real activity: user/assistant messages and
        // tool calls. Everything else under `transcript` (admitted markers, etc.) is
        // bookkeeping.
        if (row.kind === "transcript" || type) {
            if (type === "user" && typeof o.text === "string") {
                return { key, text: `You asked: “${truncate(o.text)}”`, tone: "you", _rank: 0, _source: "user", _pos };
            }
            if (type === "assistant" && typeof o.text === "string" && o.text.trim()) {
                return { key, text: truncate(o.text), tone: "did", _rank: 1, _source: "assistant", _pos };
            }
            if (type === "tool" && typeof o.tool === "string") {
                const verb = friendlyToolVerb(o.tool);
                const target = typeof o.target === "string" ? o.target : undefined;
                return { key, text: target ? `${verb} ${target}` : verb, tone: "did", _rank: 1, _source: "tool", _pos };
            }
            if (type === "blocked" && typeof o.tool === "string") {
                return { key, text: `Blocked: ${friendlyToolVerb(o.tool)}`, tone: "did", _rank: 1, _source: "tool", _pos };
            }
            // type === "toolresult" / "admitted" / resource registrations → drop.
            return null;
        }
        return null;
    }

    return null;
}

function truncate(s: string, max = 80): string {
    const t = s.trim().replace(/\s+/g, " ");
    return t.length > max ? t.slice(0, max - 1) + "…" : t;
}

/** Normalise an activity line for de-duplication: a tool action ("Wrote
 *  agent-note.txt") and the assistant's restatement of it ("Wrote agent-note.txt.")
 *  are different source events but read as a duplicate, so collapse them. */
function normalize(text: string): string {
    return text.trim().toLowerCase().replace(/[.…\s]+$/g, "");
}

/** Fold the raw audit rows into the plain-language activity list, **in causal
 *  order**. The raw log writes the transcript (the actual work) *after* the
 *  RunCompleted marker, so neither fetch-order nor `position` is causal — we
 *  re-sequence by phase (start → work → end), then by position within a phase,
 *  so "Finished this turn" lands after the actions it finished (round-12 B).
 *  The assistant's end-of-turn restatement of a tool action is dropped as an echo. */
export function toActivity(rows: readonly RawAuditRow[]): ActivityItem[] {
    const items: RankedItem[] = [];
    for (const row of rows) {
        const it = translate(row);
        if (it) items.push(it);
    }
    // Stable sort (Array.sort is stable): phase first, then write-position.
    items.sort((a, b) => a._rank - b._rank || a._pos - b._pos);

    const toolTexts = new Set(items.filter((i) => i._source === "tool").map((i) => normalize(i.text)));
    const out: ActivityItem[] = [];
    for (const it of items) {
        // The assistant often closes a turn by restating what it just did ("Wrote
        // agent-note.txt.") — drop that when a tool row already reported the action.
        if (it._source === "assistant" && toolTexts.has(normalize(it.text))) continue;
        const prev = out[out.length - 1];
        if (prev && normalize(prev.text) === normalize(it.text)) continue;
        out.push({ key: it.key, text: it.text, tone: it.tone }); // clean public shape
    }
    return out;
}

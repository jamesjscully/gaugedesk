/**
 * Group a chat list by workstream (WS-F). A workstream is a shared auto-sync line of the
 * **same work**: its members are co-rooted chats. The browse pane groups chats by their
 * workstream **everywhere** chats are listed (Chats / Projects / Library) — this is the
 * pure reducer behind that, kept out of the renderer so it is unit-testable.
 *
 * Given a chat list and the candidate workstreams (a root's workstreams in the Projects/
 * Library facets, or the flat all-workstreams list in the Chats facet), it returns one
 * group per **active** workstream (shown even when empty, so it can still be joined /
 * promoted / archived) plus the ungrouped tail (chats homed to the mainline, or to an
 * archived/foreign workstream).
 */

import type { WorkstreamId, WorkstreamNode } from "@gaugewright/control-plane-client";

export interface ChatLike {
    readonly workstream?: WorkstreamId | null;
}

export interface WorkstreamGroup<T extends ChatLike> {
    readonly ws: WorkstreamNode;
    readonly chats: T[];
}

export interface GroupedChats<T extends ChatLike> {
    /** One per active workstream (in the candidates' order), each with its member chats. */
    readonly groups: WorkstreamGroup<T>[];
    /** Chats not in any active candidate workstream — the mainline tail. */
    readonly ungrouped: T[];
}

export function groupChatsByWorkstream<T extends ChatLike>(
    chats: T[],
    workstreams: WorkstreamNode[],
): GroupedChats<T> {
    const active = workstreams.filter((w) => w.status === "active");
    const groups: WorkstreamGroup<T>[] = active.map((ws) => ({ ws, chats: [] as T[] }));
    const indexOf = new Map(active.map((w, i) => [w.id, i] as const));
    const ungrouped: T[] = [];
    for (const c of chats) {
        const gi = c.workstream != null ? indexOf.get(c.workstream) : undefined;
        if (gi !== undefined) groups[gi].chats.push(c);
        else ungrouped.push(c);
    }
    return { groups, ungrouped };
}

/** Whether grouping is worth rendering at all: there is at least one active workstream. */
export function hasWorkstreams(workstreams: WorkstreamNode[]): boolean {
    return workstreams.some((w) => w.status === "active");
}

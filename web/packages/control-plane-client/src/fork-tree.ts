/**
 * The **fork forest** (`UX-8`) — the typed, parsed shape of `GET /fork-tree`: the live chats
 * nested by their `forked_from` lineage (roots = an original chat or one whose parent is
 * gone). A derived, read-only projection (`INV-5`); the client renders it, never folds it.
 */

import type { RouteJson } from "./control-plane-transport";

/** One node in the fork forest: a chat plus its fork children, nested. */
export interface ForkNode {
    readonly id: string;
    readonly title: string;
    readonly children: readonly ForkNode[];
}

function str(v: unknown): string {
    return typeof v === "string" ? v : "";
}

/** Parse a raw fork node (total — missing/odd fields degrade; never throws). */
export function parseForkNode(raw: unknown): ForkNode {
    const o = (raw ?? {}) as Record<string, unknown>;
    const children = Array.isArray(o.children) ? o.children.map(parseForkNode) : [];
    return { id: str(o.id), title: str(o.title), children };
}

/** Parse the `{ forest: [...] }` envelope into the typed forest. */
export function parseForkForest(raw: unknown): ForkNode[] {
    const o = (raw ?? {}) as Record<string, unknown>;
    return Array.isArray(o.forest) ? o.forest.map(parseForkNode) : [];
}

/** Fetch the fork forest — the live chats nested by fork lineage (UX-8). */
export async function forkTree(json: RouteJson): Promise<ForkNode[]> {
    return parseForkForest(await json("GET", "/fork-tree"));
}

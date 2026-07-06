/**
 * **Fork tree** (`UX-8`): a read-only visualization of the chat **fork forest** — the live
 * chats nested by their `forked_from` lineage (`GET /fork-tree`, `INV-5`). Opened from a
 * chat's "fork tree" menu; clicking a node opens that chat. A pure, total renderer over the
 * parsed forest.
 */

import { createResource, For, Show, type JSX } from "solid-js";
import type { ForkNode } from "@gaugewright/control-plane-client";

export interface ForkTreeApi {
    forkTree(): Promise<ForkNode[]>;
}

function ForkNodes(props: {
    nodes: readonly ForkNode[];
    highlight?: string;
    onOpen: (id: string) => void;
}): JSX.Element {
    return (
        <ul class="fork-tree-list">
            <For each={props.nodes}>
                {(node) => (
                    <li class="fork-tree-node" data-fork-node={node.id}>
                        <button
                            type="button"
                            class="fork-tree-label"
                            classList={{ "is-current": node.id === props.highlight }}
                            title={`open ${node.title || node.id}`}
                            onClick={() => props.onOpen(node.id)}
                        >
                            {node.title || "untitled chat"}
                        </button>
                        <Show when={node.children.length > 0}>
                            <ForkNodes nodes={node.children} highlight={props.highlight} onOpen={props.onOpen} />
                        </Show>
                    </li>
                )}
            </For>
        </ul>
    );
}

export function ForkTreePanel(props: {
    api: ForkTreeApi;
    /** The chat whose menu opened this — highlighted in the tree. */
    highlight?: string;
    onOpenChat: (id: string) => void;
    onClose: () => void;
}): JSX.Element {
    const [forest] = createResource(() => props.api.forkTree());

    return (
        <div class="modal-overlay" onClick={() => props.onClose()}>
            <div
                class="modal fork-tree-panel"
                data-fork-tree
                role="dialog"
                aria-label="fork tree"
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => e.key === "Escape" && props.onClose()}
            >
                <div class="modal-head">
                    <h3>Fork tree</h3>
                    <button type="button" onClick={() => props.onClose()}>
                        ×
                    </button>
                </div>
                <section class="admin-section">
                    <p class="muted">
                        Chats nested by where they were forked from. Click a chat to open it.
                    </p>
                    <Show
                        when={(forest() ?? []).length > 0}
                        fallback={<p class="muted">No chats yet.</p>}
                    >
                        <ForkNodes
                            nodes={forest() ?? []}
                            highlight={props.highlight}
                            onOpen={(id) => {
                                props.onClose();
                                props.onOpenChat(id);
                            }}
                        />
                    </Show>
                </section>
            </div>
        </div>
    );
}

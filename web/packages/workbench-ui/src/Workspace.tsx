/**
 * The WORKSPACE panel (4th column, `navigation.md`): the active engagement's
 * worktree files. Selecting a file retargets the content viewer. Protected
 * method resources are marked 🔒 (visibility ≠ access, `INV-10`).
 */

import { createMemo, createResource, createSignal, For, Show } from "solid-js";
import { useSession } from "./session-context";
import { LoadError } from "./LoadError";

const isProtected = (path: string) => path.startsWith("builder_only/") || path.includes("/builder_only/");

// Config/plumbing artifacts (#6): dotfiles like `.agent-config.json` are
// implementation, not the user's deliverable. Hidden from the Files list by
// default — the review should surface the content the user cares about, not config.
const isInternal = (path: string) => path.split("/").some((seg) => seg.startsWith("."));

export function Workspace() {
    const session = useSession();
    const [tree, { refetch }] = createResource(
        () => [session.engagementId(), session.worktreeRev()] as const,
        ([id]) => (id ? session.api.getTree(id) : Promise.resolve([])),
    );
    const [showInternal, setShowInternal] = createSignal(false);
    const allFiles = () => (tree() ?? []).filter((e) => !e.isDir);
    const files = () => (showInternal() ? allFiles() : allFiles().filter((e) => !isInternal(e.path)));
    const hiddenCount = createMemo(() => allFiles().filter((e) => isInternal(e.path)).length);

    return (
        <Show when={!tree.error} fallback={<LoadError what="the files" onRetry={() => void refetch()} />}>
        <Show when={tree()} fallback={<div class="status">loading…</div>}>
            <Show when={files().length} fallback={<div class="status">No files yet. Use "add files" above, or the agent will create them as it works.</div>}>
                <div class="filetree" data-worktree>
                    <For each={files()}>
                        {(e) => (
                            <div
                                class="file"
                                classList={{ active: session.selectedFile() === e.path, locked: isProtected(e.path) }}
                                onClick={() => session.selectFile(e.path)}
                            >
                                {isProtected(e.path) ? "🔒 " : ""}
                                {e.path}
                            </div>
                        )}
                    </For>
                </div>
            </Show>
            {/* Progressive disclosure (#6): the config plumbing is reachable but quiet. */}
            <Show when={hiddenCount() > 0}>
                <button
                    class="show-internal"
                    data-show-internal
                    onClick={() => setShowInternal((v) => !v)}
                >
                    {showInternal()
                        ? "hide internal files"
                        : `show ${hiddenCount()} internal file${hiddenCount() === 1 ? "" : "s"}`}
                </button>
            </Show>
        </Show>
        </Show>
    );
}

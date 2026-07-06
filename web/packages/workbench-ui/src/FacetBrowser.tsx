/**
 * The facet browser (`navigation.md` B2, ADR 0035/0036): the nav is
 * **project-first** — facets **Chats | Projects | Library**, defaulting to
 * Projects. It renders a projection (`GET /workspace`) and submits commands; it
 * never owns truth (`INV-5`).
 *
 * The model (ADR 0035): an **archetype** is the reusable, named behaviour shown
 * in the Library (the UI calls it an "archetype" throughout); a
 * **placement** is an archetype installed on a **project** (Projects) — what you
 * chat with to do work. A chat's **kind** is its ROOT, fixed at creation: rooted
 * on an archetype ⇒ an **edit** chat; rooted on a placement ⇒ a **work** chat.
 * There is no mode toggle. The default "Personal" project is hidden by the backend.
 *
 * The archetype↔placement relation is many-to-many and navigable **both ways**
 * (the C1 pivot): a placement shows its archetype's name as lineage, and an
 * archetype lists everywhere it is placed.
 */

import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import {
    Rejected,
    type ArchetypeId,
    type Engagement,
    type EngagementId,
    type PlacementId,
    type ProjectId,
    type SearchHit,
    type Workspace,
    type WorkstreamId,
    type WorkstreamNode,
} from "@gaugewright/control-plane-client";
import type { ProjectionCarriage } from "@gaugewright/control-plane-client";
import { ContextMenu, type MenuState } from "./ContextMenu";
import { displayChatTitle, untitledTag } from "./chat-title";
import { type ChatRunTone } from "./chat-run-state";
import { StatusGem } from "./StatusGem";
import { forkSource } from "./fork-lineage";
import { groupChatsByWorkstream } from "./workstream-grouping";
import {
    archetypeVisible,
    childrenFor,
    markMatch,
    placementVisible,
    projectVisible,
    searching as isSearching,
} from "./facet-filter";

type Facet = "projects" | "library" | "all-chats";

/** Plain-language blast radius for deleting a non-empty project (round-6 #6):
 *  state what else goes before the confirming click, instead of a blind "delete". */
function projectBlastRadius(p: { placements: { chats: unknown[] }[] }): string | undefined {
    const methods = p.placements.length;
    const chats = p.placements.reduce((n, pl) => n + pl.chats.length, 0);
    if (methods === 0 && chats === 0) return undefined; // empty project — no warning needed
    const parts: string[] = [];
    if (methods > 0) parts.push(`${methods} archetype${methods === 1 ? "" : "s"}`);
    if (chats > 0) parts.push(`${chats} chat${chats === 1 ? "" : "s"}`);
    return `also removes ${parts.join(" and ")} — this can't be undone`;
}

const FACETS: { id: Facet; label: string }[] = [
    { id: "all-chats", label: "Chats" },
    { id: "projects", label: "Projects" },
    { id: "library", label: "Library" },
];

export interface FacetBrowserApi {
    getWorkspaceCarriage(): Promise<ProjectionCarriage<Workspace>>;
    search(query: string): Promise<SearchHit[]>;
    getPlacementConfig(placementId: PlacementId): Promise<{ config: string; notes: string }>;
    setPlacementConfig(placementId: PlacementId, config: string, notes: string): Promise<void>;
    createArchetype(name: string): Promise<ArchetypeId>;
    createProject(name: string): Promise<ProjectId>;
    renameArchetype(id: ArchetypeId, name: string): Promise<void>;
    renameProject(id: ProjectId, name: string): Promise<void>;
    renameChat(id: EngagementId, title: string): Promise<void>;
    createWorkstream(placementId: PlacementId, name: string): Promise<WorkstreamNode>;
    joinWorkstream(ws: WorkstreamId, chat: EngagementId): Promise<void>;
    leaveWorkstream(ws: WorkstreamId, chat: EngagementId): Promise<void>;
    promoteWorkstream(ws: WorkstreamId): Promise<void>;
    archiveWorkstream(ws: WorkstreamId): Promise<void>;
    createChatUnderArchetype(archetypeId: ArchetypeId, title: string): Promise<EngagementId>;
    createChatUnderPlacement(pid: ProjectId, placementId: PlacementId, title: string): Promise<EngagementId>;
    useArchetype(archetypeId: ArchetypeId, title: string): Promise<EngagementId>;
    createEngagement(): Promise<Engagement>;
    deleteChat(id: EngagementId): Promise<void>;
    forkChat(id: EngagementId): Promise<EngagementId>;
    deleteProject(id: ProjectId): Promise<void>;
    upgradePlacement(placementId: PlacementId): Promise<number>;
    acceptPlacement(placementId: PlacementId): Promise<void>;
    removePlacement(pid: ProjectId, placementId: PlacementId): Promise<void>;
    publishArchetype(id: ArchetypeId, autoUpgrade?: boolean): Promise<{ version: number; autoUpgraded: number }>;
    forkArchetype(id: ArchetypeId, name?: string): Promise<ArchetypeId>;
    pullFromSource(id: ArchetypeId): Promise<void>;
    deleteArchetype(id: ArchetypeId): Promise<void>;
    placeArchetype(pid: ProjectId, archetypeId: ArchetypeId): Promise<PlacementId>;
}

export function FacetBrowser(props: {
    api: FacetBrowserApi;
    selected: EngagementId | null;
    onSelect: (id: EngagementId) => void;
    onOpenArchetypeSettings: (id: ArchetypeId, name: string) => void;
    /** Open the per-project Engagement pane (hand off / share a project, FED-7). */
    onOpenEngagement: (id: ProjectId, name: string) => void;
    onOpenModelAccess: (id: ProjectId, name: string) => void;
    onOpenProjectHome: (id: ProjectId, name: string) => void;
    onOpenForkTree: (chat: EngagementId) => void;
    onChatDeleted: (id: EngagementId) => void;
    onStatus: (msg: string) => void;
    /** A chat's agent run tone (round-13): drives the status dot beside its name —
     *  working / needs-review / error, or undefined when idle (no dot). Optional so
     *  the mobile shell can omit it. */
    runToneOf?: (id: EngagementId) => ChatRunTone | undefined;
    /** Bumped by the shell after a turn settles so the tree picks up changes made
     *  outside the nav (e.g. auto-titling a chat from its first message, #4). */
    refreshKey?: unknown;
}) {
    // Open on Chats (WS-H / navigation.md): the resting question is "what was I
    // doing," not "show me the project tree." Projects/Library are structural lenses
    // over the same chats; Chats is the most work-first default.
    const [facet, setFacet] = createSignal<Facet>("all-chats");
    const [query, setQuery] = createSignal("");
    // The nav reads the workspace through its **freshness carriage** (ADR 0037), not
    // bare: a `refreshKey` bump (driven by the workspace event-stream push) re-reads
    // it. `tree` is the carried value; `fresh` is the freshness stamp surfaced below
    // so a stale tree is never shown as current.
    const [carriage, { refetch }] = createResource(
        () => [props.refreshKey] as const,
        () => props.api.getWorkspaceCarriage(),
    );
    // Reconcile each refetch into a store, keyed by `id`, instead of swapping in the
    // fresh JSON wholesale (round-13 flakiness fix). The fetcher returns brand-new
    // objects every refetch; a reference-keyed `<For>` would destroy and re-create
    // every chat row, so a click that lands during a refetch (which fires on each
    // workspace event) hits a detached node and is silently lost — the "click the
    // chat twice" bug. Reconciling preserves node identity for unchanged rows, so
    // the row under the pointer stays put.
    const [store, setStore] = createStore<{ tree: Workspace | null }>({ tree: null });
    createEffect(() => {
        const v = carriage()?.value;
        setStore("tree", v ? reconcile(v, { key: "id" }) : null);
    });
    const tree = () => store.tree ?? undefined;
    const fresh = () => carriage()?.freshness;

    // A filter/search box spanning all facets (navigation.md B2). The match/filter
    // doctrine ("a node shows iff it or a descendant matches") lives in the pure
    // state/facet-filter reducers; the renderers below just bind the live query().
    // Mark the matched substring in a surviving row so a filtered list shows *why*
    // each row stayed (#6 round-9): on "mail", both "Marketing" and "Email helper"
    // survive (one as an ancestor of the hit), but only the literal match gets
    // bolded, so the eye lands on the real reason. Empty/no-match ⇒ plain text.
    const mark = (label: string) => {
        const m = markMatch(label, query());
        if (!m) return label;
        return (
            <>
                {m.pre}
                <mark class="search-hit">{m.match}</mark>
                {m.post}
            </>
        );
    };
    const searching = () => isSearching(query());

    // The chat-log relevance tier (SEARCH-1): the server's `GET /search` finds the
    // chats whose *content* matches, which we merge into the title-filtered tree so
    // a chat surfaces even when only its transcript (not its title) mentions the
    // query. `contentMatches` maps a hit chat id → a snippet of the match (shown as
    // a sublabel); `contentHits` is just its id set, fed to the filter predicates.
    // Debounced + min-2-chars so a fast typist doesn't fan out a fold-per-keystroke.
    const [contentMatches, setContentMatches] = createSignal<Map<string, string>>(new Map());
    const contentHits = createMemo(() => new Set(contentMatches().keys()));
    createEffect(() => {
        const q = query().trim();
        if (q.length < 2) {
            setContentMatches(new Map());
            return;
        }
        let cancelled = false;
        const handle = setTimeout(() => {
            void props.api
                .search(q)
                .then((hits) => {
                    if (!cancelled) setContentMatches(new Map(hits.map((h) => [h.id, h.snippet])));
                })
                .catch(() => {
                    // A failed content search must not blank the title tier — just
                    // drop content hits; the tree still filters on titles.
                    if (!cancelled) setContentMatches(new Map());
                });
        }, 250);
        onCleanup(() => {
            cancelled = true;
            clearTimeout(handle);
        });
    });

    // For a parent node: keep it if its own label matches, else keep only the
    // descendants that match — by title or content (so search narrows into a group,
    // surfacing content-only hits, never hides a hit).
    const chatsFor = <T extends { title: string; id?: EngagementId }>(label: string, chats: T[]) =>
        childrenFor(label, chats, query(), contentHits());

    const [menu, setMenu] = createSignal<MenuState | null>(null);
    // Collapsed tree groups (project / archetype ids). Click a node's ▾/▸ icon to
    // fold its children; local UI state, like facet/selection.
    const [collapsed, setCollapsed] = createSignal<Set<string>>(new Set());
    const isCollapsed = (id: string) => collapsed().has(id);
    const toggleCollapse = (id: string) =>
        setCollapsed((s) => {
            const next = new Set(s);
            if (next.has(id)) next.delete(id);
            else next.add(id);
            return next;
        });
    // inline editor: creating or renaming a named node.
    const [editing, setEditing] = createSignal<
        | { kind: "new-archetype" }
        | { kind: "new-project" }
        | { kind: "rename-archetype"; id: ArchetypeId }
        | { kind: "rename-project"; id: ProjectId }
        | { kind: "rename-chat"; id: EngagementId }
        | { kind: "new-workstream"; placementId: PlacementId }
        | { kind: "new-workstream-from-chat"; placementId: PlacementId; chat: EngagementId }
        | null
    >(null);
    const [editText, setEditText] = createSignal("");

    // The "add a method" picker (#1): from a project, choose *which* archetype to
    // install on it, rather than the app silently placing an arbitrary one. (The
    // reverse "place this archetype on a project" direction was retired by ADR 0045
    // — an archetype is usable in Personal with no placement.)
    const [picker, setPicker] = createSignal<
        { dir: "to-project"; pid: ProjectId; projectName: string } | null
    >(null);
    const [pickerQuery, setPickerQuery] = createSignal("");

    // Per-placement config-only customization (placement.md): a small editor over a
    // placement's `.agent-config.json` overlay + project notes — tweak a method for one
    // client without forking. Loaded on open, written via the InstanceState reducer.
    const [configFor, setConfigFor] = createSignal<{ placementId: PlacementId; name: string } | null>(null);
    const [cfgConfig, setCfgConfig] = createSignal("");
    const [cfgNotes, setCfgNotes] = createSignal("");
    const [cfgStatus, setCfgStatus] = createSignal("");
    async function openConfig(placementId: PlacementId, name: string) {
        setConfigFor({ placementId, name });
        setCfgConfig("");
        setCfgNotes("");
        setCfgStatus("loading…");
        try {
            const { config, notes } = await props.api.getPlacementConfig(placementId);
            setCfgConfig(config);
            setCfgNotes(notes);
            setCfgStatus("");
        } catch (e) {
            setCfgStatus(e instanceof Error ? e.message : String(e));
        }
    }
    async function saveConfig() {
        const f = configFor();
        if (!f) return;
        // Config overlay must be valid JSON (empty = no overlay) so a run never reads a
        // broken `.agent-config.json`.
        if (cfgConfig().trim()) {
            try {
                JSON.parse(cfgConfig());
            } catch {
                setCfgStatus("config must be valid JSON (or empty)");
                return;
            }
        }
        try {
            await props.api.setPlacementConfig(f.placementId, cfgConfig().trim(), cfgNotes());
            setConfigFor(null);
            await refetch();
            props.onStatus(`customized ${f.name}`);
        } catch (e) {
            setCfgStatus(e instanceof Error ? e.message : String(e));
        }
    }

    // The C1 pivot index: archetype id → the placements (project + lineage) of it,
    // so an archetype node can list "everywhere it is placed".
    const placementsOf = createMemo(() => {
        const t = tree();
        const idx = new Map<string, { project: string; placementId: PlacementId }[]>();
        if (!t) return idx;
        for (const p of t.projects) {
            for (const pl of p.placements) {
                const list = idx.get(pl.archetypeId) ?? [];
                list.push({ project: p.name, placementId: pl.placementId });
                idx.set(pl.archetypeId, list);
            }
        }
        return idx;
    });

    // In All chats, the archetype label is only worth showing when chats come from
    // more than one archetype — otherwise it's the same word on every row (#7).
    // Whether a node has anything to fold. The ▾/▸ caret promises collapsible
    // children (#2 round-9); on a method with no chats and nowhere placed it
    // expanded to nothing, so the caret lied. Render a real caret only when there
    // are children, and a fixed-width spacer otherwise so labels stay aligned.
    const archetypeHasChildren = (a: { id: string; chats: unknown[] }) =>
        a.chats.length > 0 || (placementsOf().get(a.id)?.length ?? 0) > 0;
    const caret = (id: string, hasChildren: boolean) =>
        hasChildren ? (
            <span class="node-icon" onClick={(e) => { e.stopPropagation(); toggleCollapse(id); }}>
                {isCollapsed(id) ? "▸" : "▾"}
            </span>
        ) : (
            <span class="node-icon node-icon-empty" aria-hidden="true" />
        );

    async function withRefresh(action: () => Promise<unknown>, ok: string) {
        try {
            await action();
            props.onStatus(ok);
        } catch (e) {
            props.onStatus(e instanceof Rejected ? e.reason : String(e));
        } finally {
            await refetch();
        }
    }

    function startEdit(kind: ReturnType<typeof editing>, initial = "") {
        setEditText(initial);
        setEditing(kind);
    }

    async function commitEdit() {
        const e = editing();
        const text = editText().trim();
        setEditing(null);
        if (!e || !text) return;
        switch (e.kind) {
            case "new-archetype":
                // "method" everywhere in the user-facing string (round-6 #3): the
                // create flow says "+ archetype" / "place this method" — the success
                // toast must not leak the implementation word "archetype".
                return withRefresh(() => props.api.createArchetype(text), `archetype "${text}" created`);
            case "new-project":
                return withRefresh(() => props.api.createProject(text), `project "${text}" created`);
            case "rename-archetype":
                return withRefresh(() => props.api.renameArchetype(e.id, text), "renamed");
            case "rename-project":
                return withRefresh(() => props.api.renameProject(e.id, text), "renamed");
            case "rename-chat":
                return withRefresh(() => props.api.renameChat(e.id, text), "renamed");
            case "new-workstream":
                // Workstreams (WS-F): a named shared auto-sync line in this placement.
                return withRefresh(() => props.api.createWorkstream(e.placementId, text), `workstream "${text}" created`);
            case "new-workstream-from-chat":
                // Create + join in one step (WS-H): the line is born on the chat's own
                // placement with the chat already a member, so it's a visible, non-empty
                // group from the start — the cross-cutting create path for the Chats facet.
                return withRefresh(async () => {
                    const ws = await props.api.createWorkstream(e.placementId, text);
                    await props.api.joinWorkstream(ws.id, e.chat);
                }, `workstream "${text}" created`);
        }
    }

    // --- workstreams (WS-F): create, membership, promote/archive ---
    async function joinWs(wsId: WorkstreamId, chat: EngagementId) {
        await withRefresh(() => props.api.joinWorkstream(wsId, chat), "joined the workstream — its turns now auto-sync");
    }
    async function leaveWs(wsId: WorkstreamId, chat: EngagementId) {
        await withRefresh(() => props.api.leaveWorkstream(wsId, chat), "left the workstream — back on the mainline");
    }
    async function promoteWs(wsId: WorkstreamId) {
        await withRefresh(() => props.api.promoteWorkstream(wsId), "promoted the workstream into the mainline");
    }
    async function archiveWs(wsId: WorkstreamId) {
        await withRefresh(() => props.api.archiveWorkstream(wsId), "workstream archived — its chats are back on the mainline");
    }

    // An EDIT chat is rooted on an archetype (improve the method).
    async function newEditChat(archetypeId: ArchetypeId) {
        await withRefresh(async () => {
            const id = await props.api.createChatUnderArchetype(archetypeId, "edit chat");
            props.onSelect(id);
        }, "editing this archetype");
    }
    // A WORK chat is rooted on a placement (do the job).
    async function newWorkChat(pid: ProjectId, placementId: PlacementId) {
        await withRefresh(async () => {
            const id = await props.api.createChatUnderPlacement(pid, placementId, "new chat");
            props.onSelect(id);
        }, "new work chat");
    }
    // USE an archetype with no placement ceremony (ADR 0045/0036): a work chat in
    // the hidden Personal project. The server finds/creates the Personal placement.
    async function useArchetype(archetypeId: ArchetypeId) {
        await withRefresh(async () => {
            const id = await props.api.useArchetype(archetypeId, "new chat");
            props.onSelect(id);
        }, "new work chat");
    }

    // "Just start typing" (All chats): a WORK chat on the hidden Personal default
    // placement, no project/method setup (ADR 0036). The server roots + mints the id;
    // it lands in this same list, lineage-tagged `assistant · Personal`.
    async function newDefaultChat() {
        await withRefresh(async () => {
            const eng = await props.api.createEngagement();
            props.onSelect(eng.id);
        }, "new chat");
    }

    function openMenu(e: MouseEvent, items: MenuState["items"]) {
        e.preventDefault();
        e.stopPropagation();
        setMenu({ x: e.clientX, y: e.clientY, items });
    }

    function deleteChat(id: EngagementId) {
        void withRefresh(async () => {
            await props.api.deleteChat(id);
            props.onChatDeleted(id);
        }, "chat deleted");
    }

    const editingIs = (kind: string, id: string) => {
        const e = editing();
        return !!e && e.kind === kind && "id" in e && e.id === id;
    };

    // The inline name editor for create/rename. A placeholder tells a first-timer
    // exactly what to do (type a name, Enter to confirm, Esc to cancel) — a bare
    // empty box was a dead end (#7).
    const renameInput = (placeholder = "name…") => (
        <input
            class="inline-edit"
            // autofocus alone is unreliable when the field is inserted by a click
            // that doesn't carry focus in (#smaller); focus it explicitly on mount.
            // Select the existing text on open (#smaller round-9) so a rename can be
            // typed straight over — the familiar convention — instead of forcing the
            // user to hand-clear the old name first. A fresh "+ create" opens empty,
            // so select() is a harmless no-op there.
            ref={(el) => queueMicrotask(() => { el.focus(); el.select(); })}
            aria-label={placeholder}
            placeholder={placeholder}
            value={editText()}
            onInput={(ev) => setEditText(ev.currentTarget.value)}
            onBlur={commitEdit}
            onClick={(ev) => ev.stopPropagation()}
            onKeyDown={(ev) => {
                if (ev.key === "Enter") commitEdit();
                if (ev.key === "Escape") setEditing(null);
            }}
        />
    );

    // --- shared create affordances (consistency across every facet) ---
    // One button look (`.create-btn`) and one row container (`.action-row`) for every
    // "+ create" action — same size/colour/padding, grouped on a single row at the top
    // of a facet or right under a container's title. `createBtn` is the atom; `wsCreate`
    // pairs the "+ workstream" button with its inline name editor for a placement target.
    type BtnOpts = { testid?: string; wsRoot?: PlacementId; data?: string; title?: string };
    const createBtn = (label: string, onClick: () => void, opts?: BtnOpts) => (
        <button
            type="button"
            class="create-btn"
            data-testid={opts?.testid}
            data-ws-new={opts?.wsRoot}
            data-create={opts?.data}
            title={opts?.title}
            onClick={(e) => { e.stopPropagation(); onClick(); }}
        >
            {label}
        </button>
    );
    // The "+ workstream" button for a placement + its naming editor, rendered just below
    // the action row it sits in. `placementId` is the line's home (a project's general
    // placement, a deliberate placement, an archetype's authoring instance, or Personal).
    const wsCreateBtn = (placementId: PlacementId) =>
        createBtn("+ workstream", () => startEdit({ kind: "new-workstream", placementId }), {
            wsRoot: placementId,
            title: "Create a shared auto-sync line here",
        });
    const wsEditorFor = (placementId: PlacementId) => (
        <Show when={editing()?.kind === "new-workstream" && (editing() as { placementId?: PlacementId }).placementId === placementId}>
            <div class="tree-leaf ws-new-inline">{renameInput("name this workstream, then Enter")}</div>
        </Show>
    );

    // --- node renderers ---

    // The single chat-leaf renderer for EVERY facet (Projects, Library, All chats).
    // One element ⇒ one behavior: select+focus on click, the same rename/delete
    // context menu, the same active styling + kind badge. `meta` is an optional
    // right-aligned lineage label (All chats shows the owning archetype).
    // A still-unnamed chat carries a generic placeholder title until its first
    // message renames it (#5). Render those as "Untitled" with a 1-based ordinal so
    // two un-started chats never read identically in the same list. A user-chosen
    // title always wins. The placeholder→display logic is shared (state/chat-title)
    // so the tree, All-chats, the chat-lane header, and the TASKS bar all agree (#4).
    // A still-unnamed chat reads "Untitled · {tag}", where the tag is a STABLE
    // token derived from the chat id (round-11 #6) — not its position in the list,
    // which drifts as chats are added/removed. The same chat keeps the same label
    // forever, in every facet. `displayChatTitle` ignores the tag once the chat has
    // a real (user/auto) title, so we can always pass it.
    const displayTitle = (chat: { title: string; id: EngagementId }): string =>
        displayChatTitle(chat.title, untitledTag(chat.id));

    const chatRow = (
        chat: { id: EngagementId; title: string; kind: "edit" | "work"; workstream?: WorkstreamId | null; placement?: PlacementId | null; changes?: boolean; conflict?: boolean },
        meta?: string,
        // The placement's workstreams (WS-F), present only in the Projects facet, so a
        // work chat's menu can offer join/leave and its row can badge membership.
        workstreams?: WorkstreamNode[],
    ) => (
        <>
        <div
            class="tree-leaf chat-item"
            classList={{ active: props.selected === chat.id }}
            data-chat={chat.id}
            data-kind={chat.kind}
            data-mode={chat.kind}
            // Keyboard/screen-reader reachable (#4 round-5): the tree rows were plain
            // clickable <div>s with no role/tabindex, so a keyboard or SR user could
            // reach the three facet tabs and the search box and then hit a wall —
            // they couldn't open a single chat. A nested <button> would be invalid
            // here (the row already contains badges and a rename input), so we use the
            // standard tree pattern: role="treeitem" + tabindex + Enter/Space activate.
            role="treeitem"
            tabindex="0"
            aria-label={`open chat ${displayTitle(chat)}`}
            onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    props.onSelect(chat.id);
                }
            }}
            onClick={() => props.onSelect(chat.id)}
            onContextMenu={(e) =>
                // A chat's kind (edit/work) is fixed at creation by its root
                // (ADR 0035) — no mid-life toggle.
                openMenu(e, [
                    {
                        label: "fork",
                        run: () =>
                            void withRefresh(async () => {
                                const fid = await props.api.forkChat(chat.id);
                                props.onSelect(fid);
                            }, "chat forked"),
                    },
                    { label: "fork tree", hint: "Show this chat's fork lineage (UX-8)", run: () => props.onOpenForkTree(chat.id) },
                    // Workstream membership (WS-F): leave the one this chat is in, or
                    // join any other active workstream in the placement.
                    ...(chat.workstream
                        ? [{
                            label: "leave workstream",
                            hint: "Stop auto-syncing — go back to the project mainline",
                            run: () => void leaveWs(chat.workstream!, chat.id),
                        }]
                        : []),
                    ...(workstreams ?? [])
                        // Only lines this chat *can* join: active, not its current one, and
                        // co-located in its placement (workstreams are placement-scoped). The
                        // placement filter lets the cross-root Chats facet pass the full list
                        // and still offer only the chat's own lines (WS-H).
                        .filter((w) => w.status === "active" && w.id !== chat.workstream && (!chat.placement || w.placementId === chat.placement))
                        .map((w) => ({
                            label: `join "${w.name}"`,
                            hint: "Greedily auto-sync this chat's work into the shared line",
                            run: () => void joinWs(w.id, chat.id),
                        })),
                    // Start a fresh shared line from this chat (WS-H): works in any facet,
                    // resolving the placement to the chat's own home. Offered when the chat
                    // has a known placement and isn't already on a workstream.
                    ...(chat.placement && !chat.workstream
                        ? [{
                            label: "new workstream",
                            hint: "Start a shared auto-sync line here and put this chat on it",
                            run: () => startEdit({ kind: "new-workstream-from-chat", placementId: chat.placement!, chat: chat.id }),
                        }]
                        : []),
                    { label: "rename", run: () => startEdit({ kind: "rename-chat", id: chat.id }, chat.title) },
                    { label: "delete", danger: true, run: () => deleteChat(chat.id) },
                ])
            }
        >
            {/* Per-row status gem (WS-H): a kind glyph (work ◧ / edit ✎) that doubles as
                the row's status light — quiet when idle, coloured working / needs-review /
                error. It replaces the old standalone status dot and the "editing" badge;
                the change-count and conflict/sync lights wire in once the projection
                carries them (WS-H b/c). */}
            <StatusGem kind={chat.kind} tone={props.runToneOf?.(chat.id)} conflict={chat.conflict} changes={chat.changes} />
            {/* Workstream membership badge (WS-F): this chat auto-syncs into a shared line. */}
            <Show when={chat.workstream}>
                <span class="ws-badge" data-chat-workstream={chat.workstream!} title="Auto-syncing into a workstream">⇄</span>
            </Show>
            <Show
                when={editingIs("rename-chat", chat.id)}
                fallback={
                    <span class="leaf-label leaf-label-stack">
                        {/* The title is its own ellipsising line: the stack is a flex
                            column, where text-overflow can't reach a bare text child,
                            so a long title clipped mid-word with no "…" (round-11 #6). */}
                        <span class="leaf-title">{mark(displayTitle(chat))}</span>
                        {/* Chat-log hit (SEARCH-1): when this row surfaced because the
                            query matched its transcript, show a one-line snippet of the
                            match so the row tells you *why* it stayed — the matched term
                            bolded, like a title hit. */}
                        <Show when={searching() && contentMatches().get(chat.id)}>
                            {(snip) => (
                                <span
                                    class="leaf-sub leaf-snippet"
                                    data-snippet={chat.id}
                                    title="matched in this chat's content"
                                >
                                    {mark(snip())}
                                </span>
                            )}
                        </Show>
                        {/* Fork lineage (#3): a "(fork)" chat is a flat sibling of its
                            source, indistinguishable but for the suffix. Show a quiet
                            "copy of {source}" sublabel so the relationship is legible. */}
                        <Show when={forkSource(chat.title)}>
                            {(src) => (
                                <span
                                    class="leaf-sub"
                                    data-fork-source={src()}
                                    title={`Started as a copy of "${src()}" — its files came along, the conversation started fresh`}
                                >
                                    copy of {src()}
                                </span>
                            )}
                        </Show>
                    </span>
                }
            >
                {renameInput()}
            </Show>
            <Show when={meta}>
                <span class="leaf-meta">{meta}</span>
            </Show>
        </div>
        {/* Create-a-workstream-from-this-chat (WS-H): the cross-cutting way to start a
            shared line without picking a root — the placement resolves to the chat's own
            home, and the chat joins immediately, so the new line is never an invisible
            empty group. The naming input renders right under the row. */}
        <Show when={(() => { const e = editing(); return e?.kind === "new-workstream-from-chat" && e.chat === chat.id; })()}>
            <div class="tree-leaf ws-new-inline">{renameInput("name this workstream, then Enter")}</div>
        </Show>
        </>
    );

    // Group a chat list by workstream (WS-F): a header per active workstream (name +
    // member count + promote/archive), its member chats, then the ungrouped mainline
    // tail. The browse pane groups chats by workstream in every facet. `rootInstanceId`
    // (a placement or an archetype's authoring instance) shows the `+ workstream` create
    // affordance; `joinTargets` are the co-rooted workstreams a chat may join (passed to
    // chatRow's menu) — empty in the cross-root Chats facet, where you join from a root.
    const chatGroups = (
        chats: { id: EngagementId; title: string; kind: "edit" | "work"; workstream?: WorkstreamId | null; placement?: PlacementId | null; changes?: boolean; conflict?: boolean }[],
        workstreams: WorkstreamNode[],
        opts?: { rootInstanceId?: PlacementId; joinTargets?: WorkstreamNode[] },
    ) => {
        const { groups, ungrouped } = groupChatsByWorkstream(chats, workstreams);
        const joinTargets = opts?.joinTargets ?? workstreams;
        return (
            <>
                <For each={groups}>
                    {(g) => (
                        <div class="ws-group" data-workstream={g.ws.id}>
                            {/* The shared line renders as a lightweight grouping label
                                over its chats (WS-H d) — not a node peer of placements
                                with standing buttons. Its lifecycle actions (promote /
                                archive) live in the right-click menu, surfaced when
                                acted on. */}
                            <div
                                class="ws-label"
                                data-workstream={g.ws.id}
                                title="A shared auto-sync line — member chats sync into it automatically. Right-click for actions."
                                onContextMenu={(e) =>
                                    openMenu(e, [
                                        {
                                            label: "promote into mainline",
                                            hint: "Bring this line's settled work into the project mainline (explicit)",
                                            run: () => void promoteWs(g.ws.id),
                                        },
                                        {
                                            label: "archive",
                                            danger: true,
                                            hint: "Close this line — its chats return to the mainline",
                                            run: () => void archiveWs(g.ws.id),
                                        },
                                    ])
                                }
                            >
                                <span class="ws-badge" aria-hidden="true">⇄</span>
                                <span class="ws-label-name">{mark(g.ws.name)}</span>
                                <span
                                    class="ws-label-count"
                                    title={`${g.chats.length} chat${g.chats.length === 1 ? "" : "s"} on this line`}
                                >
                                    {g.chats.length}
                                </span>
                            </div>
                            <div class="ws-members">
                                <For each={g.chats}>
                                    {(c) => chatRow(c, undefined, joinTargets)}
                                </For>
                            </div>
                        </div>
                    )}
                </For>
                <For each={ungrouped}>{(c) => chatRow(c, undefined, joinTargets)}</For>
            </>
        );
    };

    return (
        <div class="facet-browser">
            <div class="facets" role="tablist" aria-label="Browse by">
                <For each={FACETS}>
                    {(f) => (
                        <button
                            type="button"
                            class="facet"
                            role="tab"
                            aria-selected={facet() === f.id}
                            data-facet={f.id}
                            classList={{ active: facet() === f.id }}
                            onClick={() => setFacet(f.id)}
                        >
                            {f.label}
                        </button>
                    )}
                </For>
                {/* Freshness caveat (ADR 0037): the tree is never shown as current
                    when its carriage is not `live` — an explicit "stale, tap to
                    refresh", never a silent stale view. */}
                <Show when={fresh() && fresh()!.marker !== "live"}>
                    <button
                        type="button"
                        class="facet-stale"
                        data-facet-freshness={fresh()!.marker}
                        title={fresh()!.repairHint ?? "refresh"}
                        onClick={() => void refetch()}
                    >
                        {fresh()!.marker} ↻
                    </button>
                </Show>
            </div>
            {/* Search with a clear control (#6 round-5): typing filters the tree,
                but there was no way to reset it short of selecting-and-deleting. */}
            <div class="facet-search-row">
                <input
                    class="facet-search"
                    data-testid="facet-search"
                    aria-label="Search projects, archetypes, and chats"
                    placeholder="search…"
                    value={query()}
                    onInput={(e) => setQuery(e.currentTarget.value)}
                    onKeyDown={(e) => e.key === "Escape" && setQuery("")}
                />
                <Show when={query()}>
                    <button
                        type="button"
                        class="facet-search-clear"
                        data-testid="facet-search-clear"
                        title="Clear search"
                        aria-label="Clear search"
                        onClick={() => setQuery("")}
                    >
                        ✕
                    </button>
                </Show>
            </div>

            <Show when={tree()} fallback={<div class="status">loading…</div>}>
                {(t) => (
                    <>
                        {/* PROJECTS — the default facet: project → placements → work chats. */}
                        <Show when={facet() === "projects"}>
                            {/* Hide the create affordance while a search is active (#6
                                round-9): "+ project" rendered inside filtered results
                                read like a stray search hit. */}
                            <Show when={!searching()}>
                            <div class="action-row" data-actions="projects">
                                {createBtn("+ project", () => startEdit({ kind: "new-project" }), { title: "Create a new project" })}
                            </div>
                            </Show>
                            <Show when={editing()?.kind === "new-project"}>
                                <div class="tree-leaf">{renameInput("name this project, then Enter")}</div>
                            </Show>
                            <For
                                each={t().projects.filter((p) => projectVisible(p, query(), contentHits()))}
                                fallback={<div class="status">no projects</div>}
                            >
                                {(p) => (
                                    <div class="tree-group" data-project={p.id}>
                                        <div
                                            class="tree-node project"
                                            role="treeitem"
                                            tabindex="0"
                                            aria-expanded={!isCollapsed(p.id)}
                                            aria-label={`project ${p.name}`}
                                            onKeyDown={(e) => {
                                                if (e.key === "Enter" || e.key === " ") { e.preventDefault(); toggleCollapse(p.id); }
                                            }}
                                            onContextMenu={(e) => {
                                                const home = p.placements.find((pl) => pl.isDefault)?.placementId;
                                                openMenu(e, [
                                                    ...(home ? [
                                                        { label: "new chat", hint: "Start a new chat in this project", run: () => void newWorkChat(p.id, home) },
                                                        { label: "new workstream", hint: "Create a shared auto-sync line in this project", run: () => startEdit({ kind: "new-workstream", placementId: home }) },
                                                    ] : []),
                                                    { label: "add an archetype", run: () => openAddMethod(p.id, p.name) },
                                                    { label: "project home…", hint: "Recent runs, outputs under review, and an audit rollup for this project", run: () => props.onOpenProjectHome(p.id, p.name) },
                                                    { label: "model access…", hint: "Pin a per-project LLM provider key (overrides the account default)", run: () => props.onOpenModelAccess(p.id, p.name) },
                                                    { label: "share & hand off…", run: () => props.onOpenEngagement(p.id, p.name) },
                                                    { label: "rename", run: () => startEdit({ kind: "rename-project", id: p.id }, p.name) },
                                                    { label: "delete", danger: true, confirmHint: projectBlastRadius(p), run: () => void withRefresh(() => props.api.deleteProject(p.id), "project deleted") },
                                                ]);
                                            }}
                                        >
                                            {caret(p.id, true)}
                                            <Show
                                                when={editingIs("rename-project", p.id)}
                                                fallback={<span class="node-label">{mark(p.name)}</span>}
                                            >
                                                {renameInput()}
                                            </Show>
                                        </div>
                                        <Show when={!isCollapsed(p.id)}>
                                        {/* The project's general workspace: its built-in
                                            general-assistant placement is implementation detail —
                                            never a node. Its chats sit directly under the project
                                            with a "+ new chat" that starts work without choosing an
                                            archetype. Deliberately placed archetypes still appear as
                                            their own nodes below. */}
                                        {(() => {
                                            const generals = p.placements.filter((pl) => pl.isDefault);
                                            const home = generals[0];
                                            const homeChats = generals.flatMap((pl) => pl.chats);
                                            const homeWs = generals.flatMap((pl) => pl.workstreams);
                                            return (
                                                <Show when={home}>
                                                    {/* Action row right under the project title (WS-H):
                                                        start a chat or a shared line in the project — the
                                                        same buttons, same row, everywhere. */}
                                                    <div class="action-row" data-actions="project">
                                                        {createBtn("+ chat", () => void newWorkChat(p.id, home!.placementId), { data: "new-project-chat", title: "Start a new chat in this project" })}
                                                        {wsCreateBtn(home!.placementId)}
                                                    </div>
                                                    {wsEditorFor(home!.placementId)}
                                                    <div class="project-home" data-project-home={p.id}>
                                                        {chatGroups(chatsFor(p.name, homeChats), homeWs)}
                                                    </div>
                                                </Show>
                                            );
                                        })()}
                                        <For
                                            each={p.placements.filter((pl) => !pl.isDefault && placementVisible(p.name, pl, query(), contentHits()))}
                                        >
                                            {(pl) => (
                                                <div class="tree-subgroup" data-placement={pl.placementId}>
                                                    <div
                                                        class="tree-node placement"
                                                        role="treeitem"
                                                        tabindex="0"
                                                        aria-expanded={pl.chats.length > 0 ? !isCollapsed(pl.placementId) : undefined}
                                                        aria-label={
                                                            pl.chats.length > 0
                                                                ? `archetype ${pl.archetypeName} on ${p.name} — open its chats`
                                                                : `archetype ${pl.archetypeName} on ${p.name} — start a chat`
                                                        }
                                                        title={pl.chats.length > 0 ? "open this archetype's chats" : "start a chat with this archetype"}
                                                        // Clicking the row is the obvious "start working" path: with no
                                                        // chats yet it opens a new work chat; otherwise it reveals the
                                                        // existing ones (the `+ chat` button always adds another).
                                                        onClick={() =>
                                                            pl.chats.length > 0
                                                                ? toggleCollapse(pl.placementId)
                                                                : void newWorkChat(p.id, pl.placementId)
                                                        }
                                                        onKeyDown={(e) => {
                                                            if (e.key === "Enter" || e.key === " ") {
                                                                e.preventDefault();
                                                                if (pl.chats.length > 0) toggleCollapse(pl.placementId);
                                                                else void newWorkChat(p.id, pl.placementId);
                                                            }
                                                        }}
                                                        onContextMenu={(e) =>
                                                            openMenu(e, [
                                                                { label: "new chat", hint: "Start a new chat on this placement", run: () => newWorkChat(p.id, pl.placementId) },
                                                                { label: "new workstream", hint: "Create a shared auto-sync line for chats here to collaborate on", run: () => startEdit({ kind: "new-workstream", placementId: pl.placementId }) },
                                                                { label: "edit", run: () => newEditChat(pl.archetypeId) },
                                                                { label: "customize…", hint: "Tweak this method for this project — config + notes, no fork (placement.md)", run: () => void openConfig(pl.placementId, `${pl.archetypeName} · ${p.name}`) },
                                                                ...(pl.pending
                                                                    ? [{ label: "accept", hint: "Approve this archetype so it can host work chats (APPROVE-1)", run: () => void withRefresh(() => props.api.acceptPlacement(pl.placementId), "placement accepted") }]
                                                                    : []),
                                                                ...(pl.upgradeAvailable
                                                                    ? [{ label: "upgrade to latest", hint: `Take the newer published version (v${pl.currentVersion}) of this archetype`, run: () => void withRefresh(() => props.api.upgradePlacement(pl.placementId), "upgraded to the latest version") }]
                                                                    : []),
                                                                { label: "remove from project", danger: true, run: () => void withRefresh(() => props.api.removePlacement(p.id, pl.placementId), "removed") },
                                                            ])
                                                        }
                                                    >
                                                        {caret(pl.placementId, pl.chats.length > 0)}
                                                        {/* Just the method name here (round-6 #6): this row is
                                                            already nested under its project, so the "· project"
                                                            half of the old lineage was redundant noise. Keep a
                                                            stable hook for the pivot via the data attribute. */}
                                                        <span class="node-label" data-lineage-archetype={pl.archetypeId} title="the archetype this placement runs">{mark(pl.archetypeName)}</span>
                                                        {/* This placement carries client-specific config/notes
                                                            (config-only customization, no fork). */}
                                                        <Show when={pl.hasConfig}>
                                                            <span class="cfg-badge" data-placement-customized={pl.placementId} title="Customized for this project (config + notes, no fork)">customized</span>
                                                        </Show>
                                                        {/* APPROVE-1: this placement is pending approval — click to accept
                                                            it (the owner's second act), after which it can host work chats. */}
                                                        <Show when={pl.pending}>
                                                            <button
                                                                class="pending-badge"
                                                                data-placement-pending={pl.placementId}
                                                                title="Pending approval — click to accept so this archetype can host work chats"
                                                                onClick={(e) => { e.stopPropagation(); void withRefresh(() => props.api.acceptPlacement(pl.placementId), "placement accepted"); }}
                                                            >
                                                                pending — accept
                                                            </button>
                                                        </Show>
                                                        {/* UX-9: a newer archetype version is published — a notice,
                                                            not an action (manual by default, ADR 0063). Click to take
                                                            the upgrade. */}
                                                        <Show when={pl.upgradeAvailable}>
                                                            <button
                                                                class="upgrade-badge"
                                                                data-upgrade-available={pl.placementId}
                                                                title={`v${pl.version} → v${pl.currentVersion} available — click to upgrade`}
                                                                onClick={(e) => { e.stopPropagation(); void withRefresh(() => props.api.upgradePlacement(pl.placementId), "upgraded to the latest version"); }}
                                                            >
                                                                update available
                                                            </button>
                                                        </Show>
                                                    </div>
                                                    <Show when={!isCollapsed(pl.placementId)}>
                                                        {/* Same action row as everywhere else: chat or shared
                                                            line, rooted on this deliberate placement. */}
                                                        <div class="action-row" data-actions="placement">
                                                            {createBtn("+ chat", () => void newWorkChat(p.id, pl.placementId), { title: "Start a new chat with this archetype" })}
                                                            {wsCreateBtn(pl.placementId)}
                                                        </div>
                                                        {wsEditorFor(pl.placementId)}
                                                        {/* Chats grouped by workstream (WS-F). */}
                                                        {chatGroups(
                                                            chatsFor(`${p.name} ${pl.archetypeName}`, pl.chats),
                                                            pl.workstreams,
                                                        )}
                                                    </Show>
                                                </div>
                                            )}
                                        </For>
                                        </Show>
                                    </div>
                                )}
                            </For>
                        </Show>

                        {/* LIBRARY — archetypes (the methods) → edit chats. */}
                        <Show when={facet() === "library"}>
                            <Show when={!searching()}>
                            {/* No facet-level "+ workstream" here: a workstream is a shared line
                                over one archetype's edit chats, so it lives per-archetype (below),
                                not at the Library root where there's no single target. */}
                            <div class="action-row" data-actions="library">
                                {createBtn("+ archetype", () => startEdit({ kind: "new-archetype" }), { title: "Create a new archetype" })}
                            </div>
                            </Show>
                            <Show when={editing()?.kind === "new-archetype"}>
                                <div class="tree-leaf">{renameInput("name this archetype, then Enter")}</div>
                            </Show>
                            <For
                                each={t().archetypes.filter((a) => archetypeVisible(a, query(), contentHits()))}
                                fallback={<div class="status">no archetypes yet</div>}
                            >
                                {(a) => (
                                    <div class="tree-group" data-archetype={a.id}>
                                        <div
                                            class="tree-node archetype"
                                            role="treeitem"
                                            tabindex="0"
                                            aria-expanded={archetypeHasChildren(a) ? !isCollapsed(a.id) : undefined}
                                            aria-label={`archetype ${a.name}`}
                                            onKeyDown={(e) => {
                                                if ((e.key === "Enter" || e.key === " ") && archetypeHasChildren(a)) { e.preventDefault(); toggleCollapse(a.id); }
                                            }}
                                            onContextMenu={(e) =>
                                                openMenu(e, [
                                                    { label: "test", hint: "Try this method out — opens a chat that runs it (in your Personal space)", run: () => void useArchetype(a.id) },
                                                    { label: "edit", hint: "Open a chat to edit what this archetype does — you review every change before it's kept", run: () => newEditChat(a.id) },
                                                    { label: "new workstream", hint: "Create a shared auto-sync line over this method's edit chats", run: () => startEdit({ kind: "new-workstream", placementId: a.instanceId }) },
                                                    { label: "settings", run: () => props.onOpenArchetypeSettings(a.id, a.name) },
                                                    { label: "publish a new version", hint: "Make this the current version — placements of it get an upgrade-available notice (UX-9)", run: () => void withRefresh(() => props.api.publishArchetype(a.id), "published a new version") },
                                                    { label: "fork", run: () => void withRefresh(() => props.api.forkArchetype(a.id), "archetype forked") },
                                                    ...(a.forkedFrom
                                                        ? [{ label: "pull updates from source", hint: `Merge improvements from “${a.forkedFromName ?? "the source"}” into this fork (ADR 0038)`, run: () => void withRefresh(() => props.api.pullFromSource(a.id), "pulled updates from the source") }]
                                                        : []),
                                                    { label: "rename", run: () => startEdit({ kind: "rename-archetype", id: a.id }, a.name) },
                                                    ...(a.isDefault
                                                        ? []
                                                        : [{ label: "delete", danger: true, run: () => void withRefresh(() => props.api.deleteArchetype(a.id), "archetype deleted") }]),
                                                ])
                                            }
                                        >
                                            {caret(a.id, archetypeHasChildren(a))}
                                            <Show
                                                when={editingIs("rename-archetype", a.id)}
                                                fallback={<span class="node-label">{mark(a.name)}</span>}
                                            >
                                                {renameInput()}
                                            </Show>
                                        </div>
                                        <Show when={!isCollapsed(a.id)}>
                                        {/* Fork lineage (ADR 0038): a fork shows its source so you know it
                                            tracks an upstream method — "pull updates from source" (its menu)
                                            merges the source's improvements down. */}
                                        <Show when={a.forkedFrom}>
                                            <div class="fork-lineage muted" data-forked-from={a.forkedFrom!}>
                                                ↰ forked from {a.forkedFromName ?? "another method"}
                                            </div>
                                        </Show>
                                        {/* Library is where you EDIT and TEST a method. The action row
                                            offers both, plus a shared line to coordinate concurrent edits
                                            — you can have several edit chats open at once (each is its own
                                            chat on the method's authoring instance), and the merge model
                                            keeps them in sync. "test" runs the method in a throwaway chat;
                                            "edit" changes it (every change reviewed). */}
                                        <div class="action-row" data-actions="archetype">
                                            {createBtn("+ edit", () => newEditChat(a.id), { title: "Open a chat to edit what this method does — every change is reviewed before it's kept" })}
                                            {createBtn("+ test", () => void useArchetype(a.id), { data: "test-archetype", title: "Try this method out — opens a chat that runs it (in your Personal space)" })}
                                            {wsCreateBtn(a.instanceId)}
                                        </div>
                                        {wsEditorFor(a.instanceId)}
                                        {/* The method's edit chats, grouped by workstream (WS-F). */}
                                        {chatGroups(chatsFor(a.name, a.chats), a.workstreams)}
                                        </Show>
                                    </div>
                                )}
                            </For>
                        </Show>

                        {/* ALL CHATS — grouped by archetype (#5). A header per
                            archetype replaces the same label repeated on every row;
                            with a single archetype the header is dropped (it'd be
                            noise). Per-row status dots tell you where the pending
                            decisions are without opening each chat. */}
                        <Show when={facet() === "all-chats"}>
                            {/* "Just start typing": a work chat on the hidden Personal
                                default placement, no setup (ADR 0036). Hidden during a
                                search, like the other facets' create affordances (#6). */}
                            <Show when={!searching()}>
                                <div class="action-row" data-actions="chats">
                                    {createBtn("+ new chat", () => void newDefaultChat(), { testid: "new-default-chat", title: "Start a new chat — no project or archetype setup" })}
                                    <Show when={t().personalPlacement}>{wsCreateBtn(t().personalPlacement!)}</Show>
                                </div>
                                <Show when={t().personalPlacement}>{wsEditorFor(t().personalPlacement!)}</Show>
                            </Show>
                            {/* Grouped by workstream across all roots (WS-F): a workstream
                                header per shared line, then the ungrouped mainline chats.
                                Pass the full workstream list as join targets — each chat row
                                filters it to its own placement's lines (WS-H), so you can join
                                a co-located line right here without pivoting to a root. */}
                            {chatGroups(chatsFor("all chats", t().recent), t().workstreams, { joinTargets: t().workstreams })}
                        </Show>
                    </>
                )}
            </Show>

            <ContextMenu menu={menu()} onClose={() => setMenu(null)} />

            {/* Per-placement customization (placement.md): config-only tweaks for one
                project/client — a `.agent-config.json` overlay + notes, applied to new
                chats here, without forking the shared method. Closes on backdrop / Escape. */}
            <Show when={configFor()}>
                {(f) => (
                    <div class="modal-overlay" data-placement-config onClick={() => setConfigFor(null)}>
                        <div class="modal" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.key === "Escape" && setConfigFor(null)}>
                            <div class="modal-head">
                                <h3>Customize “{f().name}”</h3>
                                <button type="button" onClick={() => setConfigFor(null)}>close</button>
                            </div>
                            <p class="muted">
                                Config-only — tweaks this method for this project without forking it. Applies to new chats here; the shared method is untouched.
                            </p>
                            <label class="cfg-field">
                                <span>Config overlay (JSON) — overrides the method's defaults</span>
                                <textarea
                                    data-cfg-config
                                    rows={5}
                                    spellcheck={false}
                                    value={cfgConfig()}
                                    onInput={(e) => setCfgConfig(e.currentTarget.value)}
                                    placeholder={'{ "model": "claude-opus-4-8" }'}
                                />
                            </label>
                            <label class="cfg-field">
                                <span>Project notes — context fed to the method on every chat here</span>
                                <textarea
                                    data-cfg-notes
                                    rows={5}
                                    value={cfgNotes()}
                                    onInput={(e) => setCfgNotes(e.currentTarget.value)}
                                    placeholder="e.g. AcmeCo prefers terse, formal output; never mention competitors."
                                />
                            </label>
                            <div class="modal-actions">
                                <button type="button" class="create-btn" data-cfg-save onClick={() => void saveConfig()}>Save</button>
                            </div>
                            <p class="status" data-cfg-status>{cfgStatus()}</p>
                        </div>
                    </div>
                )}
            </Show>

            {/* The place picker (#1): pick the other end of a placement. From a
                project you choose an archetype; from an archetype you choose a project.
                One screen, with a "create new" row so an empty library is not a
                dead end. Closes on backdrop click / Escape. */}
            <Show when={picker()}>
                {(pk) => (
                    <div class="modal-overlay" data-place-picker onClick={() => setPicker(null)}>
                        <div
                            class="modal place-picker"
                            onClick={(e) => e.stopPropagation()}
                            onKeyDown={(e) => e.key === "Escape" && setPicker(null)}
                        >
                            <div class="modal-head">
                                <h3>{pickerTitle(pk())}</h3>
                                <button onClick={() => setPicker(null)}>close</button>
                            </div>
                            <p class="status" style={{ margin: "0 0 8px" }}>
                                {pk().dir === "to-project"
                                    ? "Choose which archetype this project should work with."
                                    : "Choose which project to install this archetype on."}
                            </p>
                            <input
                                class="picker-search"
                                autofocus
                                data-picker-search
                                aria-label={pk().dir === "to-project" ? "Find an archetype to add" : "Find a project to place this archetype on"}
                                placeholder={pk().dir === "to-project" ? "find an archetype…" : "find a project…"}
                                value={pickerQuery()}
                                onInput={(e) => setPickerQuery(e.currentTarget.value)}
                            />
                            <div class="picker-list" data-picker-list>
                                {/* Method side: choose which library method to install. */}
                                <Show when={pk().dir === "to-project" ? pk() : null} keyed>
                                    {(p) => p.dir === "to-project" && (
                                        <>
                                            <For
                                                each={(tree()?.archetypes ?? []).filter((a) =>
                                                    a.name.toLowerCase().includes(pickerQuery().trim().toLowerCase()),
                                                )}
                                                fallback={<div class="status">No archetypes yet — create one in the Library first.</div>}
                                            >
                                                {(a) => (
                                                    <button
                                                        type="button"
                                                        class="picker-row"
                                                        data-picker-archetype={a.id}
                                                        onClick={() => placeChosen(p.pid, a.id, a.name)}
                                                    >
                                                        {a.name}
                                                    </button>
                                                )}
                                            </For>
                                            {/* Empty-library escape hatch: jump to the Library to
                                                define a new method, so the picker is never a dead end. */}
                                            <button
                                                type="button"
                                                class="picker-row picker-create"
                                                data-picker-create
                                                onClick={() => { setPicker(null); setFacet("library"); startEdit({ kind: "new-archetype" }); }}
                                            >
                                                + create a new archetype
                                            </button>
                                        </>
                                    )}
                                </Show>
                            </div>
                        </div>
                    </div>
                )}
            </Show>
        </div>
    );

    function pickerTitle(p: NonNullable<ReturnType<typeof picker>>): string {
        return `Add an archetype to ${p.projectName}`;
    }

    // Open the picker to add a method to a project (#1) — the user chooses *which*
    // method, rather than the app silently placing an arbitrary one. (The reverse
    // "place this archetype on a project" direction was retired by ADR 0045: an
    // archetype is usable in Personal with no placement, so the only deliberate
    // placement left is adding a method into a specific named project, from here.)
    function openAddMethod(pid: ProjectId, projectName: string) {
        setPickerQuery("");
        setPicker({ dir: "to-project", pid, projectName });
    }
    // Commit a placement once a method is chosen, then close the picker.
    async function placeChosen(pid: ProjectId, archetypeId: ArchetypeId, label: string) {
        setPicker(null);
        await withRefresh(() => props.api.placeArchetype(pid, archetypeId), `placed ${label}`);
    }
}

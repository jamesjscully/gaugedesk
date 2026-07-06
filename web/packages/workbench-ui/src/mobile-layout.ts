/**
 * The mobile carousel's **layout vocabulary** (`mobile-client.md`, MOB-008):
 * the four panes, the carousel's current state, which panes are reachable for a
 * given selection, and the gestures that move between them. This module is
 * *types only* — the pure transition logic (swipe-depth semantics) lives in the
 * carousel/selection reducer (MOB-009). Keeping the vocabulary separate lets the
 * island, the reducer, and the deep-link resolver all name the same panes.
 *
 * The four desktop panels become four carousel stops in the **same left→right
 * order** — `nav → chat → files → content` — shown one at a time, where
 * left→right is *broader → deeper* (swipe left = deeper, swipe right = back).
 * A phone shows one pane, a tablet two, the desktop four: same projections,
 * different count visible at once.
 */

// ----- Panes -----------------------------------------------------------------

/** A carousel stop, in broad→deep order. The desktop order
 *  (`nav | chat | content-viewer | workspace`) reads on mobile as
 *  `nav | chat | files-tree | file-content`. */
export type PaneKind = "nav" | "chat" | "files" | "content";

/** The panes in canonical left→right (broad→deep) carousel order. Index in this
 *  array *is* the pane's depth; the reducer (MOB-009) walks it for swipes. */
export const PANE_ORDER: readonly PaneKind[] = ["nav", "chat", "files", "content"];

/** Depth of a pane = its index in `PANE_ORDER` (`nav` = 0, deepest = 3). Swipe
 *  left increases depth, swipe right decreases it. */
export function paneDepth(pane: PaneKind): number {
    return PANE_ORDER.indexOf(pane);
}

// ----- Selection-gated reachability ------------------------------------------

/** What the user currently has selected. A stop exists only when it has
 *  something to show, so reachability is *derived from selection* — this is what
 *  keeps the carousel from ever stranding the user on an empty pane
 *  (`mobile-client.md`, "Pane existence is selection-gated"). */
export interface Selection {
    /** A chat is open. With no chat, only `nav` is reachable. */
    readonly chatSelected: boolean;
    /** A file is picked. Without one, `content` is greyed. */
    readonly fileSelected: boolean;
}

/** Which panes a given selection can reach. Each flag mirrors the
 *  selection-state table: `nav` is always reachable; `chat`/`files` need a chat;
 *  `content` additionally needs a picked file. */
export interface PaneVisibility {
    readonly nav: boolean;
    readonly chat: boolean;
    readonly files: boolean;
    readonly content: boolean;
}

// ----- The carousel's current state ------------------------------------------

/** The carousel's pure state: which pane is shown, and the selection that gates
 *  which other panes are reachable. The reducer (MOB-009) maps a gesture + this
 *  state to the next state. */
export interface CarouselState {
    /** The pane currently on screen (one at a time on a phone). */
    readonly current: PaneKind;
    /** The selection that gates pane reachability. */
    readonly selection: Selection;
}

// ----- Gestures --------------------------------------------------------------

/** The inputs that move the carousel. `swipe-right` goes broader (back),
 *  `swipe-left` goes deeper (more specific); `tap` jumps to the next-deeper pane
 *  for whatever was tapped (`mobile-client.md`, gesture/tap/state table). The
 *  hardware back button and iOS edge-swipe-back both map to `swipe-right`. */
export type CarouselGesture =
    /** Back / broader — pop one pane rightward (deeper→broader). */
    | { readonly kind: "swipe-right" }
    /** Deeper / more specific — advance one pane leftward, if reachable. */
    | { readonly kind: "swipe-left" }
    /** Jump to a specific pane (the labelled top-toggle, or a tap that resolves
     *  to a pane). The reducer honours it only when the target is reachable. */
    | { readonly kind: "tap"; readonly target: PaneKind };

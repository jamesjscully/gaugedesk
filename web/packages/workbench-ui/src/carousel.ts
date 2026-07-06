/**
 * The carousel's **pure transition logic** (`mobile-client.md`, MOB-009): given
 * the current {@link CarouselState} and a {@link CarouselGesture}, compute the
 * next state. This is the swipe-depth semantics the island (MOB-014) and the
 * deep-link resolver drive — kept pure so it is testable in isolation and the
 * view layer owns no navigation truth.
 *
 * Two laws hold for every transition:
 *  - **Never strand the user** — the carousel only ever rests on a pane that is
 *    *reachable* for the current selection (`paneVisibility`). A gesture that
 *    would land on a greyed pane is a no-op (the state is returned unchanged).
 *  - **Depth is the `PANE_ORDER` index** — `swipe-left` goes one deeper,
 *    `swipe-right` one broader, both clamped to the reachable range.
 */

import {
    PANE_ORDER,
    paneDepth,
    type CarouselGesture,
    type CarouselState,
    type PaneKind,
    type PaneVisibility,
    type Selection,
} from "./mobile-layout";

// ----- Reachability ----------------------------------------------------------

/** Which panes the given selection can reach (the selection-gated table):
 *  `nav` is always reachable; `chat`/`files` need a chat; `content` additionally
 *  needs a picked file. The carousel must never rest on a `false` pane. */
export function paneVisibility(selection: Selection): PaneVisibility {
    return {
        nav: true,
        chat: selection.chatSelected,
        files: selection.chatSelected,
        content: selection.chatSelected && selection.fileSelected,
    };
}

/** Whether a single pane is reachable for the given selection. */
export function isReachable(pane: PaneKind, selection: Selection): boolean {
    return paneVisibility(selection)[pane];
}

// ----- The reducer -----------------------------------------------------------

/** The canonical starting state: on `nav` (the only always-reachable pane) with
 *  nothing selected. */
export const initial: CarouselState = {
    current: "nav",
    selection: { chatSelected: false, fileSelected: false },
};

/**
 * Map a gesture onto the carousel. Pure: same `state` + `gesture` ⇒ same result,
 * and the returned `current` pane is always reachable for `state.selection`.
 *
 *  - `swipe-right` pops one pane broader (deeper→broader), clamped at `nav`.
 *  - `swipe-left` advances one pane deeper, but only if that next pane is
 *    reachable; otherwise it is a no-op (we never strand on a greyed pane).
 *  - `tap` jumps straight to `target` when reachable; an unreachable target is
 *    ignored.
 */
export function reduce(state: CarouselState, gesture: CarouselGesture): CarouselState {
    const { selection, current } = state;

    switch (gesture.kind) {
        case "swipe-right": {
            const next = paneAtDepth(paneDepth(current) - 1);
            return next === undefined ? state : moveTo(state, next);
        }
        case "swipe-left": {
            const next = paneAtDepth(paneDepth(current) + 1);
            if (next === undefined || !isReachable(next, selection)) return state;
            return moveTo(state, next);
        }
        case "tap": {
            if (!isReachable(gesture.target, selection)) return state;
            return moveTo(state, gesture.target);
        }
    }
}

/**
 * Apply a new selection, repairing `current` if it became unreachable. Selecting
 * a chat opens up Chat/Files; clearing the file selection while on Content must
 * fall back to the deepest pane that is still reachable, so the carousel never
 * keeps the user on a stranded pane.
 */
export function select(state: CarouselState, selection: Selection): CarouselState {
    if (isReachable(state.current, selection)) {
        return { ...state, selection };
    }
    return { current: deepestReachable(selection), selection };
}

// ----- internals -------------------------------------------------------------

function moveTo(state: CarouselState, pane: PaneKind): CarouselState {
    return pane === state.current ? state : { ...state, current: pane };
}

function paneAtDepth(depth: number): PaneKind | undefined {
    return PANE_ORDER[depth];
}

/** The deepest pane still reachable for a selection (always at least `nav`). */
function deepestReachable(selection: Selection): PaneKind {
    const visibility = paneVisibility(selection);
    for (let depth = PANE_ORDER.length - 1; depth >= 0; depth--) {
        const pane = PANE_ORDER[depth];
        if (visibility[pane]) return pane;
    }
    return "nav";
}

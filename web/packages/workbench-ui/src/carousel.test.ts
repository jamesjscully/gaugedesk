import { describe, expect, it } from "vitest";
import {
    initial,
    isReachable,
    paneVisibility,
    reduce,
    select,
} from "./carousel";
import {
    PANE_ORDER,
    type CarouselState,
    type PaneKind,
    type Selection,
} from "./mobile-layout";

const noChat: Selection = { chatSelected: false, fileSelected: false };
const chatOnly: Selection = { chatSelected: true, fileSelected: false };
const withFile: Selection = { chatSelected: true, fileSelected: true };

function at(current: PaneKind, selection: Selection): CarouselState {
    return { current, selection };
}

describe("carousel reachability (selection-gated)", () => {
    it("nav only when no chat is selected", () => {
        expect(paneVisibility(noChat)).toEqual({ nav: true, chat: false, files: false, content: false });
    });

    it("chat selected, no file → Browse · Chat · Files (Content greyed)", () => {
        expect(paneVisibility(chatOnly)).toEqual({ nav: true, chat: true, files: true, content: false });
    });

    it("file selected → all four reachable", () => {
        expect(paneVisibility(withFile)).toEqual({ nav: true, chat: true, files: true, content: true });
    });
});

describe("carousel gestures (swipe-depth semantics)", () => {
    it("swipe-left walks nav → chat → files → content when reachable", () => {
        let s = at("nav", withFile);
        s = reduce(s, { kind: "swipe-left" });
        expect(s.current).toBe("chat");
        s = reduce(s, { kind: "swipe-left" });
        expect(s.current).toBe("files");
        s = reduce(s, { kind: "swipe-left" });
        expect(s.current).toBe("content");
    });

    it("swipe-right walks back content → files → chat → nav", () => {
        let s = at("content", withFile);
        s = reduce(s, { kind: "swipe-right" });
        expect(s.current).toBe("files");
        s = reduce(s, { kind: "swipe-right" });
        expect(s.current).toBe("chat");
        s = reduce(s, { kind: "swipe-right" });
        expect(s.current).toBe("nav");
    });

    it("swipe-right at nav (leftmost) is a no-op", () => {
        const s = at("nav", withFile);
        expect(reduce(s, { kind: "swipe-right" })).toBe(s);
    });

    it("swipe-left at content (deepest) is a no-op", () => {
        const s = at("content", withFile);
        expect(reduce(s, { kind: "swipe-left" })).toBe(s);
    });

    it("swipe-left does not strand on a greyed pane (no chat → stays on nav)", () => {
        const s = at("nav", noChat);
        expect(reduce(s, { kind: "swipe-left" })).toBe(s);
    });

    it("swipe-left from files into greyed content is a no-op", () => {
        const s = at("files", chatOnly);
        expect(reduce(s, { kind: "swipe-left" })).toBe(s);
    });

    it("tap jumps directly to a reachable pane", () => {
        const s = at("nav", withFile);
        expect(reduce(s, { kind: "tap", target: "content" }).current).toBe("content");
    });

    it("tap on an unreachable pane is ignored", () => {
        const s = at("chat", chatOnly);
        expect(reduce(s, { kind: "tap", target: "content" })).toBe(s);
    });
});

describe("carousel selection changes", () => {
    it("keeps the current pane when it is still reachable", () => {
        const s = select(at("chat", withFile), chatOnly);
        expect(s.current).toBe("chat");
        expect(s.selection).toBe(chatOnly);
    });

    it("falls back to the deepest reachable pane when current is stranded", () => {
        // on Content, then the file selection is cleared
        const s = select(at("content", withFile), chatOnly);
        expect(s.current).toBe("files");
    });

    it("collapses all the way to nav when the chat is closed", () => {
        const s = select(at("content", withFile), noChat);
        expect(s.current).toBe("nav");
    });
});

describe("carousel invariants", () => {
    const selections = [noChat, chatOnly, withFile];
    const gestures = [
        { kind: "swipe-left" as const },
        { kind: "swipe-right" as const },
        ...PANE_ORDER.map((target) => ({ kind: "tap" as const, target })),
    ];

    it("starts reachable and never rests on a greyed pane", () => {
        expect(isReachable(initial.current, initial.selection)).toBe(true);
        for (const selection of selections) {
            for (const start of PANE_ORDER) {
                if (!isReachable(start, selection)) continue;
                for (const gesture of gestures) {
                    const next = reduce(at(start, selection), gesture);
                    expect(isReachable(next.current, next.selection)).toBe(true);
                }
            }
        }
    });

    it("select() always lands on a reachable pane", () => {
        for (const from of selections) {
            for (const start of PANE_ORDER) {
                for (const to of selections) {
                    const next = select(at(start, from), to);
                    expect(isReachable(next.current, next.selection)).toBe(true);
                }
            }
        }
    });
});

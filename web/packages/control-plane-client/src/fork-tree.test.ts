import { describe, expect, it } from "vitest";
import { parseForkForest, parseForkNode } from "./fork-tree";

describe("fork-tree parse (UX-8)", () => {
    it("parses a nested forest envelope", () => {
        const forest = parseForkForest({
            forest: [
                { id: "root", title: "Root", children: [{ id: "child", title: "Child", children: [] }] },
            ],
        });
        expect(forest).toHaveLength(1);
        expect(forest[0].id).toBe("root");
        expect(forest[0].children[0].id).toBe("child");
    });

    it("is total: missing/garbage fields degrade, never throw", () => {
        expect(parseForkForest(null)).toEqual([]);
        expect(parseForkForest({})).toEqual([]);
        const n = parseForkNode({ id: "x" });
        expect(n).toEqual({ id: "x", title: "", children: [] });
        // a non-array children degrades to empty.
        expect(parseForkNode({ id: "y", children: "nope" }).children).toEqual([]);
    });
});

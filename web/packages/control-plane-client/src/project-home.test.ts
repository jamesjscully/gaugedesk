import { describe, expect, it } from "vitest";
import { parseProjectHome } from "./project-home";

describe("parseProjectHome (UX-2)", () => {
    it("parses a full rollup envelope into the branded shape", () => {
        const h = parseProjectHome({
            project_id: "proj-1",
            recent_runs: [
                { chat: "c1", title: "Landing page", phase: "Completed", ran: true },
                { chat: "c2", title: "Pricing", phase: "Init", ran: false },
            ],
            outputs: [{ chat: "c1", title: "Landing page", phase: "Rejected" }],
            audit: { placements: 2, chats: 3, events: 17 },
        });
        expect(h.projectId).toBe("proj-1");
        expect(h.recentRuns).toHaveLength(2);
        expect(h.recentRuns[0]).toEqual({ chat: "c1", title: "Landing page", phase: "Completed", ran: true });
        expect(h.recentRuns[1].ran).toBe(false);
        expect(h.outputs).toEqual([{ chat: "c1", title: "Landing page", phase: "Rejected" }]);
        expect(h.audit).toEqual({ placements: 2, chats: 3, events: 17 });
    });

    it("is total: a partial / empty envelope degrades to empty lists + zero counts", () => {
        const h = parseProjectHome({ project_id: "p" });
        expect(h.projectId).toBe("p");
        expect(h.recentRuns).toEqual([]);
        expect(h.outputs).toEqual([]);
        expect(h.audit).toEqual({ placements: 0, chats: 0, events: 0 });
    });

    it("never throws on garbage input", () => {
        const h = parseProjectHome(null);
        expect(h.projectId).toBe("");
        expect(h.audit.events).toBe(0);
        // odd run rows degrade field-by-field rather than crashing.
        const h2 = parseProjectHome({ recent_runs: [{}, { chat: "x" }] });
        expect(h2.recentRuns).toHaveLength(2);
        expect(h2.recentRuns[0]).toEqual({ chat: "", title: "", phase: "", ran: false });
        expect(h2.recentRuns[1].chat).toBe("x");
    });
});

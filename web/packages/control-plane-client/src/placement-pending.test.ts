import { describe, expect, it } from "vitest";
import { parseWorkspace } from "./control-plane-domain";

/** A minimal workspace envelope with one project holding the given placements. */
function workspaceWith(placements: unknown[]) {
    return { archetypes: [], recent: [], projects: [{ id: "p1", name: "acme", placements }] };
}

describe("parseWorkspace — placement admission (APPROVE-1)", () => {
    it("parses a pending placement's flag through to the domain node", () => {
        const ws = parseWorkspace(
            workspaceWith([
                { placement_id: "i1", archetype_id: "a1", archetype_name: "Reviewer", pinned_version: null, pending: true, chats: [] },
            ]),
        );
        expect(ws.projects[0].placements[0].pending).toBe(true);
    });

    it("defaults pending to false when the field is absent (frictionless / older projection)", () => {
        const ws = parseWorkspace(
            workspaceWith([
                { placement_id: "i1", archetype_id: "a1", archetype_name: "Reviewer", pinned_version: null, chats: [] },
            ]),
        );
        expect(ws.projects[0].placements[0].pending).toBe(false);
    });
});

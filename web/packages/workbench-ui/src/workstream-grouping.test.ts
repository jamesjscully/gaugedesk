import { describe, expect, it } from "vitest";
import type { EngagementId, WorkstreamId, WorkstreamNode } from "@gaugewright/control-plane-client";
import { groupChatsByWorkstream, hasWorkstreams } from "./workstream-grouping";

const ws = (id: string, status: "active" | "archived" = "active"): WorkstreamNode => ({
    id: id as WorkstreamId,
    name: id,
    placementId: "p1" as never,
    status,
    members: [],
});
const chat = (id: string, workstream: string | null) => ({
    id: id as EngagementId,
    workstream: workstream as WorkstreamId | null,
});

describe("groupChatsByWorkstream", () => {
    it("buckets chats under their workstream and leaves the rest ungrouped", () => {
        const chats = [chat("a", "w1"), chat("b", null), chat("c", "w1"), chat("d", "w2")];
        const { groups, ungrouped } = groupChatsByWorkstream(chats, [ws("w1"), ws("w2")]);
        expect(groups.map((g) => g.ws.id)).toEqual(["w1", "w2"]);
        expect(groups[0].chats.map((c) => c.id)).toEqual(["a", "c"]);
        expect(groups[1].chats.map((c) => c.id)).toEqual(["d"]);
        expect(ungrouped.map((c) => c.id)).toEqual(["b"]);
    });

    it("shows an active workstream even when it has no chats (joinable/promotable)", () => {
        const { groups } = groupChatsByWorkstream([chat("a", null)], [ws("empty")]);
        expect(groups).toHaveLength(1);
        expect(groups[0].chats).toEqual([]);
    });

    it("treats membership in an archived/foreign workstream as ungrouped", () => {
        const { groups, ungrouped } = groupChatsByWorkstream(
            [chat("a", "old"), chat("b", "gone")],
            [ws("old", "archived")],
        );
        expect(groups).toHaveLength(0); // archived stream is not a group
        expect(ungrouped.map((c) => c.id)).toEqual(["a", "b"]);
    });

    it("preserves candidate order for stable rendering", () => {
        const { groups } = groupChatsByWorkstream([], [ws("z"), ws("a"), ws("m")]);
        expect(groups.map((g) => g.ws.id)).toEqual(["z", "a", "m"]);
    });
});

describe("hasWorkstreams", () => {
    it("is true only when an active workstream exists", () => {
        expect(hasWorkstreams([])).toBe(false);
        expect(hasWorkstreams([ws("w", "archived")])).toBe(false);
        expect(hasWorkstreams([ws("w")])).toBe(true);
    });
});

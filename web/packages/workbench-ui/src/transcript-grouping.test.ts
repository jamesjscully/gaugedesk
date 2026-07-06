import { describe, expect, it } from "vitest";
import { fromSnapshot, groupTurns, type StreamEvent } from "./transcript";

/** Build a transcript's lines from a flat event list, the way the client does. */
const lines = (evs: StreamEvent[]) => fromSnapshot(evs).lines;

describe("groupTurns — fold agent prose + its tool calls into one turn", () => {
    it("brackets a run of agent lines (text/tool/blocked) into a single turn", () => {
        const segs = groupTurns(
            lines([
                { type: "user", text: "do it" },
                { type: "assistant", text: "Let me check" },
                { type: "tool", tool: "read", mediated: false, call_id: "c1", target: "a.txt" },
                { type: "assistant", text: "Done" },
            ]),
        );
        expect(segs.map((s) => s.type)).toEqual(["line", "turn"]);
        const turn = segs[1];
        if (turn.type !== "turn") throw new Error("expected a turn");
        expect(turn.lines).toHaveLength(3);
        // The turn id is the seq of its first line — stable as the turn streams.
        expect(turn.id).toBe(turn.lines[0].seq);
    });

    it("your messages, lifecycle notes and errors stand alone, splitting turns", () => {
        const segs = groupTurns(
            lines([
                { type: "user", text: "one" },
                { type: "assistant", text: "first" },
                { type: "admitted", kind: "run", text: "run → Completed" },
                { type: "user", text: "two" },
                { type: "assistant", text: "second" },
            ]),
        );
        // user · turn · run · user · turn
        expect(segs.map((s) => s.type)).toEqual(["line", "turn", "line", "line", "turn"]);
    });

    it("is empty for an empty transcript and order-preserving", () => {
        expect(groupTurns([])).toEqual([]);
        const segs = groupTurns(lines([{ type: "user", text: "hi" }]));
        expect(segs).toHaveLength(1);
        expect(segs[0].type).toBe("line");
    });
});

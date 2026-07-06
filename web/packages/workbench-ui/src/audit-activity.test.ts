import { describe, expect, it } from "vitest";
import { toActivity, type RawAuditRow } from "./audit-activity";

const row = (position: number, kind: string, payload: string): RawAuditRow => ({ position, kind, payload });

describe("toActivity — plain-language history (round-6 #1)", () => {
    it("drops internal bookkeeping lifecycle markers, never showing raw type names", () => {
        const rows = [
            row(0, "run", '"RunRequested"'),
            row(1, "run", '"RunAdmitted"'),
            row(2, "merge", '"MergeStarted"'),
            row(3, "merge", '"GitCleaned"'),
        ];
        // None of these mean anything to a user — all dropped, nothing leaked.
        expect(toActivity(rows)).toEqual([]);
    });

    it("translates run lifecycle to plain language", () => {
        const out = toActivity([
            row(0, "run", '"RunStarted"'),
            row(1, "run", '"RunCompleted"'),
        ]);
        expect(out.map((a) => a.text)).toEqual(["The assistant started working", "Finished this turn"]);
    });

    it("reads a user message and an assistant tool call from transcript JSON", () => {
        const out = toActivity([
            row(0, "transcript", JSON.stringify({ type: "user", text: "Draft a thank-you email" })),
            row(1, "transcript", JSON.stringify({ type: "tool", tool: "write", target: "agent-note.txt" })),
        ]);
        expect(out[0]).toMatchObject({ text: "You asked: “Draft a thank-you email”", tone: "you" });
        expect(out[1]).toMatchObject({ text: "Wrote agent-note.txt", tone: "did" });
    });

    it("translates a bash tool call to a plain verb, never the raw name", () => {
        const out = toActivity([row(0, "transcript", JSON.stringify({ type: "tool", tool: "bash", args: "echo hi" }))]);
        expect(out).toEqual([{ key: 0, text: "Ran a command", tone: "did" }]);
        expect(out[0].text).not.toMatch(/bash/i);
    });

    it("drops resource registrations and serialized resource IDs entirely", () => {
        const out = toActivity([
            row(12, "resource", JSON.stringify({ resource: { id: "out-chat-279cb184813d", kind: "chat" } })),
            row(13, "transcript", JSON.stringify({ type: "admitted", kind: "run" })),
        ]);
        expect(out).toEqual([]);
    });

    it("translates keep/discard outcomes plainly", () => {
        const out = toActivity([
            row(0, "merge", '"Integrated"'),
            row(1, "merge", '"Rejected"'),
        ]);
        expect(out.map((a) => a.tone)).toEqual(["kept", "discarded"]);
        expect(out[0].text).toBe("Kept into the shared copy");
        expect(out[1].text).toBe("Discarded — nothing was kept");
    });

    it("truncates long messages so the line reads like a title", () => {
        const long = "x".repeat(200);
        const out = toActivity([row(0, "transcript", JSON.stringify({ type: "user", text: long }))]);
        expect(out[0].text.length).toBeLessThan(100);
        expect(out[0].text).toMatch(/…”$/);
    });

    it("survives a malformed payload by dropping it rather than crashing or leaking", () => {
        const out = toActivity([row(0, "run", "not json at all {")]);
        expect(out).toEqual([]);
    });

    it("orders by position, so 'Finished this turn' follows the actions it finished (round-12 B)", () => {
        // Fed out of causal order (RunCompleted arrives before the tool rows).
        const out = toActivity([
            row(0, "run", '"RunStarted"'),
            row(1, "transcript", JSON.stringify({ type: "user", text: "make a change" })),
            row(4, "run", '"RunCompleted"'),
            row(2, "transcript", JSON.stringify({ type: "tool", tool: "write", target: "note.txt" })),
            row(3, "transcript", JSON.stringify({ type: "tool", tool: "bash", args: "echo hi" })),
        ]);
        expect(out.map((a) => a.text)).toEqual([
            "The assistant started working",
            "You asked: “make a change”",
            "Wrote note.txt",
            "Ran a command",
            "Finished this turn",
        ]);
    });

    it("drops a consecutive echo (tool action + the assistant's restatement of it)", () => {
        const out = toActivity([
            row(0, "transcript", JSON.stringify({ type: "tool", tool: "write", target: "agent-note.txt" })),
            row(1, "transcript", JSON.stringify({ type: "assistant", text: "Wrote agent-note.txt." })),
        ]);
        expect(out.map((a) => a.text)).toEqual(["Wrote agent-note.txt"]);
    });
});

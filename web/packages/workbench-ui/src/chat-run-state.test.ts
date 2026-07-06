import { describe, expect, it } from "vitest";
import { runDotTitle, type ChatRunTone } from "./chat-run-state";

describe("runDotTitle (per-chat status dot, round-13)", () => {
    it("gives a plain hover label per tone", () => {
        expect(runDotTitle("working")).toMatch(/working/i);
        expect(runDotTitle("review")).toMatch(/review/i);
        expect(runDotTitle("error")).toMatch(/didn't finish|did not finish|finish/i);
    });

    it("returns null when idle (no dot)", () => {
        expect(runDotTitle(undefined)).toBeNull();
        // exhaustiveness guard — unknown tone is treated as idle
        expect(runDotTitle("???" as unknown as ChatRunTone)).toBeNull();
    });
});

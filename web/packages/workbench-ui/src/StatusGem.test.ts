import { describe, expect, it } from "vitest";
import { gemState } from "./StatusGem";

// The gem paints ONE state, resolved most-urgent first (WS-H b/c): a conflict the
// human must resolve outranks a live turn, which outranks pending review.
describe("gemState precedence", () => {
    it("is idle when nothing is set", () => {
        expect(gemState({})).toBe("idle");
    });

    it("conflict outranks every other signal", () => {
        expect(gemState({ conflict: true, tone: "working", changes: true })).toBe("conflict");
        expect(gemState({ conflict: true, tone: "error" })).toBe("conflict");
    });

    it("a live working/error turn outranks pending review", () => {
        expect(gemState({ tone: "working", changes: true })).toBe("working");
        expect(gemState({ tone: "error", changes: true })).toBe("error");
    });

    it("pending changes (projection) light the review state", () => {
        expect(gemState({ changes: true })).toBe("review");
    });

    it("the live review tone also lights review (no projection field needed)", () => {
        expect(gemState({ tone: "review" })).toBe("review");
    });

    it("false/absent conflict and changes stay idle", () => {
        expect(gemState({ conflict: false, changes: false })).toBe("idle");
    });
});

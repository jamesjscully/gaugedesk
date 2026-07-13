/**
 * The fold's composition is where a resolve could silently corrupt the
 * document, so it's pinned pure: pieces in document order, one choice per
 * conflict region, output = exact concatenation. (The save-side merge
 * semantics are whip's, tested there.)
 */

import { describe, expect, it } from "vitest";
import {
    composeResolution,
    conflictRegionCount,
    regionResolutions,
    type RegionChoice,
} from "./ConflictFold";
import { type MergePiece } from "@gaugewright/control-plane-client";

const pieces: MergePiece[] = [
    { kind: "merged", text: "The swift brown fox jumps over the lazy ", provenance: "base" },
    { kind: "conflict", base_text: "dog", ours_text: "tiger", theirs_text: "lion" },
    { kind: "merged", text: " today.", provenance: "base" },
];

describe("composeResolution", () => {
    it("keeps merged spans verbatim and splices the chosen side", () => {
        expect(composeResolution(pieces, [{ pick: "ours" }])).toBe(
            "The swift brown fox jumps over the lazy tiger today.",
        );
        expect(composeResolution(pieces, [{ pick: "theirs" }])).toBe(
            "The swift brown fox jumps over the lazy lion today.",
        );
    });

    it("splices custom text for a hand-blended region", () => {
        expect(composeResolution(pieces, [{ pick: "custom", text: "liger" }])).toBe(
            "The swift brown fox jumps over the lazy liger today.",
        );
    });

    it("resolves multiple regions in document order", () => {
        const two: MergePiece[] = [
            { kind: "conflict", base_text: "a", ours_text: "A", theirs_text: "x" },
            { kind: "merged", text: " mid ", provenance: "both" },
            { kind: "conflict", base_text: "b", ours_text: "B", theirs_text: "y" },
        ];
        const choices: RegionChoice[] = [{ pick: "theirs" }, { pick: "ours" }];
        expect(composeResolution(two, choices)).toBe("x mid B");
        expect(conflictRegionCount(two)).toBe(2);
    });

    it("refuses a missing choice instead of guessing", () => {
        expect(() => composeResolution(pieces, [])).toThrow(/no choice/);
    });
});

describe("regionResolutions", () => {
    it("carries the exact triple the user saw plus the text they chose", () => {
        expect(regionResolutions(pieces, [{ pick: "custom", text: "liger" }])).toEqual([
            {
                base_text: "dog",
                ours_text: "tiger",
                theirs_text: "lion",
                resolution_text: "liger",
            },
        ]);
        expect(regionResolutions(pieces, [{ pick: "ours" }])[0].resolution_text).toBe("tiger");
        expect(regionResolutions(pieces, [{ pick: "theirs" }])[0].resolution_text).toBe("lion");
    });

    it("emits one triple per region, in document order", () => {
        const two: MergePiece[] = [
            { kind: "conflict", base_text: "a", ours_text: "A", theirs_text: "x" },
            { kind: "merged", text: " mid ", provenance: "both" },
            { kind: "conflict", base_text: "b", ours_text: "B", theirs_text: "y" },
        ];
        const choices: RegionChoice[] = [{ pick: "theirs" }, { pick: "ours" }];
        expect(regionResolutions(two, choices).map((r) => r.resolution_text)).toEqual(["x", "B"]);
    });

    it("refuses a missing choice instead of minting wrong memory", () => {
        expect(() => regionResolutions(pieces, [])).toThrow(/no choice/);
    });
});

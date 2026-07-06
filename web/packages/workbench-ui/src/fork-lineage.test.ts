import { describe, it, expect } from "vitest";
import { isFork, forkSource } from "./fork-lineage";

describe("fork-lineage (round-8 #3: make fork lineage legible)", () => {
    it("detects a fork by its '(fork)' suffix", () => {
        expect(isFork("hello kitt (fork)")).toBe(true);
        expect(isFork("hello kitt")).toBe(false);
        expect(isFork(undefined)).toBe(false);
        expect(isFork("")).toBe(false);
    });

    it("reads the source name a fork was copied from", () => {
        expect(forkSource("hello kitt (fork)")).toBe("hello kitt");
        expect(forkSource("Draft a tagline (fork)")).toBe("Draft a tagline");
    });

    it("tolerates a numbered fork suffix", () => {
        expect(isFork("hello kitt (fork 2)")).toBe(true);
        expect(forkSource("hello kitt (fork 2)")).toBe("hello kitt");
    });

    it("strips only ONE suffix so a fork-of-a-fork points at its immediate source", () => {
        expect(forkSource("X (fork) (fork)")).toBe("X (fork)");
    });

    it("returns null for non-forks and for a title that strips to empty", () => {
        expect(forkSource("hello kitt")).toBeNull();
        expect(forkSource(undefined)).toBeNull();
        expect(forkSource("(fork)")).toBeNull();
    });

    it("is case-insensitive on the suffix", () => {
        expect(isFork("Thing (Fork)")).toBe(true);
        expect(forkSource("Thing (FORK)")).toBe("Thing");
    });
});

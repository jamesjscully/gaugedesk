import { describe, expect, it } from "vitest";
import { readPolicyDiff } from "./policy-diff";

describe("readPolicyDiff", () => {
    it("reads a removed shell-block as a loosening change in plain words", () => {
        const diff = [
            "diff --git a/.agent-config.json b/.agent-config.json",
            "--- a/.agent-config.json",
            "+++ b/.agent-config.json",
            "@@ -1,3 +1,1 @@",
            '-  "policy": { "block_tools": ["bash"] }',
            "+{}",
        ].join("\n");
        const r = readPolicyDiff(diff);
        expect(r.touchesConfig).toBe(true);
        expect(r.onlyConfig).toBe(true);
        expect(r.notes.some((n) => n.direction === "loosen")).toBe(true);
        expect(r.notes[0].text).toMatch(/run shell commands/i);
    });

    it("reads an added block as a tightening change", () => {
        const diff = [
            "--- a/.agent-config.json",
            "+++ b/.agent-config.json",
            '+  "block_tools": ["bash"]',
        ].join("\n");
        const r = readPolicyDiff(diff);
        expect(r.notes.some((n) => n.direction === "tighten")).toBe(true);
    });

    it("flags onlyConfig=false when a real deliverable also changed", () => {
        const diff = [
            "--- a/agent-note.txt",
            "+++ b/agent-note.txt",
            "+hello",
            "--- a/.agent-config.json",
            "+++ b/.agent-config.json",
            '-  "block_tools": ["bash"]',
        ].join("\n");
        const r = readPolicyDiff(diff);
        expect(r.touchesConfig).toBe(true);
        expect(r.onlyConfig).toBe(false);
    });

    it("returns no notes for a non-config diff", () => {
        const diff = ["--- a/readme.md", "+++ b/readme.md", "+a line"].join("\n");
        const r = readPolicyDiff(diff);
        expect(r.touchesConfig).toBe(false);
        expect(r.notes).toHaveLength(0);
    });

    it("orders loosening notes before tightening ones", () => {
        const diff = [
            "--- a/.agent-config.json",
            "+++ b/.agent-config.json",
            '+  "allow_tools": ["bash"]',
            '+  "block_tools": ["network"]',
        ].join("\n");
        const r = readPolicyDiff(diff);
        expect(r.notes[0].direction).toBe("loosen");
    });
});

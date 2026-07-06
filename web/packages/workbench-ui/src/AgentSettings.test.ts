/**
 * The Settings modal's plain form (#5 round-5) reads/writes the config keys the
 * boundary actually respects. These guard the read↔write round-trip and the
 * "preserve unknown keys / don't fight the Advanced JSON" contract.
 */

import { describe, expect, it } from "vitest";
import { plainConfigError, readFormConfig, writeFormConfig } from "./AgentSettings";

/** The boundary-shaped `policy` block `writeFormConfig` emits. The function's
 *  return type is the open `Record<string, unknown>` of a config document (it
 *  preserves arbitrary unknown keys), so the test names the precise shape it
 *  asserts on rather than casting each access to `any`. */
interface BoundaryPolicy {
    posture?: string;
    allow_network?: boolean;
    block_tools?: string[];
    allow_tools?: string[];
}

/** Read the typed `policy` block off a written config for assertions. */
function policyOf(config: Record<string, unknown>): BoundaryPolicy {
    return (config.policy ?? {}) as BoundaryPolicy;
}

describe("readFormConfig", () => {
    it("falls back to safe defaults on an empty config", () => {
        // The boundary's unset default is trust-by-default, so the form reads "auto".
        expect(readFormConfig({})).toEqual({
            model: "",
            posture: "auto",
            allowNetwork: false,
            allowShell: true, // shell is allowed unless explicitly blocked
        });
    });

    it("reads model, posture, network, and a blocked shell", () => {
        expect(
            readFormConfig({
                model: "gpt-5.5",
                // The backend enum value — trust-by-default maps to the "auto" toggle.
                policy: { posture: "trust-by-default", allow_network: true, block_tools: ["bash"] },
            }),
        ).toEqual({ model: "gpt-5.5", posture: "auto", allowNetwork: true, allowShell: false });
    });

    it("maps the prompting postures onto the 'ask' toggle", () => {
        expect(readFormConfig({ policy: { posture: "prompt-on-risk" } }).posture).toBe("ask");
        expect(readFormConfig({ policy: { posture: "policy-only-block" } }).posture).toBe("ask");
    });

    it("treats a missing/garbage parse as defaults", () => {
        expect(readFormConfig(null)).toEqual({
            model: "",
            posture: "auto",
            allowNetwork: false,
            allowShell: true,
        });
    });
});

describe("writeFormConfig", () => {
    it("omits model when blank and writes the boundary's posture enum", () => {
        const out = writeFormConfig({}, { model: "", posture: "ask", allowNetwork: false, allowShell: true });
        expect(out.model).toBeUndefined();
        // The "ask" toggle must serialize to a valid boundary variant, not "ask".
        expect(policyOf(out).posture).toBe("prompt-on-risk");
        expect(policyOf(out).allow_network).toBe(false);
        expect(policyOf(out).block_tools).toBeUndefined();
    });

    it("writes trust-by-default for the 'auto' toggle", () => {
        const out = writeFormConfig({}, { model: "", posture: "auto", allowNetwork: false, allowShell: true });
        expect(policyOf(out).posture).toBe("trust-by-default");
    });

    it("blocks the shell by adding bash to block_tools", () => {
        const out = writeFormConfig({}, { model: "", posture: "ask", allowNetwork: false, allowShell: false });
        expect(policyOf(out).block_tools).toContain("bash");
    });

    it("preserves unknown top-level and policy keys (form ↔ Advanced JSON never fight)", () => {
        const prev = { provider: "openai-codex", policy: { allow_tools: ["read"] } };
        const out = writeFormConfig(prev, { model: "x", posture: "auto", allowNetwork: true, allowShell: true });
        expect(out.provider).toBe("openai-codex");
        expect(policyOf(out).allow_tools).toEqual(["read"]);
    });

    it("round-trips read→write→read", () => {
        const original = { model: "m", policy: { posture: "trust-by-default", allow_network: true, block_tools: ["bash"] } };
        const form = readFormConfig(original);
        const written = writeFormConfig(original, form);
        expect(readFormConfig(written)).toEqual(form);
    });
});

describe("plainConfigError", () => {
    it("collapses a parser error to one plain sentence", () => {
        expect(plainConfigError('Error: invalid config: {"error":"trailing characters at line 1 column 3"}')).toMatch(
            /isn't valid settings text/,
        );
    });
});

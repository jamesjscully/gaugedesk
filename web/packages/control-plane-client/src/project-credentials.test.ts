import { describe, expect, it } from "vitest";
import {
    linkProjectCredential,
    projectCredentials,
    unlinkProjectCredential,
} from "./control-plane-account";

/** Records every (method, path, body) a client call issues, returning a canned reply. */
function recorder(reply: unknown = {}) {
    const calls: { method: string; path: string; body?: unknown }[] = [];
    const json = async (method: string, path: string, body?: unknown) => {
        calls.push({ method, path, body });
        return reply;
    };
    return { json, calls };
}

describe("per-project credential client (LLM-2)", () => {
    it("lists a project's pinned providers, unwrapping the envelope", async () => {
        const { json, calls } = recorder({ credentials: [{ provider: "openai", linked: true }] });
        const out = await projectCredentials(json, "proj-1");
        expect(out).toEqual([{ provider: "openai", linked: true }]);
        expect(calls[0]).toEqual({ method: "GET", path: "/projects/proj-1/credentials", body: undefined });
    });

    it("pins a provider token at project scope", async () => {
        const { json, calls } = recorder();
        await linkProjectCredential(json, "proj-1", "anthropic", "sk-secret");
        expect(calls[0]).toEqual({
            method: "POST",
            path: "/projects/proj-1/credentials",
            body: { provider: "anthropic", token: "sk-secret" },
        });
    });

    it("unpins a provider and url-encodes the path segments", async () => {
        const { json, calls } = recorder();
        await unlinkProjectCredential(json, "proj/x", "open ai");
        expect(calls[0]).toEqual({
            method: "DELETE",
            path: "/projects/proj%2Fx/credentials/open%20ai",
            body: undefined,
        });
    });
});

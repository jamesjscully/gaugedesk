import { afterEach, describe, expect, it, vi } from "vitest";
import { engagementId } from "./control-plane-domain";
import { RemoteControlPlane } from "./remote-control-plane";

afterEach(() => vi.unstubAllGlobals());

describe("RemoteControlPlane", () => {
    it("routes product commands through the shared control-plane contract", async () => {
        const calls: unknown[][] = [];
        const route = async (method: string, path: string, body?: unknown) => {
            calls.push([method, path, body]);
            if (path === "/chats") return { engagements: ["chat-1"] };
            return { stopped: true };
        };
        const control = new RemoteControlPlane("https://cp.example/", { route });
        expect(await control.listEngagements()).toEqual([engagementId("chat-1")]);
        expect(await control.stopTurn(engagementId("chat-1"))).toEqual({ stopped: true });
        expect(calls).toEqual([
            ["GET", "/chats", undefined],
            ["POST", "/chats/chat-1/stop", undefined],
        ]);
    });

    it("authenticates raw file transport and never sends it over an ambient local path", async () => {
        const fetch = vi.fn(async () => new Response("hello", { status: 200 }));
        vi.stubGlobal("fetch", fetch);
        const control = new RemoteControlPlane("https://cp.example", {
            bearer: "member-token",
            route: async () => null,
        });
        expect(await control.getFile(engagementId("chat-1"), "notes/a b.md")).toBe("hello");
        expect(fetch).toHaveBeenCalledWith(
            "https://cp.example/chats/chat-1/file?path=notes%2Fa%20b.md",
            expect.objectContaining({
                method: "GET",
                credentials: "include",
                headers: expect.objectContaining({ authorization: "Bearer member-token" }),
            }),
        );
    });
});

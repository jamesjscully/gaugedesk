import { afterEach, describe, expect, it, vi } from "vitest";
import { browserRouteJson } from "./browser-route-json";
import { Rejected } from "./control-plane-domain";

/** Stub `fetch` with a canned Response for the one call under test. */
function stubFetch(res: Response) {
    vi.stubGlobal(
        "fetch",
        vi.fn(async () => res),
    );
}

afterEach(() => vi.unstubAllGlobals());

describe("browserRouteJson error surfacing", () => {
    it("includes the server's `error` message from a JSON body", async () => {
        stubFetch(
            new Response(JSON.stringify({ error: "no Pi runtime found" }), {
                status: 502,
                headers: { "content-type": "application/json" },
            }),
        );
        const json = browserRouteJson("http://cp");
        await expect(json("POST", "/account/oauth/openai-codex/start", {})).rejects.toThrow(
            /502 no Pi runtime found/,
        );
    });

    it("falls back to the raw body when it is not JSON", async () => {
        stubFetch(new Response("upstream exploded", { status: 500 }));
        const json = browserRouteJson("http://cp");
        await expect(json("GET", "/thing", undefined)).rejects.toThrow(/500 upstream exploded/);
    });

    it("falls back to the bare status when the body is empty", async () => {
        stubFetch(new Response(null, { status: 503 }));
        const json = browserRouteJson("http://cp");
        await expect(json("GET", "/thing", undefined)).rejects.toThrow(/GET \/thing: 503$/);
    });

    it("still maps 409 to Rejected with its reason", async () => {
        stubFetch(
            new Response(JSON.stringify({ rejected: "over budget" }), {
                status: 409,
                headers: { "content-type": "application/json" },
            }),
        );
        const json = browserRouteJson("http://cp");
        await expect(json("POST", "/scopes/s/run", {})).rejects.toThrowError(Rejected);
    });
});

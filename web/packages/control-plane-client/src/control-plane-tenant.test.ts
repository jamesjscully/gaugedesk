import { describe, expect, it, vi } from "vitest";
import {
    accountAttachFacility,
    accountDetachFacility,
    accountFacilities,
    accountTenants,
    parseFacility,
    parseTenant,
} from "./control-plane-tenant";
import type { RouteJson } from "./control-plane-transport";

/** A RouteJson double that records calls and returns a canned response. */
function fakeJson(response: unknown): { json: RouteJson; calls: [string, string, unknown?][] } {
    const calls: [string, string, unknown?][] = [];
    const json = vi.fn(async (method: string, path: string, body?: unknown) => {
        calls.push([method, path, body]);
        return response;
    }) as unknown as RouteJson;
    return { json, calls };
}

describe("control-plane-tenant (ADR 0077 §7/§9)", () => {
    it("lists facilities, mapping snake_case + defaulting missing fields", async () => {
        const { json, calls } = fakeJson({
            facilities: [
                { id: "lib", kind: "library_sync", owner: "person", status: "active", display_name: "Library sync" },
                { id: "bare" }, // degrades field-by-field
            ],
        });
        const out = await accountFacilities(json);
        expect(calls[0]).toEqual(["GET", "/account/facilities", undefined]);
        expect(out[0]).toEqual({
            id: "lib",
            kind: "library_sync",
            owner: "person",
            status: "active",
            displayName: "Library sync",
        });
        // a bare record defaults kind/owner/status, empty display name.
        expect(out[1]).toEqual({ id: "bare", kind: "library_sync", owner: "person", status: "active", displayName: "" });
    });

    it("attaches a facility, sending display_name (snake_case) and returning the parsed record", async () => {
        const { json, calls } = fakeJson({ facility: { id: "lib", kind: "library_sync", display_name: "Library sync" } });
        const f = await accountAttachFacility(json, { id: "lib", kind: "library_sync", displayName: "Library sync" });
        expect(calls[0]).toEqual(["POST", "/account/facilities", { id: "lib", kind: "library_sync", display_name: "Library sync" }]);
        expect(f.id).toBe("lib");
        expect(f.displayName).toBe("Library sync");
    });

    it("detaches a facility, url-encoding the id", async () => {
        const { json, calls } = fakeJson({});
        await accountDetachFacility(json, "lib/one");
        expect(calls[0]).toEqual(["DELETE", "/account/facilities/lib%2Fone", undefined]);
    });

    it("lists tenants and flags the personal one", async () => {
        const { json } = fakeJson({
            tenants: [{ id: "personal:root", display_name: "Personal", role: "owner", personal: true }],
        });
        const out = await accountTenants(json);
        expect(out).toEqual([{ id: "personal:root", displayName: "Personal", role: "owner", personal: true }]);
    });

    it("is total: garbage / empty envelopes degrade to empty lists, never throw", async () => {
        expect(await accountFacilities(fakeJson(null).json)).toEqual([]);
        expect(await accountFacilities(fakeJson({ facilities: "nope" }).json)).toEqual([]);
        expect(await accountTenants(fakeJson({}).json)).toEqual([]);
        expect(parseFacility(null).id).toBe("");
        expect(parseTenant(undefined).personal).toBe(false);
    });
});

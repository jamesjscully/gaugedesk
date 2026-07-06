import { describe, expect, it } from "vitest";
import { readDevMode } from "./dev-mode";

const store = (v: string | null): Pick<Storage, "getItem"> => ({ getItem: () => v });

describe("readDevMode — gate engine-only surfaces (round-6 #1)", () => {
    it("is off by default (no query, no storage)", () => {
        expect(readDevMode("", store(null))).toBe(false);
        expect(readDevMode("?foo=1", store(null))).toBe(false);
    });

    it("turns on via ?dev=1 or ?dev=true", () => {
        expect(readDevMode("?dev=1", store(null))).toBe(true);
        expect(readDevMode("?dev=true", store(null))).toBe(true);
    });

    it("turns on via the localStorage flag", () => {
        expect(readDevMode("", store("1"))).toBe(true);
        expect(readDevMode("", store("0"))).toBe(false);
    });

    it("an explicit ?dev=0 overrides a sticky storage flag", () => {
        expect(readDevMode("?dev=0", store("1"))).toBe(false);
    });

    it("tolerates a null storage", () => {
        expect(readDevMode("?dev=1", null)).toBe(true);
        expect(readDevMode("", null)).toBe(false);
    });
});

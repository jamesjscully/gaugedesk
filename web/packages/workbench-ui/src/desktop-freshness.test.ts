import { describe, expect, it } from "vitest";
import {
    DEFAULT_FRESHNESS_POLICY,
    deriveFreshness,
    freshnessEventForMarker,
    initialFreshness,
    isFresh,
    reduceFreshness,
    shouldOfferRetry,
    type FreshnessState,
} from "./desktop-freshness";

const POLICY = DEFAULT_FRESHNESS_POLICY;

describe("initial state", () => {
    it("is stuck before any success (no basis to vouch for)", () => {
        expect(deriveFreshness(initialFreshness, 0)).toBe("stuck");
    });
    it("offers no fresh truth and asks for a retry", () => {
        expect(isFresh("stuck")).toBe(false);
        expect(shouldOfferRetry("stuck")).toBe(true);
    });
});

describe("reduceFreshness", () => {
    it("an ok records the success time and clears any failure", () => {
        const failed = reduceFreshness(initialFreshness, { kind: "fail", error: "boom" });
        const ok = reduceFreshness(failed, { kind: "ok", now: 5_000 });
        expect(ok).toEqual({
            lastSuccessAt: 5_000,
            lastAttemptFailed: false,
            error: null,
            serverStale: false,
        });
        expect(deriveFreshness(ok, 5_000)).toBe("fresh");
    });

    it("a server-stale read holds data but derives to stale, not fresh (UX-13)", () => {
        // The load succeeded (we have a basis) but the carriage marker was non-live.
        const s = reduceFreshness(initialFreshness, {
            kind: "server-stale",
            now: 2_000,
            caveat: "server-declared stale",
        });
        expect(s.lastSuccessAt).toBe(2_000); // we hold the data
        expect(s.lastAttemptFailed).toBe(false); // the load itself did not fail
        expect(s.serverStale).toBe(true);
        expect(s.error).toBe("server-declared stale");
        expect(deriveFreshness(s, 2_000)).toBe("stale"); // shown with a caveat, not fresh
        // a subsequent live `ok` clears the server-stale flag back to fresh.
        const ok = reduceFreshness(s, { kind: "ok", now: 3_000 });
        expect(ok.serverStale).toBe(false);
        expect(deriveFreshness(ok, 3_000)).toBe("fresh");
    });

    it("a fail raises the flag and caveat but preserves the last success", () => {
        const ok = reduceFreshness(initialFreshness, { kind: "ok", now: 1_000 });
        const failed = reduceFreshness(ok, { kind: "fail", error: "network down" });
        expect(failed.lastSuccessAt).toBe(1_000); // prior basis preserved
        expect(failed.lastAttemptFailed).toBe(true);
        expect(failed.error).toBe("network down");
    });
});

describe("freshnessEventForMarker (UX-13)", () => {
    it("a live carriage marker is an ordinary ok success", () => {
        const ev = freshnessEventForMarker("live", null, 7);
        expect(ev).toEqual({ kind: "ok", now: 7 });
    });

    it("every non-live marker becomes a server-stale caveat", () => {
        for (const marker of ["stale", "partial", "redacted", "indeterminate"] as const) {
            const ev = freshnessEventForMarker(marker, null, 9);
            expect(ev.kind).toBe("server-stale");
            if (ev.kind === "server-stale") {
                expect(ev.now).toBe(9);
                expect(ev.caveat).toContain(marker);
            }
        }
    });

    it("prefers the server's repair hint as the caveat when present", () => {
        const ev = freshnessEventForMarker("redacted", "ask the owner to share", 3);
        expect(ev).toEqual({ kind: "server-stale", now: 3, caveat: "ask the owner to share" });
    });

    it("a non-live marker derives to stale (held data, caveat), not fresh", () => {
        const ev = freshnessEventForMarker("partial", null, 12);
        const s = reduceFreshness(initialFreshness, ev);
        expect(deriveFreshness(s, 12)).toBe("stale");
        expect(isFresh(deriveFreshness(s, 12))).toBe(false);
    });
});

describe("deriveFreshness", () => {
    const okThenFail = (successAt: number, error = "x"): FreshnessState =>
        reduceFreshness(
            reduceFreshness(initialFreshness, { kind: "ok", now: successAt }),
            { kind: "fail", error },
        );

    it("is fresh while the last attempt succeeded", () => {
        const ok = reduceFreshness(initialFreshness, { kind: "ok", now: 1_000 });
        expect(deriveFreshness(ok, 9_999_999)).toBe("fresh"); // success never ages on its own
    });

    it("is stale when a failure follows a recent success", () => {
        const s = okThenFail(1_000);
        expect(deriveFreshness(s, 1_000 + POLICY.stuckAfter)).toBe("stale");
        expect(deriveFreshness(s, 1_000 + POLICY.stuckAfter - 1)).toBe("stale");
    });

    it("is stuck when a failure follows an aged success", () => {
        const s = okThenFail(1_000);
        expect(deriveFreshness(s, 1_000 + POLICY.stuckAfter + 1)).toBe("stuck");
    });

    it("is stuck when a failure follows no success at all", () => {
        const s = reduceFreshness(initialFreshness, { kind: "fail", error: "first load failed" });
        expect(deriveFreshness(s, 0)).toBe("stuck");
        expect(deriveFreshness(s, 9_999_999)).toBe("stuck");
    });

    it("recovers to fresh after a successful retry", () => {
        const s = okThenFail(1_000);
        const recovered = reduceFreshness(s, { kind: "ok", now: 200_000 });
        expect(deriveFreshness(recovered, 200_000)).toBe("fresh");
    });
});

describe("status helpers", () => {
    it("only fresh is current; stale and stuck both ask for retry", () => {
        expect(isFresh("fresh")).toBe(true);
        expect(shouldOfferRetry("fresh")).toBe(false);
        expect(shouldOfferRetry("stale")).toBe(true);
        expect(shouldOfferRetry("stuck")).toBe(true);
    });
});

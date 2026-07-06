import { describe, expect, it } from "vitest";
import { scopeId } from "@gaugewright/control-plane-client";
import {
    clientRequestId,
    type Freshness,
    type FreshnessMarker,
    type ProjectionCarriage,
} from "@gaugewright/control-plane-client";
import {
    type CacheStorage,
    DEFAULT_DECAY,
    ProjectionCache,
    cacheKey,
    decayFreshness,
} from "./projection-cache";

/** A minimal in-memory `localStorage` stand-in (vitest runs without a DOM). */
function memStorage(): CacheStorage & { size: () => number } {
    const map = new Map<string, string>();
    return {
        getItem: (k) => (map.has(k) ? (map.get(k) as string) : null),
        setItem: (k, v) => void map.set(k, v),
        removeItem: (k) => void map.delete(k),
        size: () => map.size,
    };
}

function carriage<T>(
    value: T,
    marker: FreshnessMarker,
    generatedAt: number,
    rid: string | null = null,
): ProjectionCarriage<T> {
    const freshness: Freshness = { marker, generatedAt, repairHint: null };
    return {
        value,
        freshness,
        clientRequestId: rid == null ? null : clientRequestId(rid),
    };
}

const PEACH = scopeId("peach");

describe("cacheKey", () => {
    it("namespaces and percent-encodes scope and kind", () => {
        expect(cacheKey(scopeId("a/b"), "run")).toBe("gaugewright:projection:a%2Fb:run");
    });
    it("rejects an empty kind", () => {
        expect(() => cacheKey(PEACH, "")).toThrow();
    });
});

describe("decayFreshness", () => {
    it("keeps a live marker live within the stale window", () => {
        const f: Freshness = { marker: "live", generatedAt: 1_000, repairHint: null };
        const out = decayFreshness(f, 1_000 + DEFAULT_DECAY.staleAfter - 1);
        expect(out.marker).toBe("live");
        expect(out).toBe(f); // unchanged → same reference
    });

    it("decays a live marker to stale past the stale threshold", () => {
        const f: Freshness = { marker: "live", generatedAt: 0, repairHint: null };
        const out = decayFreshness(f, DEFAULT_DECAY.staleAfter);
        expect(out.marker).toBe("stale");
        expect(out.generatedAt).toBe(0);
        expect(out.repairHint).toBeTruthy(); // gains a refresh affordance
    });

    it("decays to indeterminate past the hard TTL", () => {
        const f: Freshness = { marker: "live", generatedAt: 0, repairHint: null };
        const out = decayFreshness(f, DEFAULT_DECAY.indeterminateAfter);
        expect(out.marker).toBe("indeterminate");
    });

    it("never raises confidence: a stale marker stays stale when fresh-aged", () => {
        const f: Freshness = { marker: "stale", generatedAt: 0, repairHint: "x" };
        const out = decayFreshness(f, 1);
        expect(out.marker).toBe("stale");
        expect(out).toBe(f);
    });

    it("preserves a redacted caveat within the stale window", () => {
        const f: Freshness = { marker: "redacted", generatedAt: 0, repairHint: null };
        const out = decayFreshness(f, 1);
        expect(out.marker).toBe("redacted");
    });

    it("still decays a redacted caveat to indeterminate past the TTL", () => {
        const f: Freshness = { marker: "redacted", generatedAt: 0, repairHint: null };
        const out = decayFreshness(f, DEFAULT_DECAY.indeterminateAfter);
        expect(out.marker).toBe("indeterminate");
    });

    it("does not mutate the input", () => {
        const f: Freshness = { marker: "live", generatedAt: 0, repairHint: null };
        decayFreshness(f, DEFAULT_DECAY.indeterminateAfter);
        expect(f.marker).toBe("live");
        expect(f.repairHint).toBeNull();
    });
});

describe("ProjectionCache", () => {
    it("returns null on a miss", () => {
        const cache = new ProjectionCache(memStorage());
        expect(cache.get(PEACH, "run", 0)).toBeNull();
    });

    it("round-trips a live carriage read back inside the stale window", () => {
        const cache = new ProjectionCache(memStorage());
        cache.put(PEACH, "run", carriage({ n: 7 }, "live", 1_000, "cmd-1"));
        const out = cache.get<{ n: number }>(PEACH, "run", 1_000);
        expect(out).not.toBeNull();
        expect(out!.value).toEqual({ n: 7 });
        expect(out!.freshness.marker).toBe("live");
        expect(String(out!.clientRequestId)).toBe("cmd-1");
    });

    it("never serves a stored live entry as live once it has aged out", () => {
        const cache = new ProjectionCache(memStorage());
        cache.put(PEACH, "run", carriage({ n: 7 }, "live", 0));
        const out = cache.get<{ n: number }>(PEACH, "run", DEFAULT_DECAY.staleAfter);
        expect(out!.freshness.marker).toBe("stale");
        // The value survives; only the confidence drops.
        expect(out!.value).toEqual({ n: 7 });
    });

    it("overwrites a prior entry for the same scope+kind", () => {
        const store = memStorage();
        const cache = new ProjectionCache(store);
        cache.put(PEACH, "run", carriage({ n: 1 }, "live", 0));
        cache.put(PEACH, "run", carriage({ n: 2 }, "live", 0));
        expect(store.size()).toBe(1);
        expect(cache.get<{ n: number }>(PEACH, "run", 0)!.value).toEqual({ n: 2 });
    });

    it("keeps distinct scope+kind entries apart", () => {
        const cache = new ProjectionCache(memStorage());
        cache.put(PEACH, "run", carriage("r", "live", 0));
        cache.put(PEACH, "review", carriage("v", "live", 0));
        cache.put(scopeId("plum"), "run", carriage("p", "live", 0));
        expect(cache.get<string>(PEACH, "run", 0)!.value).toBe("r");
        expect(cache.get<string>(PEACH, "review", 0)!.value).toBe("v");
        expect(cache.get<string>(scopeId("plum"), "run", 0)!.value).toBe("p");
    });

    it("evicts an entry", () => {
        const cache = new ProjectionCache(memStorage());
        cache.put(PEACH, "run", carriage("r", "live", 0));
        cache.evict(PEACH, "run");
        expect(cache.get(PEACH, "run", 0)).toBeNull();
    });

    it("treats a corrupt stored entry as a miss and drops it", () => {
        const store = memStorage();
        store.setItem(cacheKey(PEACH, "run"), "{not json");
        const cache = new ProjectionCache(store);
        expect(cache.get(PEACH, "run", 0)).toBeNull();
        expect(store.size()).toBe(0);
    });

    it("runs the stored value through an injected parser", () => {
        const cache = new ProjectionCache(memStorage(), DEFAULT_DECAY, (v) => ({
            wrapped: v,
        }));
        cache.put(PEACH, "run", carriage(42, "live", 0));
        expect(cache.get(PEACH, "run", 0)!.value).toEqual({ wrapped: 42 });
    });
});

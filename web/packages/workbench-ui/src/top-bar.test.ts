import { describe, expect, it } from "vitest";
import {
    contextHeader,
    dotState,
    dotView,
    nextTaskBadge,
    topBarView,
    type DotState,
} from "./top-bar";
import { type ConnectionStatus } from "./connection";
import { type FreshnessMarker } from "@gaugewright/control-plane-client";
import { PANE_ORDER, type CarouselState } from "./mobile-layout";

const ALL_STATUSES: readonly ConnectionStatus[] = [
    "unpaired",
    "paired",
    "active",
    "offline",
    "revoked",
    "expired",
];
const ALL_MARKERS: readonly FreshnessMarker[] = [
    "live",
    "stale",
    "partial",
    "redacted",
    "indeterminate",
];

describe("context header (passive awareness)", () => {
    it("reads chat · environment, deepest first (spec: price-leveling · Peach)", () => {
        const header = contextHeader("price-leveling", "Peach");
        expect(header.label).toBe("price-leveling · Peach");
        expect(header.returnsToNav).toBe(true);
    });

    it("shows the environment alone when no chat is addressed", () => {
        expect(contextHeader(null, "Peach").label).toBe("Peach");
    });

    it("is idle — no label, no return — when nothing is addressed", () => {
        const header = contextHeader(null, null);
        expect(header.label).toBe(null);
        expect(header.returnsToNav).toBe(false);
    });

    it("treats an empty-string label as no label (no phantom · separator)", () => {
        expect(contextHeader("", "").label).toBe(null);
        expect(contextHeader("chat", "").label).toBe("chat");
    });
});

describe("freshness / connection dot (the law-bearing fold)", () => {
    it("only `live` reads as current truth", () => {
        const fresh: Array<[ConnectionStatus, FreshnessMarker | null]> = [
            ["active", "live"],
            ["paired", null],
        ];
        for (const [status, marker] of fresh) {
            expect(dotView(status, marker).isCurrent).toBe(true);
        }
    });

    it("connection trouble dominates the freshness marker", () => {
        // Even a `live` projection cannot make a downed bridge read as live.
        const degraded: Array<[ConnectionStatus, DotState]> = [
            ["offline", "offline"],
            ["revoked", "revoked"],
            ["expired", "expired"],
            ["unpaired", "unpaired"],
        ];
        for (const [status, expected] of degraded) {
            for (const marker of [...ALL_MARKERS, null]) {
                expect(dotState(status, marker)).toBe(expected);
            }
        }
    });

    it("when connected, the freshness marker decides the dot", () => {
        expect(dotState("active", "live")).toBe("live");
        expect(dotState("active", "stale")).toBe("stale");
        expect(dotState("active", "partial")).toBe("caveated");
        expect(dotState("active", "redacted")).toBe("caveated");
        expect(dotState("active", "indeterminate")).toBe("caveated");
    });

    it("a connected bridge with no projection on screen is still live", () => {
        expect(dotState("active", null)).toBe("live");
        expect(dotState("paired", null)).toBe("live");
    });

    it("never paints `live` while the bridge is degraded (no false-current dot)", () => {
        for (const status of ALL_STATUSES) {
            for (const marker of [...ALL_MARKERS, null]) {
                const view = dotView(status, marker);
                if (view.state === "live") {
                    // The only way to a live dot is a connected bridge + live (or no) marker.
                    expect(status === "active" || status === "paired").toBe(true);
                    expect(marker === "live" || marker === null).toBe(true);
                }
                // isCurrent is exactly the live-dot predicate.
                expect(view.isCurrent).toBe(view.state === "live");
            }
        }
    });

    it("captions every dot state", () => {
        for (const status of ALL_STATUSES) {
            for (const marker of [...ALL_MARKERS, null]) {
                expect(dotView(status, marker).label.length).toBeGreaterThan(0);
            }
        }
    });
});

describe("next-task badge (header affordance, not a carousel stop)", () => {
    it("shows the depth as the badge count", () => {
        const badge = nextTaskBadge(3);
        expect(badge.depth).toBe(3);
        expect(badge.visible).toBe(true);
        expect(badge.hasCurrent).toBe(true);
    });

    it("hides entirely at an empty queue", () => {
        const badge = nextTaskBadge(0);
        expect(badge.visible).toBe(false);
        expect(badge.hasCurrent).toBe(false);
        expect(badge.depth).toBe(0);
    });

    it("clamps a negative / non-finite depth to a hidden empty badge", () => {
        for (const bad of [-1, -10, NaN, Infinity, -Infinity]) {
            const badge = nextTaskBadge(bad);
            expect(badge.depth).toBe(0);
            expect(badge.visible).toBe(false);
        }
    });

    it("floors a fractional depth", () => {
        expect(nextTaskBadge(2.9).depth).toBe(2);
    });
});

describe("composed top-bar view", () => {
    const carousel: CarouselState = {
        current: "chat",
        selection: { chatSelected: true, fileSelected: false },
    };

    it("derives context, dot, next-task and the canonical toggle in one pass", () => {
        const view = topBarView({
            carousel,
            status: "active",
            freshness: "live",
            chatTitle: "price-leveling",
            environment: "Peach",
            queueDepth: 2,
        });
        expect(view.context.label).toBe("price-leveling · Peach");
        expect(view.dot.state).toBe("live");
        expect(view.dot.isCurrent).toBe(true);
        expect(view.nextTask.depth).toBe(2);
        // The toggle is exactly the canonical MOB-014 control, in canonical order.
        expect(view.toggle.map((s) => s.pane)).toEqual([...PANE_ORDER]);
        expect(view.toggle.filter((s) => s.current).map((s) => s.pane)).toEqual(["chat"]);
    });

    it("a stale projection over a live bridge yields a non-current dot but still a context", () => {
        const view = topBarView({
            carousel,
            status: "active",
            freshness: "stale",
            chatTitle: "price-leveling",
            environment: "Peach",
            queueDepth: 0,
        });
        expect(view.dot.state).toBe("stale");
        expect(view.dot.isCurrent).toBe(false);
        expect(view.context.label).toBe("price-leveling · Peach");
        expect(view.nextTask.visible).toBe(false);
    });
});

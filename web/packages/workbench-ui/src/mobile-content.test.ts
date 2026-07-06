import { describe, expect, it } from "vitest";
import {
    accessDenial,
    freshnessCaveat,
    presentContent,
    type ContentPayload,
    type SelectedHandle,
} from "./mobile-content";
import type { AccessPhase } from "./mobile-files";
import type {
    Freshness,
    FreshnessMarker,
    ProjectionCarriage,
} from "@gaugewright/control-plane-client";

const ALL_PHASES: readonly AccessPhase[] = [
    "init",
    "requested",
    "granted",
    "revoked",
    "denied",
];

const ALL_MARKERS: readonly FreshnessMarker[] = [
    "live",
    "stale",
    "partial",
    "redacted",
    "indeterminate",
];

function fresh(marker: FreshnessMarker, repairHint: string | null = null): Freshness {
    return { marker, generatedAt: 1, repairHint };
}

function carriage(
    diff: string,
    marker: FreshnessMarker,
    repairHint: string | null = null,
): ProjectionCarriage<ContentPayload> {
    return {
        value: { path: "notes.md", diff },
        freshness: fresh(marker, repairHint),
        clientRequestId: null,
    };
}

function handle(
    access: AccessPhase,
    payload: ProjectionCarriage<ContentPayload> | null = null,
): SelectedHandle {
    return { path: "notes.md", access, payload };
}

describe("empty pane (nothing selected)", () => {
    it("renders the empty panel with no body, caveat, or denial", () => {
        const p = presentContent({ selection: null });
        expect(p.kind).toBe("empty");
        expect(p.path).toBeNull();
        expect(p.diff).toBeNull();
        expect(p.freshnessCaveat).toBeNull();
        expect(p.denial).toBeNull();
    });
});

describe("access gates the body (INV-10: holding a handle is not holding the payload)", () => {
    it("only a granted, carried handle reaches the body panel", () => {
        for (const phase of ALL_PHASES) {
            const carried = handle(phase, carriage("@@ diff @@", "live"));
            const p = presentContent({ selection: carried });
            expect(p.kind).toBe(phase === "granted" ? "body" : "denied");
        }
    });

    it("a non-granted phase yields an access-denied panel, never a silent blank", () => {
        for (const phase of ["init", "requested", "revoked", "denied"] as const) {
            const p = presentContent({ selection: handle(phase) });
            expect(p.kind).toBe("denied");
            expect(p.diff).toBeNull();
            expect(p.denial?.phase).toBe(phase);
            // The name survives an access denial.
            expect(p.path).toBe("notes.md");
        }
    });

    it("a granted handle whose payload the wire did not carry is denied, not blank", () => {
        const p = presentContent({ selection: handle("granted", null) });
        expect(p.kind).toBe("denied");
        expect(p.diff).toBeNull();
        // Falls back to the most-conservative requestable denial rather than
        // claiming a granted body it does not have.
        expect(p.denial?.phase).toBe("init");
        expect(p.path).toBe("notes.md");
    });

    it("a granted, carried handle renders the diff body", () => {
        const p = presentContent({ selection: handle("granted", carriage("@@ body @@", "live")) });
        expect(p.kind).toBe("body");
        expect(p.diff).toBe("@@ body @@");
        expect(p.denial).toBeNull();
        expect(p.path).toBe("notes.md");
    });
});

describe("freshness caveat over a granted body (a non-live body is never bare current truth)", () => {
    it("a live body carries no caveat", () => {
        const p = presentContent({ selection: handle("granted", carriage("x", "live")) });
        expect(p.kind).toBe("body");
        expect(p.freshnessCaveat).toBeNull();
    });

    it("every non-live marker surfaces a captioned caveat over the body", () => {
        for (const marker of ALL_MARKERS) {
            const p = presentContent({ selection: handle("granted", carriage("x", marker)) });
            if (marker === "live") {
                expect(p.freshnessCaveat).toBeNull();
            } else {
                expect(p.freshnessCaveat?.marker).toBe(marker);
                expect(p.freshnessCaveat?.label).toBeTruthy();
            }
        }
    });

    it("carries the repair hint through when the projection offered one", () => {
        const p = presentContent({ selection: handle("granted", carriage("x", "stale", "pull to refresh")) });
        expect(p.freshnessCaveat?.repairHint).toBe("pull to refresh");
    });

    it("a withheld body never carries a freshness caveat (access decided first)", () => {
        const p = presentContent({ selection: handle("requested") });
        expect(p.kind).toBe("denied");
        expect(p.freshnessCaveat).toBeNull();
    });
});

describe("freshnessCaveat (pure)", () => {
    it("returns null only for live", () => {
        for (const marker of ALL_MARKERS) {
            expect(freshnessCaveat(fresh(marker)) === null).toBe(marker === "live");
        }
    });
});

describe("accessDenial requestability (you may ask for the payload only when it makes sense)", () => {
    it("init and revoked are requestable", () => {
        for (const phase of ["init", "revoked"] as const) {
            expect(accessDenial(phase).requestable).toBe(true);
        }
    });

    it("a pending request and a terminal denial are not re-requestable", () => {
        for (const phase of ["requested", "denied"] as const) {
            expect(accessDenial(phase).requestable).toBe(false);
        }
    });

    it("every denial carries a human label", () => {
        for (const phase of ALL_PHASES) {
            expect(accessDenial(phase).label).toBeTruthy();
        }
    });
});

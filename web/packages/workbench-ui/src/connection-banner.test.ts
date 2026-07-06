import { describe, expect, it } from "vitest";
import {
    canSendOnConnection,
    connectionBanner,
    type ConnectionBannerView,
} from "./connection-banner";
import { canCommand, type ConnectionStatus } from "./connection";

const ALL_STATUSES: readonly ConnectionStatus[] = [
    "unpaired",
    "paired",
    "active",
    "offline",
    "revoked",
    "expired",
];

/** The degraded statuses that surface a banner — every status that is neither
 *  command-capable (`active`) nor the idle-but-usable bridge (`paired`). */
const BANNERED: readonly ConnectionStatus[] = ["offline", "revoked", "expired", "unpaired"];

describe("connection banner (shown iff the connection cannot command)", () => {
    it("shows no banner on the happy path (active can command)", () => {
        expect(connectionBanner("active")).toBeNull();
    });

    it("shows no banner when idle-but-usable (paired: nothing addressed to command)", () => {
        expect(connectionBanner("paired")).toBeNull();
    });

    it("shows a banner for every degraded status", () => {
        for (const status of BANNERED) {
            const banner = connectionBanner(status);
            expect(banner, status).not.toBeNull();
            expect((banner as ConnectionBannerView).status).toBe(status);
            // Every banner carries a non-empty, human caption.
            expect((banner as ConnectionBannerView).message.length).toBeGreaterThan(0);
        }
    });

    it("LAW: a banner is shown exactly when the connection cannot command (and is addressed)", () => {
        // The banner predicate is the negation of canCommand, minus the idle
        // `paired` state — so banner ⇔ (!canCommand ∧ addressed). This ties the
        // banner to the same gate the composer's send uses; they cannot drift.
        for (const status of ALL_STATUSES) {
            const hasBanner = connectionBanner(status) !== null;
            const shouldShow = !canCommand(status) && status !== "paired";
            expect(hasBanner, status).toBe(shouldShow);
        }
    });
});

describe("banner severity and recovery", () => {
    it("offline is info, self-healing, and keeps cached reads available", () => {
        const banner = connectionBanner("offline") as ConnectionBannerView;
        expect(banner.severity).toBe("info");
        // Offline self-heals when the relay returns — there is nothing to repair.
        expect(banner.repairHint).toBeNull();
        // A usable grant is still held, so cached projections may be shown.
        expect(banner.cachedReadsAvailable).toBe(true);
    });

    it("grant-failure statuses are warnings with a repair hint and no cached reads", () => {
        for (const status of ["revoked", "expired", "unpaired"] as const) {
            const banner = connectionBanner(status) as ConnectionBannerView;
            expect(banner.severity, status).toBe("warning");
            // The grant itself is unusable — the user (or owner) must act.
            expect(banner.repairHint, status).not.toBeNull();
            expect((banner.repairHint as string).length, status).toBeGreaterThan(0);
            expect(banner.cachedReadsAvailable, status).toBe(false);
        }
    });
});

describe("send gate (disable send when offline / degraded)", () => {
    it("permits a send only when the connection can command (active)", () => {
        for (const status of ALL_STATUSES) {
            expect(canSendOnConnection(status), status).toBe(status === "active");
        }
    });

    it("LAW: the send gate and the banner agree — a send is refused exactly when a banner shows (addressed)", () => {
        // The one predicate behind both the disabled send and the visible notice:
        // an addressed-but-degraded connection never both shows the banner and
        // lets the send through.
        for (const status of BANNERED) {
            expect(canSendOnConnection(status), status).toBe(false);
            expect(connectionBanner(status), status).not.toBeNull();
        }
        // …and the command-capable state does neither.
        expect(canSendOnConnection("active")).toBe(true);
        expect(connectionBanner("active")).toBeNull();
    });
});

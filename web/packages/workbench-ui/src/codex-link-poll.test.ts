import { describe, expect, it } from "vitest";
import { waitForCodexLink } from "./codex-link-poll";

const instant = () => Promise.resolve();

describe("waitForCodexLink", () => {
    it("resolves true once the status flips to linked", async () => {
        let polls = 0;
        const status = async () => ({ linked: ++polls >= 3, expired: false, expires: 99 });
        await expect(
            waitForCodexLink(status, { sleep: instant }),
        ).resolves.toBe(true);
        expect(polls).toBe(3);
    });

    it("does not treat an expired link as signed in", async () => {
        const status = async () => ({ linked: true, expired: true, expires: 1 });
        await expect(
            waitForCodexLink(status, { intervalMs: 1, timeoutMs: 3, sleep: instant }),
        ).resolves.toBe(false);
    });

    it("re-sign-in: ignores the old credential until the expiry changes", async () => {
        let polls = 0;
        // The pre-sign-in credential (expires 100) stays linked while the user
        // re-authenticates; the new bundle lands on the third poll (expires 200).
        const status = async () => ({ linked: true, expired: false, expires: ++polls >= 3 ? 200 : 100 });
        await expect(
            waitForCodexLink(status, { baselineExpires: 100, sleep: instant }),
        ).resolves.toBe(true);
        expect(polls).toBe(3);
    });

    it("keeps polling through transient status failures", async () => {
        let polls = 0;
        const status = async () => {
            polls += 1;
            if (polls < 3) throw new Error("control plane busy");
            return { linked: true, expired: false };
        };
        await expect(
            waitForCodexLink(status, { sleep: instant }),
        ).resolves.toBe(true);
    });

    it("stops when cancelled", async () => {
        let polls = 0;
        const status = async () => {
            polls += 1;
            return { linked: false, expired: false };
        };
        await expect(
            waitForCodexLink(status, { cancelled: () => polls >= 2, sleep: instant }),
        ).resolves.toBe(false);
        expect(polls).toBe(2);
    });

    it("gives up after the poll budget", async () => {
        let polls = 0;
        const status = async () => {
            polls += 1;
            return { linked: false, expired: false };
        };
        await expect(
            waitForCodexLink(status, { intervalMs: 10, timeoutMs: 50, sleep: instant }),
        ).resolves.toBe(false);
        expect(polls).toBe(5);
    });
});

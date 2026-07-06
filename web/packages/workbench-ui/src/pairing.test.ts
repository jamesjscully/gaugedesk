import { describe, expect, it } from "vitest";
import {
    deriveStep,
    initialPairing,
    pairingTicket,
    parsePairingStatus,
    parseTicket,
    presentPairing,
    reducePairing,
    type PairingState,
    type PairingStatus,
    type PairingTicket,
} from "./pairing";

// ----- Ticket minting (owner side, MOB-F5) -----------------------------------

describe("pairingTicket — the owner-minted ticket round-trips through parseTicket", () => {
    it("builds gaugewright-pair://<env>/<device> and parses back identically", () => {
        const raw = pairingTicket("local", "device:abc123");
        expect(raw).toBe("gaugewright-pair://local/device:abc123");
        const t = parseTicket(raw);
        expect(t).not.toBeNull();
        expect(t!.environment).toBe("local");
        expect(t!.device).toBe("device:abc123");
        expect(t!.bridgeGrant).toBeNull();
    });
});

// ----- Ticket parsing --------------------------------------------------------

describe("parseTicket — opaque QR/code payload becomes a typed ticket", () => {
    it("parses environment + device (grant minted server-side)", () => {
        const t = parseTicket("gaugewright-pair://peach/device:pixel-9");
        expect(t).not.toBeNull();
        expect(t!.environment).toBe("peach");
        expect(t!.device).toBe("device:pixel-9");
        expect(t!.bridgeGrant).toBeNull();
    });

    it("parses environment + device + explicit bridge grant", () => {
        const t = parseTicket("gaugewright-pair://peach/device:pixel-9/grant-7");
        expect(t!.bridgeGrant).toBe("grant-7");
    });

    it("trims surrounding whitespace before parsing", () => {
        expect(parseTicket("  gaugewright-pair://peach/device:x  ")).not.toBeNull();
    });

    it("rejects a payload with the wrong scheme", () => {
        expect(parseTicket("https://peach/device:x")).toBeNull();
        expect(parseTicket("gaugewright://peach/chat/42")).toBeNull();
    });

    it("rejects a payload missing the device segment", () => {
        expect(parseTicket("gaugewright-pair://peach")).toBeNull();
    });

    it("rejects a payload with too many segments", () => {
        expect(parseTicket("gaugewright-pair://peach/device/grant/extra")).toBeNull();
    });
});

// ----- Status parsing (mirror pairing_status_json) ---------------------------

describe("parsePairingStatus — wire status folds into the branded type", () => {
    it("parses a bound, not-yet-accepted DeviceBinding status", () => {
        const s = parsePairingStatus({
            phase: "DeviceBinding",
            bound: { device: "device:pixel-9", bridge_grant: "grant-7" },
            paired: false,
        });
        expect(s.phase).toBe("DeviceBinding");
        expect(s.bound).toEqual({ device: "device:pixel-9", bridgeGrant: "grant-7" });
        expect(s.paired).toBe(false);
    });

    it("parses an Active, accepted (paired) status", () => {
        const s = parsePairingStatus({ phase: "Active", bound: null, paired: true });
        expect(s.paired).toBe(true);
    });

    it("degrades an unknown phase to Proposed (never reads as paired)", () => {
        const s = parsePairingStatus({ phase: "Bogus", bound: null, paired: false });
        expect(s.phase).toBe("Proposed");
    });

    it("treats a missing/non-true paired flag as not paired", () => {
        expect(parsePairingStatus({ phase: "Active" }).paired).toBe(false);
        expect(parsePairingStatus({ phase: "Active", paired: "yes" as unknown }).paired).toBe(false);
    });

    it("ignores a malformed bound object", () => {
        expect(parsePairingStatus({ phase: "DeviceBinding", bound: { device: "x" } }).bound).toBeNull();
    });
});

// ----- Pure derivation -------------------------------------------------------

const TICKET: PairingTicket = parseTicket("gaugewright-pair://peach/device:pixel-9")!;

function status(phase: PairingStatus["phase"], paired: boolean): PairingStatus {
    return { phase, bound: null, paired };
}

describe("deriveStep — the step is a pure function of the facts", () => {
    it("is entry with nothing entered", () => {
        expect(deriveStep(null, null, null, null)).toBe("entry");
    });

    it("is submitting once a ticket is parsed but no request landed", () => {
        expect(deriveStep(TICKET, null, null, null)).toBe("submitting");
    });

    it("is awaiting-approval once a request is accepted", () => {
        expect(deriveStep(TICKET, "pairing-1", null, null)).toBe("awaiting-approval");
    });

    it("is awaiting-approval while bound but not yet accepted", () => {
        expect(deriveStep(TICKET, "pairing-1", status("DeviceBinding", false), null)).toBe(
            "awaiting-approval",
        );
    });

    it("is paired once the server reports the boundary active", () => {
        expect(deriveStep(TICKET, "pairing-1", status("Active", true), null)).toBe("paired");
    });

    it("is failed whenever an error is recorded (error wins over any progress)", () => {
        expect(deriveStep(TICKET, "pairing-1", status("Active", true), "boom")).toBe("failed");
    });
});

// ----- The reducer (the user-facing arc) -------------------------------------

describe("reducePairing — the pairing arc", () => {
    it("starts at entry with clean facts", () => {
        expect(initialPairing.step).toBe("entry");
        expect(initialPairing.ticket).toBeNull();
        expect(initialPairing.pairingId).toBeNull();
    });

    it("walks entry → submitting → awaiting → paired", () => {
        let s: PairingState = initialPairing;
        s = reducePairing(s, { kind: "ticket-entered", ticket: TICKET });
        expect(s.step).toBe("submitting");
        s = reducePairing(s, { kind: "request-accepted", pairingId: "pairing-1" });
        expect(s.step).toBe("awaiting-approval");
        expect(s.pairingId).toBe("pairing-1");
        s = reducePairing(s, { kind: "status", status: status("DeviceBinding", false) });
        expect(s.step).toBe("awaiting-approval");
        s = reducePairing(s, { kind: "status", status: status("Active", true) });
        expect(s.step).toBe("paired");
    });

    it("fails on an invalid (unparseable) ticket with an explicit reason", () => {
        const s = reducePairing(initialPairing, { kind: "ticket-entered", ticket: null });
        expect(s.step).toBe("failed");
        expect(s.error).toBe("invalid ticket");
    });

    it("fails when the owner denies the pairing (never sits awaiting forever)", () => {
        let s: PairingState = reducePairing(initialPairing, { kind: "ticket-entered", ticket: TICKET });
        s = reducePairing(s, { kind: "request-accepted", pairingId: "pairing-1" });
        s = reducePairing(s, { kind: "status", status: status("Denied", false) });
        expect(s.step).toBe("failed");
        expect(s.error).toBe("the owner denied the pairing");
    });

    it("fails when the boundary is torn down", () => {
        const s = reducePairing(
            { ...initialPairing, pairingId: "pairing-1", step: "awaiting-approval" },
            { kind: "status", status: status("TornDown", false) },
        );
        expect(s.step).toBe("failed");
        expect(s.error).toBe("the pairing was torn down");
    });

    it("fails on a transport error", () => {
        const s = reducePairing(initialPairing, { kind: "error", reason: "network down" });
        expect(s.step).toBe("failed");
        expect(s.error).toBe("network down");
    });

    it("retry resets a failed flow back to a clean entry", () => {
        let s = reducePairing(initialPairing, { kind: "error", reason: "boom" });
        expect(s.step).toBe("failed");
        s = reducePairing(s, { kind: "retry" });
        expect(s).toBe(initialPairing);
    });

    it("is a no-op (same reference) when an event moves no fact", () => {
        const s = reducePairing(initialPairing, { kind: "retry" });
        expect(s).toBe(initialPairing);
        const accepted = reducePairing(
            { ...initialPairing, pairingId: "pairing-1" },
            { kind: "request-accepted", pairingId: "pairing-1" },
        );
        expect(accepted.pairingId).toBe("pairing-1");
    });

    it("a fresh status while paired holds is not lost (idempotent poll)", () => {
        let s: PairingState = reducePairing(initialPairing, { kind: "ticket-entered", ticket: TICKET });
        s = reducePairing(s, { kind: "request-accepted", pairingId: "pairing-1" });
        s = reducePairing(s, { kind: "status", status: status("Active", true) });
        const again = reducePairing(s, { kind: "status", status: status("Active", true) });
        expect(again.step).toBe("paired");
    });
});

// ----- Presentation ----------------------------------------------------------

describe("presentPairing — the island's render plan", () => {
    it("offers submit only at entry", () => {
        expect(presentPairing(initialPairing).canSubmit).toBe(true);
        const submitting = reducePairing(initialPairing, { kind: "ticket-entered", ticket: TICKET });
        expect(presentPairing(submitting).canSubmit).toBe(false);
    });

    it("marks the waiting steps as waiting and unsettled", () => {
        const submitting = reducePairing(initialPairing, { kind: "ticket-entered", ticket: TICKET });
        const p = presentPairing(submitting);
        expect(p.waiting).toBe(true);
        expect(p.settled).toBe(false);
    });

    it("marks paired and failed as settled, and offers retry only on failure", () => {
        const paired = presentPairing({ ...initialPairing, step: "paired" });
        expect(paired.settled).toBe(true);
        expect(paired.canRetry).toBe(false);

        const failed = presentPairing({ ...initialPairing, step: "failed", error: "boom" });
        expect(failed.settled).toBe(true);
        expect(failed.canRetry).toBe(true);
        expect(failed.error).toBe("boom");
    });
});

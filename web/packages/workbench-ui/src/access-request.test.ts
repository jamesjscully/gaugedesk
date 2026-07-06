import { describe, expect, it } from "vitest";
import {
    initialAccessRequest,
    presentAccessRequest,
    reduceAccessRequest,
    type AccessRequestState,
} from "./access-request";
import { type AccessPhase } from "./mobile-files";
import { clientRequestId } from "@gaugewright/control-plane-client";

const ALL_PHASES: readonly AccessPhase[] = [
    "init",
    "requested",
    "granted",
    "revoked",
    "denied",
];

const RID = clientRequestId("req-1");

function at(phase: AccessPhase): AccessRequestState {
    return initialAccessRequest("notes.md", phase);
}

describe("initial state", () => {
    it("opens for a handle at the given phase with no ask in flight", () => {
        const s = initialAccessRequest("notes.md", "init");
        expect(s.path).toBe("notes.md");
        expect(s.phase).toBe("init");
        expect(s.requestId).toBeNull();
    });
});

describe("submit (the user asks for the payload)", () => {
    it("moves a withheld (init) handle to requested under the correlation id", () => {
        const s = reduceAccessRequest(at("init"), { kind: "submit", requestId: RID });
        expect(s.phase).toBe("requested");
        expect(s.requestId).toBe(RID);
    });

    it("re-requests a revoked handle (a withdrawn grant may be asked for again)", () => {
        const s = reduceAccessRequest(at("revoked"), { kind: "submit", requestId: RID });
        expect(s.phase).toBe("requested");
        expect(s.requestId).toBe(RID);
    });

    it("is a no-op from a phase that cannot request (never claim an ask the owner would refuse)", () => {
        for (const phase of ["requested", "granted", "denied"] as const) {
            const before = at(phase);
            const after = reduceAccessRequest(before, { kind: "submit", requestId: RID });
            expect(after).toBe(before); // same reference: pure no-op
        }
    });
});

describe("cancel (withdraw a pending request)", () => {
    it("returns a pending request to init with no ask in flight", () => {
        const requested = reduceAccessRequest(at("init"), { kind: "submit", requestId: RID });
        const canceled = reduceAccessRequest(requested, { kind: "cancel" });
        expect(canceled.phase).toBe("init");
        expect(canceled.requestId).toBeNull();
    });

    it("is a no-op when no request is pending", () => {
        for (const phase of ["init", "granted", "revoked", "denied"] as const) {
            const before = at(phase);
            expect(reduceAccessRequest(before, { kind: "cancel" })).toBe(before);
        }
    });
});

describe("phase (the server's answer is authoritative)", () => {
    it("the owner granting moves a pending request to granted and clears the id", () => {
        const requested = reduceAccessRequest(at("init"), { kind: "submit", requestId: RID });
        const granted = reduceAccessRequest(requested, { kind: "phase", phase: "granted" });
        expect(granted.phase).toBe("granted");
        expect(granted.requestId).toBeNull();
    });

    it("the owner denying settles the request and clears the id", () => {
        const requested = reduceAccessRequest(at("init"), { kind: "submit", requestId: RID });
        const denied = reduceAccessRequest(requested, { kind: "phase", phase: "denied" });
        expect(denied.phase).toBe("denied");
        expect(denied.requestId).toBeNull();
    });

    it("a server revocation overrides a stale granted view (the owner controls access)", () => {
        const revoked = reduceAccessRequest(at("granted"), { kind: "phase", phase: "revoked" });
        expect(revoked.phase).toBe("revoked");
    });

    it("preserves the optimistic id while the server still reports requested", () => {
        const requested = reduceAccessRequest(at("init"), { kind: "submit", requestId: RID });
        const echoed = reduceAccessRequest(requested, { kind: "phase", phase: "requested" });
        expect(echoed.phase).toBe("requested");
        expect(echoed.requestId).toBe(RID);
    });

    it("is a no-op when the phase and id are unchanged", () => {
        const before = at("granted");
        expect(reduceAccessRequest(before, { kind: "phase", phase: "granted" })).toBe(before);
    });
});

describe("presentation (the panel's derived affordances)", () => {
    it("init and revoked may request; nothing else may", () => {
        for (const phase of ALL_PHASES) {
            const p = presentAccessRequest(at(phase));
            expect(p.canRequest).toBe(phase === "init" || phase === "revoked");
        }
    });

    it("only a pending request may be canceled, and only it waits", () => {
        for (const phase of ALL_PHASES) {
            const p = presentAccessRequest(at(phase));
            expect(p.canCancel).toBe(phase === "requested");
            expect(p.waiting).toBe(phase === "requested");
        }
    });

    it("granted/denied are settled; only granted admits the payload", () => {
        for (const phase of ALL_PHASES) {
            const p = presentAccessRequest(at(phase));
            expect(p.settled).toBe(phase === "granted" || phase === "denied");
            expect(p.granted).toBe(phase === "granted");
        }
    });

    it("never offers both request and cancel at once (the affordances are exclusive)", () => {
        for (const phase of ALL_PHASES) {
            const p = presentAccessRequest(at(phase));
            expect(p.canRequest && p.canCancel).toBe(false);
        }
    });

    it("always carries the handle name (visibility survives an access denial, INV-10)", () => {
        for (const phase of ALL_PHASES) {
            expect(presentAccessRequest(at(phase)).path).toBe("notes.md");
            expect(presentAccessRequest(at(phase)).label).toBeTruthy();
        }
    });
});

describe("end-to-end arc (request → approve)", () => {
    it("walks init → requested → granted, clearing the id at the grant", () => {
        let s = at("init");
        expect(presentAccessRequest(s).canRequest).toBe(true);
        s = reduceAccessRequest(s, { kind: "submit", requestId: RID });
        expect(presentAccessRequest(s).waiting).toBe(true);
        expect(s.requestId).toBe(RID);
        s = reduceAccessRequest(s, { kind: "phase", phase: "granted" });
        expect(presentAccessRequest(s).granted).toBe(true);
        expect(s.requestId).toBeNull();
    });
});

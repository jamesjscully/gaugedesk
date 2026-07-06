import { describe, expect, it } from "vitest";
import {
    initialChatApproval,
    presentChatApproval,
    reduceChatApproval,
    type ChatApprovalState,
} from "./chat-approval";
import { type ReviewPhase } from "@gaugewright/control-plane-client";
import { clientRequestId } from "@gaugewright/control-plane-client";

const ALL_PHASES: readonly ReviewPhase[] = [
    "Init",
    "Proposed",
    "Cleared",
    "Released",
    "Withheld",
];

const RID = clientRequestId("req-1");
const PARTIES = ["A", "B"] as const;

/** A card for party "A" at the given phase, requiring A+B, with the given consents. */
function at(
    phase: ReviewPhase,
    consented: readonly string[] = [],
    party = "A",
): ChatApprovalState {
    return initialChatApproval("merge-1", party, phase, PARTIES, consented);
}

describe("initial state", () => {
    it("threads a card for a proposal at the given phase with no command in flight", () => {
        const s = initialChatApproval("merge-1", "A", "Proposed", PARTIES, ["B"]);
        expect(s.proposalId).toBe("merge-1");
        expect(s.party).toBe("A");
        expect(s.phase).toBe("Proposed");
        expect(s.required).toEqual(["A", "B"]);
        expect(s.consented).toEqual(["B"]);
        expect(s.pendingId).toBeNull();
    });
});

describe("consent (this party approves)", () => {
    it("records this party's consent under the correlation id while Proposed", () => {
        const s = reduceChatApproval(at("Proposed"), { kind: "consent", requestId: RID });
        expect(s.consented).toContain("A");
        expect(s.pendingId).toBe(RID);
    });

    it("is a no-op when this party has already consented (consent is once)", () => {
        const before = at("Proposed", ["A"]);
        expect(reduceChatApproval(before, { kind: "consent", requestId: RID })).toBe(before);
    });

    it("is a no-op when this party is not required (never consent uninvited)", () => {
        const before = initialChatApproval("merge-1", "C", "Proposed", PARTIES, []);
        expect(reduceChatApproval(before, { kind: "consent", requestId: RID })).toBe(before);
    });

    it("is a no-op from any phase but Proposed (never claim a consent the server would refuse)", () => {
        for (const phase of ["Init", "Cleared", "Released", "Withheld"] as const) {
            const before = at(phase);
            expect(reduceChatApproval(before, { kind: "consent", requestId: RID })).toBe(before);
        }
    });
});

describe("release (the user releases a cleared result)", () => {
    it("optimistically moves a Cleared proposal to Released under the id", () => {
        const s = reduceChatApproval(at("Cleared", ["A", "B"]), { kind: "release", requestId: RID });
        expect(s.phase).toBe("Released");
        expect(s.pendingId).toBe(RID);
    });

    it("is a no-op from any phase but Cleared", () => {
        for (const phase of ["Init", "Proposed", "Released", "Withheld"] as const) {
            const before = at(phase);
            expect(reduceChatApproval(before, { kind: "release", requestId: RID })).toBe(before);
        }
    });
});

describe("review (the server's state is authoritative)", () => {
    it("the other party consenting and clearing moves the card to Cleared and clears the id", () => {
        const consented = reduceChatApproval(at("Proposed"), { kind: "consent", requestId: RID });
        expect(consented.pendingId).toBe(RID);
        const cleared = reduceChatApproval(consented, {
            kind: "review",
            phase: "Cleared",
            required: PARTIES,
            consented: ["A", "B"],
        });
        expect(cleared.phase).toBe("Cleared");
        expect(cleared.pendingId).toBeNull();
    });

    it("a withdrawal (Withheld) settles the card and clears the id", () => {
        const consented = reduceChatApproval(at("Proposed"), { kind: "consent", requestId: RID });
        const withheld = reduceChatApproval(consented, {
            kind: "review",
            phase: "Withheld",
            required: PARTIES,
            consented: [],
        });
        expect(withheld.phase).toBe("Withheld");
        expect(withheld.pendingId).toBeNull();
    });

    it("retires the optimistic id once this party's consent is reflected server-side", () => {
        const consented = reduceChatApproval(at("Proposed"), { kind: "consent", requestId: RID });
        const echoed = reduceChatApproval(consented, {
            kind: "review",
            phase: "Proposed",
            required: PARTIES,
            consented: ["A"],
        });
        expect(echoed.pendingId).toBeNull();
    });

    it("preserves the optimistic id while the server has not yet reflected the consent", () => {
        const consented = reduceChatApproval(at("Proposed"), { kind: "consent", requestId: RID });
        const echoed = reduceChatApproval(consented, {
            kind: "review",
            phase: "Proposed",
            required: PARTIES,
            consented: ["B"], // server saw B, not yet our A
        });
        expect(echoed.consented).toEqual(["B"]);
        expect(echoed.pendingId).toBe(RID);
    });

    it("a server change overrides the optimistic view (the parties control the result)", () => {
        const released = reduceChatApproval(at("Cleared", ["A", "B"]), {
            kind: "review",
            phase: "Withheld",
            required: PARTIES,
            consented: [],
        });
        expect(released.phase).toBe("Withheld");
    });

    it("is a no-op when phase, sets, and id are all unchanged (set order ignored)", () => {
        const before = at("Proposed", ["A", "B"]);
        expect(
            reduceChatApproval(before, {
                kind: "review",
                phase: "Proposed",
                required: ["B", "A"],
                consented: ["B", "A"],
            }),
        ).toBe(before);
    });
});

describe("presentation (the card's derived affordances)", () => {
    it("only a required, not-yet-consented party may consent, and only while Proposed", () => {
        // required + outstanding
        expect(presentChatApproval(at("Proposed")).canConsent).toBe(true);
        // required but already consented
        expect(presentChatApproval(at("Proposed", ["A"])).canConsent).toBe(false);
        // not required
        const c = initialChatApproval("merge-1", "C", "Proposed", PARTIES, []);
        expect(presentChatApproval(c).canConsent).toBe(false);
        // wrong phase
        for (const phase of ["Init", "Cleared", "Released", "Withheld"] as const) {
            expect(presentChatApproval(at(phase)).canConsent).toBe(false);
        }
    });

    it("only Cleared may release", () => {
        for (const phase of ALL_PHASES) {
            expect(presentChatApproval(at(phase, ["A", "B"])).canRelease).toBe(phase === "Cleared");
        }
    });

    it("Released/Withheld are settled; only Released is released", () => {
        for (const phase of ALL_PHASES) {
            const p = presentChatApproval(at(phase, ["A", "B"]));
            expect(p.settled).toBe(phase === "Released" || phase === "Withheld");
            expect(p.released).toBe(phase === "Released");
        }
    });

    it("waits only while Proposed with consent still outstanding", () => {
        expect(presentChatApproval(at("Proposed", [])).waiting).toBe(true);
        expect(presentChatApproval(at("Proposed", ["A", "B"])).waiting).toBe(false);
        expect(presentChatApproval(at("Cleared", ["A", "B"])).waiting).toBe(false);
    });

    it("reports conjunctive progress: consents-in / consents-needed and the outstanding set", () => {
        const p = presentChatApproval(at("Proposed", ["B"]));
        expect(p.consentsNeeded).toBe(2);
        expect(p.consentsIn).toBe(1);
        expect(p.outstanding).toEqual(["A"]);
    });

    it("never offers both consent and release at once (the affordances are exclusive)", () => {
        for (const phase of ALL_PHASES) {
            const p = presentChatApproval(at(phase));
            expect(p.canConsent && p.canRelease).toBe(false);
        }
    });

    it("always carries a human label for every phase", () => {
        for (const phase of ALL_PHASES) {
            expect(presentChatApproval(at(phase)).label).toBeTruthy();
        }
    });
});

describe("end-to-end arc (propose → consent → clear → release)", () => {
    it("walks Proposed → (B consents, clears) → release, clearing the id at each settle", () => {
        // Party A's view of a proposal requiring A+B.
        let s = at("Proposed", []);
        expect(presentChatApproval(s).canConsent).toBe(true);

        // A consents optimistically.
        s = reduceChatApproval(s, { kind: "consent", requestId: RID });
        expect(s.consented).toContain("A");
        expect(s.pendingId).toBe(RID);

        // Server reflects A's consent (still waiting on B) — the id retires.
        s = reduceChatApproval(s, {
            kind: "review",
            phase: "Proposed",
            required: PARTIES,
            consented: ["A"],
        });
        expect(s.pendingId).toBeNull();
        expect(presentChatApproval(s).waiting).toBe(true);

        // B consents server-side and the proposal clears.
        s = reduceChatApproval(s, {
            kind: "review",
            phase: "Cleared",
            required: PARTIES,
            consented: ["A", "B"],
        });
        expect(presentChatApproval(s).canRelease).toBe(true);

        // The user releases.
        s = reduceChatApproval(s, { kind: "release", requestId: clientRequestId("req-2") });
        expect(presentChatApproval(s).released).toBe(true);
    });
});

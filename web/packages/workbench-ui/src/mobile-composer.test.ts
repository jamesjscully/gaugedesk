import { describe, expect, it } from "vitest";
import {
    clearDraft,
    edit,
    empty,
    isSendable,
    presentComposer,
    reconcile,
    reconcileCarriage,
    send,
    type ComposerState,
    type TurnPhase,
} from "./mobile-composer";
import {
    clientRequestId,
    type ClientRequestId,
    type Freshness,
    type ProjectionCarriage,
} from "@gaugewright/control-plane-client";

const RID1 = clientRequestId("req-1");
const RID2 = clientRequestId("req-2");

function fresh(): Freshness {
    return { marker: "live", generatedAt: 1, repairHint: null };
}

function carriage(rid: ClientRequestId | null): ProjectionCarriage<{ ok: boolean }> {
    return { value: { ok: true }, freshness: fresh(), clientRequestId: rid };
}

function withDraft(draft: string): ComposerState {
    return { ...empty, draft };
}

describe("draft editing (local view state, never a command)", () => {
    it("starts empty with nothing pending", () => {
        expect(empty.draft).toBe("");
        expect(empty.pending).toEqual([]);
    });

    it("edit replaces the draft", () => {
        expect(edit(empty, "hello").draft).toBe("hello");
    });

    it("edit is a no-op when the text is unchanged (same reference back)", () => {
        const s = withDraft("hello");
        expect(edit(s, "hello")).toBe(s);
    });

    it("clearDraft empties the draft and is a no-op when already empty", () => {
        expect(clearDraft(withDraft("x")).draft).toBe("");
        expect(clearDraft(empty)).toBe(empty);
    });
});

describe("send gating (no empty-message command)", () => {
    it("isSendable is true only for a non-blank, trimmed draft", () => {
        expect(isSendable("")).toBe(false);
        expect(isSendable("   ")).toBe(false);
        expect(isSendable("\n\t ")).toBe(false);
        expect(isSendable("hi")).toBe(true);
        expect(isSendable("  hi  ")).toBe(true);
    });

    it("a blank draft never produces a send (same reference back)", () => {
        for (const phase of ["idle", "running"] as const) {
            const s = withDraft("   ");
            expect(send(s, RID1, phase)).toBe(s);
        }
    });
});

describe("optimistic send / reconcile (mirrors RunState.pending_commands, MOB-003)", () => {
    it("a send always clears the draft", () => {
        for (const phase of ["idle", "running"] as const) {
            expect(send(withDraft("hello"), RID1, phase).draft).toBe("");
        }
    });

    it("a send while running records the optimistic pending id (RecordPending)", () => {
        const s = send(withDraft("hello"), RID1, "running");
        expect(s.pending).toEqual([RID1]);
    });

    it("an idle send records NO pending id — nothing to reconcile yet (INV-5: fewer claims)", () => {
        const s = send(withDraft("hello"), RID1, "idle");
        expect(s.pending).toEqual([]);
    });

    it("two distinct running sends never collapse onto one pending id", () => {
        let s = send(withDraft("first"), RID1, "running");
        s = send({ ...s, draft: "second" }, RID2, "running");
        expect(s.pending).toEqual([RID1, RID2]);
    });

    it("a duplicate rid does not double-record (RecordPending is fresh-keyed)", () => {
        let s = send(withDraft("first"), RID1, "running");
        s = send({ ...s, draft: "again" }, RID1, "running");
        expect(s.pending).toEqual([RID1]);
        expect(s.draft).toBe("");
    });

    it("reconcile retires exactly the matching pending id (Reconcile)", () => {
        let s = send(withDraft("a"), RID1, "running");
        s = send({ ...s, draft: "b" }, RID2, "running");
        s = reconcile(s, RID1);
        expect(s.pending).toEqual([RID2]);
    });

    it("reconcile is idempotent / a no-op for an unknown id (same reference back)", () => {
        const s = send(withDraft("a"), RID1, "running");
        const after = reconcile(s, RID1);
        expect(after.pending).toEqual([]);
        // reconciling again, or an unrelated id, changes nothing.
        expect(reconcile(after, RID1)).toBe(after);
        expect(reconcile(s, RID2)).toBe(s);
    });

    it("reconcileCarriage retires the id the projection names, and ignores an uncorrelated one", () => {
        const s = send(withDraft("a"), RID1, "running");
        expect(reconcileCarriage(s, carriage(RID1)).pending).toEqual([]);
        // a plain projection (no correlation id) leaves the ledger untouched.
        expect(reconcileCarriage(s, carriage(null))).toBe(s);
    });
});

describe("derived presentation (the island paints this, decides nothing itself)", () => {
    it("canSend tracks a non-blank draft", () => {
        expect(presentComposer(withDraft(""), "idle").canSend).toBe(false);
        expect(presentComposer(withDraft("  "), "idle").canSend).toBe(false);
        expect(presentComposer(withDraft("hi"), "idle").canSend).toBe(true);
    });

    it("canStop is offered only while a turn is running", () => {
        const phases: TurnPhase[] = ["idle", "running"];
        for (const phase of phases) {
            expect(presentComposer(empty, phase).canStop).toBe(phase === "running");
        }
    });

    it("surfaces the optimistic in-flight count", () => {
        let s = send(withDraft("a"), RID1, "running");
        s = send({ ...s, draft: "b" }, RID2, "running");
        const p = presentComposer(s, "running");
        expect(p.pendingCount).toBe(2);
        expect(p.hasPending).toBe(true);

        const settled = presentComposer(reconcile(reconcile(s, RID1), RID2), "running");
        expect(settled.pendingCount).toBe(0);
        expect(settled.hasPending).toBe(false);
    });
});

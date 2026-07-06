import { describe, expect, it } from "vitest";
import {
    parseFileNode,
    payloadAccessible,
    presentNode,
    presentTree,
    type AccessPhase,
    type FileNode,
} from "./mobile-files";

const ALL_PHASES: readonly AccessPhase[] = [
    "init",
    "requested",
    "granted",
    "revoked",
    "denied",
];

function file(path: string, access: AccessPhase, protectedResource = false): FileNode {
    return { path, isDir: false, access, protectedResource };
}

describe("payload access mirrors core resource_access (only granted admits payload)", () => {
    it("only the granted phase admits the payload", () => {
        for (const phase of ALL_PHASES) {
            expect(payloadAccessible(phase)).toBe(phase === "granted");
        }
    });
});

describe("name-visibility split (INV-10: holding a handle is not holding the payload)", () => {
    it("a handle's name is visible in every phase", () => {
        for (const phase of ALL_PHASES) {
            expect(presentNode(file("notes.md", phase)).nameVisible).toBe(true);
        }
    });

    it("payload is openable only for a granted, non-directory handle", () => {
        for (const phase of ALL_PHASES) {
            expect(presentNode(file("notes.md", phase)).payloadOpenable).toBe(phase === "granted");
        }
    });

    it("a directory is never payload-openable, even if granted", () => {
        const dir: FileNode = { path: "src", isDir: true, access: "granted", protectedResource: false };
        expect(presentNode(dir).payloadOpenable).toBe(false);
    });

    it("a protected handle without payload is locked; a locked node is never openable", () => {
        for (const phase of ALL_PHASES) {
            const p = presentNode(file("builder_only/method.md", phase, true));
            expect(p.locked).toBe(phase !== "granted");
            expect(p.locked && p.payloadOpenable).toBe(false);
        }
    });

    it("an unprotected handle is never locked", () => {
        for (const phase of ALL_PHASES) {
            expect(presentNode(file("readme.md", phase)).locked).toBe(false);
        }
    });
});

describe("requestability (you can ask for payload only when it makes sense)", () => {
    it("a locked init/revoked handle is requestable", () => {
        for (const phase of ["init", "revoked"] as const) {
            expect(presentNode(file("secret.md", phase, true)).requestable).toBe(true);
        }
    });

    it("a mid-request or terminally-denied handle is not re-requestable", () => {
        for (const phase of ["requested", "denied"] as const) {
            expect(presentNode(file("secret.md", phase, true)).requestable).toBe(false);
        }
    });

    it("a granted handle is not requestable (already open)", () => {
        expect(presentNode(file("secret.md", "granted", true)).requestable).toBe(false);
    });
});

describe("tree presentation", () => {
    it("orders nodes stably by path and never drops one", () => {
        const tree = presentTree([
            file("zeta.md", "init"),
            file("alpha.md", "granted"),
            file("mid.md", "requested"),
        ]);
        expect(tree.map((n) => n.path)).toEqual(["alpha.md", "mid.md", "zeta.md"]);
    });

    it("does not mutate its input", () => {
        const nodes = [file("b.md", "init"), file("a.md", "init")];
        const before = nodes.map((n) => n.path);
        presentTree(nodes);
        expect(nodes.map((n) => n.path)).toEqual(before);
    });
});

describe("parse at the transport edge (safe default is less access)", () => {
    it("parses a granted protected node", () => {
        expect(
            parseFileNode({ path: "m.md", is_dir: false, access: "granted", protected: true }),
        ).toEqual({ path: "m.md", isDir: false, access: "granted", protectedResource: true });
    });

    it("defaults missing access to init (name-only), never to granted", () => {
        const node = parseFileNode({ path: "x.md" });
        expect(node.access).toBe("init");
        expect(presentNode(node).payloadOpenable).toBe(false);
    });

    it("rejects an empty path and an unknown phase", () => {
        expect(() => parseFileNode({ path: "" })).toThrow();
        expect(() => parseFileNode({ path: "x", access: "bogus" })).toThrow();
    });
});

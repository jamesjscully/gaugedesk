import { describe, expect, it } from "vitest";
import { bearer, decodeSubject, parseCallbackFragment, setBearer, signedIn } from "./auth-session";

/** Build an unsigned JWT-shaped string with the given payload (base64url, no padding) —
 *  enough to exercise the display-only `sub` decode (the client never verifies). */
function fakeJwt(payload: object): string {
    const b64url = (o: object) =>
        btoa(JSON.stringify(o)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
    return `${b64url({ alg: "RS256" })}.${b64url(payload)}.sig`;
}

describe("parseCallbackFragment", () => {
    it("pulls the id-token out of the callback fragment", () => {
        expect(parseCallbackFragment("#id_token=abc.def.ghi&token_type=Bearer")).toBe("abc.def.ghi");
        expect(parseCallbackFragment("id_token=xyz")).toBe("xyz"); // tolerates a missing leading '#'
    });

    it("returns null when no token is present", () => {
        expect(parseCallbackFragment("")).toBeNull();
        expect(parseCallbackFragment("#")).toBeNull();
        expect(parseCallbackFragment("#error=access_denied")).toBeNull();
        expect(parseCallbackFragment("#id_token=")).toBeNull();
    });
});

describe("decodeSubject", () => {
    it("decodes the sub claim for display", () => {
        expect(decodeSubject(fakeJwt({ sub: "alice@example.test", aud: "x" }))).toBe(
            "alice@example.test",
        );
    });

    it("returns null for a non-JWT or a token without a sub", () => {
        expect(decodeSubject("not-a-jwt")).toBeNull();
        expect(decodeSubject("a.b")).toBeNull(); // wrong segment count
        expect(decodeSubject(fakeJwt({ aud: "x" }))).toBeNull(); // no sub
        expect(decodeSubject("a.!!!notbase64!!!.c")).toBeNull();
    });
});

describe("bearer is in-memory only (ENTSEC-6)", () => {
    it("never writes the token to a Storage, even when one is available", () => {
        // Install recording Storage stubs (the test env is `node`, no DOM): if setBearer ever
        // persisted the credential, these would capture the write.
        const writes: Record<string, string> = {};
        const stub = {
            store: writes,
            getItem: (k: string) => writes[k] ?? null,
            setItem: (k: string, v: string) => { writes[k] = v; },
            removeItem: (k: string) => { delete writes[k]; },
        };
        const g = globalThis as unknown as { localStorage?: unknown; sessionStorage?: unknown };
        const prevLocal = g.localStorage;
        const prevSession = g.sessionStorage;
        g.localStorage = stub;
        g.sessionStorage = stub;
        try {
            setBearer("header.payload.sig");
            expect(bearer()).toBe("header.payload.sig"); // held in the in-memory signal
            expect(signedIn()).toBe(true);
            // The credential must NOT be at rest anywhere a later local access / XSS could scrape.
            expect(Object.keys(writes)).toHaveLength(0);

            setBearer(null);
            expect(bearer()).toBeNull();
            expect(signedIn()).toBe(false);
            expect(Object.keys(writes)).toHaveLength(0);
        } finally {
            g.localStorage = prevLocal;
            g.sessionStorage = prevSession;
        }
    });
});

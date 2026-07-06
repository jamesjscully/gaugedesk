import { describe, expect, it } from "vitest";
import {
    canCommand,
    deriveStatus,
    initialConnection,
    reduce,
    type ConnectionState,
    type ConnectionStatus,
} from "./connection";
import {
    bridgeGrantId,
    deviceId,
    publicKey,
    type BridgeGrant,
    type DeviceIdentity,
    type LocalState,
} from "@gaugewright/control-plane-client";

const DEVICE_KEY = publicKey("device-pub-hex");
const OTHER_KEY = publicKey("other-device-pub-hex");

const identity: DeviceIdentity = {
    id: deviceId("dev-1"),
    deviceKey: DEVICE_KEY,
};

const NOW = 1000;

/** A grant for `environment`, bound to this device by default, active and far
 *  from expiry unless overridden. */
function grant(environment: string, over: Partial<BridgeGrant> = {}): BridgeGrant {
    return {
        id: bridgeGrantId(`grant-${environment}`),
        sourceAuthorityRootPubkey: publicKey("auth-root"),
        sourceAuthorityKeyId: "key-1",
        targetEnvironment: environment,
        targetRoute: "/",
        deviceKey: DEVICE_KEY,
        governanceScope: "scope-1",
        expiry: NOW + 1000,
        active: true,
        ...over,
    };
}

function local(grants: readonly BridgeGrant[]): LocalState {
    return { identity, grants };
}

function state(over: Partial<ConnectionState> & { local: LocalState }): ConnectionState {
    const environment = over.environment ?? null;
    const relayReachable = over.relayReachable ?? true;
    const now = over.now ?? NOW;
    return {
        environment,
        relayReachable,
        now,
        ...over,
        status: deriveStatus(over.local, environment, relayReachable, now),
    };
}

describe("deriveStatus — idle (no environment addressed)", () => {
    it("unpaired when the device holds no grant", () => {
        expect(deriveStatus(local([]), null, true, NOW)).toBe("unpaired");
    });

    it("paired when a usable device-bound grant is held but no env is addressed", () => {
        expect(deriveStatus(local([grant("peach")]), null, true, NOW)).toBe("paired");
    });

    it("unpaired when the only held grant binds a different device", () => {
        const foreign = grant("peach", { deviceKey: OTHER_KEY });
        expect(deriveStatus(local([foreign]), null, true, NOW)).toBe("unpaired");
    });

    it("unpaired when the only held grant is expired", () => {
        expect(deriveStatus(local([grant("peach", { expiry: NOW })]), null, true, NOW)).toBe(
            "unpaired",
        );
    });
});

describe("deriveStatus — addressed environment", () => {
    it("active when a usable grant exists and the relay is up", () => {
        expect(deriveStatus(local([grant("peach")]), "peach", true, NOW)).toBe("active");
    });

    it("offline when a usable grant exists but the relay is down", () => {
        expect(deriveStatus(local([grant("peach")]), "peach", false, NOW)).toBe("offline");
    });

    it("revoked when the grant for the env is inactive (even with relay up)", () => {
        expect(deriveStatus(local([grant("peach", { active: false })]), "peach", true, NOW)).toBe(
            "revoked",
        );
    });

    it("expired when the grant is active but past expiry", () => {
        expect(deriveStatus(local([grant("peach", { expiry: NOW })]), "peach", true, NOW)).toBe(
            "expired",
        );
    });

    it("revoked takes precedence over expired (most actionable failure first)", () => {
        const both = local([grant("peach", { active: false, expiry: NOW })]);
        expect(deriveStatus(both, "peach", true, NOW)).toBe("revoked");
    });

    it("unpaired when no grant addresses this environment", () => {
        expect(deriveStatus(local([grant("apple")]), "peach", true, NOW)).toBe("unpaired");
    });

    it("unpaired when the env grant binds a different device", () => {
        const foreign = grant("peach", { deviceKey: OTHER_KEY });
        expect(deriveStatus(local([foreign]), "peach", true, NOW)).toBe("unpaired");
    });

    it("prefers a usable grant when env has both a usable and a revoked grant", () => {
        const grants = local([grant("peach", { active: false }), grant("peach")]);
        expect(deriveStatus(grants, "peach", true, NOW)).toBe("active");
    });
});

describe("canCommand", () => {
    it("permits standing commands only when active", () => {
        const statuses: ConnectionStatus[] = [
            "unpaired",
            "paired",
            "active",
            "offline",
            "revoked",
            "expired",
        ];
        for (const s of statuses) {
            expect(canCommand(s)).toBe(s === "active");
        }
    });
});

describe("initialConnection", () => {
    it("starts paired when a usable grant is already held", () => {
        const s = initialConnection(local([grant("peach")]), NOW);
        expect(s.status).toBe("paired");
        expect(s.environment).toBeNull();
        expect(s.relayReachable).toBe(true);
    });

    it("starts unpaired with no grants", () => {
        expect(initialConnection(local([]), NOW).status).toBe("unpaired");
    });
});

describe("reduce — event transitions", () => {
    it("unpaired → paired when a grant arrives, then → active when addressed", () => {
        let s = initialConnection(local([]), NOW);
        expect(s.status).toBe("unpaired");

        s = reduce(s, { kind: "grants-changed", grants: [grant("peach")] });
        expect(s.status).toBe("paired");

        s = reduce(s, { kind: "address", environment: "peach" });
        expect(s.status).toBe("active");
    });

    it("active → offline when the relay drops, and back when it returns", () => {
        let s = reduce(
            initialConnection(local([grant("peach")]), NOW),
            { kind: "address", environment: "peach" },
        );
        expect(s.status).toBe("active");

        s = reduce(s, { kind: "relay", reachable: false });
        expect(s.status).toBe("offline");

        s = reduce(s, { kind: "relay", reachable: true });
        expect(s.status).toBe("active");
    });

    it("active → revoked when the grant is observed revoked", () => {
        let s = reduce(
            initialConnection(local([grant("peach")]), NOW),
            { kind: "address", environment: "peach" },
        );
        expect(s.status).toBe("active");

        s = reduce(s, { kind: "grants-changed", grants: [grant("peach", { active: false })] });
        expect(s.status).toBe("revoked");
        expect(canCommand(s.status)).toBe(false);
    });

    it("active → expired when the clock advances past expiry", () => {
        let s = reduce(
            initialConnection(local([grant("peach", { expiry: NOW + 10 })]), NOW),
            { kind: "address", environment: "peach" },
        );
        expect(s.status).toBe("active");

        s = reduce(s, { kind: "tick", now: NOW + 10 });
        expect(s.status).toBe("expired");
    });

    it("going idle (address null) returns to paired while a usable grant is held", () => {
        let s = reduce(
            initialConnection(local([grant("peach")]), NOW),
            { kind: "address", environment: "peach" },
        );
        expect(s.status).toBe("active");
        s = reduce(s, { kind: "address", environment: null });
        expect(s.status).toBe("paired");
    });

    it("is a no-op (same reference) when an event does not move any input", () => {
        const s = initialConnection(local([grant("peach")]), NOW);
        expect(reduce(s, { kind: "relay", reachable: true })).toBe(s);
        expect(reduce(s, { kind: "address", environment: null })).toBe(s);
        expect(reduce(s, { kind: "tick", now: NOW })).toBe(s);
        expect(reduce(s, { kind: "grants-changed", grants: s.local.grants })).toBe(s);
    });
});

describe("connection invariants", () => {
    const envs = [null, "peach", "apple"] as const;
    const grantSets: readonly BridgeGrant[][] = [
        [],
        [grant("peach")],
        [grant("peach", { active: false })],
        [grant("peach", { expiry: NOW })],
        [grant("peach", { deviceKey: OTHER_KEY })],
        [grant("peach"), grant("apple", { active: false })],
    ];
    const relays = [true, false];
    const nows = [NOW - 1, NOW, NOW + 1000];

    it("canCommand ⇒ a usable device-bound grant for the addressed env exists and relay is up", () => {
        for (const grants of grantSets) {
            for (const env of envs) {
                for (const relay of relays) {
                    for (const now of nows) {
                        const status = deriveStatus(local(grants), env, relay, now);
                        if (!canCommand(status)) continue;
                        // active implies: an env is addressed, the relay is up, and a
                        // bound, active, unexpired grant exists for that env.
                        expect(env).not.toBeNull();
                        expect(relay).toBe(true);
                        const usable = grants.some(
                            (g) =>
                                g.targetEnvironment === env &&
                                g.deviceKey === identity.deviceKey &&
                                g.active &&
                                now < g.expiry,
                        );
                        expect(usable).toBe(true);
                    }
                }
            }
        }
    });

    it("a revoked or expired grant is never reported active", () => {
        // No usable grant present, relay up: must never be active.
        const status = deriveStatus(
            local([grant("peach", { active: false }), grant("peach", { expiry: NOW })]),
            "peach",
            true,
            NOW,
        );
        expect(status).not.toBe("active");
        expect(canCommand(status)).toBe(false);
    });

    it("the reduced status equals a fresh derivation from the same inputs", () => {
        for (const grants of grantSets) {
            for (const env of envs) {
                for (const relay of relays) {
                    for (const now of nows) {
                        const s = state({ local: local(grants), environment: env, relayReachable: relay, now });
                        // reduce with a no-op tick to the same now keeps status consistent
                        const after = reduce(s, { kind: "tick", now });
                        expect(after.status).toBe(deriveStatus(local(grants), env, relay, now));
                    }
                }
            }
        }
    });
});

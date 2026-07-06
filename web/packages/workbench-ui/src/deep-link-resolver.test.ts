import { describe, expect, it } from "vitest";
import {
    resolveDeepLink,
    resolveDeepLinkUrl,
    type AccessBasisLookup,
    type DeepLinkResolution,
} from "./deep-link-resolver";
import { parse_deep_link } from "./deep-link";
import {
    bridgeGrantId,
    deviceId,
    publicKey,
    type BridgeGrant,
    type DeviceIdentity,
    type LocalState,
} from "@gaugewright/control-plane-client";
import type { AccessPhase } from "./mobile-files";

const DEVICE_KEY = publicKey("device-pub-hex");
const OTHER_KEY = publicKey("other-device-pub-hex");

const identity: DeviceIdentity = { id: deviceId("dev-1"), deviceKey: DEVICE_KEY };

const NOW = 1000;

/** A grant for `environment`, bound to this device, active and far from expiry
 *  unless overridden (mirrors the connection test's fixture). */
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

/** An access-basis lookup that grants every handle (the happy `resource` path). */
const ALL_GRANTED: AccessBasisLookup = () => "granted";
/** An access-basis lookup with no granted handle (name only). */
const NONE_GRANTED: AccessBasisLookup = () => "init";

/** Resolve from a URL with a held, active grant for `peach` and the relay up. */
function resolvePeach(
    url: string,
    accessBasis: AccessBasisLookup = ALL_GRANTED,
    relayReachable = true,
    now = NOW,
): DeepLinkResolution {
    return resolveDeepLinkUrl(url, local([grant("peach")]), relayReachable, now, accessBasis);
}

describe("resolveDeepLink — external", () => {
    it("hands an external link to the system browser without an env gate", () => {
        // No grant held at all — external must still resolve (never an in-app surface).
        const link = parse_deep_link("gaugewright://peach/external/https://example.com/a");
        const res = resolveDeepLink(link, local([]), false, NOW, NONE_GRANTED);
        expect(res).toEqual({ kind: "external", url: "https://example.com/a" });
    });
});

describe("resolveDeepLink — pairing", () => {
    it("routes a pairing link into the pairing flow even with a held grant", () => {
        const res = resolvePeach("gaugewright://peach/pairing/ticket-7");
        expect(res.kind).toBe("needs-pairing");
        if (res.kind === "needs-pairing") {
            expect(String(res.environment)).toBe("peach");
            expect(String(res.scope)).toBe("peach");
        }
    });
});

describe("resolveDeepLink — grant gate (reuses the connection machine)", () => {
    it("needs-pairing when the device holds no grant for the environment", () => {
        const res = resolveDeepLinkUrl(
            "gaugewright://peach/navigation/chat-42",
            local([]),
            true,
            NOW,
            ALL_GRANTED,
        );
        expect(res.kind).toBe("needs-pairing");
    });

    it("needs-pairing when the only grant is bound to another device", () => {
        const res = resolveDeepLinkUrl(
            "gaugewright://peach/navigation/chat-42",
            local([grant("peach", { deviceKey: OTHER_KEY })]),
            true,
            NOW,
            ALL_GRANTED,
        );
        expect(res.kind).toBe("needs-pairing");
    });

    it("grant-revoked when a device-bound grant for the env is inactive", () => {
        const res = resolveDeepLinkUrl(
            "gaugewright://peach/navigation/chat-42",
            local([grant("peach", { active: false })]),
            true,
            NOW,
            ALL_GRANTED,
        );
        expect(res.kind).toBe("grant-revoked");
    });

    it("needs-pairing (re-issue) when the grant is expired", () => {
        const res = resolveDeepLinkUrl(
            "gaugewright://peach/navigation/chat-42",
            local([grant("peach", { expiry: NOW - 1 })]),
            true,
            NOW,
            ALL_GRANTED,
        );
        expect(res.kind).toBe("needs-pairing");
    });

    it("offline when a usable grant exists but the relay is unreachable", () => {
        const res = resolvePeach("gaugewright://peach/navigation/chat-42", ALL_GRANTED, false);
        expect(res.kind).toBe("offline");
        if (res.kind === "offline") expect(String(res.environment)).toBe("peach");
    });
});

describe("resolveDeepLink — routing", () => {
    it("lands a bare-environment navigation on the nav pane, no target", () => {
        const res = resolvePeach("gaugewright://peach");
        expect(res.kind).toBe("route");
        if (res.kind === "route") {
            expect(res.route.pane).toBe("nav");
            expect(res.route.target).toBeNull();
            expect(String(res.route.environment)).toBe("peach");
        }
    });

    it("lands a navigation with a target on the chat pane carrying target+sub", () => {
        const res = resolvePeach("gaugewright://peach/navigation/chat-42/turn-7-diff");
        expect(res.kind).toBe("route");
        if (res.kind === "route") {
            expect(res.route.pane).toBe("chat");
            expect(String(res.route.target)).toBe("chat-42");
            expect(String(res.route.sub)).toBe("turn-7-diff");
        }
    });

    it("lands a notification link on the chat pane (back-path to nav intact)", () => {
        const res = resolvePeach("gaugewright://peach/notification/review-9");
        expect(res.kind).toBe("route");
        if (res.kind === "route") expect(res.route.pane).toBe("chat");
    });

    it("a cross-environment link with a held grant is a silent switch (routes)", () => {
        const res = resolvePeach("gaugewright://peach/cross-environment/chat-1");
        expect(res.kind).toBe("route");
        if (res.kind === "route") expect(res.route.pane).toBe("chat");
    });
});

describe("resolveDeepLink — access-basis gate (INV-10)", () => {
    it("routes a resource link to content when the payload basis is granted", () => {
        const res = resolvePeach("gaugewright://peach/resource/handle-3", ALL_GRANTED);
        expect(res.kind).toBe("route");
        if (res.kind === "route") {
            expect(res.route.pane).toBe("content");
            expect(String(res.route.target)).toBe("handle-3");
        }
    });

    it("returns no-access (with the named route) when the basis is not granted", () => {
        const res = resolvePeach("gaugewright://peach/resource/handle-3", NONE_GRANTED);
        expect(res.kind).toBe("no-access");
        if (res.kind === "no-access") {
            // The named, locked handle still lands on content (visibility ≠ access).
            expect(res.route.pane).toBe("content");
            expect(String(res.route.target)).toBe("handle-3");
            expect(res.access).toBe("init");
        }
    });

    it.each<AccessPhase>(["init", "requested", "revoked", "denied"])(
        "no-access for a non-granted phase %s",
        (phase) => {
            const res = resolvePeach("gaugewright://peach/resource/handle-3", () => phase);
            expect(res.kind).toBe("no-access");
        },
    );

    it("a resource link with no target handle is unsupported", () => {
        // `gaugewright://peach/resource` parses to a resource link with target null.
        const link = parse_deep_link("gaugewright://peach/resource");
        const res = resolveDeepLink(link, local([grant("peach")]), true, NOW, ALL_GRANTED);
        expect(res.kind).toBe("unsupported");
    });

    it("does not consult the access basis until the grant gate passes", () => {
        let consulted = false;
        const spy: AccessBasisLookup = () => {
            consulted = true;
            return "granted";
        };
        // No grant held → needs-pairing short-circuits before the access gate.
        const res = resolveDeepLinkUrl(
            "gaugewright://peach/resource/handle-3",
            local([]),
            true,
            NOW,
            spy,
        );
        expect(res.kind).toBe("needs-pairing");
        expect(consulted).toBe(false);
    });
});

describe("resolveDeepLinkUrl — parse failures surface as unsupported", () => {
    it("maps a non-gaugewright url to unsupported (not a throw)", () => {
        const res = resolvePeach("https://example.com");
        expect(res.kind).toBe("unsupported");
    });

    it("maps an unknown kind to unsupported", () => {
        const res = resolvePeach("gaugewright://peach/teleport/t1");
        expect(res.kind).toBe("unsupported");
    });

    it("maps an empty url to unsupported", () => {
        const res = resolvePeach("");
        expect(res.kind).toBe("unsupported");
    });
});

describe("resolveDeepLink — purity", () => {
    it("is a pure function of its inputs (same inputs ⇒ deep-equal outcome)", () => {
        const link = parse_deep_link("gaugewright://peach/navigation/chat-42/turn-7-diff");
        const l = local([grant("peach")]);
        const a = resolveDeepLink(link, l, true, NOW, ALL_GRANTED);
        const b = resolveDeepLink(link, l, true, NOW, ALL_GRANTED);
        expect(a).toEqual(b);
    });
});

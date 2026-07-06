import { describe, expect, it } from "vitest";
import { DeepLinkParseError, parse_deep_link } from "./deep-link";

describe("parse_deep_link", () => {
    it("parses a bare environment as a navigation link to its root", () => {
        const link = parse_deep_link("gaugewright://peach");
        expect(link.kind).toBe("navigation");
        expect(String(link.environment)).toBe("peach");
        expect(String(link.scope)).toBe("peach");
        expect(link.target).toBeNull();
        expect(link.sub).toBeNull();
        expect(link.externalUrl).toBeNull();
    });

    it("parses environment › kind › target › sub-target", () => {
        const link = parse_deep_link("gaugewright://peach/navigation/chat-42/turn-7-diff");
        expect(link.kind).toBe("navigation");
        expect(String(link.environment)).toBe("peach");
        expect(String(link.target)).toBe("chat-42");
        expect(String(link.sub)).toBe("turn-7-diff");
    });

    it("parses a target with no sub-target", () => {
        const link = parse_deep_link("gaugewright://peach/notification/review-9");
        expect(link.kind).toBe("notification");
        expect(String(link.target)).toBe("review-9");
        expect(link.sub).toBeNull();
    });

    it("parses each supported kind", () => {
        for (const kind of [
            "navigation",
            "cross-environment",
            "notification",
            "pairing",
            "resource",
        ] as const) {
            const link = parse_deep_link(`gaugewright://env/${kind}/t1`);
            expect(link.kind).toBe(kind);
        }
    });

    it("carries an external link's url verbatim and lands on no target", () => {
        const link = parse_deep_link("gaugewright://peach/external/https://example.com/a/b");
        expect(link.kind).toBe("external");
        expect(link.externalUrl).toBe("https://example.com/a/b");
        expect(link.target).toBeNull();
    });

    it("percent-decodes segments", () => {
        const link = parse_deep_link("gaugewright://peach/navigation/a%2Fb");
        expect(String(link.target)).toBe("a/b");
    });

    it("rejects a non-gaugewright url", () => {
        expect(() => parse_deep_link("https://example.com")).toThrow(DeepLinkParseError);
    });

    it("rejects an empty url", () => {
        expect(() => parse_deep_link("")).toThrow(DeepLinkParseError);
    });

    it("rejects a missing environment", () => {
        expect(() => parse_deep_link("gaugewright://")).toThrow(DeepLinkParseError);
    });

    it("rejects an unsupported kind", () => {
        expect(() => parse_deep_link("gaugewright://peach/teleport/t1")).toThrow(DeepLinkParseError);
    });

    it("rejects an external link with no url", () => {
        expect(() => parse_deep_link("gaugewright://peach/external")).toThrow(DeepLinkParseError);
    });
});

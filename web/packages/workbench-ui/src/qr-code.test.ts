import { describe, expect, it } from "vitest";
import { qrSvg } from "./qr-code";

describe("qrSvg", () => {
    it("renders a payload as inline SVG without echoing the raw text", () => {
        const svg = qrSvg("gaugewright://invite?d=abc123", 3);

        expect(svg).toContain("<svg");
        expect(svg).toContain("<path");
        expect(svg).toContain("preserveAspectRatio=");
        expect(svg).not.toContain("abc123");
    });
});

import { describe, expect, it } from "vitest";
import { parseAdvancementScopes, serializeAdvancementScopes } from "./advancement";

describe("advancement scopes (ATTN-3)", () => {
    it("round-trips scopes through the rules document", () => {
        const doc = serializeAdvancementScopes(["docs/**", "*.md"]);
        expect(JSON.parse(doc)).toEqual({
            version: 1,
            rules: [{ advance: "writes-within", paths: ["docs/**", "*.md"] }],
        });
        expect(parseAdvancementScopes(doc)).toEqual(["docs/**", "*.md"]);
    });

    it("empty scopes serialize to no rules (hold everything)", () => {
        expect(JSON.parse(serializeAdvancementScopes([]))).toEqual({ version: 1, rules: [] });
        expect(JSON.parse(serializeAdvancementScopes(["  ", "**"]))).toEqual({
            version: 1,
            rules: [],
        });
    });

    it("is total over garbage and refuses the blanket scope", () => {
        for (const raw of [null, "not json", "{}", '{"rules":"x"}']) {
            expect(parseAdvancementScopes(raw)).toEqual([]);
        }
        expect(
            parseAdvancementScopes(
                '{"version":1,"rules":[{"advance":"writes-within","paths":["**","docs/**"]}]}',
            ),
        ).toEqual(["docs/**"]);
    });
});

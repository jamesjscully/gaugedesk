import { describe, expect, it } from "vitest";
import { readChatModel, readChatProvider, readChatThinking, writeChatModelPin, writeChatThinking } from "./chat-model";

describe("chat-model config IO", () => {
    it("reads model / provider / thinking, or '' when absent", () => {
        const raw = '{"model":"gpt-5.5","provider":"openai-codex","thinking":"high"}';
        expect(readChatModel(raw)).toBe("gpt-5.5");
        expect(readChatProvider(raw)).toBe("openai-codex");
        expect(readChatThinking(raw)).toBe("high");
        expect(readChatModel("{}")).toBe("");
        expect(readChatProvider("")).toBe("");
        expect(readChatThinking("not json")).toBe(""); // tolerant
    });

    it("writes model + provider together, preserving other keys", () => {
        const raw = '{"network":"open","policy":{"x":1}}';
        const out = JSON.parse(writeChatModelPin(raw, { id: "claude-opus-4-6", provider: "anthropic" }));
        expect(out.model).toBe("claude-opus-4-6");
        expect(out.provider).toBe("anthropic");
        expect(out.network).toBe("open"); // untouched
        expect(out.policy).toEqual({ x: 1 }); // untouched
    });

    it("the empty pin clears both model and provider (default resolves)", () => {
        const raw = '{"model":"gpt-5.5","provider":"openai-codex","network":"open"}';
        const out = JSON.parse(writeChatModelPin(raw, { id: "", provider: "" }));
        expect(out.model).toBeUndefined();
        expect(out.provider).toBeUndefined();
        expect(out.network).toBe("open"); // untouched
    });

    it("writes the reasoning level; empty clears it (provider default), 'off' is stored", () => {
        expect(JSON.parse(writeChatThinking("{}", "high")).thinking).toBe("high");
        expect(JSON.parse(writeChatThinking("{}", "off")).thinking).toBe("off"); // explicit no-thinking
        expect(JSON.parse(writeChatThinking('{"thinking":"high","model":"x"}', "")).thinking).toBeUndefined();
        expect(JSON.parse(writeChatThinking('{"thinking":"high","model":"x"}', "")).model).toBe("x"); // untouched
    });
});

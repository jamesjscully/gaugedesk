import { describe, expect, it } from "vitest";
import { chatIdFromSearch, fileFromSearch, searchWithChat, searchWithFile } from "./chat-url";

describe("chat-url — URL-addressable desktop selection (UX-4)", () => {
    it("reads the chat id from a search string, else null", () => {
        expect(chatIdFromSearch("?chat=c1")).toBe("c1");
        expect(chatIdFromSearch("")).toBeNull();
        expect(chatIdFromSearch("?foo=1")).toBeNull();
        expect(chatIdFromSearch("?chat=")).toBeNull();
    });

    it("writes the chat id, preserving other params", () => {
        expect(searchWithChat("", "c1")).toBe("?chat=c1");
        expect(searchWithChat("?cp=http://x", "c2")).toBe("?cp=http%3A%2F%2Fx&chat=c2");
        // round-trips: writing then reading yields the same id.
        expect(chatIdFromSearch(searchWithChat("?dev=1", "c9"))).toBe("c9");
    });

    it("replaces an existing chat param rather than duplicating it", () => {
        expect(searchWithChat("?chat=old", "new")).toBe("?chat=new");
    });

    it("reads the in-chat file path from a search string, else null", () => {
        expect(fileFromSearch("?chat=c1&file=src/a.rs")).toBe("src/a.rs");
        expect(fileFromSearch("?chat=c1")).toBeNull();
        expect(fileFromSearch("")).toBeNull();
        expect(fileFromSearch("?file=")).toBeNull();
    });

    it("writes the file path and clears it on null, preserving the chat param", () => {
        expect(searchWithFile("?chat=c1", "src/a.rs")).toBe("?chat=c1&file=src%2Fa.rs");
        // clearing removes only the file param, keeping chat.
        expect(searchWithFile("?chat=c1&file=src/a.rs", null)).toBe("?chat=c1");
        // round-trips through the chat helper untouched.
        expect(chatIdFromSearch(searchWithFile("?chat=c7", "x.txt"))).toBe("c7");
    });

    it("chat and file mirroring compose without clobbering each other", () => {
        // set a chat, then a file, then re-set the chat: the file survives.
        const a = searchWithChat("", "c1");
        const b = searchWithFile(a, "f.rs");
        const c = searchWithChat(b, "c2");
        expect(chatIdFromSearch(c)).toBe("c2");
        expect(fileFromSearch(c)).toBe("f.rs");
    });
});

import { describe, expect, it } from "vitest";
import { type Attachment, buildOutgoing, classifyAttachment } from "./attachments";

describe("classifyAttachment", () => {
    it("treats the Pi-native image mimes as images", () => {
        for (const type of ["image/png", "image/jpeg", "image/webp", "image/gif"]) {
            expect(classifyAttachment({ type, name: `pic.${type.split("/")[1]}` })).toBe("image");
        }
    });

    it("treats text/* and known textual files as text", () => {
        expect(classifyAttachment({ type: "text/plain", name: "notes.txt" })).toBe("text");
        expect(classifyAttachment({ type: "application/json", name: "data.json" })).toBe("text");
        // No mime from the browser, but a known code extension.
        expect(classifyAttachment({ type: "", name: "main.rs" })).toBe("text");
        expect(classifyAttachment({ type: "", name: "README.md" })).toBe("text");
    });

    it("rejects PDF and Office formats as not-yet-supported", () => {
        expect(classifyAttachment({ type: "application/pdf", name: "report.pdf" })).toBe("unsupported");
        expect(
            classifyAttachment({
                type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                name: "doc.docx",
            }),
        ).toBe("unsupported");
        expect(
            classifyAttachment({
                type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                name: "sheet.xlsx",
            }),
        ).toBe("unsupported");
        expect(classifyAttachment({ type: "application/octet-stream", name: "blob.bin" })).toBe("unsupported");
    });
});

describe("buildOutgoing", () => {
    const img: Attachment = { kind: "image", name: "shot.png", mimeType: "image/png", data: "QUJD" };
    const txt: Attachment = { kind: "text", name: "notes.txt", text: "hello" };

    it("inlines text files into the message and leaves images out of the text", () => {
        const { message, images } = buildOutgoing("look", [txt]);
        expect(message).toBe("look\n\n--- attached: notes.txt ---\nhello\n--- end notes.txt ---");
        expect(images).toEqual([]);
    });

    it("sends images in images[] and keeps base64 OUT of the message (INV-10)", () => {
        const { message, images } = buildOutgoing("describe this", [img]);
        expect(images).toEqual([{ name: "shot.png", mimeType: "image/png", data: "QUJD" }]);
        // Only a byte-free note in the durable text — never the base64 payload.
        expect(message).toBe("describe this\n\n[attached image: shot.png]");
        expect(message).not.toContain("QUJD");
    });

    it("combines a body, text files, and images", () => {
        const { message, images } = buildOutgoing("do it", [txt, img]);
        expect(message).toBe(
            "do it\n\n--- attached: notes.txt ---\nhello\n--- end notes.txt ---\n\n[attached image: shot.png]",
        );
        expect(images).toHaveLength(1);
    });

    it("an image-only message still carries a non-empty (byte-free) text note", () => {
        const { message, images } = buildOutgoing("", [img]);
        expect(message).toBe("[attached image: shot.png]");
        expect(message).not.toContain("QUJD");
        expect(images).toHaveLength(1);
    });
});

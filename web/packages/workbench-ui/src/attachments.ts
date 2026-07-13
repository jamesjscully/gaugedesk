/**
 * Message attachments (UX-14): the pure core behind the composer's paperclip.
 *
 * A picked file is either a **native image** (sent as a WhippleScript resource),
 * an **inline-able text file** (folded into the prompt text), or **not yet supported**
 * (PDF/Office/binary — `classifyAttachment` returns "unsupported"). `buildOutgoing`
 * assembles the neutral turn shape: text inlined into `message`, images in
 * `images[]`, and only a byte-free `[attached image: …]` note left in the durable
 * text so the transcript is honest while base64 never enters the log (`INV-10`).
 *
 * Kept here (pure, browser-API-free except `fileToBase64`) so it is unit-testable
 * apart from the Solid component. It is also the seam where future client-side
 * PDF/Office extraction lands: an extractor turns an "unsupported" file into text
 * and/or image `Attachment`s, and the rest of the pipeline is unchanged.
 */

/** An image clipped to a message: base64 bytes + mime. `name` is for the chip only. */
export type ImageRef = { name: string; mimeType: string; data: string };

/** A file pending in the composer before send. */
export type Attachment =
    | { kind: "text"; name: string; text: string }
    | { kind: "image"; name: string; mimeType: string; data: string };

/** Image types the current WhippleScript host resource accepts natively. */
export const IMAGE_MIMES = new Set(["image/png", "image/jpeg", "image/webp", "image/gif"]);

/** Textual `application/*` mimes worth inlining. Matched EXACTLY — a substring test
 *  would wrongly catch Office's `application/vnd.openxmlformats-…` (it contains "xml"). */
export const TEXT_MIMES = new Set([
    "application/json",
    "application/ld+json",
    "application/xml",
    "application/javascript",
    "application/x-yaml",
    "application/yaml",
    "application/toml",
    "application/csv",
]);

/** Extensions we treat as text when the browser gives no/wrong mime. */
export const TEXT_EXT =
    /\.(txt|md|markdown|mdx|json|jsonc|csv|tsv|ya?ml|toml|ini|cfg|conf|env|xml|html?|css|scss|less|js|jsx|mjs|cjs|ts|tsx|py|rs|go|rb|java|kt|c|h|cc|cpp|hpp|cs|php|swift|sh|bash|zsh|sql|log|gitignore|dockerfile|makefile)$/i;

/** A picked file is a native image, an inline-able text file, or (PDF/Office/binary)
 *  not-yet-supported. An explicit `application/*` mime (PDF, Office) is binary even
 *  with a texty filename, so a known extension never overrides it. */
export function classifyAttachment(f: { type: string; name: string }): "image" | "text" | "unsupported" {
    if (IMAGE_MIMES.has(f.type)) return "image";
    if (f.type.startsWith("text/")) return "text";
    if (TEXT_MIMES.has(f.type)) return "text";
    if (!f.type.startsWith("application/") && TEXT_EXT.test(f.name)) return "text";
    return "unsupported";
}

/** Read a file's bytes as base64 (no data-URL prefix) — the neutral `ImageContent`
 *  wants. Chunked so a large image doesn't blow `fromCharCode`'s argument stack. */
export async function fileToBase64(f: Blob): Promise<string> {
    const bytes = new Uint8Array(await f.arrayBuffer());
    let binary = "";
    const CHUNK = 0x8000;
    for (let i = 0; i < bytes.length; i += CHUNK) {
        binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
    }
    return btoa(binary);
}

/** Assemble the outgoing turn from the draft body + pending attachments: text files
 *  inline as delimited blocks; images become `images[]` with only a byte-free note
 *  left in the message. Pure — the component clears the attachments after calling. */
export function buildOutgoing(body: string, atts: Attachment[]): { message: string; images: ImageRef[] } {
    const trimmed = body.trim();
    const textBlocks = atts
        .filter((a): a is Extract<Attachment, { kind: "text" }> => a.kind === "text")
        .map((a) => `--- attached: ${a.name} ---\n${a.text}\n--- end ${a.name} ---`);
    const images: ImageRef[] = atts
        .filter((a): a is Extract<Attachment, { kind: "image" }> => a.kind === "image")
        .map((a) => ({ name: a.name, mimeType: a.mimeType, data: a.data }));
    const imageNotes = images.map((i) => `[attached image: ${i.name}]`);
    const message = [trimmed, ...textBlocks, ...imageNotes].filter((s) => s.length > 0).join("\n\n");
    return { message, images };
}

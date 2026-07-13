/**
 * Rendered markdown for the content viewer's View tab. Agent-written files are
 * UNTRUSTED input, so safety comes from micromark's defaults: raw HTML inside
 * the markdown is escaped (never executed) and dangerous link protocols are
 * refused — there is no sanitizer to forget because nothing dangerous is
 * emitted. GFM (tables, task lists, strikethrough, autolinks) is on because
 * agents write it constantly. The raw source stays one tab away (Edit).
 */

import { createMemo } from "solid-js";
import { micromark } from "micromark";
import { gfm, gfmHtml } from "micromark-extension-gfm";

/** File paths the viewer renders as markdown (everything else stays `<pre>`). */
export function isMarkdownPath(path: string): boolean {
    return /\.(md|markdown)$/i.test(path);
}

export function renderMarkdown(text: string): string {
    return micromark(text, {
        extensions: [gfm()],
        htmlExtensions: [gfmHtml()],
    });
}

export function MarkdownView(props: { text: string }) {
    const html = createMemo(() => renderMarkdown(props.text));
    // innerHTML is safe here by construction: micromark escapes embedded HTML
    // and refuses javascript:/data: link destinations by default.
    return <div class="filebody markdown-body" data-file-view innerHTML={html()} />;
}

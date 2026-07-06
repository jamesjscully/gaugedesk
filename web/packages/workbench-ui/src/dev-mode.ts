/**
 * Developer/debug mode gate (round-6 #1).
 *
 * Some surfaces are engine-authoring tools, not user-facing UI — the raw
 * review/export state-machine control panel (propose / consent A / target admit /
 * release) is the clearest example. They must never appear in the layperson
 * build, but are useful for development. This flag gates them: off by default,
 * opt-in via `?dev=1` in the URL or `localStorage["ui.dev"] === "1"`.
 *
 * Pure given its inputs so it's unit-testable without a real `window`.
 */
export function readDevMode(search: string, storage: Pick<Storage, "getItem"> | null): boolean {
    try {
        const params = new URLSearchParams(search);
        const q = params.get("dev");
        if (q === "1" || q === "true") return true;
        if (q === "0" || q === "false") return false;
    } catch {
        // malformed search string → fall through to storage
    }
    return storage?.getItem("ui.dev") === "1";
}

/** Resolve dev-mode from the live browser environment. */
export function isDevMode(): boolean {
    if (typeof window === "undefined") return false;
    return readDevMode(window.location.search, window.localStorage);
}

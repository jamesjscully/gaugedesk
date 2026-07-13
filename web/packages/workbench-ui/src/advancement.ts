/**
 * The **advancement policy** schema, client side (ATTN-3, ADR 0082 §4): the
 * operator's auto-keep scopes, read and written against the same rules document
 * the server's settle-time evaluator (`crates/app/src/advancement.rs`) consumes.
 *
 * The settings surface edits exactly one `writes-within` rule — a list of path
 * scopes (`docs/**`, `*.md`, or an exact path). Empty = no rules = everything
 * holds for review (fail-closed). The evaluator's safety conjuncts (a config
 * touch or an externally-tainted read never auto-advances) are not represented
 * here because they are not configurable.
 */

/** The account-settings key holding the rules document. */
export const ADVANCEMENT_RULES_SETTING = "advancement.rules";

/** The auto-keep path scopes: the first `writes-within` rule's paths (the one
 *  the settings surface edits). Total: missing/malformed → `[]` (hold all). */
export function parseAdvancementScopes(raw: string | null | undefined): string[] {
    if (!raw) return [];
    try {
        const doc = JSON.parse(raw) as { rules?: unknown };
        if (!Array.isArray(doc.rules)) return [];
        for (const rule of doc.rules as { advance?: unknown; paths?: unknown }[]) {
            if (rule?.advance !== "writes-within" || !Array.isArray(rule.paths)) continue;
            return rule.paths
                .filter((p): p is string => typeof p === "string")
                .map((p) => p.trim())
                .filter((p) => p.length > 0 && p !== "**");
        }
    } catch {
        /* malformed → hold everything */
    }
    return [];
}

/** Serialize the scopes as the rules document; no scopes → no rules. */
export function serializeAdvancementScopes(paths: readonly string[]): string {
    const cleaned = paths.map((p) => p.trim()).filter((p) => p.length > 0 && p !== "**");
    return JSON.stringify({
        version: 1,
        rules: cleaned.length ? [{ advance: "writes-within", paths: cleaned }] : [],
    });
}

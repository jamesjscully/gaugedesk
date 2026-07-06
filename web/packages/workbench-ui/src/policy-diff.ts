/**
 * Plain-language reading of a *security/permission* change in the method's config
 * file (UX round 3, #3). The merge review can surface a change that touches only
 * `.agent-config.json` — a file the Files panel deliberately hides as "internal".
 * A raw JSON unified-diff is exactly the wrong representation for "this lets the
 * assistant run shell commands": the user can't see the safety meaning.
 *
 * This is a pure, conservative reading of the unified-diff text — the diff string
 * stays the source of truth; we only *summarise* the lines a layperson must
 * understand before keeping the change. It never decides the merge; it only
 * decides what warning to render above the diff.
 */

/** A config/agent-policy file whose changes carry a safety meaning. */
const CONFIG_FILE_RE = /(^|\/)\.?agent-config\.json$/i;

/** One plain-language note about a security-relevant line in the diff. */
export interface PolicyNote {
    /** "loosen" widens what the assistant may do; "tighten" narrows it. */
    readonly direction: "loosen" | "tighten";
    /** A full plain sentence a non-technical person can act on. */
    readonly text: string;
}

export interface PolicyReading {
    /** True when the diff touches *only* the config file (the Files-vs-Changes
     *  disagreement the review must explain rather than hide). */
    readonly onlyConfig: boolean;
    /** True when any config file is part of the change at all. */
    readonly touchesConfig: boolean;
    /** Plain-language notes, loosening first (the ones that matter most). */
    readonly notes: readonly PolicyNote[];
}

/** The set of file paths a unified diff touches (`+++ b/<path>` lines). */
function changedPaths(diff: string): string[] {
    const paths: string[] = [];
    for (const line of diff.split("\n")) {
        if (line.startsWith("+++ ")) {
            const p = line.slice(4).split("\t")[0].trim();
            const clean = p === "/dev/null" ? p : p.replace(/^[ab]\//, "");
            if (clean && clean !== "/dev/null") paths.push(clean);
        } else if (line.startsWith("diff --git ")) {
            const m = line.match(/ b\/(\S+)$/);
            if (m) paths.push(m[1]);
        }
    }
    return [...new Set(paths)];
}

/** Words that, when added/removed, signal a permission/safety change. The map is
 *  intentionally small and explicit — we'd rather under-warn than mislabel. */
const TOOL_PHRASE: Record<string, string> = {
    bash: "run shell commands",
    shell: "run shell commands",
    exec: "run shell commands",
    write: "create or change files",
    edit: "change files",
    delete: "delete files",
    network: "reach the network",
    fetch: "reach the network",
    web: "reach the network",
};

/** Map a raw policy/tool token to a plain phrase, or undefined if we don't have a
 *  confident reading for it. */
function phraseFor(token: string): string | undefined {
    return TOOL_PHRASE[token.toLowerCase()];
}

/** Pull bare tool tokens out of a `block_tools` / `allow_tools` style JSON line. */
function tokensOnLine(line: string): string[] {
    const out: string[] = [];
    for (const m of line.matchAll(/"([a-z_]+)"/gi)) out.push(m[1]);
    return out;
}

/**
 * Read a unified diff for security-relevant config changes. Conservative: a line
 * is only flagged when it both lives in a config file's hunk *and* mentions a
 * known policy key (block/allow/deny/permit/tools/policy) with a known tool token.
 */
export function readPolicyDiff(diff: string): PolicyReading {
    const paths = changedPaths(diff);
    const touchesConfig = paths.some((p) => CONFIG_FILE_RE.test(p));
    const onlyConfig = touchesConfig && paths.every((p) => CONFIG_FILE_RE.test(p));
    if (!touchesConfig) return { onlyConfig: false, touchesConfig: false, notes: [] };

    const notes: PolicyNote[] = [];
    const seen = new Set<string>();
    const POLICY_KEY = /block|deny|allow|permit|tool|policy|sandbox|grant/i;

    for (const raw of diff.split("\n")) {
        if (raw[0] !== "+" && raw[0] !== "-") continue;
        if (raw.startsWith("+++") || raw.startsWith("---")) continue;
        const body = raw.slice(1);
        if (!POLICY_KEY.test(body)) continue;
        const blocking = /block|deny/i.test(body);
        for (const tok of tokensOnLine(body)) {
            const phrase = phraseFor(tok);
            if (!phrase) continue;
            // A removed "block" line, or an added "allow" line → loosening.
            // A removed "allow" line, or an added "block" line → tightening.
            const added = raw[0] === "+";
            const loosen = blocking ? !added : added;
            const direction: PolicyNote["direction"] = loosen ? "loosen" : "tighten";
            const text = loosen
                ? `This will let the assistant ${phrase}.`
                : `This will stop the assistant being able to ${phrase}.`;
            const key = `${direction}:${phrase}`;
            if (seen.has(key)) continue;
            seen.add(key);
            notes.push({ direction, text });
        }
    }
    // Loosening notes first — they're the ones the user must not keep blindly.
    notes.sort((a, b) => (a.direction === b.direction ? 0 : a.direction === "loosen" ? -1 : 1));
    return { onlyConfig, touchesConfig, notes };
}

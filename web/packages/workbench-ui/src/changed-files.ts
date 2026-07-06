/**
 * The user-facing files touched by a turn, read from the unified-diff string
 * (round-7 #3). The Changes tab auto-renders the diff, but View used to sit empty
 * with a "pick a file" hint even when the turn modified exactly one file — two
 * tabs of one panel in different states for the same file, reading like a bug.
 *
 * This is a pure read of the diff's `+++ b/<path>` (and `--- a/<path>`) headers,
 * filtered to the same "user-facing" set the review leads with: internal dotfile
 * artifacts (`.agent-config.json`, other dotfiles) are excluded, matching the
 * Files panel and the diff review. The diff string stays the source of truth; this
 * only decides which file View may auto-open.
 */

/** Strip git's `a/`,`b/` prefix and any trailing tab metadata from a path line. */
function cleanPath(raw: string): string {
    const p = raw.split("\t")[0].trim();
    if (p === "/dev/null") return "";
    return p.startsWith("a/") || p.startsWith("b/") ? p.slice(2) : p;
}

/** Dotfiles are the internal artifacts the review hides (mirrors DiffView). */
function isInternal(path: string): boolean {
    const name = path.split("/").pop() ?? path;
    return name.startsWith(".");
}

/** Whether the unified diff actually contains a file to review — i.e. at least
 *  one `diff --git` segment, mirroring DiffView's `splitFiles`. This is the
 *  predicate that decides "is there a review surface at all": the keep/discard
 *  prompt must not appear when the diff is empty (round-11 #3). It counts internal
 *  (dotfile) changes too, because DiffView still renders those when they're the
 *  only thing changed — so the two surfaces agree on emptiness. */
export function diffHasFiles(diff: string): boolean {
    if (!diff.trim()) return false;
    return diff.split("\n").some((l) => l.startsWith("diff --git "));
}

/** The distinct user-facing paths changed by the diff, in first-seen order. */
export function changedUserFiles(diff: string): string[] {
    if (!diff.trim()) return [];
    const seen = new Set<string>();
    const out: string[] = [];
    for (const line of diff.split("\n")) {
        let path = "";
        if (line.startsWith("+++ ")) path = cleanPath(line.slice(4));
        else if (line.startsWith("--- ")) path = cleanPath(line.slice(4));
        if (!path || isInternal(path) || seen.has(path)) continue;
        seen.add(path);
        out.push(path);
    }
    return out;
}

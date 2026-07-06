/**
 * Per-chat agent run state (round-13). A single global `busy` flag could never
 * represent more than one running agent; the workbench drives each chat
 * independently, so run state is **per chat**, surfaced as a colored dot beside
 * the chat in the Browse panel.
 *
 * Two sources fold into one tone:
 *   - **live, local** — `working` while a turn this client started is in flight,
 *     `error` if it failed. Held in a per-id map in the shell (in-memory).
 *   - **server truth** — `review` (a finished turn awaiting keep/discard) comes
 *     from the `GET /tasks` projection, so it survives reload and reflects turns
 *     finished on *other* clients too.
 *
 * `working`/`error` win over `review` (a turn in flight is the most current fact).
 * Idle is the absence of a tone — no dot.
 */

/** What a chat's status dot conveys. Idle = `undefined` (no dot). */
export type ChatRunTone = "working" | "review" | "error";

/** The hover label for a chat's status dot, or `null` when idle (no dot). */
export function runDotTitle(tone: ChatRunTone | undefined): string | null {
    switch (tone) {
        case "working":
            return "the agent is working on this chat";
        case "review":
            return "this chat has a finished turn waiting for your review";
        case "error":
            return "this chat's last turn didn't finish";
        default:
            return null;
    }
}

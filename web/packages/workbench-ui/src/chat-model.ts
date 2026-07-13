/**
 * Per-chat **model / provider / reasoning-effort** config IO (LLM-1,
 * [ADR 0062](../../specs/decisions/0062-llm-access-byok-and-managed-with-per-project-credentials.md)).
 *
 * The model is the orthogonal `global → archetype → per-chat` axis. These
 * helpers read/write the **per-chat** override in the chat's `.agent-config.json` — the
 * same file `AgentSettings` edits — preserving every other key so a change never clobbers
 * the chat's policy/network config. Three keys live on this axis:
 *
 * - `model` + `provider` — pinned **together** on purpose: a bare model name can silently
 *   resolve to an *unauthenticated* provider (the codex-vs-azure gotcha the engine guards
 *   against), so we never set a model without the provider that authenticates it. Cleared
 *   together → the archetype/global default resolves.
 * - `thinking` — the reasoning-effort level (`off|minimal|low|medium|high|xhigh`).
 *   Absent → the provider's per-model default; present → pinned for this chat.
 *
 * The *catalogue* of which models/efforts exist for the linked accounts lives in
 * `model-picker.ts`; this module only reads and writes the chosen values.
 */

const parseConfig = (raw: string): Record<string, unknown> => {
    try {
        const o = JSON.parse(raw || "{}");
        return o && typeof o === "object" && !Array.isArray(o) ? (o as Record<string, unknown>) : {};
    } catch {
        return {};
    }
};

const readString = (raw: string, key: string): string => {
    const v = parseConfig(raw)[key];
    return typeof v === "string" ? v : "";
};

/** The chat's pinned model id (`""` when none). */
export const readChatModel = (raw: string): string => readString(raw, "model");
/** The chat's pinned provider (`""` when none). */
export const readChatProvider = (raw: string): string => readString(raw, "provider");
/** The chat's pinned reasoning-effort level (`""` = provider default). */
export const readChatThinking = (raw: string): string => readString(raw, "thinking");

/** Write the `(model, provider)` pin into the raw config JSON, preserving every other key.
 *  An empty `id` clears **both** so the archetype/global default resolves. */
export function writeChatModelPin(raw: string, pin: { id: string; provider: string }): string {
    const o = parseConfig(raw);
    if (pin.id) {
        o.model = pin.id;
        o.provider = pin.provider;
    } else {
        delete o.model;
        delete o.provider;
    }
    return JSON.stringify(o, null, 2);
}

/** Write the reasoning-effort `thinking` level, preserving every other key. An empty level
 *  clears the key so the provider's default applies; any level (including `"off"`, an
 *  explicit "no thinking") is stored verbatim. */
export function writeChatThinking(raw: string, level: string): string {
    const o = parseConfig(raw);
    if (level) o.thinking = level;
    else delete o.thinking;
    return JSON.stringify(o, null, 2);
}

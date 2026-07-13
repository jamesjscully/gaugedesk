/**
 * Wait for a Codex OAuth sign-in to land in GaugeDesk's account store (LLM-1,
 * ADR 0062). The sign-in completes in an external browser tab and the control
 * plane seals the credential out-of-band, so the UI's only completion signal is
 * the status projection — poll it instead of making the human press refresh.
 *
 * "Landed" means a live link whose expiry differs from `baselineExpires` (the
 * expiry observed *before* starting the sign-in): a **re**-sign-in starts with a
 * still-linked old credential, so `linked` alone would read as success on the
 * very first poll. A fresh sign-in passes a `null` baseline, which any live
 * link differs from.
 *
 * Resolves `true` when the new credential lands, `false` on cancellation or
 * when the poll budget lapses. A failed poll (control plane busy/restarting)
 * is transient: keep polling rather than giving up.
 */
export async function waitForCodexLink(
    status: () => Promise<{ linked: boolean; expired: boolean; expires?: number | null }>,
    opts: {
        /** The credential expiry seen before the sign-in started (null = none). */
        baselineExpires?: number | null;
        /** Delay between polls (default 2s). */
        intervalMs?: number;
        /** Total budget before giving up (default 5min — OAuth involves a human). */
        timeoutMs?: number;
        /** Checked before each poll — return true to stop (e.g. panel closed). */
        cancelled?: () => boolean;
        /** Injectable delay, for tests. */
        sleep?: (ms: number) => Promise<void>;
    } = {},
): Promise<boolean> {
    const baseline = opts.baselineExpires ?? null;
    const intervalMs = opts.intervalMs ?? 2_000;
    const timeoutMs = opts.timeoutMs ?? 5 * 60_000;
    const sleep = opts.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
    const rounds = Math.max(1, Math.ceil(timeoutMs / intervalMs));
    for (let i = 0; i < rounds; i++) {
        if (opts.cancelled?.()) return false;
        try {
            const s = await status();
            // No baseline (fresh sign-in) → any live link is the new one. With a
            // baseline (re-sign-in) the link must carry a *different* expiry.
            if (s.linked && !s.expired && (baseline === null || (s.expires ?? null) !== baseline))
                return true;
        } catch {
            /* transient — keep polling */
        }
        await sleep(intervalMs);
    }
    return false;
}

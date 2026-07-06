#!/usr/bin/env node
/**
 * OpenAI **codex OAuth** link helper (LLM-1, ADR 0062).
 *
 * Reuses Pi's own tested codex OAuth flow (`@mariozechner/pi-ai`
 * `utils/oauth/openai-codex.js` — `loginOpenAICodex`): PKCE authorize, a local
 * `:1455/auth/callback` server, token exchange. On success it writes the credential
 * into `~/.pi/agent/auth.json` under `openai-codex` (the exact shape Pi stores and the
 * gaugewright engine reads), so a real codex turn can authenticate.
 *
 * The control plane spawns this and reads newline-delimited JSON events on stdout:
 *   {"event":"auth_url","url":"…"}   — open this in a browser to authorize
 *   {"event":"linked","accountId":"…","expires":<ms>}
 *   {"event":"error","message":"…"}
 *
 * We never see the user's password — OAuth is the user's action in their browser.
 * Locate the pi-ai module from GAUGEWRIGHT_PI_BIN (the same binary the engine spawns).
 */

import { existsSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

const emit = (o) => process.stdout.write(JSON.stringify(o) + "\n");
const fail = (message) => {
    emit({ event: "error", message: String(message) });
    process.exit(1);
};

function codexOAuthModulePath() {
    const piBin = process.env.GAUGEWRIGHT_PI_BIN || "pi";
    let real;
    try {
        real = realpathSync(piBin);
    } catch {
        return null; // not an absolute/resolvable path
    }
    // real = <root>/dist/cli.js  ⇒  package root = dirname(dirname(real))
    const root = dirname(dirname(real));
    const p = join(root, "node_modules", "@mariozechner", "pi-ai", "dist", "utils", "oauth", "openai-codex.js");
    return existsSync(p) ? p : null;
}

const modPath = codexOAuthModulePath();
if (!modPath) fail("could not locate the pi-ai codex OAuth module from GAUGEWRIGHT_PI_BIN");

const { loginOpenAICodex } = await import(modPath);
if (typeof loginOpenAICodex !== "function") fail("pi-ai does not export loginOpenAICodex (incompatible Pi version)");

try {
    const cred = await loginOpenAICodex({
        originator: "pi",
        onAuth: ({ url }) => {
            // The orchestrator (control plane → web client) opens this in the user's
            // browser; the helper only runs the callback server + token exchange.
            emit({ event: "auth_url", url });
        },
        // The browser callback is the happy path; if no code arrives we fail cleanly
        // rather than hang on an interactive prompt (this runs headless under the CP).
        onPrompt: async () => {
            throw new Error("no authorization code received on the callback");
        },
    });

    const authPath = join(homedir(), ".pi", "agent", "auth.json");
    mkdirSync(dirname(authPath), { recursive: true });
    let store = {};
    if (existsSync(authPath)) {
        try {
            store = JSON.parse(readFileSync(authPath, "utf8"));
        } catch {
            store = {};
        }
    }
    store["openai-codex"] = {
        type: "oauth",
        access: cred.access,
        refresh: cred.refresh,
        expires: cred.expires,
        accountId: cred.accountId,
    };
    writeFileSync(authPath, JSON.stringify(store, null, 2));
    emit({ event: "linked", accountId: cred.accountId, expires: cred.expires });
    process.exit(0);
} catch (e) {
    fail(e?.message || e);
}

#!/usr/bin/env node
/**
 * Regenerate `packages/workbench-ui/src/model-catalog.generated.ts` from the Pi runtime's bundled
 * model registry (`@mariozechner/pi-ai`) — the catalog the runtime actually resolves
 * models against (LLM-1, ADR 0062). Run after a Pi version bump:
 *
 *     node scripts/gen-model-catalog.mjs
 *
 * We bake the catalog (rather than calling provider `/v1/models` at run time) because
 * it is what Pi can actually run, it carries each model's reasoning/thinking levels
 * (which no provider listing reports), and it works for `openai-codex` OAuth, which has
 * no listing endpoint. We resolve pi-ai from the globally-installed `pi` so the data
 * matches the runtime the app spawns.
 *
 * Only providers the app can plausibly bind a credential to are emitted (keeps the
 * client bundle focused); extend ALLOW when a new provider becomes linkable.
 */
import { execSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

// Providers the app can actually bind a credential to today (account.rs maps
// openai/anthropic keys; openai-codex is the OAuth path). Add a provider here AND
// wire its credential mapping in `crates/app/src/account.rs` before its models can
// resolve — otherwise its entries would render in the picker but always fail closed.
const ALLOW = new Set(["openai-codex", "openai", "anthropic"]);

const globalRoot = execSync("npm root -g").toString().trim();
const piAiDist = join(globalRoot, "@mariozechner/pi-coding-agent/node_modules/@mariozechner/pi-ai/dist/models.js");
const { getProviders, getModels, getSupportedThinkingLevels } = await import(pathToFileURL(piAiDist).href);

const out = [];
for (const provider of getProviders()) {
    if (!ALLOW.has(provider)) continue;
    for (const m of getModels(provider)) {
        out.push({
            provider,
            id: m.id,
            name: m.name,
            reasoning: Boolean(m.reasoning),
            thinking: getSupportedThinkingLevels(m),
            input: m.input ?? [],
        });
    }
}
// Stable order so the generated diff is legible: provider, then display name.
out.sort((a, b) => a.provider.localeCompare(b.provider) || a.name.localeCompare(b.name));

const file = `// AUTO-GENERATED — do not edit by hand.
// Regenerate with: node scripts/gen-model-catalog.mjs  (after a Pi version bump)
// Source: @mariozechner/pi-ai model registry (the catalog the Pi runtime resolves).

/** A model the Pi runtime knows how to run, with the metadata the picker needs. */
export interface CatalogModel {
    readonly provider: string;
    readonly id: string;
    readonly name: string;
    readonly reasoning: boolean;
    /** Supported \`--thinking\` levels (off | minimal | low | medium | high | xhigh). */
    readonly thinking: readonly string[];
    /** Input modalities the model accepts (e.g. "text", "image"). */
    readonly input: readonly string[];
}

export const MODEL_CATALOG: readonly CatalogModel[] = ${JSON.stringify(out, null, 4)};
`;

writeFileSync(new URL("../packages/workbench-ui/src/model-catalog.generated.ts", import.meta.url), file);
console.log(`wrote ${out.length} models from ${[...ALLOW].length} allowed providers → packages/workbench-ui/src/model-catalog.generated.ts`);

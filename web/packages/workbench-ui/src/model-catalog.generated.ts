// AUTO-GENERATED — do not edit by hand.
// Regenerate with: node scripts/gen-model-catalog.mjs  (after a Pi version bump)
// Source: @mariozechner/pi-ai model registry (the catalog the Pi runtime resolves).

/** A model the Pi runtime knows how to run, with the metadata the picker needs. */
export interface CatalogModel {
    readonly provider: string;
    readonly id: string;
    readonly name: string;
    readonly reasoning: boolean;
    /** Supported `--thinking` levels (off | minimal | low | medium | high | xhigh). */
    readonly thinking: readonly string[];
    /** Input modalities the model accepts (e.g. "text", "image"). */
    readonly input: readonly string[];
}

export const MODEL_CATALOG: readonly CatalogModel[] = [
    {
        "provider": "anthropic",
        "id": "claude-3-haiku-20240307",
        "name": "Claude Haiku 3",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-3-5-haiku-20241022",
        "name": "Claude Haiku 3.5",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-3-5-haiku-latest",
        "name": "Claude Haiku 3.5 (latest)",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-haiku-4-5-20251001",
        "name": "Claude Haiku 4.5",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-haiku-4-5",
        "name": "Claude Haiku 4.5 (latest)",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-3-opus-20240229",
        "name": "Claude Opus 3",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-opus-4-20250514",
        "name": "Claude Opus 4",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-opus-4-0",
        "name": "Claude Opus 4 (latest)",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-opus-4-1-20250805",
        "name": "Claude Opus 4.1",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-opus-4-1",
        "name": "Claude Opus 4.1 (latest)",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-opus-4-5-20251101",
        "name": "Claude Opus 4.5",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-opus-4-5",
        "name": "Claude Opus 4.5 (latest)",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-opus-4-6",
        "name": "Claude Opus 4.6",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-opus-4-7",
        "name": "Claude Opus 4.7",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-3-sonnet-20240229",
        "name": "Claude Sonnet 3",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-3-5-sonnet-20240620",
        "name": "Claude Sonnet 3.5",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-3-5-sonnet-20241022",
        "name": "Claude Sonnet 3.5 v2",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-3-7-sonnet-20250219",
        "name": "Claude Sonnet 3.7",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-sonnet-4-20250514",
        "name": "Claude Sonnet 4",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-sonnet-4-0",
        "name": "Claude Sonnet 4 (latest)",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-sonnet-4-5-20250929",
        "name": "Claude Sonnet 4.5",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-sonnet-4-5",
        "name": "Claude Sonnet 4.5 (latest)",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "anthropic",
        "id": "claude-sonnet-4-6",
        "name": "Claude Sonnet 4.6",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-4",
        "name": "GPT-4",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-4-turbo",
        "name": "GPT-4 Turbo",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-4.1",
        "name": "GPT-4.1",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-4.1-mini",
        "name": "GPT-4.1 mini",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-4.1-nano",
        "name": "GPT-4.1 nano",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-4o",
        "name": "GPT-4o",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-4o-2024-05-13",
        "name": "GPT-4o (2024-05-13)",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-4o-2024-08-06",
        "name": "GPT-4o (2024-08-06)",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-4o-2024-11-20",
        "name": "GPT-4o (2024-11-20)",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-4o-mini",
        "name": "GPT-4o mini",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5",
        "name": "GPT-5",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5-chat-latest",
        "name": "GPT-5 Chat Latest",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5-mini",
        "name": "GPT-5 Mini",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5-nano",
        "name": "GPT-5 Nano",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5-pro",
        "name": "GPT-5 Pro",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5-codex",
        "name": "GPT-5-Codex",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.1",
        "name": "GPT-5.1",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.1-chat-latest",
        "name": "GPT-5.1 Chat",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.1-codex",
        "name": "GPT-5.1 Codex",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.1-codex-max",
        "name": "GPT-5.1 Codex Max",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.1-codex-mini",
        "name": "GPT-5.1 Codex mini",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.2",
        "name": "GPT-5.2",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.2-chat-latest",
        "name": "GPT-5.2 Chat",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.2-codex",
        "name": "GPT-5.2 Codex",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.2-pro",
        "name": "GPT-5.2 Pro",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.3-chat-latest",
        "name": "GPT-5.3 Chat (latest)",
        "reasoning": false,
        "thinking": [
            "off"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.3-codex",
        "name": "GPT-5.3 Codex",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.3-codex-spark",
        "name": "GPT-5.3 Codex Spark",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.4",
        "name": "GPT-5.4",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.4-mini",
        "name": "GPT-5.4 mini",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.4-nano",
        "name": "GPT-5.4 nano",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.4-pro",
        "name": "GPT-5.4 Pro",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.5",
        "name": "GPT-5.5",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "gpt-5.5-pro",
        "name": "GPT-5.5 Pro",
        "reasoning": true,
        "thinking": [
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "o1",
        "name": "o1",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "o1-pro",
        "name": "o1-pro",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "o3",
        "name": "o3",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "o3-deep-research",
        "name": "o3-deep-research",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "o3-mini",
        "name": "o3-mini",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text"
        ]
    },
    {
        "provider": "openai",
        "id": "o3-pro",
        "name": "o3-pro",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "o4-mini",
        "name": "o4-mini",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai",
        "id": "o4-mini-deep-research",
        "name": "o4-mini-deep-research",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai-codex",
        "id": "gpt-5.1",
        "name": "GPT-5.1",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai-codex",
        "id": "gpt-5.1-codex-max",
        "name": "GPT-5.1 Codex Max",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai-codex",
        "id": "gpt-5.1-codex-mini",
        "name": "GPT-5.1 Codex Mini",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai-codex",
        "id": "gpt-5.2",
        "name": "GPT-5.2",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai-codex",
        "id": "gpt-5.2-codex",
        "name": "GPT-5.2 Codex",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai-codex",
        "id": "gpt-5.3-codex",
        "name": "GPT-5.3 Codex",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai-codex",
        "id": "gpt-5.3-codex-spark",
        "name": "GPT-5.3 Codex Spark",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text"
        ]
    },
    {
        "provider": "openai-codex",
        "id": "gpt-5.4",
        "name": "GPT-5.4",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai-codex",
        "id": "gpt-5.4-mini",
        "name": "GPT-5.4 Mini",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    },
    {
        "provider": "openai-codex",
        "id": "gpt-5.5",
        "name": "GPT-5.5",
        "reasoning": true,
        "thinking": [
            "off",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh"
        ],
        "input": [
            "text",
            "image"
        ]
    }
];

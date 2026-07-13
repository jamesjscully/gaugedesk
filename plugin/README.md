# gaugewright Pi plugin

> **Retired production adapter (SUB-1, 2026-07-09).** GaugeDesk now invokes
> WhippleScript for every real local turn. This plugin remains with
> `crates/pi-bridge` as historical protocol/conformance evidence and is not
> shipped in the desktop bundle. The `.pi/**` definition layout is a separate
> SUB-3 migration input; retaining that layout does not reactivate this plugin.

The former default plugin surface Pi loaded in RPC mode
(`pi --mode rpc -e plugin/gaugewright-plugin.ts`). It is the **in-process egress
membrane**: it enforces the agent's
`.agent-config.json` `policy` on Pi's `tool_call` hook so an out-of-policy effect
is blocked before it executes (the no-prompt static path of `pi-rpc.md`).

- **One membrane, two enforcement points.** The Rust host (`crates/boundary`)
  holds the policy model; this plugin is its enforcement point *inside* Pi. The
  two share the same posture semantics (`trust-by-default` / `prompt-on-risk` /
  `policy-only-block`) so a decision is the same on either side.
- **Type-only import.** It imports `@mariozechner/pi-coding-agent` for types only
  (erased at runtime), so it loads without that package resolvable at runtime —
  only `node:fs` / `node:path` are imported for real. Verified loading against
  Pi 0.73.1.
- **Fail closed.** A malformed policy, or a staged effect with no approver in a
  headless run, blocks rather than allows.

The retained Rust `pi-bridge` corpus spawns Pi with this plugin in conformance
tests via `PiConfig.extra_args`
(`["-e", "plugin/gaugewright-plugin.ts"]`) and reads the resulting allow/block
decisions off the event stream as runtime-session observations.

`gaugewright-plugin.ts` is retained source evidence for the former open/local
adapter. It is not a current managed-host or packaging contract.

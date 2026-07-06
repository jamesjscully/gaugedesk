# gaugewright Pi plugin

The default plugin surface Pi loads in RPC mode
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

The Rust `pi-bridge` spawns Pi with this plugin via `PiConfig.extra_args`
(`["-e", "plugin/gaugewright-plugin.ts"]`) and reads the resulting allow/block
decisions off the event stream as runtime-session observations.

`gaugewright-plugin.ts` is the open/local plugin. Managed hosted sandboxes use a
hosted variant that lives in the private `gaugewright-cloud` repo; it keeps the
same membrane and adds the Cloudflare AI Gateway provider registration used for
paid hosted model egress. Private sandbox packaging vendors that hosted file
under the runtime name `gaugewright-plugin.ts` so existing
`GAUGEWRIGHT_PLUGIN_PATH` values keep working.

# gaugewright desktop shell (Tauri v2)

Packages the Solid workbench (`../web`) as a desktop app and starts the
co-resident control plane on loopback. The webview connects over **HTTP, not
Tauri IPC** (`app-stack.md`) — the same client is the web build and (later) the
remote client; Tauri is packaging + a window, not a second transport.

## Status

**Scaffolded, not built here.** Building needs the Tauri CLI and system webkit
deps (`webkit2gtk`, etc. on Linux), plus app icons under `icons/`, which aren't
available in this environment. The Rust shell (`src/main.rs`), config
(`tauri.conf.json`), and capabilities are in place and consistent with the
backend (`gaugewright_app::open_api::open_serve`).

## Build / run (where the toolchain exists)

```
cargo install tauri-cli --version '^2'   # once
# add icons/icon.png (and platform icons) — `cargo tauri icon <png>` generates them
cargo tauri dev      # runs vite dev + the window + control plane
cargo tauri build    # bundles the app
```

It is deliberately **outside the backend cargo workspace** (the root `Cargo.toml`
excludes it) so `cargo test` over `crates/*` stays self-contained and green.

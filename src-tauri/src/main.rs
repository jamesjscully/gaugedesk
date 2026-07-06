//! gaugewright desktop shell (Tauri v2).
//!
//! One coherent workbench (`app-stack.md`): Tauri hosts the Solid island, and a
//! **co-resident control plane** runs on loopback. The webview talks to it over
//! **HTTP, not Tauri IPC** — so the exact same client works as a browser/web
//! build and (later) against a remote. Tauri here is packaging + a window, not a
//! second transport.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        // Native OS folder picker for "add files" (the webview opens a real
        // folder browser; the chosen absolute path is ingested over HTTP).
        .plugin(tauri_plugin_dialog::init())
        // Self-update (SELFHOST-1 / D-RELEASE-LANES): checks the GitHub Release
        // `latest.json` against the pubkey in tauri.conf.json (plugins.updater).
        // Registration only — a check/apply call is a follow-on UX; the release
        // lane signs update artifacts with the matching TAURI_SIGNING_PRIVATE_KEY.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Self-contained runtime (SELFHOST-1): a packaged bundle vendors Pi +
            // git + the membrane plugin and points the control plane at them via
            // the env seam (GAUGEWRIGHT_PI_BIN / GAUGEWRIGHT_GIT_BIN / GAUGEWRIGHT_PLUGIN_PATH).
            // This MUST run *before* the control-plane thread spawns: `open_serve()` →
            // `open_workbench()` runs git at startup (reconciling engagements), so
            // git must already resolve to the vendored binary. A dev `tauri dev`
            // run has no vendored payload — each var is set only when its file
            // exists, otherwise the resolver falls back to PATH/cwd (the dev path).
            if let Ok(resource_dir) = app.path().resource_dir() {
                set_if_present(
                    "GAUGEWRIGHT_PI_BIN",
                    resource_dir.join("bin").join(exe("pi")),
                );
                set_if_present(
                    "GAUGEWRIGHT_GIT_BIN",
                    resource_dir.join("bin").join(exe("git")),
                );
                set_if_present(
                    "GAUGEWRIGHT_PLUGIN_PATH",
                    resource_dir.join("plugin").join("gaugewright-plugin.ts"),
                );
            }

            // Spawn-vs-connect (DEPLOY-5): the **solo** shell spawns a co-resident control
            // plane; an **enterprise** deployment that names an org control plane
            // (`GAUGEWRIGHT_ORG_CP`) skips the spawn — the webview connects to that org CP
            // through its own DEPLOY-5 seam (the persisted endpoint / `?cp=`). One shell,
            // two runtime configs.
            // ENTSEC-8 (ADR 0065) fail-loud guard: an enterprise/thin install pins
            // `GAUGEWRIGHT_REQUIRE_ORG_CP=1`. If it is set but no org CP is configured, refuse to
            // silently fall back to spawning a co-resident on-disk store (which would write the
            // client's data — db, git repos, transcripts — onto the consultant's unmanaged
            // endpoint, the exact leak thin mode exists to prevent). Hard-exit with a clear
            // operator message instead of degrading open.
            let org_cp = std::env::var("GAUGEWRIGHT_ORG_CP").ok();
            let require_org_cp = std::env::var("GAUGEWRIGHT_REQUIRE_ORG_CP").as_deref() == Ok("1");
            let decision =
                cp_launch_decision(org_cp.as_deref(), require_org_cp).unwrap_or_else(|msg| {
                    eprintln!("[gaugewright] FATAL: {msg}");
                    std::process::exit(1);
                });
            match decision {
                Some(bind) => {
                    // Start the control plane in the background before the window is
                    // interactive. The store + git instance live under the OS app-data dir
                    // (cwd `.gaugewright` in dev), resolved by the workspace crate.
                    std::thread::spawn(move || {
                        let root = open_control_plane_root();
                        let rt = tokio::runtime::Builder::new_multi_thread()
                            .enable_all()
                            .build()
                            .expect("tokio runtime");
                        rt.block_on(async move {
                            if let Err(e) = gaugewright_app::open_api::open_serve(bind, &root).await
                            {
                                eprintln!("control plane exited: {e}");
                            }
                        });
                    });
                }
                None => {
                    // Enterprise: no co-resident control plane; the webview talks to the
                    // enrolled org control plane. **Seed** that endpoint into the webview at
                    // launch (DEPLOY-5) — the client's `resolveControlPlaneBase` reads the
                    // persisted `gw.cp` key, so seeding it here means a fresh enterprise install
                    // connects to the org CP without first having to run a manual enrollment
                    // that persisted it. (First-load timing — vs. a refresh — would want an
                    // initialization script; that needs a windowed run to verify.)
                    eprintln!("enterprise mode: connecting to the enrolled org control plane");
                    if let Some(script) =
                        webview_org_cp_script(std::env::var("GAUGEWRIGHT_ORG_CP").ok().as_deref())
                    {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.eval(&script);
                        }
                    }
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running gaugewright desktop");
}

/// The OS-specific executable name (`pi` / `pi.exe`). Tauri can't tell us the
/// target at runtime, so cfg picks it.
fn exe(stem: &str) -> String {
    #[cfg(windows)]
    {
        format!("{stem}.exe")
    }
    #[cfg(not(windows))]
    {
        stem.to_string()
    }
}

/// Point an asset env var at a vendored path **only if it exists**, so a dev run
/// (no vendored payload under `resource_dir`) cleanly falls back to the resolver's
/// PATH/cwd default rather than naming a missing file.
fn set_if_present(var: &str, path: std::path::PathBuf) {
    if path.exists() {
        std::env::set_var(var, path);
    }
}

fn open_control_plane_root() -> std::path::PathBuf {
    // Delegate to the workspace resolver (GAUGEWRIGHT_ROOT → OS app-data dir →
    // `./.gaugewright`), which is the unit-tested source of truth. src-tauri sits
    // outside the cargo workspace, so this thin wrapper is not cargo-tested.
    gaugewright_app::open_api::open_control_plane_root()
}

/// The co-resident control-plane bind address, or `None` when the shell should **not** spawn
/// one (DEPLOY-5). Solo → `Some(127.0.0.1:7878)`; an enterprise deployment that names an org
/// control plane (a non-empty `GAUGEWRIGHT_ORG_CP`) → `None`, so the webview connects to that
/// org CP instead. Pure in its input, so the spawn-vs-connect decision is unit-testable.
fn local_cp_bind(org_cp: Option<&str>) -> Option<&'static str> {
    match org_cp {
        Some(s) if !s.trim().is_empty() => None,
        _ => Some("127.0.0.1:7878"),
    }
}

/// The spawn-vs-connect decision with the `ENTSEC-8` fail-loud guard. Normally this is just
/// [`local_cp_bind`]: solo spawns a local CP, an enterprise install (a named `GAUGEWRIGHT_ORG_CP`)
/// connects to it. But when the install is **pinned to thin/enterprise** mode
/// (`require_org_cp`, from `GAUGEWRIGHT_REQUIRE_ORG_CP=1`) and no org CP is configured, this
/// returns `Err` rather than silently spawning a co-resident on-disk store — so a misconfigured
/// launch fails loudly instead of leaking the client's data onto the consultant's endpoint. Pure
/// in its inputs, so the guard is unit-testable without env/Tauri.
fn cp_launch_decision(
    org_cp: Option<&str>,
    require_org_cp: bool,
) -> Result<Option<&'static str>, String> {
    let thin = matches!(org_cp, Some(s) if !s.trim().is_empty());
    if require_org_cp && !thin {
        return Err(
            "GAUGEWRIGHT_REQUIRE_ORG_CP=1 but GAUGEWRIGHT_ORG_CP is unset/empty — refusing to spawn \
             a local on-disk store (thin-client mode was required). Set GAUGEWRIGHT_ORG_CP to the \
             org control plane, or unset GAUGEWRIGHT_REQUIRE_ORG_CP to run solo."
                .to_string(),
        );
    }
    Ok(local_cp_bind(org_cp))
}

/// The webview **init script** that seeds the enrolled org control-plane endpoint (DEPLOY-5):
/// it persists `org_cp` under the `gw.cp` localStorage key the web client's
/// `resolveControlPlaneBase` reads, so the enterprise webview connects to the org CP. `None`
/// for solo (no/empty `GAUGEWRIGHT_ORG_CP`), so the solo path injects nothing. The URL is
/// JSON-escaped, so an operator-configured endpoint cannot break out of the string. Pure in its
/// input → the produced script (and the key it writes) is unit-testable without a window.
fn webview_org_cp_script(org_cp: Option<&str>) -> Option<String> {
    let url = org_cp.map(str::trim).filter(|s| !s.is_empty())?;
    // serde_json::to_string yields a safely-quoted JS string literal (escapes quotes/backslashes).
    let lit = serde_json::to_string(url).ok()?;
    Some(format!(
        "try {{ window.localStorage.setItem('gw.cp', {lit}); }} catch (e) {{}}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{cp_launch_decision, local_cp_bind, webview_org_cp_script};

    #[test]
    fn solo_spawns_the_co_resident_control_plane() {
        // No org CP configured (or an empty one) → solo: spawn the co-resident CP.
        assert_eq!(local_cp_bind(None), Some("127.0.0.1:7878"));
        assert_eq!(local_cp_bind(Some("")), Some("127.0.0.1:7878"));
        assert_eq!(local_cp_bind(Some("   ")), Some("127.0.0.1:7878"));
    }

    #[test]
    fn enterprise_skips_the_spawn_and_connects_to_the_org_cp() {
        // A named org control plane → enterprise: no co-resident spawn.
        assert_eq!(local_cp_bind(Some("https://cp.acme.example")), None);
    }

    #[test]
    fn require_org_cp_fails_loud_when_no_org_cp_is_configured() {
        // ENTSEC-8: pinned-thin install + no org CP → refuse, do NOT fall back to a local store.
        let err = cp_launch_decision(None, true).unwrap_err();
        assert!(
            err.contains("GAUGEWRIGHT_ORG_CP"),
            "names the missing var: {err}"
        );
        assert!(cp_launch_decision(Some(""), true).is_err());
        assert!(cp_launch_decision(Some("   "), true).is_err());
    }

    #[test]
    fn require_org_cp_with_a_configured_cp_connects_thin() {
        // Pinned-thin AND an org CP configured → connect (no local spawn).
        assert_eq!(
            cp_launch_decision(Some("https://cp.acme.example"), true).unwrap(),
            None
        );
    }

    #[test]
    fn without_the_pin_the_decision_is_unchanged_solo_or_thin() {
        // No pin → the existing behavior: solo spawns, a named org CP connects.
        assert_eq!(
            cp_launch_decision(None, false).unwrap(),
            Some("127.0.0.1:7878")
        );
        assert_eq!(
            cp_launch_decision(Some(""), false).unwrap(),
            Some("127.0.0.1:7878")
        );
        assert_eq!(
            cp_launch_decision(Some("https://cp.acme.example"), false).unwrap(),
            None
        );
    }

    #[test]
    fn solo_seeds_no_org_cp_endpoint() {
        // DEPLOY-5: nothing injected on the solo path (the client uses the solo default).
        assert_eq!(webview_org_cp_script(None), None);
        assert_eq!(webview_org_cp_script(Some("")), None);
        assert_eq!(webview_org_cp_script(Some("   ")), None);
    }

    #[test]
    fn enterprise_seeds_the_org_cp_into_the_gw_cp_key() {
        // The injected script persists the endpoint under the exact key the web seam reads.
        let script = webview_org_cp_script(Some("https://cp.acme.example")).unwrap();
        assert!(script.contains("window.localStorage.setItem('gw.cp'"));
        assert!(script.contains("\"https://cp.acme.example\""));
    }

    #[test]
    fn the_seeded_endpoint_is_json_escaped_against_breakout() {
        // The value is wrapped in a double-quoted JS string literal, so a double-quote in the
        // endpoint must be escaped (else it could break out). serde_json does this.
        let script = webview_org_cp_script(Some("https://x\"+alert(1)+\"y")).unwrap();
        assert!(script.contains("setItem('gw.cp'"));
        // The embedded double-quotes are backslash-escaped, not left to close the literal early.
        assert!(script.contains("x\\\"+alert(1)+\\\"y"));
    }
}

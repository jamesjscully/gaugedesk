//! OpenAI **codex OAuth** link (LLM-1, [ADR 0062]).
//!
//! The codex credential is an OAuth token Pi stores in `~/.pi/<user>/auth.json` — *not*
//! the account sealed store (which holds BYOK API keys). The engine's default provider
//! (`openai-codex`) reads Pi's store, so to make a real codex turn authenticate the
//! token must live there. These routes:
//!
//!   - `GET  /account/oauth/openai-codex`       — is a codex credential present, until when?
//!   - `POST /account/oauth/openai-codex/start` — start the OAuth link: spawn the
//!     `sidecar/codex-oauth-login.mjs` helper (which reuses Pi's own tested flow — PKCE,
//!     a local `:1455` callback, token exchange) and return the **authorize URL**. The
//!     helper keeps running its callback server and writes the credential to Pi's store
//!     on success; the client polls `GET` for the result.
//!
//! We never see the user's password — OAuth is the user's action in their browser.

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Pi's auth store: the observed layout is `~/.pi/agent/auth.json`; fall back to any
/// `~/.pi/<user>/auth.json` (`pi-rpc.md` §A6 `auth/{userId}/auth.json`).
fn pi_auth_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let agent = std::path::Path::new(&home).join(".pi/agent/auth.json");
    if agent.is_file() {
        return Some(agent);
    }
    let pi = std::path::Path::new(&home).join(".pi");
    std::fs::read_dir(&pi)
        .ok()?
        .flatten()
        .map(|e| e.path().join("auth.json"))
        .find(|p| p.is_file())
}

/// GET /account/oauth/openai-codex — codex credential presence + expiry (never the token).
pub async fn get_codex_status() -> impl IntoResponse {
    let codex = pi_auth_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("openai-codex").cloned());
    let expires = codex
        .as_ref()
        .and_then(|c| c.get("expires"))
        .and_then(Value::as_i64);
    let linked = codex.is_some();
    let expired = expires.map(|e| e <= now_ms()).unwrap_or(false);
    Json(
        json!({ "provider": "openai-codex", "linked": linked, "expires": expires, "expired": expired }),
    )
}

fn node_bin(pi_bin: &str) -> String {
    if let Ok(n) = std::env::var("GAUGEWRIGHT_NODE_BIN") {
        return n;
    }
    // node usually sits beside the `pi` the engine points at (e.g. the same nvm bin dir).
    if let Some(dir) = std::path::Path::new(pi_bin).parent() {
        let node = dir.join("node");
        if node.is_file() {
            return node.to_string_lossy().into_owned();
        }
    }
    "node".to_string()
}

fn codex_helper_path() -> String {
    std::env::var("GAUGEWRIGHT_CODEX_LOGIN")
        .unwrap_or_else(|_| "sidecar/codex-oauth-login.mjs".to_string())
}

/// Resolve the Pi executable for the login helper: an explicit `GAUGEWRIGHT_PI_BIN`
/// wins, else the first executable `pi` on `PATH`. The engine (`crates/pi-bridge`)
/// has the same env-first-then-PATH fallback, but it can let the OS resolve `pi` at
/// spawn time; the helper must `import` Pi's bundled codex-OAuth module from *beside*
/// the binary, so it needs the concrete path. We compute it here and hand it to the
/// child, so a bare CLI/dev control plane links exactly like the packaged desktop
/// bundle (which sets `GAUGEWRIGHT_PI_BIN` from its vendored payload). `None` ⇒ no Pi.
fn resolve_pi_bin() -> Option<String> {
    resolve_pi_bin_from(
        std::env::var("GAUGEWRIGHT_PI_BIN").ok(),
        std::env::var_os("PATH"),
    )
}

/// Pure resolution (env override, then a `PATH` search), split out so it is testable
/// without touching the process environment.
fn resolve_pi_bin_from(
    override_var: Option<String>,
    path: Option<std::ffi::OsString>,
) -> Option<String> {
    if let Some(bin) = override_var.filter(|s| !s.is_empty()) {
        return Some(bin);
    }
    std::env::split_paths(&path?)
        .map(|dir| dir.join("pi"))
        .find(|cand| is_executable_file(cand))
        .map(|cand| cand.to_string_lossy().into_owned())
}

#[cfg(unix)]
fn is_executable_file(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(p: &std::path::Path) -> bool {
    p.is_file()
}

/// Spawn the login helper and read newline-JSON events until the authorize URL (or an
/// error). On the URL, hand the still-running child + reader to a drain thread so the
/// helper never SIGPIPEs on its later writes (it writes the credential before reporting
/// `linked`), and return the URL.
fn start_login_blocking() -> Result<String, String> {
    let pi_bin = resolve_pi_bin().ok_or_else(|| {
        "no Pi runtime found: set GAUGEWRIGHT_PI_BIN or put `pi` on PATH".to_string()
    })?;
    let mut child = Command::new(node_bin(&pi_bin))
        .arg(codex_helper_path())
        // Hand the resolved Pi path to the helper so it locates Pi's codex-OAuth module
        // beside the binary even when the process env left `GAUGEWRIGHT_PI_BIN` unset
        // (the dev/CLI control plane — the packaged bundle sets it from its payload).
        .env("GAUGEWRIGHT_PI_BIN", &pi_bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn codex login helper: {e}"))?;
    let stdout = child.stdout.take().ok_or("login helper: no stdout")?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    for _ in 0..6 {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF before a URL
            Ok(_) => {}
            Err(e) => return Err(format!("login helper read: {e}")),
        }
        let Ok(ev) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        match ev.get("event").and_then(Value::as_str) {
            Some("auth_url") => {
                let url = ev
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or("login helper: auth_url without url")?
                    .to_string();
                // Keep the helper alive: drain its stdout (so it never SIGPIPEs) and reap
                // on exit. It writes the credential to Pi's store, polled via GET status.
                std::thread::spawn(move || {
                    let mut sink = String::new();
                    while reader.read_line(&mut sink).map(|n| n > 0).unwrap_or(false) {
                        sink.clear();
                    }
                    let _ = child.wait();
                });
                return Ok(url);
            }
            Some("error") => {
                return Err(ev
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("codex login failed")
                    .to_string());
            }
            _ => {}
        }
    }
    let _ = child.kill();
    Err("codex login helper produced no authorize URL".to_string())
}

/// POST /account/oauth/openai-codex/start — begin the OAuth link, return the authorize URL.
pub async fn post_codex_login_start() -> impl IntoResponse {
    match tokio::task::spawn_blocking(start_login_blocking).await {
        Ok(Ok(url)) => Json(json!({ "url": url })).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "login task panicked").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_override_wins() {
        assert_eq!(
            resolve_pi_bin_from(Some("/opt/pi/bin/pi".into()), None),
            Some("/opt/pi/bin/pi".into()),
        );
    }

    #[test]
    fn empty_or_absent_override_searches_path() {
        let dir = tempfile::tempdir().unwrap();
        let pi = dir.path().join("pi");
        std::fs::write(&pi, "#!/bin/sh\n").unwrap();
        make_executable(&pi);
        let path = std::env::join_paths([dir.path()]).unwrap();
        let want = Some(pi.to_string_lossy().into_owned());
        // An empty override and a missing one both fall back to PATH (mirrors the engine).
        assert_eq!(
            resolve_pi_bin_from(Some(String::new()), Some(path.clone())),
            want
        );
        assert_eq!(resolve_pi_bin_from(None, Some(path)), want);
    }

    #[test]
    fn none_when_pi_absent() {
        let empty = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([empty.path()]).unwrap();
        assert_eq!(resolve_pi_bin_from(None, Some(path)), None);
        assert_eq!(resolve_pi_bin_from(None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_pi_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pi"), "not a program").unwrap();
        let path = std::env::join_paths([dir.path()]).unwrap();
        assert_eq!(resolve_pi_bin_from(None, Some(path)), None);
    }

    #[cfg(unix)]
    fn make_executable(p: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    fn make_executable(_p: &std::path::Path) {}
}

//! GaugeDesk-owned OpenAI Codex OAuth lifecycle (LLM-1, ADR 0062).
//!
//! The sidecar performs PKCE + the loopback callback and returns the token bundle
//! over a private pipe. GaugeDesk seals access + refresh material in its account
//! record, refreshes it here, and gives the runtime only an ephemeral access
//! token/account id. Neither Pi nor WhippleScript owns or locates credentials.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use crate::account::{resolve_token, seal_token, ACCOUNT_SCOPE};
use crate::{LockUnpoisoned, SharedWorkbench};

const PROVIDER: &str = "openai-codex";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REFRESH_SKEW_MS: i64 = 60_000;
pub const CODEX_ACCESS_BINDING: &str = "GAUGEDESK_CODEX_ACCESS_TOKEN";
pub const CODEX_ACCOUNT_BINDING: &str = "GAUGEDESK_CODEX_ACCOUNT_ID";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CodexOAuthCredential {
    access: String,
    refresh: String,
    expires: i64,
    account_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexRuntimeCredential {
    pub access: String,
    pub account_id: String,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn load_credential(wb: &SharedWorkbench) -> Option<CodexOAuthCredential> {
    let workbench = wb.lock_unpoisoned();
    let encoded = resolve_token(workbench.store_ref(), workbench.account_key(), PROVIDER)?;
    serde_json::from_str(&encoded).ok()
}

fn store_credential(wb: &SharedWorkbench, credential: &CodexOAuthCredential) -> Result<(), String> {
    let plaintext = serde_json::to_string(credential).map_err(|error| error.to_string())?;
    let mut workbench = wb.lock_unpoisoned();
    let sealed = seal_token(workbench.account_key(), &plaintext)
        .ok_or_else(|| "could not seal Codex OAuth credential".to_owned())?;
    workbench
        .upsert_account_credential_in(ACCOUNT_SCOPE, PROVIDER.to_owned(), sealed)
        .map_err(|error| format!("could not store Codex OAuth credential: {error:?}"))?;
    workbench.advance_onboarding("credential", &json!({ "provider": PROVIDER }).to_string());
    Ok(())
}

/// Status projection; plaintext token fields never cross HTTP.
pub async fn get_codex_status(State(wb): State<SharedWorkbench>) -> impl IntoResponse {
    let credential = load_credential(&wb);
    let expires = credential.as_ref().map(|value| value.expires);
    Json(json!({
        "provider": PROVIDER,
        "linked": credential.is_some(),
        "expires": expires,
        "expired": expires.is_some_and(|value| value <= now_ms()),
    }))
}

fn node_bin() -> String {
    std::env::var("GAUGEWRIGHT_NODE_BIN").unwrap_or_else(|_| "node".to_owned())
}

/// The helper script rides inside the binary: spawning it must not depend on the
/// process cwd or a bundled payload (the packaged app ships no `sidecar/` tree).
/// `GAUGEWRIGHT_CODEX_LOGIN` still points at an on-disk script when set — the
/// dev/test seam for substituting a fake helper.
const HELPER_SOURCE: &str = include_str!("../../../sidecar/codex-oauth-login.mjs");

/// Start the helper, return the authorization URL, then retain its private pipe
/// in a background thread until the credential bundle can be sealed.
fn start_login_blocking(wb: SharedWorkbench) -> Result<String, String> {
    let mut command = Command::new(node_bin());
    let override_path = std::env::var("GAUGEWRIGHT_CODEX_LOGIN").ok();
    match &override_path {
        Some(path) => {
            command.arg(path).stdin(Stdio::null());
        }
        None => {
            // `node --input-type=module -` runs the embedded ESM source from stdin.
            command
                .arg("--input-type=module")
                .arg("-")
                .stdin(Stdio::piped());
        }
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn Codex login helper: {error}"))?;
    if override_path.is_none() {
        use std::io::Write;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Codex login helper exposed no input pipe".to_owned())?;
        stdin
            .write_all(HELPER_SOURCE.as_bytes())
            .map_err(|error| format!("Codex login helper feed: {error}"))?;
        // Dropping the handle closes the pipe so node sees EOF and runs the program.
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex login helper exposed no output pipe".to_owned())?;
    let mut stderr = child.stderr.take();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    for _ in 0..6 {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => return Err(format!("Codex login helper read: {error}")),
        }
        let Ok(event) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        match event.get("event").and_then(Value::as_str) {
            Some("auth_url") => {
                let url = event
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Codex login helper returned no URL".to_owned())?
                    .to_owned();
                // Drain stderr so a chatty helper can never block on a full pipe.
                if let Some(mut pipe) = stderr.take() {
                    std::thread::spawn(move || {
                        let _ = std::io::copy(&mut pipe, &mut std::io::sink());
                    });
                }
                std::thread::spawn(move || {
                    let mut result = String::new();
                    while reader
                        .read_line(&mut result)
                        .map(|count| count > 0)
                        .unwrap_or(false)
                    {
                        if let Ok(event) = serde_json::from_str::<Value>(result.trim()) {
                            if event.get("event").and_then(Value::as_str) == Some("linked") {
                                if let Ok(credential) =
                                    serde_json::from_value::<CodexOAuthCredential>(event)
                                {
                                    let _ = store_credential(&wb, &credential);
                                }
                            }
                        }
                        result.clear();
                    }
                    let _ = child.wait();
                });
                return Ok(url);
            }
            Some("error") => {
                return Err(event
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex login failed")
                    .to_owned())
            }
            _ => {}
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    // The helper died before emitting a URL; its stderr is the actual reason
    // (e.g. a missing node module) — surface it instead of a blind 502.
    let mut detail = String::new();
    if let Some(mut pipe) = stderr.take() {
        use std::io::Read;
        let _ = pipe.read_to_string(&mut detail);
    }
    let detail = detail.trim();
    if detail.is_empty() {
        Err("Codex login helper produced no authorization URL".to_owned())
    } else {
        Err(format!(
            "Codex login helper produced no authorization URL: {detail}"
        ))
    }
}

pub async fn post_codex_login_start(State(wb): State<SharedWorkbench>) -> impl IntoResponse {
    match tokio::task::spawn_blocking(move || start_login_blocking(wb)).await {
        Ok(Ok(url)) => Json(json!({ "url": url })).into_response(),
        Ok(Err(error)) => {
            (StatusCode::BAD_GATEWAY, Json(json!({ "error": error }))).into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Codex login task panicked",
        )
            .into_response(),
    }
}

fn refresh_credential(credential: &CodexOAuthCredential) -> Result<CodexOAuthCredential, String> {
    let response = ureq::post(TOKEN_URL)
        .set("content-type", "application/x-www-form-urlencoded")
        .send_form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", credential.refresh.as_str()),
            ("client_id", CLIENT_ID),
        ])
        .map_err(|error| format!("Codex token refresh failed: {error}"))?;
    let body: Value = response
        .into_json()
        .map_err(|error| format!("Codex token refresh returned invalid JSON: {error}"))?;
    let access = body
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "Codex token refresh returned no access token".to_owned())?;
    let refresh = body
        .get("refresh_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "Codex token refresh returned no refresh token".to_owned())?;
    let expires_in = body
        .get("expires_in")
        .and_then(Value::as_i64)
        .ok_or_else(|| "Codex token refresh returned no expiry".to_owned())?;
    Ok(CodexOAuthCredential {
        access: access.to_owned(),
        refresh: refresh.to_owned(),
        expires: now_ms() + expires_in * 1_000,
        account_id: credential.account_id.clone(),
    })
}

/// Resolve the short-lived material used for one turn. Refresh runs outside the
/// workbench lock; a successful replacement is sealed before it is returned.
pub fn resolve_runtime_credential(
    wb: &SharedWorkbench,
) -> Result<Option<CodexRuntimeCredential>, String> {
    let Some(mut credential) = load_credential(wb) else {
        return Ok(None);
    };
    if credential.expires <= now_ms() + REFRESH_SKEW_MS {
        credential = refresh_credential(&credential)?;
        store_credential(wb, &credential)?;
    }
    Ok(Some(CodexRuntimeCredential {
        access: credential.access,
        account_id: credential.account_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_bundle_round_trips_without_changing_wire_names() {
        let credential = CodexOAuthCredential {
            access: "access".to_owned(),
            refresh: "refresh".to_owned(),
            expires: 42,
            account_id: "account".to_owned(),
        };
        let value = serde_json::to_value(&credential).expect("serialize");
        assert_eq!(value["accountId"], "account");
        assert_eq!(
            serde_json::from_value::<CodexOAuthCredential>(value).expect("deserialize"),
            credential
        );
    }
}

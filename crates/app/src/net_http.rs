//! The shared outbound HTTP client (M3): a thin blocking `ureq` wrapper for setup,
//! integration, and operator-facing calls that need plain HTTPS without introducing
//! a larger async client stack.
//!
//! Blocking (`ureq`), matching the sync seam signatures — every caller is
//! setup/login/operator frequency, never the request hot path, so a blocking call (run from
//! async handlers via [`tokio::task::spawn_blocking`]) is appropriate. TLS is `rustls` on
//! **`ring`** (no native-tls), keeping the build OpenSSL- and cmake-free like the rest of
//! the stack.

use std::time::Duration;

use axum::{http::StatusCode, response::IntoResponse, Json};
use gaugewright_store::AdmitError;

/// The name of the shared web-account session cookie (ADR 0077): the hosted hub sets it
/// `Domain=.gaugewright.com` on login, so one sign-in authenticates the whole site.
pub const SESSION_COOKIE: &str = "gw_session";

/// The credential a request presents, from **either** the `Authorization: Bearer <token>`
/// header **or** the [`SESSION_COOKIE`] cookie. `pub` so the extracted enterprise band
/// (`gaugewright-ee`) and the private route lanes parse it exactly like the open routes.
///
/// The cookie fallback is what makes the hosted Console work: a browser cannot set an
/// `Authorization` header on an `EventSource` (SSE) or a top-level navigation, but it *does*
/// send cookies — so the shared `Domain=.gaugewright.com` session cookie authenticates the
/// live streams and cross-subdomain requests. The header still wins when both are present
/// (explicit programmatic clients, tests).
pub fn bearer(headers: &axum::http::HeaderMap) -> Option<&str> {
    if let Some(tok) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    {
        return Some(tok.trim());
    }
    session_cookie(headers)
}

/// The [`SESSION_COOKIE`] value from the `Cookie` header, if present and non-empty.
pub fn session_cookie(headers: &axum::http::HeaderMap) -> Option<&str> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        if let Some(v) = part.trim().strip_prefix("gw_session=") {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Liveness/readiness probe handler — a fixed 200 `{"ok":true}` once the router is serving.
/// No store access (`INV-5`): it reports the process is up, not any truth.
pub(crate) async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

/// `ENTSEC-7` (ADR 0065): set HSTS on every response so a browser that ever reaches the control
/// plane over HTTPS refuses to downgrade to plain HTTP thereafter (defeating an SSL-strip / first-
/// request-over-http MITM once TLS is in front). Harmless on the loopback/dev path — browsers
/// ignore an HSTS header received over plain HTTP, so solo/e2e are unaffected; it only arms once a
/// TLS-terminating proxy serves the same headers over HTTPS. Two years, subdomains included.
pub async fn security_headers(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut resp = next.run(req).await;
    resp.headers_mut().insert(
        axum::http::header::STRICT_TRANSPORT_SECURITY,
        axum::http::HeaderValue::from_static("max-age=63072000; includeSubDomains"),
    );
    resp
}

/// The default CORS origin allowlist (FED-2): the Vite dev server, the built preview, and the
/// Tauri webview — instead of permissive `*`. Extended by `GAUGEWRIGHT_ALLOWED_ORIGINS`
/// (comma-separated). Shared by the control-plane CORS and the per-deployment embed CORS (which
/// allows the deployment's configured origins on top of these).
pub fn default_allowed_origins() -> Vec<String> {
    const DEFAULT_ORIGINS: &[&str] = &[
        "http://localhost:5173",
        "http://127.0.0.1:5173",
        "http://localhost:4173",
        "http://127.0.0.1:4173",
        // Tauri v2 webview origins (platform-dependent).
        "tauri://localhost",
        "http://tauri.localhost",
    ];
    let mut v: Vec<String> = DEFAULT_ORIGINS.iter().map(|s| s.to_string()).collect();
    if let Ok(extra) = std::env::var("GAUGEWRIGHT_ALLOWED_ORIGINS") {
        for o in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            v.push(o.to_string());
        }
    }
    v
}

/// The CORS layer for the control-plane API (FED-2): a pinned origin allowlist.
pub fn cors_layer() -> tower_http::cors::CorsLayer {
    use axum::http::{header, HeaderValue, Method};
    use tower_http::cors::{AllowOrigin, CorsLayer};
    let origins: Vec<HeaderValue> = default_allowed_origins()
        .iter()
        .filter_map(|s| HeaderValue::from_str(s).ok())
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        // The hosted Console is a cross-subdomain browser client (app.gaugewright.com →
        // auth/api host) that authenticates with the shared `Domain=.gaugewright.com`
        // session cookie (ADR 0077). Credentialed CORS is required for the browser to send
        // that cookie and for SSE `withCredentials`. Safe here: the origin is a **pinned
        // allowlist** (never wildcard), which is the one case credentials forbid.
        .allow_credentials(true)
}

/// Neutral store-error → HTTP response formatting. `pub` so extracted private
/// route lanes (e.g. the attested operator surface) format admission errors the
/// same way the open routes do.
pub fn err_response(e: AdmitError) -> axum::response::Response {
    (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response()
}

/// A reusable blocking HTTP client (connection-pooled `ureq::Agent`).
pub struct HttpClient {
    agent: ureq::Agent,
}

impl HttpClient {
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(20))
    }

    /// A client with an explicit overall timeout. The 20s [`new`](Self::new) default
    /// suits one-shot setup/login/payment calls; the on-request JWKS self-refresh
    /// (`ID-3`) uses a shorter bound so an unreachable IdP can't stall an admin request
    /// (which holds the workbench lock) for long.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            agent: ureq::AgentBuilder::new().timeout(timeout).build(),
        }
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient {
    /// GET `url`, returning the response body. A non-2xx response is an error because
    /// callers use this for documents that must exist to proceed.
    pub fn get_string(&self, url: &str) -> Result<String, String> {
        match self.agent.get(url).call() {
            Ok(resp) => resp.into_string().map_err(|e| format!("read body: {e}")),
            Err(ureq::Error::Status(code, resp)) => Err(format!(
                "HTTP {code}: {}",
                resp.into_string().unwrap_or_default()
            )),
            Err(ureq::Error::Transport(t)) => Err(format!("transport: {t}")),
        }
    }

    /// JSON POST with custom headers (e.g. `Authorization: Bearer …`), returning
    /// `(status, body)`. A transport failure is `Err`; an HTTP error status is `Ok`
    /// with that status so the caller can inspect it.
    pub fn post_json_headers(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &str,
    ) -> Result<(u16, String), String> {
        let mut req = self.agent.post(url).set("Content-Type", "application/json");
        for (k, v) in headers {
            req = req.set(k, v);
        }
        match req.send_string(body) {
            Ok(resp) => Ok((resp.status(), resp.into_string().unwrap_or_default())),
            Err(ureq::Error::Status(code, resp)) => {
                Ok((code, resp.into_string().unwrap_or_default()))
            }
            Err(ureq::Error::Transport(t)) => Err(format!("transport: {t}")),
        }
    }

    /// JSON `PUT`, returning `(status, body)`. A transport failure is `Err`; an HTTP error status
    /// is `Ok` with that status (the caller inspects it). Used by the blind-directory publish
    /// (`PUT /directory/:root`, ADR 0054).
    pub fn put_json(&self, url: &str, body: &str) -> Result<(u16, String), String> {
        match self
            .agent
            .put(url)
            .set("Content-Type", "application/json")
            .send_string(body)
        {
            Ok(resp) => Ok((resp.status(), resp.into_string().unwrap_or_default())),
            Err(ureq::Error::Status(code, resp)) => {
                Ok((code, resp.into_string().unwrap_or_default()))
            }
            Err(ureq::Error::Transport(t)) => Err(format!("transport: {t}")),
        }
    }

    /// `application/x-www-form-urlencoded` POST (e.g. an OAuth2 token request), returning
    /// `(status, body)`.
    pub fn post_form(&self, url: &str, fields: &[(&str, &str)]) -> Result<(u16, String), String> {
        match self.agent.post(url).send_form(fields) {
            Ok(resp) => Ok((resp.status(), resp.into_string().unwrap_or_default())),
            Err(ureq::Error::Status(code, resp)) => {
                Ok((code, resp.into_string().unwrap_or_default()))
            }
            Err(ureq::Error::Transport(t)) => Err(format!("transport: {t}")),
        }
    }

    /// `application/x-www-form-urlencoded` POST with bearer auth,
    /// returning `(status, body)`.
    pub fn post_form_auth(
        &self,
        url: &str,
        bearer: &str,
        fields: &[(&str, &str)],
    ) -> Result<(u16, String), String> {
        self.post_form_auth_headers(url, bearer, &[], fields)
    }

    /// `application/x-www-form-urlencoded` POST with bearer auth and extra headers,
    /// returning `(status, body)`.
    pub fn post_form_auth_headers(
        &self,
        url: &str,
        bearer: &str,
        headers: &[(&str, &str)],
        fields: &[(&str, &str)],
    ) -> Result<(u16, String), String> {
        let mut req = self
            .agent
            .post(url)
            .set("Authorization", &format!("Bearer {bearer}"));
        for (k, v) in headers {
            req = req.set(k, v);
        }
        match req.send_form(fields) {
            Ok(resp) => Ok((resp.status(), resp.into_string().unwrap_or_default())),
            Err(ureq::Error::Status(code, resp)) => {
                Ok((code, resp.into_string().unwrap_or_default()))
            }
            Err(ureq::Error::Transport(t)) => Err(format!("transport: {t}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bearer, session_cookie};
    use axum::http::{header, HeaderMap, HeaderValue};

    fn headers(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (name, value) in pairs {
            h.insert(name.clone(), HeaderValue::from_str(value).unwrap());
        }
        h
    }

    #[test]
    fn bearer_reads_the_authorization_header() {
        let h = headers(&[(header::AUTHORIZATION, "Bearer abc.def.ghi")]);
        assert_eq!(bearer(&h), Some("abc.def.ghi"));
    }

    #[test]
    fn bearer_falls_back_to_the_session_cookie() {
        // A browser SSE / navigation carries no Authorization header, only cookies.
        let h = headers(&[(header::COOKIE, "other=1; gw_session=tok-123; theme=dark")]);
        assert_eq!(bearer(&h), Some("tok-123"));
        assert_eq!(session_cookie(&h), Some("tok-123"));
    }

    #[test]
    fn the_authorization_header_wins_over_the_cookie() {
        let h = headers(&[
            (header::AUTHORIZATION, "Bearer header-tok"),
            (header::COOKIE, "gw_session=cookie-tok"),
        ]);
        assert_eq!(bearer(&h), Some("header-tok"));
    }

    #[test]
    fn no_credential_is_none_and_an_empty_cookie_is_ignored() {
        assert_eq!(bearer(&HeaderMap::new()), None);
        let h = headers(&[(header::COOKIE, "gw_session=; x=1")]);
        assert_eq!(session_cookie(&h), None);
        assert_eq!(bearer(&h), None);
    }
}

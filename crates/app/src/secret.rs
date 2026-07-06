//! A redacting wrapper for sensitive strings (`SECAUD-10`, SOC 2 CC6.1).
//!
//! Tokens, PKCE verifiers, and keys are bare `String`s that flow through structs
//! deriving `Debug` and through `tracing`/`format!`. A single accidental `{:?}` or
//! log line would put a live credential in a log file. [`Secret`] holds such a value
//! and **redacts it from both `Debug` and `Display`**, so it is safe to embed in a
//! `Debug`-deriving struct or pass to a log macro. The inner value is reachable only
//! through [`Secret::expose`] — an explicit, greppable call that marks every real use
//! site for review.

use serde::{Deserialize, Serialize};

/// A sensitive string whose `Debug`/`Display` render `<redacted>`. Serde-transparent
/// (serializes as the raw inner string) so it can stand in for a `String` field in an
/// in-memory or persisted record without changing the wire shape; the redaction is
/// only for *formatting* paths (logs/Debug), which is where the leak risk is.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    /// Wrap a sensitive value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Reveal the inner value. The **only** way in — keep call sites minimal and
    /// audited (grep `.expose()` to enumerate every real use of a secret).
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_redact_but_expose_reveals() {
        let s = Secret::new("pkce-verifier-abc123");
        // Neither Debug nor Display leaks the value...
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
        assert_eq!(format!("{s}"), "<redacted>");
        assert!(!format!("{s:?} {s}").contains("pkce-verifier-abc123"));
        // ...but expose() returns it verbatim for real use.
        assert_eq!(s.expose(), "pkce-verifier-abc123");
    }

    #[test]
    fn redacts_inside_a_debug_deriving_struct() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Holder {
            token: Secret,
        }
        let h = Holder {
            token: "super-secret-token".into(),
        };
        assert!(!format!("{h:?}").contains("super-secret-token"));
        assert!(format!("{h:?}").contains("<redacted>"));
    }

    #[test]
    fn serde_is_transparent() {
        let s = Secret::new("v");
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"v\"");
        let back: Secret = serde_json::from_str("\"v\"").unwrap();
        assert_eq!(back.expose(), "v");
    }
}

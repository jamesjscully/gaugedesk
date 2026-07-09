//! Host-filtering forward proxy (CORE-5 / RF-B3) — the enforcement *heart* of the
//! [`Network::Filtered`](crate::sandbox::Network::Filtered) posture.
//!
//! It speaks the minimal HTTP `CONNECT host:port` tunnel protocol: a client asks
//! to open a raw byte tunnel to `host:port`; the proxy allows it **only** when
//! `host` is on the exact-match allowlist, replies `200 Connection Established`,
//! then blind-copies bytes both ways. It **never decrypts** — TLS stays end-to-end
//! between the agent and the model endpoint; the proxy rules purely on the
//! `CONNECT` authority it is asked to reach. A host absent from the allowlist gets
//! `403` and no tunnel; a malformed request gets `400`.
//!
//! This is only *load-bearing* when it is the sandbox netns's **sole** outbound
//! path (see [`crate::sandbox`]): with the netns default-dropping every other
//! route, a host absent from the allowlist is unreachable even if the agent
//! ignores the `HTTPS_PROXY`/`HTTP_PROXY` env — the proxy is the checkpoint every
//! byte must cross. On its own (reachable alongside an open route) it would be
//! honor-system only; the non-bypassable routing is the netns's job.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Cap on the request-header bytes we will buffer before the `CONNECT` line's
/// terminating blank line. A well-formed `CONNECT` is tiny; anything larger is a
/// client that is not speaking the tunnel protocol — refuse rather than buffer.
const MAX_REQUEST_BYTES: usize = 8 * 1024;

/// Exact-match host allowlist test (ASCII case-insensitive, as hostnames are).
/// No suffix/wildcard matching: `evil-api.openai.com.attacker.com` must NOT match
/// `api.openai.com`, so the check is deliberately whole-host equality.
pub fn host_allowed(host: &str, allowlist: &[String]) -> bool {
    allowlist.iter().any(|a| a.eq_ignore_ascii_case(host))
}

/// The `(host, port)` a `CONNECT` request targets, parsed from the raw request
/// bytes (up to and including the header terminator). Pure so the parse is
/// unit-testable without a socket. Rejects anything that is not a well-formed
/// `CONNECT authority HTTP/x` request line.
pub fn parse_connect(raw: &[u8]) -> Result<(String, u16), ConnectError> {
    let text = std::str::from_utf8(raw).map_err(|_| ConnectError::Malformed)?;
    let line = text.lines().next().ok_or(ConnectError::Malformed)?;
    let mut tok = line.split_whitespace();
    match tok.next() {
        Some(m) if m.eq_ignore_ascii_case("CONNECT") => {}
        _ => return Err(ConnectError::NotConnect),
    }
    let authority = tok.next().ok_or(ConnectError::Malformed)?;
    // A request line is `METHOD SP request-target SP HTTP-version`; require the
    // version token so a truncated line is not silently accepted.
    tok.next().ok_or(ConnectError::Malformed)?;
    // `host:port` — split on the LAST colon so an IPv6 literal's inner colons stay
    // with the host (a bracketed `[::1]` host is refused later by the allowlist).
    let (host, port) = authority.rsplit_once(':').ok_or(ConnectError::Malformed)?;
    if host.is_empty() {
        return Err(ConnectError::Malformed);
    }
    let port: u16 = port.parse().map_err(|_| ConnectError::Malformed)?;
    Ok((host.to_string(), port))
}

/// Why a `CONNECT` request could not be honored — kept distinct so the reply code
/// (`400` malformed vs. `405` non-CONNECT) is precise.
#[derive(Debug, PartialEq, Eq)]
pub enum ConnectError {
    /// The request line was not a parseable `CONNECT authority HTTP/x`.
    Malformed,
    /// A syntactically valid request, but not the `CONNECT` method.
    NotConnect,
}

/// A running proxy: its bound loopback address, plus a drop-guard on the accept
/// task so the proxy is torn down when the handle is dropped (one per turn; its
/// lifetime is the turn's).
pub struct EgressProxy {
    addr: SocketAddr,
    _task: tokio::task::JoinHandle<()>,
}

impl EgressProxy {
    /// Bind an accept loop on `127.0.0.1:0` and start serving. Returns once the
    /// listener is bound (so [`Self::addr`] is immediately usable). Must run on a
    /// tokio runtime.
    pub async fn start(allowlist: Vec<String>) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let allow = Arc::new(allowlist);
        let task = tokio::spawn(async move { serve(listener, allow).await });
        Ok(Self { addr, _task: task })
    }

    /// The loopback address the proxy is listening on (host `HTTPS_PROXY` points at).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

/// Accept connections forever, handling each on its own task. A per-connection
/// error never takes the loop down — one misbehaving client must not close the
/// checkpoint for the rest of the turn.
async fn serve(listener: TcpListener, allow: Arc<Vec<String>>) {
    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                let allow = allow.clone();
                tokio::spawn(async move {
                    let _ = handle_conn(stream, &allow).await;
                });
            }
            // A transient accept error (fd pressure) shouldn't wedge the proxy.
            Err(_) => continue,
        }
    }
}

/// One client connection: read the `CONNECT` request, rule on the target host,
/// and — only on allow — open the upstream and blind-tunnel bytes both ways.
async fn handle_conn(mut inbound: TcpStream, allow: &[String]) -> io::Result<()> {
    // Read request bytes until the header terminator `\r\n\r\n`. One byte at a
    // time so we never read past the blank line into the tunnel body — after
    // `200`, the very next bytes are the client's TLS ClientHello, which must go
    // to the upstream untouched.
    let mut buf: Vec<u8> = Vec::with_capacity(128);
    let mut byte = [0u8; 1];
    loop {
        if buf.len() > MAX_REQUEST_BYTES {
            let _ = inbound.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
            return Ok(());
        }
        match inbound.read(&mut byte).await {
            Ok(0) => return Ok(()), // client hung up before a full request
            Ok(_) => buf.push(byte[0]),
            Err(e) => return Err(e),
        }
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let (host, port) = match parse_connect(&buf) {
        Ok(hp) => hp,
        Err(ConnectError::NotConnect) => {
            let _ = inbound
                .write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
                .await;
            return Ok(());
        }
        Err(ConnectError::Malformed) => {
            let _ = inbound.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
            return Ok(());
        }
    };

    // THE CHECK: a host off the allowlist is refused before any upstream socket is
    // opened — the agent cannot even probe reachability of a non-allowlisted host.
    if !host_allowed(&host, allow) {
        let _ = inbound.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n").await;
        return Ok(());
    }

    // Allowed: open the upstream. A connect failure is the upstream's, not a
    // policy refusal — report it as `502` so the two are distinguishable.
    let mut outbound = match TcpStream::connect((host.as_str(), port)).await {
        Ok(s) => s,
        Err(_) => {
            let _ = inbound.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
            return Ok(());
        }
    };

    inbound
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    // Blind, symmetric copy until either side closes — no inspection, no
    // decryption; TLS is end-to-end.
    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_connect_extracts_host_and_port() {
        let (h, p) = parse_connect(b"CONNECT api.openai.com:443 HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!(h, "api.openai.com");
        assert_eq!(p, 443);
    }

    #[test]
    fn parse_connect_is_case_insensitive_on_method() {
        assert!(parse_connect(b"connect example.com:443 HTTP/1.1\r\n\r\n").is_ok());
    }

    #[test]
    fn parse_connect_rejects_non_connect_and_malformed() {
        assert_eq!(
            parse_connect(b"GET / HTTP/1.1\r\n\r\n"),
            Err(ConnectError::NotConnect)
        );
        // No port.
        assert_eq!(
            parse_connect(b"CONNECT api.openai.com HTTP/1.1\r\n\r\n"),
            Err(ConnectError::Malformed)
        );
        // No version token.
        assert_eq!(
            parse_connect(b"CONNECT api.openai.com:443\r\n\r\n"),
            Err(ConnectError::Malformed)
        );
        // Empty host.
        assert_eq!(
            parse_connect(b"CONNECT :443 HTTP/1.1\r\n\r\n"),
            Err(ConnectError::Malformed)
        );
        // Non-numeric port.
        assert_eq!(
            parse_connect(b"CONNECT h:zz HTTP/1.1\r\n\r\n"),
            Err(ConnectError::Malformed)
        );
    }

    #[test]
    fn host_allowed_is_exact_case_insensitive_only() {
        let allow = vec!["api.openai.com".to_string(), "chatgpt.com".to_string()];
        assert!(host_allowed("api.openai.com", &allow));
        assert!(
            host_allowed("API.OpenAI.CoM", &allow),
            "hostnames are case-insensitive"
        );
        // No suffix/substring matching — the classic allowlist bypass.
        assert!(!host_allowed("api.openai.com.attacker.com", &allow));
        assert!(!host_allowed("evil.com", &allow));
        assert!(!host_allowed("openai.com", &allow), "not a suffix match");
    }

    // ---- integration over real localhost TCP -------------------------------
    //
    // These drive the proxy end to end: a real client socket speaks CONNECT to a
    // real bound proxy, which (on allow) tunnels to a real upstream echo listener.

    /// A one-shot echo listener on loopback; returns its address and a task that
    /// echoes the first client's bytes back until close.
    async fn spawn_echo() -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = l.local_addr().unwrap();
        let t = tokio::spawn(async move {
            if let Ok((mut s, _)) = l.accept().await {
                let mut b = [0u8; 1024];
                loop {
                    match s.read(&mut b).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if s.write_all(&b[..n]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });
        (addr, t)
    }

    /// Read a single `\r\n\r\n`-terminated status/header block from a stream.
    async fn read_head(s: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match s.read(&mut byte).await {
                Ok(0) | Err(_) => break,
                Ok(_) => buf.push(byte[0]),
            }
            if buf.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    #[tokio::test]
    async fn allowed_host_tunnels_bytes_end_to_end() {
        let (echo_addr, _echo) = spawn_echo().await;
        // Allowlist the loopback host the echo listener is on.
        let proxy = EgressProxy::start(vec!["127.0.0.1".to_string()])
            .await
            .unwrap();

        let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
        let req = format!("CONNECT 127.0.0.1:{} HTTP/1.1\r\n\r\n", echo_addr.port());
        client.write_all(req.as_bytes()).await.unwrap();
        let head = read_head(&mut client).await;
        assert!(
            head.starts_with("HTTP/1.1 200"),
            "expected tunnel, got: {head:?}"
        );

        // Bytes now flow blind to the echo upstream and back.
        let payload = b"ping-through-tunnel";
        client.write_all(payload).await.unwrap();
        let mut got = vec![0u8; payload.len()];
        client.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, payload);
    }

    #[tokio::test]
    async fn disallowed_host_is_refused_with_403_and_no_tunnel() {
        // Echo exists, but its host is NOT on the allowlist.
        let (echo_addr, _echo) = spawn_echo().await;
        let proxy = EgressProxy::start(vec!["api.openai.com".to_string()])
            .await
            .unwrap();

        let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
        // Ask for the echo by a disallowed name mapped to loopback.
        let req = format!("CONNECT localhost:{} HTTP/1.1\r\n\r\n", echo_addr.port());
        client.write_all(req.as_bytes()).await.unwrap();
        let head = read_head(&mut client).await;
        assert!(
            head.starts_with("HTTP/1.1 403"),
            "expected refusal, got: {head:?}"
        );
    }

    #[tokio::test]
    async fn malformed_request_is_refused() {
        let proxy = EgressProxy::start(vec!["api.openai.com".to_string()])
            .await
            .unwrap();
        let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
        // Not a CONNECT at all.
        client
            .write_all(b"GET http://api.openai.com/ HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let head = read_head(&mut client).await;
        assert!(
            head.starts_with("HTTP/1.1 405"),
            "a non-CONNECT method must be refused, got: {head:?}"
        );
    }
}

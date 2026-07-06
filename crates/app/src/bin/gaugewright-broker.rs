//! gaugewright rendezvous broker binary (D-REMOTE / `RENDEZVOUS-STUB-1` → real).
//!
//! The dumb cross-machine transport two paired control planes meet at: each
//! authority dials *out* to this broker, which pairs the two legs that name the
//! same session token and splices their byte streams. It **never** parses,
//! terminates, or inspects the carried bytes (`RELAY_NO_PAYLOAD_ACCESS` /
//! `INV-10`) — it holds no key and learns only a per-direction byte count, so it
//! cannot read payloads or forge a crossing even if it wanted to (the legs carry
//! end-to-end-encrypted, signed envelopes; M3 adds the TLS).
//!
//! Open source with the federation mechanism (ADR 0068 §4): self-operated
//! federation — including running your own broker — is free; the paid managed
//! relay is GaugeWright *operating* this same binary (`infra/relay`, the
//! gaugewright-cloud repo).
//!
//! Run alongside two control planes for a manual multi-machine session:
//!   GAUGEWRIGHT_BROKER_ADDR=0.0.0.0:7900 gaugewright-broker
//! Both `gaugewright-app` instances are pointed at this address when they pair.

#[tokio::main]
async fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    // Bind address from `GAUGEWRIGHT_BROKER_ADDR` (default loopback `127.0.0.1:7900`
    // for same-host manual testing; a real deployment binds a routable interface
    // like `0.0.0.0:7900`). An optional `GAUGEWRIGHT_BROKER_READY` file is touched once
    // the listener is bound, so a launcher / container healthcheck can gate on it.
    let addr =
        std::env::var("GAUGEWRIGHT_BROKER_ADDR").unwrap_or_else(|_| "127.0.0.1:7900".to_string());
    let ready = std::env::var("GAUGEWRIGHT_BROKER_READY").ok();

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind broker");
    if let Some(path) = ready.as_deref() {
        let _ = std::fs::File::create(path);
    }
    eprintln!("[rendezvous] broker listening on {addr}");
    gaugewright_app::fed_harness::broker_accept_loop(listener)
        .await
        .expect("broker serve");
}

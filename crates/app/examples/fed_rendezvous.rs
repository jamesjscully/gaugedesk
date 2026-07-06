//! The Tier-1 harness **rendezvous broker** binary (`COMPOSE-HARNESS-1`). Runs the
//! dumb multi-session byte-splice broker that is the only A↔B path in the
//! NAT-isolated compose topology. It never parses, terminates, or inspects the
//! carried bytes (`RELAY_NO_PAYLOAD_ACCESS` / `INV-10`).
//!
//! Env: `GAUGEWRIGHT_RENDEZVOUS_ADDR` (default `0.0.0.0:7950`), `GAUGEWRIGHT_READY_FILE`
//! (default `/tmp/rendezvous-ready`, touched once bound so the compose healthcheck
//! passes).

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr =
        std::env::var("GAUGEWRIGHT_RENDEZVOUS_ADDR").unwrap_or_else(|_| "0.0.0.0:7950".into());
    let ready =
        std::env::var("GAUGEWRIGHT_READY_FILE").unwrap_or_else(|_| "/tmp/rendezvous-ready".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let _ = std::fs::File::create(&ready);
    eprintln!("[rendezvous] broker listening on {addr}");
    gaugewright_app::fed_harness::broker_accept_loop(listener).await
}

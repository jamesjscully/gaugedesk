//! The Tier-1 harness **target authority** binary (node-b, `COMPOSE-HARNESS-1`).
//! For each session token the driver will use, it dials *out* to the rendezvous,
//! receives a signed envelope, admits it through the verified federated-delivery
//! reducer into its own store (`INV-21`/`INV-13`/`INV-14`), and writes its verdict
//! back through the broker. It has no inbound route from the source — both dial out.
//!
//! Env: `GAUGEWRIGHT_RENDEZVOUS_ADDR` (the broker `host:port`), `GAUGEWRIGHT_TARGET_SCOPE`
//! (default `scope-B`). The session list must match the driver's; it is the fixed
//! Tier-1 suite, derived from `fed_harness::scenario_suite`.

use gaugewright_app::fed_harness::{scenario_suite, target_serve, SourceClient};
use gaugewright_app::key_store::LoopbackKeyStore;
use gaugewright_store::Store;

#[tokio::main]
async fn main() {
    let broker = std::env::var("GAUGEWRIGHT_RENDEZVOUS_ADDR")
        .expect("GAUGEWRIGHT_RENDEZVOUS_ADDR must name the rendezvous broker host:port");
    let broker_addr = resolve(&broker);
    let target_scope =
        std::env::var("GAUGEWRIGHT_TARGET_SCOPE").unwrap_or_else(|_| "scope-B".into());

    // Derive the exact session list the driver uses (single source of truth). The
    // source client here is only used to enumerate the suite's sessions; the target
    // admits with its own store + key.
    let probe = SourceClient::new(broker_addr, LoopbackKeyStore);
    let sessions: Vec<String> = scenario_suite(&probe)
        .iter()
        .map(|s| s.session.clone())
        .collect();

    let mut store = Store::open_in_memory().expect("open target store");
    eprintln!(
        "[target] serving {} session(s) into {target_scope} via broker {broker}",
        sessions.len()
    );
    match target_serve(broker_addr, &mut store, &target_scope, &sessions).await {
        Ok(verdicts) => {
            let admitted = verdicts.iter().filter(|v| v.admitted).count();
            eprintln!(
                "[target] done: {admitted}/{} crossing(s) admitted into {target_scope}",
                verdicts.len()
            );
        }
        Err(e) => {
            eprintln!("[target] transport error: {e}");
            std::process::exit(1);
        }
    }
}

/// Resolve `host:port` to a `SocketAddr` (DNS for the compose service name).
fn resolve(hostport: &str) -> std::net::SocketAddr {
    use std::net::ToSocketAddrs;
    hostport
        .to_socket_addrs()
        .unwrap_or_else(|e| panic!("resolve {hostport}: {e}"))
        .next()
        .unwrap_or_else(|| panic!("no address for {hostport}"))
}

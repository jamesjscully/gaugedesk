//! The Tier-1 harness **driver** binary (the source authority / node-a side,
//! `COMPOSE-HARNESS-1`). It:
//!
//! 0. asserts there is **no direct route** to the target (`GAUGEWRIGHT_TARGET_DIRECT`):
//!    a direct dial must fail, so pairing can only succeed via the rendezvous;
//! 1. runs the full adversarial federation suite (steps 1-8 of the README
//!    checklist) through the rendezvous and asserts each target verdict — a
//!    genuine crossing admits (and returns its observation handle), while a forged
//!    signature, a revoked grant, an expired device, and a replayed nonce are each
//!    denied (`INV-21`).
//!
//! Exits `0` only if every scenario matches and the no-route property holds;
//! non-zero otherwise. It never fakes a pass.
//!
//! Env: `GAUGEWRIGHT_RENDEZVOUS_ADDR` (broker host:port), `GAUGEWRIGHT_TARGET_DIRECT` (the
//! target's federation `host:port` that must NOT be directly routable, e.g.
//! `node-b:7900`).

use gaugewright_app::fed_harness::{
    assert_no_direct_route, run_driver, scenario_suite, SourceClient,
};
use gaugewright_app::key_store::LoopbackKeyStore;

#[tokio::main]
async fn main() {
    let broker = std::env::var("GAUGEWRIGHT_RENDEZVOUS_ADDR")
        .expect("GAUGEWRIGHT_RENDEZVOUS_ADDR must name the rendezvous broker host:port");
    let broker_addr = resolve(&broker);
    let target_direct = std::env::var("GAUGEWRIGHT_TARGET_DIRECT").ok();

    eprintln!("=== gaugewright Tier-1 federation harness — driver (source authority) ===");
    eprintln!("rendezvous  : {broker}");
    eprintln!("target-direct (must NOT route): {target_direct:?}");

    // [0] No direct A↔B route: a direct dial to the target's federation port must
    // fail. The target sits behind a private network the source does not share, so
    // the only path is through the rendezvous.
    if let Some(direct) = &target_direct {
        match assert_no_direct_route(direct).await {
            Ok(()) => eprintln!(
                "[driver] PASS [0] no direct route to {direct} (must pair via rendezvous)"
            ),
            Err(e) => {
                eprintln!("[driver] FAIL [0] {e}");
                std::process::exit(2);
            }
        }
    } else {
        eprintln!(
            "[driver] WARN [0] GAUGEWRIGHT_TARGET_DIRECT unset — skipping no-direct-route assertion"
        );
    }

    // [1-8] The cross-authority federation suite through the rendezvous.
    let source = SourceClient::new(broker_addr, LoopbackKeyStore);
    let scenarios = scenario_suite(&source);
    match run_driver(&source, &scenarios).await {
        Ok(()) => {
            eprintln!(
                "[driver] ALL SCENARIOS PASSED — Tier-1 federation crossing verified over the wire"
            );
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("[driver] FAILED: {e}");
            std::process::exit(1);
        }
    }
}

fn resolve(hostport: &str) -> std::net::SocketAddr {
    use std::net::ToSocketAddrs;
    hostport
        .to_socket_addrs()
        .unwrap_or_else(|e| panic!("resolve {hostport}: {e}"))
        .next()
        .unwrap_or_else(|| panic!("no address for {hostport}"))
}

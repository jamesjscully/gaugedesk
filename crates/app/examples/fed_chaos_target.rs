//! The **chaos target** (`RF-C6`): node-b for the Byzantine-relay lane. It serves
//! the single chaos session, treating a mangled/truncated frame from the
//! tampering relay as a *denied* crossing (never a crash, never an admit), and
//! exits non-zero if the relay somehow caused an admit or wrote a fact.
//!
//! Env: `GAUGEWRIGHT_RENDEZVOUS_ADDR` (the chaos broker host:port), `GAUGEWRIGHT_TARGET_SCOPE`
//! (default `scope-B`).

use gaugewright_app::fed_harness::chaos_target_once;
use gaugewright_store::Store;

#[tokio::main]
async fn main() {
    let broker = std::env::var("GAUGEWRIGHT_RENDEZVOUS_ADDR")
        .expect("GAUGEWRIGHT_RENDEZVOUS_ADDR must name the chaos rendezvous host:port");
    let broker_addr = resolve(&broker);
    let target_scope =
        std::env::var("GAUGEWRIGHT_TARGET_SCOPE").unwrap_or_else(|_| "scope-B".into());

    let mut store = Store::open_in_memory().expect("open target store");
    match chaos_target_once(broker_addr, &mut store, &target_scope).await {
        Ok((admitted, fact_written)) => {
            if admitted || fact_written {
                eprintln!(
                    "[chaos-target] FAIL: admitted={admitted} fact_written={fact_written} \
                     — a Byzantine relay must not produce either"
                );
                std::process::exit(1);
            }
            eprintln!("[chaos-target] PASS: tampered crossing denied, no fact written");
        }
        Err(e) => {
            eprintln!("[chaos-target] transport error: {e}");
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

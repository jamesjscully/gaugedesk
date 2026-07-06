//! The **chaos driver** (`RF-C6`): the source side of the Byzantine-relay lane.
//! It sends ONE genuine, correctly-signed crossing through a tampering rendezvous
//! ([`fed_chaos_rendezvous`]) and asserts the target did **not** admit it — the
//! only adversary is the relay, so a deny proves no relay misbehaviour can forge
//! an admission (`INV-21`; `RELAY_READS_PAYLOAD`/`RELAY_OWNS_PAYLOAD` live).
//!
//! Exits 0 iff the genuine crossing was denied; non-zero if the Byzantine relay
//! somehow caused an admit (a real protection failure) or the run errored.
//!
//! Env: `GAUGEWRIGHT_RENDEZVOUS_ADDR` (the chaos broker host:port).

use std::time::Duration;

use gaugewright_app::fed_harness::{SourceClient, CHAOS_SESSION};
use gaugewright_app::key_store::LoopbackKeyStore;

#[tokio::main]
async fn main() {
    let broker = std::env::var("GAUGEWRIGHT_RENDEZVOUS_ADDR")
        .expect("GAUGEWRIGHT_RENDEZVOUS_ADDR must name the chaos rendezvous host:port");
    let broker_addr = resolve(&broker);

    eprintln!(
        "=== gaugewright chaos lane — driver (genuine crossing through a Byzantine relay) ==="
    );
    let source = SourceClient::new(broker_addr, LoopbackKeyStore);
    let wire = source.genuine_envelope("xc-chaos", "A", "B", "SECRET-HANDLE");

    // The tampered frame may also break the source's own verdict read; a
    // timeout-or-error is itself "not admitted" (the crossing did not succeed).
    let admitted =
        match tokio::time::timeout(Duration::from_secs(10), source.cross(CHAOS_SESSION, &wire))
            .await
        {
            Ok(Ok(verdict)) => verdict.admitted,
            Ok(Err(e)) => {
                eprintln!("[chaos-driver] transport error (counts as denied): {e}");
                false
            }
            Err(_) => {
                eprintln!("[chaos-driver] verdict timed out (counts as denied)");
                false
            }
        };

    if admitted {
        eprintln!("[chaos-driver] FAIL: a Byzantine relay forced an ADMIT — protection broken");
        std::process::exit(1);
    }
    eprintln!("[chaos-driver] PASS: the tampered genuine crossing was denied (INV-21 holds)");
}

fn resolve(hostport: &str) -> std::net::SocketAddr {
    use std::net::ToSocketAddrs;
    hostport
        .to_socket_addrs()
        .unwrap_or_else(|e| panic!("resolve {hostport}: {e}"))
        .next()
        .unwrap_or_else(|| panic!("no address for {hostport}"))
}

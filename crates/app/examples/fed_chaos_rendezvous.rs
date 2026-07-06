//! A **Byzantine** rendezvous broker binary (`RF-C6`): the same multi-session
//! rendezvous as `fed_rendezvous`, but it actively tampers with the source→target
//! leg (corrupt / truncate / duplicate) instead of splicing it verbatim. It is the
//! adversarial relay the chaos compose lane runs in place of the honest broker, to
//! prove over the wire that no relay misbehaviour can forge a target admission: the
//! target verifies the source signature, so a mutated envelope is denied
//! (`INV-21`; the `RELAY_READS_PAYLOAD`/`RELAY_OWNS_PAYLOAD` teeth, live).
//!
//! Env: `GAUGEWRIGHT_RENDEZVOUS_ADDR` (default `0.0.0.0:7950`), `GAUGEWRIGHT_READY_FILE`
//! (default `/tmp/rendezvous-ready`), `GAUGEWRIGHT_CHAOS` (`corrupt` | `truncate` |
//! `duplicate`, default `corrupt`).

use gaugewright_app::fed_harness::{broker_accept_loop_chaos, ChaosPolicy};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr =
        std::env::var("GAUGEWRIGHT_RENDEZVOUS_ADDR").unwrap_or_else(|_| "0.0.0.0:7950".into());
    let ready =
        std::env::var("GAUGEWRIGHT_READY_FILE").unwrap_or_else(|_| "/tmp/rendezvous-ready".into());
    let policy = match std::env::var("GAUGEWRIGHT_CHAOS").as_deref() {
        Ok("truncate") => ChaosPolicy::TruncateForward,
        Ok("duplicate") => ChaosPolicy::DuplicateForward,
        _ => ChaosPolicy::CorruptForward,
    };

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let _ = std::fs::File::create(&ready);
    eprintln!("[chaos-rendezvous] Byzantine broker on {addr} (policy={policy:?})");
    broker_accept_loop_chaos(listener, policy).await
}

//! Tier-1 multi-node federation harness (`COMPOSE-HARNESS-1`): the cross-machine
//! wiring that turns the loopback [`net_relay`](crate::net_relay) /
//! [`net_server`](crate::net_server) seams into a real, NAT-isolated container
//! flow (ADR 0042, `specs/implementation/test-and-release-infra.md`).
//!
//! Three roles share this module, one per harness container:
//!
//! 1. [`broker_accept_loop`] — the **rendezvous** broker loop: the multi-session
//!    loop around the same dumb byte-splice
//!    [`RendezvousBroker`](crate::net_relay::RendezvousBroker) proves over
//!    loopback. It pairs the two legs that name the same session token and
//!    [`copy_bidirectional`](tokio::io::copy_bidirectional)-splices them — it never
//!    parses, terminates, or inspects the carried bytes
//!    (`RELAY_NO_PAYLOAD_ACCESS` / `INV-10`). It is the only A↔B path.
//!
//! 2. [`target_serve`] — a **target authority** (node-b): for each session it
//!    dials *out* to the broker, receives a signed [`EnvelopeWire`], admits it
//!    through the same verified [`gaugewright_core::federated_delivery`] reducer the
//!    loopback relay drives (`INV-21`), and writes its **verdict** back through the
//!    broker. The fact is created only by the target's own admission
//!    (`INV-13`/`INV-14`).
//!
//! 3. [`SourceClient`] — a **source authority** (the driver / node-a): for each
//!    scenario it dials *out* to the broker, sends an envelope (genuine or
//!    adversarial), and reads the target's verdict back through the broker — the
//!    only channel it has to the target. It also proves there is **no direct
//!    route** to the target ([`assert_no_direct_route`]).
//!
//! The wire is identical to [`net_relay`](crate::net_relay): a fixed-width session
//! token (opaque routing metadata) followed by length-prefixed JSON frames. The
//! broker forwards both legs verbatim; the verdict travels back the same opaque
//! pipe, so even the admit/deny outcome crosses *only* through the rendezvous.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::timeout;

use gaugewright_core::ids::{AuthorityId, PublicKey};
use gaugewright_core::signature::Signature;
use gaugewright_store::Store;

use crate::key_store::KeyStore;
use crate::net_relay::EnvelopeWire;

/// The fixed width of a rendezvous session token on the wire — the same opaque
/// routing width [`net_relay`](crate::net_relay) uses. The broker reads exactly
/// this many bytes from each leg to learn *which pairing* it belongs to.
const TOKEN_LEN: usize = 32;

/// The bridge grant + device key a genuine default crossing is bound to (the same
/// fixed binding the loopback path presents). The reducer's teeth reject anything
/// else, so these are what a real crossing must carry.
pub const BOUND_GRANT: &str = "bridge-grant-7";
/// The bound device key the default delivery pins (`MOB-004`).
pub fn bound_device() -> PublicKey {
    PublicKey::new("04dev1ce0ke7")
}

/// The target's verdict on one received envelope, sent back through the broker so
/// the source learns the outcome over the only channel it has. The broker forwards
/// these bytes verbatim — it never decodes the verdict any more than the envelope.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Verdict {
    /// The correlation id the verdict answers (echoed so the source can match it).
    pub correlation: String,
    /// Whether the target admitted the crossing through the verified reducer.
    pub admitted: bool,
    /// The payload handle the target decoded — present only on an admitted
    /// crossing (it is the handle that crossed, never the payload, `INV-10`).
    pub admitted_handle: Option<String>,
}

/// A transport/protocol error in the harness (distinct from a *denied* crossing,
/// which is a [`Verdict`] with `admitted: false`).
#[derive(Debug)]
pub enum HarnessError {
    Io(std::io::Error),
    Protocol(String),
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessError::Io(e) => write!(f, "harness io: {e}"),
            HarnessError::Protocol(s) => write!(f, "harness protocol: {s}"),
        }
    }
}

impl std::error::Error for HarnessError {}

impl From<std::io::Error> for HarnessError {
    fn from(e: std::io::Error) -> Self {
        HarnessError::Io(e)
    }
}

// --- wire framing (identical to net_relay's private helpers) -----------------

fn token_bytes(token: &str) -> [u8; TOKEN_LEN] {
    let mut buf = [0u8; TOKEN_LEN];
    let src = token.as_bytes();
    let n = src.len().min(TOKEN_LEN);
    buf[..n].copy_from_slice(&src[..n]);
    buf
}

async fn write_frame<S>(stream: &mut S, bytes: &[u8]) -> std::io::Result<()>
where
    S: AsyncWriteExt + Unpin,
{
    let len = u32::try_from(bytes.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "frame too large"))?;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(bytes).await?;
    stream.flush().await
}

async fn read_frame<S>(stream: &mut S) -> std::io::Result<Vec<u8>>
where
    S: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn read_token(stream: &mut TcpStream) -> std::io::Result<[u8; TOKEN_LEN]> {
    let mut buf = [0u8; TOKEN_LEN];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

// --- 1. the rendezvous broker (multi-session) --------------------------------

/// Accept connections on `listener` forever, pairing the two legs that name the
/// same fixed-width session token (opaque routing metadata, never payload) and
/// splicing them with [`copy_bidirectional`](tokio::io::copy_bidirectional). The
/// broker never parses, terminates, or inspects the carried bytes
/// (`RELAY_NO_PAYLOAD_ACCESS` / `INV-10`) — it learns only a byte count per
/// direction. The first leg to name a token is parked; its partner splices it.
pub async fn broker_accept_loop(listener: TcpListener) -> std::io::Result<()> {
    broker_accept_loop_bounded(listener, MAX_INFLIGHT_LEGS, TOKEN_READ_TIMEOUT).await
}

/// Cap on connection-handling tasks the broker keeps in flight at once (RF-A6):
/// a connection flood cannot spawn unbounded tasks or park unbounded sockets —
/// once the cap is reached, new accepts wait for a slot to free.
const MAX_INFLIGHT_LEGS: usize = 1024;
/// How long a connecting leg has to announce its session token before the broker
/// drops it (RF-A6): a leg that connects and goes silent must not hold a slot or
/// a parked socket forever.
const TOKEN_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The bounded accept loop behind [`broker_accept_loop`]: at most `max_inflight`
/// connection-handling tasks run concurrently, and each leg must announce its
/// token within `token_timeout` or be dropped. The splice itself stays unbounded
/// (a paired session lives as long as both sides hold it open).
pub async fn broker_accept_loop_bounded(
    listener: TcpListener,
    max_inflight: usize,
    token_timeout: Duration,
) -> std::io::Result<()> {
    let waiting: Arc<Mutex<HashMap<[u8; TOKEN_LEN], TcpStream>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let permits = Arc::new(tokio::sync::Semaphore::new(max_inflight));
    loop {
        let (mut sock, _peer) = listener.accept().await?;
        // Bound concurrency: acquire a slot before spawning. `acquire_owned`
        // never errors here (the semaphore is never closed); on the unreachable
        // closed case we simply drop the connection.
        let Ok(permit) = Arc::clone(&permits).acquire_owned().await else {
            continue;
        };
        let waiting = Arc::clone(&waiting);
        tokio::spawn(async move {
            let _permit = permit; // held for the lifetime of this leg's handling
            let token = match timeout(token_timeout, read_token(&mut sock)).await {
                Ok(Ok(t)) => t,
                _ => return, // silent or errored leg — drop it, free the slot
            };
            let partner = { waiting.lock().await.remove(&token) };
            match partner {
                None => {
                    // Park this leg for its partner — but bound the parked set so
                    // a flood of legs that never get a partner cannot grow the map
                    // (and its held fds) without limit. At capacity, drop this leg.
                    let mut w = waiting.lock().await;
                    if w.len() < max_inflight {
                        w.insert(token, sock);
                    }
                }
                Some(mut other) => {
                    // Transparent splice: the broker terminates neither stream and
                    // parses none of the bytes — it only pipes and counts.
                    let _ = tokio::io::copy_bidirectional(&mut sock, &mut other).await;
                }
            }
        });
    }
}

/// A **Byzantine** rendezvous policy (`RF-C6`): a relay that does not behave as a
/// transparent splice but actively tampers with the bytes it carries. The point
/// is to demonstrate that no relay misbehaviour can forge a target admission —
/// the target verifies the source's signature over the canonical bytes, so any
/// mutation either fails to decode or fails the signature, and the crossing is
/// denied (`INV-21`/`RELAY_READS_PAYLOAD`/`RELAY_OWNS_PAYLOAD` teeth, live).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChaosPolicy {
    /// Flip bytes in the source→target leg (corrupt the envelope in flight).
    CorruptForward,
    /// Truncate the source→target leg (drop the tail of the frame).
    TruncateForward,
    /// Replay the source→target bytes a second time after the first copy.
    DuplicateForward,
}

/// A Byzantine broker accept loop: like [`broker_accept_loop`], but the splice
/// applies `policy` to the source→target direction instead of copying verbatim.
/// The reverse (verdict) direction is passed through so the source still learns
/// the outcome. Used by the chaos example binary and the in-crate Byzantine test.
pub async fn broker_accept_loop_chaos(
    listener: TcpListener,
    policy: ChaosPolicy,
) -> std::io::Result<()> {
    let waiting: Arc<Mutex<HashMap<[u8; TOKEN_LEN], TcpStream>>> =
        Arc::new(Mutex::new(HashMap::new()));
    loop {
        let (mut sock, _peer) = listener.accept().await?;
        let waiting = Arc::clone(&waiting);
        tokio::spawn(async move {
            let token = match read_token(&mut sock).await {
                Ok(t) => t,
                Err(_) => return,
            };
            let partner = { waiting.lock().await.remove(&token) };
            match partner {
                None => {
                    waiting.lock().await.insert(token, sock);
                }
                // `sock` is the second leg to arrive (the target, which dials out
                // after the source has parked); `other` is the parked source leg.
                Some(other) => {
                    let _ = chaos_splice(other, sock, policy).await;
                }
            }
        });
    }
}

/// Splice `source`→`target` under `policy` (tampering), and `target`→`source`
/// verbatim (the verdict path). Never panics; a broken pipe just ends the splice.
async fn chaos_splice(
    source: TcpStream,
    target: TcpStream,
    policy: ChaosPolicy,
) -> std::io::Result<()> {
    let (mut sr, mut sw) = source.into_split();
    let (mut tr, mut tw) = target.into_split();

    let forward = async move {
        let mut buf = vec![0u8; 8192];
        let mut sent_once = false;
        loop {
            let n = sr.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            match policy {
                ChaosPolicy::CorruptForward => {
                    // Flip a byte in the payload region (past the length prefix).
                    let mut chunk = buf[..n].to_vec();
                    let i = if n > 6 { 6 } else { n - 1 };
                    chunk[i] ^= 0xFF;
                    tw.write_all(&chunk).await?;
                }
                ChaosPolicy::TruncateForward => {
                    // Forward only the first half, then stop (drop the tail).
                    tw.write_all(&buf[..n / 2]).await?;
                    break;
                }
                ChaosPolicy::DuplicateForward => {
                    tw.write_all(&buf[..n]).await?;
                    if !sent_once {
                        tw.write_all(&buf[..n]).await?;
                        sent_once = true;
                    }
                }
            }
        }
        let _ = tw.shutdown().await;
        Ok::<(), std::io::Error>(())
    };
    let backward = async move {
        let _ = tokio::io::copy(&mut tr, &mut sw).await;
        let _ = sw.shutdown().await;
    };
    let (f, _) = tokio::join!(forward, backward);
    f
}

// --- 2. the target authority -------------------------------------------------

/// Admit one received [`EnvelopeWire`] through the verified federated-delivery
/// reducer and return the [`Verdict`]. This is the cross-machine twin of the
/// loopback `admit_received`: the same source-authorize → relay-queue →
/// relay-deliver → **target-admit** sequence, with the security teeth (P-256
/// signature, bridge grant, nonce, device binding) borne entirely by the target's
/// own admission (`INV-21`). Only an admitted crossing writes a fact, and only the
/// target writes it (`INV-13`/`INV-14`).
fn admit_one(store: &mut Store, target_scope: &str, wire: &EnvelopeWire) -> Verdict {
    use gaugewright_core::federated_delivery::{DeliveryCommand, DeliveryPhase, DeliveryState};

    let ds = crate::federation_relay::delivery_scope(&wire.correlation);
    // Transport-only steps. A reducer error here is a denied crossing, reported
    // as a non-admit verdict rather than a panic — the harness never fakes a pass.
    for cmd in [
        DeliveryCommand::AuthorizeFederatedMessage,
        DeliveryCommand::EnqueueFederatedMessage,
        DeliveryCommand::RecordRelayDelivery,
    ] {
        if store.admit::<DeliveryState>(&ds, cmd).is_err() {
            return Verdict {
                correlation: wire.correlation.clone(),
                admitted: false,
                admitted_handle: None,
            };
        }
    }

    let envelope = wire.to_delivery_envelope();
    let admitted = matches!(
        store.admit::<DeliveryState>(&ds, DeliveryCommand::AdmitTargetReceipt { envelope }),
        Ok(s) if s.phase == DeliveryPhase::TargetAdmitted
    );
    let admitted_handle = if admitted {
        let rec = serde_json::json!({
            "correlation": wire.correlation,
            "source": wire.source,
            "target": wire.target,
            "payload_handle": wire.payload_handle, // a handle — never the payload
        });
        let _ = store.append_record(target_scope, "federated", &rec.to_string());
        Some(wire.payload_handle.clone())
    } else {
        None
    };
    Verdict {
        correlation: wire.correlation.clone(),
        admitted,
        admitted_handle,
    }
}

/// **Target authority** loop (node-b): for each of `sessions`, dial the broker on
/// that session token, receive the source's signed [`EnvelopeWire`], admit it into
/// `target_scope`, and write the [`Verdict`] back through the broker. The target
/// dials *out* — it has no inbound route from the source, exactly as the source has
/// none to it. Each session is one crossing; the verdict crosses back over the same
/// opaque pipe.
pub async fn target_serve(
    broker: SocketAddr,
    store: &mut Store,
    target_scope: &str,
    sessions: &[String],
) -> Result<Vec<Verdict>, HarnessError> {
    let mut verdicts = Vec::with_capacity(sessions.len());
    for session in sessions {
        let mut stream = connect_with_retry(broker).await?;
        stream.write_all(&token_bytes(session)).await?;

        let bytes = read_frame(&mut stream).await?;
        let wire: EnvelopeWire = serde_json::from_slice(&bytes)
            .map_err(|e| HarnessError::Protocol(format!("decode envelope: {e}")))?;

        let verdict = admit_one(store, target_scope, &wire);
        let vbytes = serde_json::to_vec(&verdict)
            .map_err(|e| HarnessError::Protocol(format!("encode verdict: {e}")))?;
        write_frame(&mut stream, &vbytes).await?;
        // Half-close so the broker's splice sees EOF and completes.
        let _ = stream.shutdown().await;
        eprintln!(
            "[target] session {session}: correlation={} admitted={}",
            verdict.correlation, verdict.admitted
        );
        verdicts.push(verdict);
    }
    Ok(verdicts)
}

/// The single session the chaos (`RF-C6`) lane uses: one *genuine*, correctly
/// signed crossing, so the only adversary is the Byzantine relay itself.
pub const CHAOS_SESSION: &str = "sess-chaos-genuine";

/// Target side of the chaos lane: serve exactly the chaos session, but treat a
/// mangled frame (a relay that corrupted or truncated the envelope) as a
/// **denied** crossing rather than a transport error — a Byzantine relay must
/// produce a deny, never a crash and never an admit. Returns whether the target
/// admitted (it must not) and whether any fact was written (it must not be).
pub async fn chaos_target_once(
    broker: SocketAddr,
    store: &mut Store,
    target_scope: &str,
) -> std::io::Result<(bool, bool)> {
    let mut stream = connect_with_retry(broker).await?;
    stream.write_all(&token_bytes(CHAOS_SESSION)).await?;

    // A corrupt/truncated frame fails to read or decode; that IS the deny.
    let admitted = match read_frame(&mut stream).await {
        Ok(bytes) => match serde_json::from_slice::<EnvelopeWire>(&bytes) {
            Ok(wire) => {
                let v = admit_one(store, target_scope, &wire);
                // Send a verdict back so the source's read completes.
                if let Ok(vbytes) = serde_json::to_vec(&v) {
                    let _ = write_frame(&mut stream, &vbytes).await;
                }
                v.admitted
            }
            Err(_) => false, // decode failed → denied
        },
        Err(_) => false, // truncated/corrupt frame → denied
    };
    let _ = stream.shutdown().await;
    let fact_written = crate::federation_relay::admitted(store, target_scope)
        .map(|f| !f.is_empty())
        .unwrap_or(false);
    Ok((admitted, fact_written))
}

// --- 3. the source authority -------------------------------------------------

/// **Source authority** client (the driver / node-a): dials the broker for each
/// scenario, sends an [`EnvelopeWire`] (genuine or adversarial), and reads the
/// target's [`Verdict`] back through the broker — its only channel to the target.
pub struct SourceClient<K: KeyStore> {
    broker: SocketAddr,
    key_store: K,
}

impl<K: KeyStore> SourceClient<K> {
    pub fn new(broker: SocketAddr, key_store: K) -> Self {
        Self { broker, key_store }
    }

    /// A genuine source envelope for `correlation`: signed under the source
    /// authority's real P-256 governance key from the [`KeyStore`], bound to the
    /// default bridge grant + bound device — exactly what a real crossing presents.
    pub fn genuine_envelope(
        &self,
        correlation: &str,
        source: &str,
        target: &str,
        payload_handle: &str,
    ) -> EnvelopeWire {
        let key = self.key_store.signing_key(&AuthorityId::new(source));
        let signed_bytes = correlation.as_bytes().to_vec();
        EnvelopeWire {
            correlation: correlation.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            payload_handle: payload_handle.to_string(),
            signature: key.sign(&signed_bytes),
            source_pubkey: key.public_key(),
            signed_bytes,
            nonce: format!("nonce::{correlation}"),
            bridge_grant_id: BOUND_GRANT.to_string(),
            device_key: bound_device(),
            device_active: true,
        }
    }

    /// Send `wire` to the target on `session` through the broker and return the
    /// target's verdict (received back through the same broker pipe).
    pub async fn cross(&self, session: &str, wire: &EnvelopeWire) -> Result<Verdict, HarnessError> {
        let bytes = serde_json::to_vec(wire)
            .map_err(|e| HarnessError::Protocol(format!("encode envelope: {e}")))?;
        let mut stream = connect_with_retry(self.broker).await?;
        stream.write_all(&token_bytes(session)).await?;
        write_frame(&mut stream, &bytes).await?;

        let vbytes = read_frame(&mut stream).await?;
        let _ = stream.shutdown().await;
        serde_json::from_slice(&vbytes)
            .map_err(|e| HarnessError::Protocol(format!("decode verdict: {e}")))
    }
}

/// Assert there is **no direct route** from the source to `target_addr`: a direct
/// TCP dial must not connect (the target sits behind a private network the source
/// does not share, so pairing can only succeed via the rendezvous). Returns `Ok(())`
/// when the dial fails as required, or an error if it unexpectedly connected.
pub async fn assert_no_direct_route(target_addr: &str) -> Result<(), String> {
    // A short timeout: an unroutable host neither connects nor RSTs promptly, so a
    // genuine no-route shows as a timeout; a shared route would connect fast.
    match tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(target_addr)).await {
        Ok(Ok(_)) => Err(format!(
            "a direct dial to {target_addr} SUCCEEDED — the source has a route to the \
             target outside the rendezvous (network isolation broken)"
        )),
        Ok(Err(_)) => Ok(()), // connection refused / unreachable — no direct route
        Err(_) => Ok(()),     // timed out — no route to the isolated host
    }
}

/// Dial the broker, retrying briefly: in the container harness the broker and the
/// peers come up concurrently, so the first dials may race the listener. A bounded
/// retry makes the harness robust without masking a genuinely-down broker.
async fn connect_with_retry(addr: SocketAddr) -> std::io::Result<TcpStream> {
    let mut last = None;
    for _ in 0..50 {
        match TcpStream::connect(addr).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("broker unreachable")))
}

// --- the scenario suite (shared by the driver binary and the in-crate test) --

/// One driver scenario: a session token, the envelope to send (after a mutation),
/// and whether the target is expected to admit it.
pub struct Scenario {
    pub name: &'static str,
    pub session: String,
    pub wire: EnvelopeWire,
    pub expect_admit: bool,
}

/// Build the full adversarial scenario suite a genuine source presents, mirroring
/// the Tier-1 checklist in `docker/README.md` / `test-and-release-infra.md`:
/// a genuine crossing admits; a forged signature, a mismatched bridge grant, an
/// expired/foreign device, and a replayed nonce are each denied (`INV-21`).
///
/// The replay case reuses the genuine envelope under a *second* session so the
/// target — folding the same delivery scope — sees the nonce already spent.
pub fn scenario_suite<K: KeyStore>(source: &SourceClient<K>) -> Vec<Scenario> {
    let mk = |corr: &str, handle: &str| source.genuine_envelope(corr, "A", "B", handle);

    // [2-5] a genuine crossing: signed, bound grant, bound device — the target
    // admits and the handle (an observation locator) returns.
    let genuine = Scenario {
        name: "genuine crossing admits (sig · grant · device · observation returns)",
        session: "sess-genuine".into(),
        wire: mk("xc-genuine", "ctx-method-OBSERVATION-HANDLE"),
        expect_admit: true,
    };

    // [3] a forged (malformed, non-P-256) signature is denied.
    let mut forged_w = mk("xc-forged", "ctx-method");
    forged_w.signature = Signature::new(vec![0u8; 8]);
    let forged = Scenario {
        name: "forged signature denied (INV-21, real P-256)",
        session: "sess-forged".into(),
        wire: forged_w,
        expect_admit: false,
    };

    // [6] a revoked grant blocks delivery (REVOCATION-DIST-1): an envelope minted
    // under a different bridge grant is denied.
    let mut revoked_w = mk("xc-revoked", "ctx-method");
    revoked_w.bridge_grant_id = "bridge-grant-REVOKED".into();
    let revoked = Scenario {
        name: "revoked grant blocks delivery (REVOCATION-DIST-1)",
        session: "sess-revoked".into(),
        wire: revoked_w,
        expect_admit: false,
    };

    // [7] an expired / revoked device grant is rejected (MOB-004): a foreign or
    // inactive device binding denies admission — a revoked device cannot deliver.
    let mut expired_w = mk("xc-expired", "ctx-method");
    expired_w.device_active = false;
    let expired = Scenario {
        name: "expired/revoked device grant rejected (MOB-004)",
        session: "sess-expired".into(),
        wire: expired_w,
        expect_admit: false,
    };

    // [8] a reused nonce is rejected (anti-replay, INV-21): the same genuine
    // envelope re-presented to the same delivery (its nonce already spent) is
    // denied. It crosses the broker fine (transport is blind) but the target's
    // reducer fails closed.
    let replay = Scenario {
        name: "reused nonce rejected (anti-replay, INV-21)",
        session: "sess-replay".into(),
        wire: genuine.wire.clone(),
        expect_admit: false,
    };

    vec![genuine, forged, revoked, expired, replay]
}

/// Drive the full source-side flow against a live broker + target: run every
/// scenario, compare each verdict to its expectation, and return `Ok(())` only if
/// **all** match. The caller (the driver binary) turns this into the process exit
/// code; the same routine drives the in-crate integration test over loopback.
pub async fn run_driver<K: KeyStore>(
    source: &SourceClient<K>,
    scenarios: &[Scenario],
) -> Result<(), String> {
    let mut all_ok = true;
    for sc in scenarios {
        let verdict = source
            .cross(&sc.session, &sc.wire)
            .await
            .map_err(|e| format!("[{}] transport error: {e}", sc.name))?;
        let ok = verdict.admitted == sc.expect_admit;
        all_ok &= ok;
        eprintln!(
            "[driver] {} {} (admitted={}, expected={})",
            if ok { "PASS" } else { "FAIL" },
            sc.name,
            verdict.admitted,
            sc.expect_admit,
        );
        if sc.expect_admit && ok {
            // An admitted crossing must return the observation handle — never the
            // payload (INV-10).
            match &verdict.admitted_handle {
                Some(h) => eprintln!("[driver]      observation handle returned: {h}"),
                None => {
                    all_ok = false;
                    eprintln!(
                        "[driver] FAIL expected an observation handle on the admitted crossing"
                    );
                }
            }
        }
    }
    if all_ok {
        Ok(())
    } else {
        Err("one or more federation scenarios did not match expectations".into())
    }
}

/// Coordinate one full harness run in a single process over loopback (the in-crate
/// integration test): a broker, a target serving the suite's sessions, and the
/// source driver — all three concurrently, meeting only at the broker. Returns the
/// driver verdict and the target's per-session verdicts.
///
/// This is the cross-machine container flow collapsed onto loopback, so the exact
/// binaries the compose harness runs are covered by `cargo test`.
#[cfg(test)]
async fn run_loopback_once<K: KeyStore + Clone + Send + 'static>(
    key_store: K,
) -> (Result<(), String>, Vec<Verdict>) {
    // Bind the broker on an ephemeral loopback port and serve the accept loop in a
    // background task (the same loop the rendezvous container runs).
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let broker_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = broker_accept_loop(listener).await;
    });

    let source = SourceClient::new(broker_addr, key_store.clone());
    let scenarios = scenario_suite(&source);
    let sessions: Vec<String> = scenarios.iter().map(|s| s.session.clone()).collect();

    // The target serves the sessions concurrently with the driver; they meet at the
    // broker. A oneshot carries the target's verdicts back out.
    let (tv_tx, tv_rx) = tokio::sync::mpsc::channel(1);
    let sessions_for_target = sessions.clone();
    tokio::spawn(async move {
        let mut store = Store::open_in_memory().unwrap();
        let verdicts = target_serve(broker_addr, &mut store, "scope-B", &sessions_for_target)
            .await
            .unwrap();
        let _ = tv_tx.send(verdicts).await;
    });

    let driver = run_driver(&source, &scenarios).await;
    let mut tv_rx = tv_rx;
    let target_verdicts = tv_rx.recv().await.unwrap();
    (driver, target_verdicts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key_store::LoopbackKeyStore;

    /// The whole Tier-1 harness over loopback: the broker, a target authority, and
    /// the source driver run concurrently and meet only at the broker. The driver's
    /// full adversarial suite passes — a genuine crossing admits (and returns its
    /// observation handle), and a forged signature, a revoked grant, an expired
    /// device, and a replayed nonce are each denied (`INV-21`). This is the exact
    /// flow the compose containers run, collapsed onto one host.
    #[tokio::test]
    async fn the_harness_suite_passes_over_loopback() {
        let (driver, target_verdicts) = run_loopback_once(LoopbackKeyStore).await;
        assert!(driver.is_ok(), "driver suite failed: {driver:?}");

        // Exactly one genuine crossing admitted; the other four were denied.
        let admitted: Vec<_> = target_verdicts.iter().filter(|v| v.admitted).collect();
        assert_eq!(
            admitted.len(),
            1,
            "exactly one genuine crossing admits, the rest are denied"
        );
        assert_eq!(
            admitted[0].admitted_handle.as_deref(),
            Some("ctx-method-OBSERVATION-HANDLE"),
            "the admitted crossing returned the observation handle (INV-10: a handle, not payload)"
        );
    }

    /// RF-C6 — a **Byzantine relay cannot forge an admission.** For each
    /// tampering policy (corrupt / truncate the envelope in flight), a *genuine*
    /// signed crossing carried by the malicious broker is never admitted, and no
    /// fact is written into the target's scope. The target verifies the source's
    /// signature over the canonical bytes, so any mutation fails decode or the
    /// signature — `RELAY_READS_PAYLOAD` / `RELAY_OWNS_PAYLOAD` teeth, live over
    /// the wire rather than only in the Quint model.
    #[tokio::test]
    async fn a_byzantine_relay_cannot_forge_an_admission() {
        for policy in [ChaosPolicy::CorruptForward, ChaosPolicy::TruncateForward] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let broker_addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let _ = broker_accept_loop_chaos(listener, policy).await;
            });

            let source = SourceClient::new(broker_addr, LoopbackKeyStore);
            // A genuine, correctly-signed envelope — the *only* tampering is the
            // relay's. If the relay could forge admission, this would cross.
            let wire = source.genuine_envelope("xc-byz", "A", "B", "SECRET-HANDLE");
            let session = "sess-byz".to_string();

            let sessions = vec![session.clone()];
            let target = {
                let sessions = sessions.clone();
                tokio::spawn(async move {
                    let mut s = Store::open_in_memory().unwrap();
                    let r = target_serve(broker_addr, &mut s, "scope-B", &sessions).await;
                    // Hand back whether any fact was admitted into the scope.
                    let admitted = crate::federation_relay::admitted(&s, "scope-B")
                        .map(|f| !f.is_empty())
                        .unwrap_or(false);
                    (
                        r.map(|v| v.iter().any(|x| x.admitted)).unwrap_or(false),
                        admitted,
                    )
                })
            };
            // The source sends; a tampered or truncated frame may also kill its
            // own read side — that's fine, we assert on the target's outcome.
            let _ = source.cross(&session, &wire).await;
            let (verdict_admitted, fact_written) = target.await.unwrap();
            assert!(
                !verdict_admitted,
                "{policy:?}: a tampered crossing must NOT be admitted"
            );
            assert!(
                !fact_written,
                "{policy:?}: a tampered crossing must write no fact into the target scope"
            );
        }
    }

    /// RF-A6: the bounded broker still pairs correctly under a tight concurrency
    /// cap, and a parked silent leg (one that connects but never announces a
    /// token) does not block fresh pairings — the concurrency bound holds without
    /// the test racing any wall-clock timeout. (The token-timeout *drop* itself is
    /// covered by `net_relay::a_silent_peer_times_the_pairing_out`.) A generous
    /// token timeout ensures a legitimate pairing never loses a race under load.
    #[tokio::test]
    async fn the_bounded_broker_pairs_under_its_cap_despite_a_parked_silent_leg() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Tiny cap (4) to exercise the concurrency bound; a generous token timeout
        // so a legitimate pairing is never dropped under CI load.
        tokio::spawn(async move {
            let _ = broker_accept_loop_bounded(listener, 4, Duration::from_secs(30)).await;
        });
        let source = SourceClient::new(addr, LoopbackKeyStore);

        // A genuine pairing works through the bounded loop.
        let wire = source.genuine_envelope("xc-bounded", "A", "B", "H");
        let mut store = Store::open_in_memory().unwrap();
        let (verdict, _) = tokio::join!(
            source.cross("sess-bounded", &wire),
            chaos_free_target(addr, &mut store, "sess-bounded"),
        );
        assert!(verdict.is_ok(), "the bounded broker must pair legs");

        // A silent leg parks (holds one slot, never sends a token); with cap 4
        // that leaves room, so a fresh pairing still completes — the broker is not
        // wedged by the parked leg. No timeout wait, so no wall-clock race.
        let _silent = TcpStream::connect(addr).await.unwrap();
        let wire2 = source.genuine_envelope("xc-bounded-2", "A", "B", "H");
        let mut store2 = Store::open_in_memory().unwrap();
        let (verdict2, _) = tokio::join!(
            source.cross("sess-bounded-2", &wire2),
            chaos_free_target(addr, &mut store2, "sess-bounded-2"),
        );
        assert!(
            verdict2.is_ok(),
            "the broker stays responsive with a silent leg parked"
        );
    }

    /// A one-session honest target leg for the bounded-broker test (mirrors
    /// `target_serve` for a single session).
    async fn chaos_free_target(broker: SocketAddr, store: &mut Store, session: &str) {
        let _ = target_serve(broker, store, "scope-B", &[session.to_string()]).await;
    }

    /// The no-direct-route assertion holds for an unroutable address: a dial to a
    /// host with no route does not connect, so the source can only reach the target
    /// via the rendezvous. (In the container harness this is `node-b:7900` across
    /// the unshared private network; here we use a documentation/test-net address
    /// that does not route.)
    #[tokio::test]
    async fn no_direct_route_to_an_unroutable_target() {
        // 203.0.113.0/24 (TEST-NET-3, RFC 5737) is reserved for documentation and
        // does not route — a stand-in for the container's isolated peer.
        assert!(
            assert_no_direct_route("203.0.113.1:7900").await.is_ok(),
            "an unroutable target must show as no-direct-route"
        );
    }
}

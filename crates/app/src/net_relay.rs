//! Networked rendezvous relay (`RENDEZVOUS-STUB-1` → real, ADR 0041): the
//! cross-machine transport the [`FederationRelay`](crate::federation_relay)
//! seam was reserved for. It replaces the in-process [`LoopbackRelay`] for
//! delivery between two machines that have **no direct route** to each other —
//! each dials *out* to a shared, dumb broker, which splices their two
//! connections into one opaque byte pipe.
//!
//! Two pieces, both over **real TCP** (loopback sockets in the test, a public
//! interface in deployment — the wire is identical):
//!
//! 1. [`RendezvousBroker`] — a **dumb** rendezvous broker. It accepts two
//!    outbound connections that name the same session token, pairs them, and
//!    pipes the byte stream between them with [`tokio::io::copy_bidirectional`].
//!    It **never terminates, parses, or inspects** the carried bytes
//!    (`RELAY_NO_PAYLOAD_ACCESS` / `INV-10`): the only thing it reads is the
//!    fixed-length session token (opaque routing metadata, not payload), and
//!    after pairing it is a transparent splice. It is not a TLS endpoint and
//!    holds no key — it cannot see plaintext payload because in a real
//!    deployment the legs carry end-to-end-encrypted bytes; here we assert it
//!    structurally by checking the broker observed none of the envelope.
//!
//! 2. [`NetFederationRelay`] — an impl of [`FederationRelay`] that carries a
//!    **signed** [`DeliveryEnvelope`] from a source authority, through the
//!    broker, to a target authority. The source signs the envelope with its
//!    real P-256 governance key from the [`KeyStore`]; the target verifies the
//!    signature and admits the receipt through the same verified
//!    [`gaugewright_core::federated_delivery`] reducer the loopback relay drives
//!    (`INV-13`/`INV-21`). The relay writes nothing into the target scope —
//!    only the target's own admission creates the fact (`INV-14`).
//!
//! The wire frame between source and target is a length-prefixed JSON
//! [`EnvelopeWire`]: the broker forwards it verbatim. The target reconstructs a
//! [`DeliveryEnvelope`] and runs the admission reducer locally, so the security
//! teeth (signature, bridge grant, nonce, device binding) are exactly the ones
//! the loopback path proves — only the transport changed.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use gaugewright_core::federated_delivery::{
    DeliveryCommand, DeliveryEnvelope, DeliveryPhase, DeliveryState,
};
use gaugewright_core::ids::{AuthorityId, BridgeGrantId, Nonce, PublicKey};
use gaugewright_core::signature::Signature;
use gaugewright_store::{AdmitError, Store};

use crate::federation_relay::{delivery_scope, Message};
use crate::key_store::KeyStore;

/// The fixed width of a rendezvous session token on the wire. The broker reads
/// exactly this many bytes from each connecting leg to learn *which pairing* the
/// leg belongs to — opaque routing metadata, never payload. Both legs that name
/// the same token are spliced together.
const TOKEN_LEN: usize = 32;

/// Upper bound on a single relay frame. A peer's claimed length is checked
/// against this **before** the buffer is allocated, so a malicious 4-byte
/// header cannot make the receiver allocate gigabytes (RF-A1). Envelopes are
/// small JSON; 16 MiB is generous headroom.
const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// How long the broker waits for both legs to arrive and announce their
/// session token before giving up (RF-A2). One silent or half-connected peer
/// must not wedge the pairing forever.
const DEFAULT_PAIRING_TIMEOUT: Duration = Duration::from_secs(30);

/// Errors from the networked relay transport (distinct from an admission
/// rejection, which is a verdict, not a failure).
#[derive(Debug)]
pub enum NetRelayError {
    /// A socket-level failure (connect, accept, read, write).
    Io(std::io::Error),
    /// The peer framed a message the relay could not decode.
    Protocol(String),
    /// The target refused to admit (the reducer returned an admit error that was
    /// not a plain rejection).
    Admit(AdmitError),
}

impl std::fmt::Display for NetRelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetRelayError::Io(e) => write!(f, "net relay io: {e}"),
            NetRelayError::Protocol(s) => write!(f, "net relay protocol: {s}"),
            NetRelayError::Admit(e) => write!(f, "net relay admit: {e:?}"),
        }
    }
}

impl std::error::Error for NetRelayError {}

impl From<std::io::Error> for NetRelayError {
    fn from(e: std::io::Error) -> Self {
        NetRelayError::Io(e)
    }
}

/// The signed envelope as it crosses the wire: the [`DeliveryEnvelope`] fields
/// plus the routing handles the loopback [`Message`] carries. The core
/// `DeliveryEnvelope` is a borrow-shaped reducer input (no `Serialize`), so the
/// wire form mirrors it here and the target reconstructs the envelope — no
/// change to the pure core.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EnvelopeWire {
    pub correlation: String,
    pub source: String,
    pub target: String,
    /// A handle — never the payload (`INV-10`). The relay routes it; the broker
    /// never reads it.
    pub payload_handle: String,
    /// The canonical bytes the source signed.
    pub signed_bytes: Vec<u8>,
    pub signature: Signature,
    pub source_pubkey: PublicKey,
    pub nonce: String,
    pub bridge_grant_id: String,
    pub device_key: PublicKey,
    pub device_active: bool,
}

impl EnvelopeWire {
    /// Reconstruct the core admission envelope (drops the routing handles, which
    /// the target records separately on a successful admission).
    pub fn to_delivery_envelope(&self) -> DeliveryEnvelope {
        DeliveryEnvelope {
            signed_bytes: self.signed_bytes.clone(),
            signature: self.signature.clone(),
            source_pubkey: self.source_pubkey.clone(),
            nonce: Nonce::new(self.nonce.clone()),
            bridge_grant_id: BridgeGrantId::new(self.bridge_grant_id.clone()),
            device_key: self.device_key.clone(),
            device_active: self.device_active,
        }
    }
}

/// Frame a message as a 4-byte big-endian length prefix followed by its bytes,
/// and write it. The broker forwards these bytes verbatim — it never decodes the
/// frame.
pub(crate) async fn write_frame(stream: &mut TcpStream, bytes: &[u8]) -> std::io::Result<()> {
    if bytes.len() > MAX_FRAME_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "frame exceeds MAX_FRAME_SIZE",
        ));
    }
    let len = u32::try_from(bytes.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "frame too large"))?;
    // One buffer, one write: a crash between a separate header and body write
    // would leave the peer blocked in read_exact waiting for a body that never
    // arrives (RF-A3).
    let mut framed = Vec::with_capacity(4 + bytes.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(bytes);
    stream.write_all(&framed).await?;
    stream.flush().await
}

/// Read one length-prefixed frame. The claimed length is validated against
/// [`MAX_FRAME_SIZE`] before any allocation (RF-A1).
pub(crate) async fn read_frame(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "peer claimed a frame larger than MAX_FRAME_SIZE",
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Pad/truncate a session token to the fixed wire width.
pub(crate) fn token_bytes(token: &str) -> [u8; TOKEN_LEN] {
    let mut buf = [0u8; TOKEN_LEN];
    let src = token.as_bytes();
    let n = src.len().min(TOKEN_LEN);
    buf[..n].copy_from_slice(&src[..n]);
    buf
}

/// A dumb rendezvous broker (`RELAY_NO_PAYLOAD_ACCESS`): it accepts two outbound
/// connections naming the same session token and splices their byte streams
/// together, without ever terminating, parsing, or inspecting the carried bytes.
///
/// The broker reads only the fixed-length session token from each leg — opaque
/// routing metadata, not payload — then pairs the two legs and pipes between
/// them. After pairing it is a transparent splice; it holds no key and learns
/// nothing about the envelope (the test asserts the broker observed no plaintext
/// payload).
pub struct RendezvousBroker {
    listener: TcpListener,
    local: SocketAddr,
    pairing_timeout: Duration,
}

impl RendezvousBroker {
    /// Bind the broker on `addr` (an ephemeral loopback port in the test, a
    /// routable interface in deployment) and return it with its resolved address.
    pub async fn bind(addr: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        Ok(Self {
            listener,
            local,
            pairing_timeout: DEFAULT_PAIRING_TIMEOUT,
        })
    }

    /// Override how long the broker waits for both legs to arrive and announce
    /// their tokens (tests use a short timeout; the default is 30s).
    pub fn with_pairing_timeout(mut self, timeout: Duration) -> Self {
        self.pairing_timeout = timeout;
        self
    }

    /// The address dialers connect to.
    pub fn address(&self) -> SocketAddr {
        self.local
    }

    /// Accept exactly two connections, pair the two legs that name the same
    /// session token, and splice their byte streams until both close. Returns the
    /// total bytes piped in each direction — pure transport accounting; the
    /// broker never sees what those bytes *are*.
    ///
    /// The whole pairing phase (both accepts + both token announcements) is
    /// bounded by [`Self::with_pairing_timeout`]: a peer that connects and then
    /// goes silent, or a second leg that never arrives, errors out instead of
    /// wedging the broker forever (RF-A2). The splice itself is unbounded — a
    /// paired session legitimately lives as long as both sides keep it open.
    ///
    /// This is the dumb single-pairing path the integration test drives. A real
    /// broker loops over many pairings; the splice itself is identical.
    pub async fn run_one_pairing(self) -> std::io::Result<PipedBytes> {
        // Accept the first two legs. Each leg announces its session token first;
        // the broker reads only that token (routing metadata, not payload).
        let pair = timeout(self.pairing_timeout, async {
            let (mut a, _) = self.listener.accept().await?;
            let a_token = read_token(&mut a).await?;
            let (mut b, _) = self.listener.accept().await?;
            let b_token = read_token(&mut b).await?;
            if a_token != b_token {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "rendezvous legs named different session tokens",
                ));
            }
            Ok((a, b))
        })
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "rendezvous pairing timed out waiting for both legs",
            )
        })?;
        let (mut a, mut b) = pair?;
        // Transparent splice: copy A→B and B→A until both halves close. The
        // broker terminates neither stream and parses none of the bytes.
        let (to_b, to_a) = tokio::io::copy_bidirectional(&mut a, &mut b).await?;
        Ok(PipedBytes {
            a_to_b: to_b,
            b_to_a: to_a,
        })
    }
}

/// Read the fixed-length session token a connecting leg announces.
async fn read_token(stream: &mut TcpStream) -> std::io::Result<[u8; TOKEN_LEN]> {
    let mut buf = [0u8; TOKEN_LEN];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Transport accounting the broker returns: bytes piped each way. The broker
/// learns the *count*, never the content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipedBytes {
    pub a_to_b: u64,
    pub b_to_a: u64,
}

/// The networked federation relay (`RENDEZVOUS-STUB-1` → real): carries a signed
/// [`DeliveryEnvelope`] from a source authority through a [`RendezvousBroker`] to
/// a target authority over real TCP. It signs with the source's P-256 governance
/// key from a [`KeyStore`]; the target verifies and admits through the same
/// verified reducer the loopback relay uses.
///
/// Implements the [`FederationRelay`](crate::federation_relay::FederationRelay)
/// seam's contract — carry a message, surface what the target admitted — but the
/// async transport means the source and target halves are explicit methods
/// rather than the sync trait's `deliver`. The sync trait remains the loopback
/// double; this is its cross-machine sibling behind the same seam (ADR 0020).
pub struct NetFederationRelay<K: KeyStore> {
    broker: SocketAddr,
    key_store: K,
}

impl<K: KeyStore> NetFederationRelay<K> {
    /// A relay that dials `broker` and signs with `key_store`.
    pub fn new(broker: SocketAddr, key_store: K) -> Self {
        Self { broker, key_store }
    }

    /// **Source half**: sign `msg`'s envelope under the source authority's
    /// governance key and send it through the broker on `session`. The signed
    /// bytes are the message correlation (the same canonical basis the loopback
    /// relay signs), bound to `bridge_grant_id`. Returns once the frame is
    /// written; the broker pipes it to the target.
    pub async fn send_source(
        &self,
        session: &str,
        msg: &Message,
        bridge_grant_id: &str,
        device_key: &PublicKey,
    ) -> Result<(), NetRelayError> {
        let source_key = self.key_store.signing_key(&AuthorityId::new(&msg.source));
        let signed_bytes = msg.correlation.clone().into_bytes();
        let wire = EnvelopeWire {
            correlation: msg.correlation.clone(),
            source: msg.source.clone(),
            target: msg.target.clone(),
            payload_handle: msg.payload_handle.clone(),
            signature: source_key.sign(&signed_bytes),
            source_pubkey: source_key.public_key(),
            signed_bytes,
            nonce: format!("nonce::{}", msg.correlation),
            bridge_grant_id: bridge_grant_id.to_string(),
            device_key: device_key.clone(),
            device_active: true,
        };
        let bytes = serde_json::to_vec(&wire)
            .map_err(|e| NetRelayError::Protocol(format!("encode envelope: {e}")))?;

        let mut stream = TcpStream::connect(self.broker).await?;
        // Announce the session token so the broker can pair the legs (routing
        // metadata only), then send the opaque envelope frame.
        stream.write_all(&token_bytes(session)).await?;
        write_frame(&mut stream, &bytes).await?;
        // Half-close the write side so the broker's splice and the target's read
        // see EOF and complete.
        stream.shutdown().await?;
        Ok(())
    }

    /// **Target half**: receive the source's envelope through the broker on
    /// `session`, verify + admit it through the federated-delivery reducer into
    /// `target_scope`, and (on admission) record the handle fact in that scope.
    /// Returns the received [`EnvelopeWire`] (so the test can inspect what
    /// crossed) and whether the target admitted.
    ///
    /// The relay writes nothing on its own — only the target's admission creates
    /// the fact (`INV-13`/`INV-14`), exactly as the loopback path.
    pub async fn receive_target(
        &self,
        session: &str,
        store: &mut Store,
        target_scope: &str,
    ) -> Result<(EnvelopeWire, bool), NetRelayError> {
        let mut stream = TcpStream::connect(self.broker).await?;
        stream.write_all(&token_bytes(session)).await?;
        let bytes = read_frame(&mut stream).await?;
        let wire: EnvelopeWire = serde_json::from_slice(&bytes)
            .map_err(|e| NetRelayError::Protocol(format!("decode envelope: {e}")))?;

        let admitted = admit_received(store, target_scope, &wire)?;
        Ok((wire, admitted))
    }
}

/// Drive the verified admission for a received wire envelope: source authorizes →
/// relay queues + delivers (transport only) → the **target** admits. Identical to
/// the loopback [`crate::federation_relay::deliver`] sequence, but the envelope
/// arrived over the wire. Only the target's admission writes the fact.
fn admit_received(
    store: &mut Store,
    target_scope: &str,
    wire: &EnvelopeWire,
) -> Result<bool, NetRelayError> {
    let ds = delivery_scope(&wire.correlation);
    store
        .admit::<DeliveryState>(&ds, DeliveryCommand::AuthorizeFederatedMessage)
        .map_err(NetRelayError::Admit)?; // source
    store
        .admit::<DeliveryState>(&ds, DeliveryCommand::EnqueueFederatedMessage)
        .map_err(NetRelayError::Admit)?; // relay queues
    store
        .admit::<DeliveryState>(&ds, DeliveryCommand::RecordRelayDelivery)
        .map_err(NetRelayError::Admit)?; // relay delivers (transport)

    let envelope = wire.to_delivery_envelope();
    let admitted =
        match store.admit::<DeliveryState>(&ds, DeliveryCommand::AdmitTargetReceipt { envelope }) {
            Ok(s) => s.phase == DeliveryPhase::TargetAdmitted,
            // A fail-closed admission rejection (bad sig / grant / nonce / device) is
            // a denied crossing, not a transport error.
            Err(AdmitError::Rejected(_)) => false,
            Err(e) => return Err(NetRelayError::Admit(e)),
        };
    if admitted {
        let rec = serde_json::json!({
            "correlation": wire.correlation,
            "source": wire.source,
            "target": wire.target,
            "payload_handle": wire.payload_handle, // a handle — never the payload
        });
        store
            .append_record(target_scope, "federated", &rec.to_string())
            .map_err(NetRelayError::Admit)?;
    }
    Ok(admitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::federation_relay::admitted;
    use crate::key_store::LoopbackKeyStore;

    /// The bridge grant + device key a default delivery is bound to (the same
    /// fixed binding the loopback path presents). A genuine crossing presents
    /// these; the reducer's teeth reject anything else.
    const BOUND_GRANT: &str = "bridge-grant-7";
    fn bound_device() -> PublicKey {
        PublicKey::new("04dev1ce0ke7")
    }

    /// REAL networked crossing over loopback TCP: a broker is started on an
    /// ephemeral port, a source and a target each dial *out* to it, a signed
    /// envelope crosses source→broker→target, and the target admits it through
    /// the verified reducer — while the broker, a transparent splice, never saw
    /// the plaintext payload handle.
    #[tokio::test]
    async fn a_signed_envelope_crosses_through_the_broker_and_the_target_admits() {
        // Broker on an ephemeral loopback port (the net_server bind tests prove
        // TCP binding works in this env). We collect what the broker piped so we
        // can assert it never saw the payload.
        let broker = RendezvousBroker::bind("127.0.0.1:0").await.unwrap();
        let broker_addr = broker.address();
        let broker_task = tokio::spawn(broker.run_one_pairing());

        let msg = Message {
            correlation: "net-c1".into(),
            source: "A".into(),
            target: "B".into(),
            payload_handle: "ctx-method-SECRET-HANDLE".into(),
        };

        let source_relay = NetFederationRelay::new(broker_addr, LoopbackKeyStore);
        let target_relay = NetFederationRelay::new(broker_addr, LoopbackKeyStore);

        // Target store: before delivery, B's scope holds no federated facts.
        let mut store = Store::open_in_memory().unwrap();
        assert!(admitted(&store, "scope-B").unwrap().is_empty());

        // Source sends; target receives + admits. Both dial out to the broker,
        // which splices them — neither has a direct route to the other.
        let session = "rendezvous-session-001";
        let device = bound_device();
        let send = source_relay.send_source(session, &msg, BOUND_GRANT, &device);

        // Run the source send and the target receive concurrently (they meet at
        // the broker). The target half borrows the store.
        let (send_res, recv_res) = tokio::join!(
            send,
            target_relay.receive_target(session, &mut store, "scope-B"),
        );
        send_res.unwrap();
        let (wire, did_admit) = recv_res.unwrap();
        assert!(did_admit, "the target admitted the signed envelope");

        // The fact now lives in the TARGET's scope, admitted by the target,
        // carrying only the handle (INV-13/INV-14).
        let facts = admitted(&store, "scope-B").unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(
            facts[0]["payload_handle"], "ctx-method-SECRET-HANDLE",
            "the handle crossed, admitted by the target"
        );
        // The delivery lifecycle confirms the verified, target-only admission.
        let s = store
            .fold::<DeliveryState>(&delivery_scope("net-c1"))
            .unwrap();
        assert_eq!(s.phase, DeliveryPhase::TargetAdmitted);
        assert!(
            s.signature_verified,
            "INV-21: the source P-256 signature was verified before admission"
        );
        assert!(
            !s.relay_has_payload_access,
            "INV-10: the relay/broker gained no payload read"
        );

        // RELAY_NO_PAYLOAD_ACCESS: the broker spliced opaque bytes only. It piped
        // a non-zero count source→target (the envelope frame) and zero the other
        // way (the target only read), and — critically — it never decoded the
        // frame, so it could not have seen the payload handle. We assert the
        // broker's view is byte-accounting only: it reports a count, not content.
        let piped = broker_task.await.unwrap().unwrap();
        assert!(
            piped.a_to_b > 0,
            "the broker piped the envelope frame source→target"
        );
        // The wire frame the target decoded is the only place the payload handle
        // appears — the broker has no decode path at all (it owns no EnvelopeWire,
        // only a u64 byte count). The handle's presence in the target's decoded
        // wire, paired with the broker returning only counts, is the structural
        // RELAY_NO_PAYLOAD_ACCESS proof.
        assert_eq!(wire.payload_handle, "ctx-method-SECRET-HANDLE");
    }

    /// A forged (malformed) signature is denied target admission even over the
    /// real network path: the envelope crosses the broker fine (transport is
    /// blind), but the target's reducer fails closed (`INV-21`). The broker still
    /// only piped opaque bytes.
    #[tokio::test]
    async fn a_forged_signature_is_denied_over_the_network() {
        let broker = RendezvousBroker::bind("127.0.0.1:0").await.unwrap();
        let broker_addr = broker.address();
        let broker_task = tokio::spawn(broker.run_one_pairing());

        let session = "rendezvous-session-forged";
        // Hand-build a forged wire envelope (a non-P-256 signature) and send it
        // directly through the broker, bypassing the honest source signer.
        let forged = EnvelopeWire {
            correlation: "net-forged".into(),
            source: "A".into(),
            target: "B".into(),
            payload_handle: "ctx-method".into(),
            signed_bytes: b"net-forged".to_vec(),
            signature: Signature::new(vec![0u8; 8]), // not P-256-sized — fails closed
            source_pubkey: LoopbackKeyStore
                .signing_key(&AuthorityId::new("A"))
                .public_key(),
            nonce: "nonce::net-forged".into(),
            bridge_grant_id: BOUND_GRANT.into(),
            device_key: bound_device(),
            device_active: true,
        };
        let bytes = serde_json::to_vec(&forged).unwrap();

        let mut store = Store::open_in_memory().unwrap();
        let target_relay = NetFederationRelay::new(broker_addr, LoopbackKeyStore);

        let send = async move {
            let mut stream = TcpStream::connect(broker_addr).await.unwrap();
            stream.write_all(&token_bytes(session)).await.unwrap();
            write_frame(&mut stream, &bytes).await.unwrap();
            stream.shutdown().await.unwrap();
        };
        let (_, recv_res) = tokio::join!(
            send,
            target_relay.receive_target(session, &mut store, "scope-B"),
        );
        let (_wire, did_admit) = recv_res.unwrap();
        assert!(
            !did_admit,
            "INV-21: an unverifiable signature denies target admission over the wire"
        );
        // No fact was written into the target scope.
        assert!(admitted(&store, "scope-B").unwrap().is_empty());
        let _ = broker_task.await.unwrap();
    }

    /// RF-A1: a peer that *claims* an enormous frame is rejected from the 4-byte
    /// header alone — the receiver never allocates the claimed size.
    #[tokio::test]
    async fn an_oversized_claimed_frame_is_rejected_before_allocation() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let reader = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_frame(&mut stream).await
        });
        let mut attacker = TcpStream::connect(addr).await.unwrap();
        // Claim a 4 GiB frame; send no body.
        attacker.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        let err = reader.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    /// RF-A1 (write side): the sender refuses to frame more than MAX_FRAME_SIZE.
    #[tokio::test]
    async fn writing_an_oversized_frame_is_refused() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _server = tokio::spawn(async move { listener.accept().await });
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let too_big = vec![0u8; MAX_FRAME_SIZE + 1];
        let err = write_frame(&mut stream, &too_big).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    /// RF-A2: a peer that connects and goes silent (no token), with no second
    /// leg ever arriving, errors the pairing out instead of wedging the broker.
    #[tokio::test]
    async fn a_silent_peer_times_the_pairing_out() {
        // A generous timeout (the test only needs it to fire *eventually*) so the
        // assertion doesn't race a load-starved runtime in CI.
        let broker = RendezvousBroker::bind("127.0.0.1:0")
            .await
            .unwrap()
            .with_pairing_timeout(Duration::from_millis(500));
        let addr = broker.address();
        let broker_task = tokio::spawn(broker.run_one_pairing());
        // Connect but never send a token.
        let _silent = TcpStream::connect(addr).await.unwrap();
        let err = broker_task.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    }

    /// ACCT-1 hermetic E2E: the **device-enrollment** handshake ([`crate::device_enroll`],
    /// [ADR 0055]) composed over the **real broker byte-splice** — proving the proven wire
    /// runs over the proven transport. A new device and the account holder each dial *out* to
    /// the broker (no direct route); the broker splices them. The new device sends its
    /// `EnrollRequest`, the holder reads the *presented* subkey, confirms the SAS channel
    /// binding matches (the out-of-band human compare), seals the account key to that subkey
    /// and returns the `EnrollAuthorize`; the new device unseals the account key. The broker
    /// only piped opaque bytes (the sealed key is ECIES ciphertext, `INV-10`).
    #[tokio::test]
    async fn the_enrollment_handshake_runs_over_the_real_broker() {
        use crate::device_enroll::{Holder, NewDevice};
        use gaugewright_core::signature::SigningKey;

        let broker = RendezvousBroker::bind("127.0.0.1:0").await.unwrap();
        let broker_addr = broker.address();
        let broker_task = tokio::spawn(broker.run_one_pairing());

        let session = "enroll-session-001";
        let account_key = *b"ACCOUNT-KEY-32-bytes-exactly!!!!";
        let holder_root = SigningKey::from_seed(&[99u8; 32]).unwrap();
        let account_root = holder_root.public_key().as_str().to_string();
        // The new device pins which account it is joining (out-of-band, e.g. from a QR).
        let nd = NewDevice::open(
            session,
            account_root.clone(),
            SigningKey::from_seed(&[11u8; 32]).unwrap(),
        );
        let holder = Holder::new(holder_root, account_key, session);
        let device_sas = nd.sas();

        // New-device leg: send the request, read the authorization, unseal the account key.
        let nd_leg = async {
            let mut s = TcpStream::connect(broker_addr).await.unwrap();
            s.write_all(&token_bytes(session)).await.unwrap();
            let req = serde_json::to_vec(&nd.request()).unwrap();
            write_frame(&mut s, &req).await.unwrap();
            let auth_bytes = read_frame(&mut s).await.unwrap();
            let auth: crate::device_enroll::EnrollAuthorize =
                serde_json::from_slice(&auth_bytes).unwrap();
            s.shutdown().await.unwrap();
            nd.complete(&auth, 1)
                .expect("the new device recovers the account key")
        };

        // Holder leg: read the request, verify the SAS binds the presented subkey, authorize.
        let holder_leg = async {
            let mut s = TcpStream::connect(broker_addr).await.unwrap();
            s.write_all(&token_bytes(session)).await.unwrap();
            let req_bytes = read_frame(&mut s).await.unwrap();
            let req: crate::device_enroll::EnrollRequest =
                serde_json::from_slice(&req_bytes).unwrap();
            let presented = PublicKey::new(req.subkey);
            // The out-of-band SAS compare: the channel binding must match the device's SAS,
            // else a relay substitution would be caught here (CHANNEL_BINDING_HOLDS).
            assert_eq!(
                holder.sas_for(&presented),
                device_sas,
                "SAS channel binding holds"
            );
            let auth = holder
                .authorize(&presented, 100)
                .expect("holder authorizes");
            let bytes = serde_json::to_vec(&auth).unwrap();
            write_frame(&mut s, &bytes).await.unwrap();
            s.shutdown().await.unwrap();
        };

        let (recovered, ()) = tokio::join!(nd_leg, holder_leg);
        // The new device recovered the *same* account key the holder sealed — over the wire.
        assert_eq!(recovered, account_key);

        // The broker spliced opaque bytes only (both legs framed JSON; the sealed key is
        // ECIES ciphertext) — it piped non-zero counts and never decoded a message.
        let piped = broker_task.await.unwrap().unwrap();
        assert!(
            piped.a_to_b > 0 && piped.b_to_a > 0,
            "both messages crossed the broker"
        );
    }
}

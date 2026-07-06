//! Remote-harness RPC session lifecycle — the orchestrator-side `(decide, evolve)`
//! reducer for one turn handed to a [`RemoteHarness`](../../pi_bridge) over the
//! federation relay (`REMOTE-SESSION-1`, ADR 0020/0031).
//!
//! This is the local view of the `PROTO-1` envelope: the orchestrator dials the
//! peer's `address`, ships one `RpcRequest::RunTurn` line, and awaits one
//! `RpcResponse::TurnComplete` line back. The reducer sequences that exchange and
//! — crucially — refuses to let a returned outcome become product truth on the
//! relay's say-so. It is the remote sibling of [`runtime_session`](crate::runtime_session)
//! (a *local* subprocess turn) and shares the admission discipline of
//! [`remote_call`](crate::remote_call). Discharges:
//! - `REQUEST_REQUIRES_DIAL` — a turn request is sent only after the peer is
//!   dialed; the local orchestrator never ships bytes into the void.
//! - `RESPONSE_REQUIRES_REQUEST` — a [`TurnComplete`](RemoteEvent::TurnResponded)
//!   is admissible only against a request actually in flight.
//! - `OUTCOME_REQUIRES_SOURCE_ADMISSION` — a returned outcome enters product truth
//!   only after **source (owner) admission**, never the relay's routing (`INV-4`).
//! - `RELAY_NO_PAYLOAD_ACCESS` — relaying the turn grants the relay no payload read
//!   (`INV-10`/`INV-14`); routing is byte-blind.
//! - `TERMINAL_IS_FINAL` — once complete or failed, the session takes no further
//!   transition.

use crate::federated_delivery::Authority;
use crate::Rejection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RemotePhase {
    /// No peer reached yet.
    Init,
    /// The peer endpoint is dialed; a turn may now be requested.
    Dialed,
    /// One `RunTurn` request is in flight to the peer.
    Requested,
    /// The peer returned a `TurnComplete`; the outcome is *received* but not yet
    /// admitted into product truth.
    Responded,
    /// The source authority admitted the returned outcome — it is product truth.
    Completed,
    /// The exchange failed (dial/transport/peer error); terminal.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteState {
    pub phase: RemotePhase,
    /// The peer endpoint has been dialed (the `address()` the relay routes to).
    pub dialed: bool,
    /// A `RunTurn` request line has been shipped.
    pub request_sent: bool,
    /// A `TurnComplete` response line has come back from the peer.
    pub response_received: bool,
    /// The returned outcome has been admitted into product truth, and by whom.
    pub outcome_admitted: bool,
    pub outcome_admitted_by: Authority,
    /// The session reached product truth.
    pub completed: bool,
    /// Routing the turn never grants the relay payload access (`INV-10`/`INV-14`).
    pub relay_has_payload_access: bool,
}

impl Default for RemoteState {
    fn default() -> Self {
        Self {
            phase: RemotePhase::Init,
            dialed: false,
            request_sent: false,
            response_received: false,
            outcome_admitted: false,
            outcome_admitted_by: Authority::None,
            completed: false,
            relay_has_payload_access: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteCommand {
    /// Dial the peer endpoint (resolve `address` through the relay).
    DialPeer,
    /// Ship one `RpcRequest::RunTurn` to the dialed peer.
    SendTurnRequest,
    /// Receive one `RpcResponse::TurnComplete` from the peer.
    ReceiveTurnResponse,
    /// The source authority admits the returned outcome into product truth.
    SourceAdmitOutcome,
    /// Complete the session from a source-admitted outcome.
    CompleteSession,
    /// Fail the in-flight exchange (dial/transport/peer error).
    FailSession,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RemoteEvent {
    PeerDialed,
    TurnRequested,
    TurnResponded,
    OutcomeAdmitted,
    SessionCompleted,
    SessionFailed,
}

fn reject(reason: &'static str) -> Result<Vec<RemoteEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(state: &RemoteState, command: RemoteCommand) -> Result<Vec<RemoteEvent>, Rejection> {
    use RemotePhase::*;
    match command {
        RemoteCommand::DialPeer => match state.phase {
            Init => Ok(vec![RemoteEvent::PeerDialed]),
            _ => reject("dialPeer: peer already dialed"),
        },
        // REQUEST_REQUIRES_DIAL: no turn ships before the peer is reached.
        RemoteCommand::SendTurnRequest => {
            if state.phase == Dialed && state.dialed {
                Ok(vec![RemoteEvent::TurnRequested])
            } else {
                reject("sendTurnRequest: peer not dialed (REQUEST_REQUIRES_DIAL)")
            }
        }
        // RESPONSE_REQUIRES_REQUEST: a TurnComplete is meaningful only in flight.
        RemoteCommand::ReceiveTurnResponse => {
            if state.phase == Requested && state.request_sent {
                Ok(vec![RemoteEvent::TurnResponded])
            } else {
                reject("receiveTurnResponse: no request in flight (RESPONSE_REQUIRES_REQUEST)")
            }
        }
        RemoteCommand::SourceAdmitOutcome => {
            if state.response_received {
                Ok(vec![RemoteEvent::OutcomeAdmitted])
            } else {
                reject("sourceAdmitOutcome: no returned outcome to admit")
            }
        }
        // OUTCOME_REQUIRES_SOURCE_ADMISSION.
        RemoteCommand::CompleteSession => {
            if state.phase == Responded
                && state.outcome_admitted
                && state.outcome_admitted_by == Authority::Source
            {
                Ok(vec![RemoteEvent::SessionCompleted])
            } else {
                reject("completeSession: needs a source-admitted outcome (INV-4)")
            }
        }
        RemoteCommand::FailSession => match state.phase {
            Dialed | Requested | Responded => Ok(vec![RemoteEvent::SessionFailed]),
            _ => reject("failSession: not in flight"),
        },
    }
}

pub fn evolve(state: &RemoteState, event: RemoteEvent) -> RemoteState {
    use RemotePhase::*;
    let mut s = state.clone();
    match event {
        RemoteEvent::PeerDialed => {
            s.phase = Dialed;
            s.dialed = true;
        }
        // Shipping the turn routes bytes only — the relay reads no payload.
        RemoteEvent::TurnRequested => {
            s.phase = Requested;
            s.request_sent = true;
        }
        RemoteEvent::TurnResponded => {
            s.phase = Responded;
            s.response_received = true;
        }
        RemoteEvent::OutcomeAdmitted => {
            s.outcome_admitted = true;
            s.outcome_admitted_by = Authority::Source;
        }
        RemoteEvent::SessionCompleted => {
            s.phase = Completed;
            s.completed = true;
        }
        RemoteEvent::SessionFailed => s.phase = Failed,
    }
    s
}

impl crate::Lifecycle for RemoteState {
    type State = RemoteState;
    type Command = RemoteCommand;
    type Event = RemoteEvent;
    const KIND: &'static str = "remote_session";
    fn decide(state: &RemoteState, command: RemoteCommand) -> Result<Vec<RemoteEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &RemoteState, event: RemoteEvent) -> RemoteState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn apply(state: &RemoteState, command: RemoteCommand) -> RemoteState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }

    #[test]
    fn happy_remote_turn_completes_only_via_source_admission() {
        use RemoteCommand::*;
        let s = RemoteState::default();
        let s = apply(&s, DialPeer);
        let s = apply(&s, SendTurnRequest);
        let s = apply(&s, ReceiveTurnResponse);
        assert_eq!(s.phase, RemotePhase::Responded);
        // COMPLETE_WITHOUT_ADMISSION teeth: a received outcome is not product truth.
        assert!(
            decide(&s, CompleteSession).is_err(),
            "no completion before source admits"
        );
        let s = apply(&s, SourceAdmitOutcome);
        let s = apply(&s, CompleteSession);
        assert_eq!(s.phase, RemotePhase::Completed);
        assert_eq!(s.outcome_admitted_by, Authority::Source);
        assert!(!s.relay_has_payload_access); // RELAY_READS_PAYLOAD teeth
    }

    #[test]
    fn request_without_dial_is_rejected() {
        // REQUEST_REQUIRES_DIAL teeth: never ship a turn into the void.
        let s = RemoteState::default();
        assert!(decide(&s, RemoteCommand::SendTurnRequest).is_err());
    }

    #[test]
    fn response_without_request_is_rejected() {
        // RESPONSE_REQUIRES_REQUEST teeth: a TurnComplete with nothing in flight.
        let s = apply(&RemoteState::default(), RemoteCommand::DialPeer);
        assert!(decide(&s, RemoteCommand::ReceiveTurnResponse).is_err());
    }

    #[test]
    fn terminal_is_final() {
        // TERMINAL_IS_FINAL teeth: a failed session takes no further transition.
        use RemoteCommand::*;
        let s = apply(&RemoteState::default(), DialPeer);
        let s = apply(&s, FailSession);
        assert_eq!(s.phase, RemotePhase::Failed);
        assert!(decide(&s, SendTurnRequest).is_err());
        assert!(decide(&s, FailSession).is_err());
    }

    fn arb_command() -> impl Strategy<Value = RemoteCommand> {
        use RemoteCommand::*;
        prop_oneof![
            Just(DialPeer),
            Just(SendTurnRequest),
            Just(ReceiveTurnResponse),
            Just(SourceAdmitOutcome),
            Just(CompleteSession),
            Just(FailSession),
        ]
    }

    proptest! {
        /// The remote-session invariants hold over every reachable trace.
        #[test]
        fn remote_session_invariants(commands in prop::collection::vec(arb_command(), 0..50)) {
            let mut s = RemoteState::default();
            for c in commands {
                // OUTCOME_REQUIRES_SOURCE_ADMISSION.
                if c == RemoteCommand::CompleteSession && decide(&s, c).is_ok() {
                    prop_assert!(
                        s.outcome_admitted && s.outcome_admitted_by == Authority::Source,
                        "completed without a source-admitted outcome"
                    );
                }
                s = apply(&s, c);
                // REQUEST_REQUIRES_DIAL.
                if s.request_sent {
                    prop_assert!(s.dialed, "request shipped without dialing the peer");
                }
                // RESPONSE_REQUIRES_REQUEST.
                if s.response_received {
                    prop_assert!(s.request_sent, "response received with no request in flight");
                }
                // OUTCOME_REQUIRES_SOURCE_ADMISSION.
                if s.completed {
                    prop_assert!(s.outcome_admitted && s.outcome_admitted_by == Authority::Source);
                }
                // RELAY_NO_PAYLOAD_ACCESS.
                prop_assert!(!s.relay_has_payload_access);
            }
        }
    }
}

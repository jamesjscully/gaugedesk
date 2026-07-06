//! Remote capability call lifecycle — the `(decide, evolve)` reducer, ported from
//! `specs/models/remote-call.qnt` (ADR 0015). M2.
//!
//! One source-owned federated capability invocation: request → authorize → send →
//! target-accept → execute → respond → source-admit → complete. Discharges:
//! - `TARGET_EXECUTION_REQUIRES_BOTH` — target execution needs source authorization
//!   **and** target admission (`INV-13`).
//! - `TARGET_ADMITTED_BY_TARGET` — admission is by the target authority, never the
//!   relay (`INV-1`).
//! - `RESPONSE_REQUIRES_SOURCE_ADMISSION` — source completion needs a
//!   source-admitted response receipt; a raw returned value is not completion (`INV-4`).
//! - `RELAY_NO_PAYLOAD_ACCESS` — routing grants the relay no payload read (`INV-10`/`INV-14`).
//! - `RETRY_DOES_NOT_WIDEN` — retry arguments stay within the original handles (`INV-17`).

use std::collections::BTreeSet;

use crate::federated_delivery::Authority;
use crate::Rejection;

fn original_handles() -> BTreeSet<String> {
    ["method", "context"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CallPhase {
    Init,
    Requested,
    Authorized,
    Sent,
    Accepted,
    Executing,
    Responded,
    Completed,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallState {
    pub phase: CallPhase,
    pub source_authorized: bool,
    pub routed: bool,
    pub target_admitted: bool,
    pub target_admission_by: Authority,
    pub target_executed: bool,
    pub response_produced: bool,
    pub response_routed: bool,
    pub response_admitted: bool,
    pub response_admitted_by: Authority,
    pub completed: bool,
    pub relay_has_payload_access: bool,
    pub retried: bool,
    pub retry_handles: BTreeSet<String>,
}

impl Default for CallState {
    fn default() -> Self {
        Self {
            phase: CallPhase::Init,
            source_authorized: false,
            routed: false,
            target_admitted: false,
            target_admission_by: Authority::None,
            target_executed: false,
            response_produced: false,
            response_routed: false,
            response_admitted: false,
            response_admitted_by: Authority::None,
            completed: false,
            relay_has_payload_access: false,
            retried: false,
            retry_handles: original_handles(),
        }
    }
}

impl CallState {
    /// Target execution is admissible only with source authorization and target
    /// admission **by the target** (`TARGET_EXECUTION_REQUIRES_BOTH`).
    fn target_ready(&self) -> bool {
        self.source_authorized
            && self.target_admitted
            && self.target_admission_by == Authority::Target
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallCommand {
    RequestCall,
    SourceAuthorize,
    SendCall,
    TargetAdmit,
    TargetExecute,
    ProduceResponse,
    RouteResponse,
    SourceAdmitResponse,
    CompleteCall,
    FailCall,
    CancelOrExpire,
    RetryCall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CallEvent {
    CallRequested,
    CallAuthorized,
    CallSent,
    TargetAccepted,
    TargetExecuted,
    ResponseProduced,
    ResponseRouted,
    ResponseAdmitted,
    CallCompleted,
    CallFailed,
    CallCanceled,
    CallRetried,
}

fn reject(reason: &'static str) -> Result<Vec<CallEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(state: &CallState, command: CallCommand) -> Result<Vec<CallEvent>, Rejection> {
    use CallPhase::*;
    match command {
        CallCommand::RequestCall => match state.phase {
            Init => Ok(vec![CallEvent::CallRequested]),
            _ => reject("requestCall: a call already exists"),
        },
        CallCommand::SourceAuthorize => match state.phase {
            Requested => Ok(vec![CallEvent::CallAuthorized]),
            _ => reject("sourceAuthorize: not a requested call"),
        },
        CallCommand::SendCall => match state.phase {
            Authorized => Ok(vec![CallEvent::CallSent]),
            _ => reject("sendCall: not authorized"),
        },
        // TARGET_ADMITTED_BY_TARGET: only the target authority admits the call.
        CallCommand::TargetAdmit => {
            if state.phase == Sent && state.routed && state.source_authorized {
                Ok(vec![CallEvent::TargetAccepted])
            } else {
                reject("targetAdmit: needs a routed, source-authorized call")
            }
        }
        // TARGET_EXECUTION_REQUIRES_BOTH.
        CallCommand::TargetExecute => {
            if state.phase == Accepted && state.target_ready() {
                Ok(vec![CallEvent::TargetExecuted])
            } else {
                reject("targetExecute: requires source authorization + target admission (INV-13)")
            }
        }
        CallCommand::ProduceResponse => {
            if state.target_executed && state.phase == Executing {
                Ok(vec![CallEvent::ResponseProduced])
            } else {
                reject("produceResponse: target has not executed")
            }
        }
        CallCommand::RouteResponse => {
            if state.response_produced {
                Ok(vec![CallEvent::ResponseRouted])
            } else {
                reject("routeResponse: no response produced")
            }
        }
        CallCommand::SourceAdmitResponse => {
            if state.response_routed {
                Ok(vec![CallEvent::ResponseAdmitted])
            } else {
                reject("sourceAdmitResponse: no routed response to admit")
            }
        }
        // RESPONSE_REQUIRES_SOURCE_ADMISSION.
        CallCommand::CompleteCall => {
            if state.phase == Responded
                && state.response_admitted
                && state.response_admitted_by == Authority::Source
            {
                Ok(vec![CallEvent::CallCompleted])
            } else {
                reject("completeCall: needs a source-admitted response (INV-4)")
            }
        }
        CallCommand::FailCall => match state.phase {
            Requested | Authorized | Sent | Accepted | Executing | Responded => {
                Ok(vec![CallEvent::CallFailed])
            }
            _ => reject("failCall: not in flight"),
        },
        CallCommand::CancelOrExpire => match state.phase {
            Requested | Authorized | Sent | Accepted => Ok(vec![CallEvent::CallCanceled]),
            _ => reject("cancelOrExpire: too late to cancel"),
        },
        CallCommand::RetryCall => match state.phase {
            Failed | Canceled => Ok(vec![CallEvent::CallRetried]),
            _ => reject("retryCall: not in a retryable terminal state"),
        },
    }
}

pub fn evolve(state: &CallState, event: CallEvent) -> CallState {
    use CallPhase::*;
    let mut s = state.clone();
    match event {
        CallEvent::CallRequested => s.phase = Requested,
        CallEvent::CallAuthorized => {
            s.phase = Authorized;
            s.source_authorized = true;
        }
        // Routing grants the relay no payload access (RELAY_READS_PAYLOAD held off).
        CallEvent::CallSent => {
            s.phase = Sent;
            s.routed = true;
        }
        CallEvent::TargetAccepted => {
            s.phase = Accepted;
            s.target_admitted = true;
            s.target_admission_by = Authority::Target;
        }
        CallEvent::TargetExecuted => {
            s.phase = Executing;
            s.target_executed = true;
        }
        CallEvent::ResponseProduced => {
            s.phase = Responded;
            s.response_produced = true;
        }
        CallEvent::ResponseRouted => s.response_routed = true,
        CallEvent::ResponseAdmitted => {
            s.response_admitted = true;
            s.response_admitted_by = Authority::Source;
        }
        CallEvent::CallCompleted => {
            s.phase = Completed;
            s.completed = true;
        }
        CallEvent::CallFailed => s.phase = Failed,
        CallEvent::CallCanceled => s.phase = Canceled,
        // RETRY_DOES_NOT_WIDEN: a retry resets to requested with the original handles.
        CallEvent::CallRetried => {
            s.phase = Requested;
            s.source_authorized = false;
            s.routed = false;
            s.target_admitted = false;
            s.target_admission_by = Authority::None;
            s.target_executed = false;
            s.response_produced = false;
            s.response_routed = false;
            s.response_admitted = false;
            s.response_admitted_by = Authority::None;
            s.completed = false;
            s.retried = true;
            s.retry_handles = original_handles();
        }
    }
    s
}

impl crate::Lifecycle for CallState {
    type State = CallState;
    type Command = CallCommand;
    type Event = CallEvent;
    const KIND: &'static str = "remote_call";
    fn decide(state: &CallState, command: CallCommand) -> Result<Vec<CallEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &CallState, event: CallEvent) -> CallState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn apply(state: &CallState, command: CallCommand) -> CallState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }
    fn accepted() -> CallState {
        let s = CallState::default();
        let s = apply(&s, CallCommand::RequestCall);
        let s = apply(&s, CallCommand::SourceAuthorize);
        let s = apply(&s, CallCommand::SendCall);
        apply(&s, CallCommand::TargetAdmit)
    }

    #[test]
    fn happy_remote_call_completes_only_via_source_admission() {
        let s = accepted();
        assert!(s.target_admitted && s.target_admission_by == Authority::Target);
        let s = apply(&s, CallCommand::TargetExecute);
        let s = apply(&s, CallCommand::ProduceResponse);
        // COMPLETE_WITHOUT_RESPONSE_ADMISSION teeth: can't complete from an unadmitted response.
        assert!(
            decide(&s, CallCommand::CompleteCall).is_err(),
            "no completion before source admits"
        );
        let s = apply(&s, CallCommand::RouteResponse);
        let s = apply(&s, CallCommand::SourceAdmitResponse);
        let s = apply(&s, CallCommand::CompleteCall);
        assert_eq!(s.phase, CallPhase::Completed);
        assert!(!s.relay_has_payload_access); // RELAY_READS_PAYLOAD teeth
    }

    #[test]
    fn execute_without_both_sides_is_rejected() {
        // EXECUTE_WITHOUT_SOURCE teeth: no execution from an un-admitted (or un-authorized) call.
        let s = CallState::default();
        let s = apply(&s, CallCommand::RequestCall);
        let s = apply(&s, CallCommand::SourceAuthorize);
        let s = apply(&s, CallCommand::SendCall); // sent, but not target-admitted yet
        assert!(decide(&s, CallCommand::TargetExecute).is_err());
    }

    #[test]
    fn retry_keeps_original_handles() {
        // RETRY_WIDENS teeth: retry never adds a handle.
        let s = apply(&accepted(), CallCommand::FailCall);
        let s = apply(&s, CallCommand::RetryCall);
        assert!(s.retried);
        assert_eq!(s.retry_handles, original_handles());
    }

    fn arb_command() -> impl Strategy<Value = CallCommand> {
        use CallCommand::*;
        prop_oneof![
            Just(RequestCall),
            Just(SourceAuthorize),
            Just(SendCall),
            Just(TargetAdmit),
            Just(TargetExecute),
            Just(ProduceResponse),
            Just(RouteResponse),
            Just(SourceAdmitResponse),
            Just(CompleteCall),
            Just(FailCall),
            Just(CancelOrExpire),
            Just(RetryCall),
        ]
    }

    proptest! {
        #[test]
        fn remote_call_invariants(commands in prop::collection::vec(arb_command(), 0..50)) {
            let mut s = CallState::default();
            let orig = original_handles();
            for c in commands {
                // TARGET_EXECUTION_REQUIRES_BOTH.
                if c == CallCommand::TargetExecute && decide(&s, c).is_ok() {
                    prop_assert!(s.target_ready(), "executed without both sides");
                }
                // RESPONSE_REQUIRES_SOURCE_ADMISSION.
                if c == CallCommand::CompleteCall && decide(&s, c).is_ok() {
                    prop_assert!(
                        s.response_admitted && s.response_admitted_by == Authority::Source,
                        "completed without a source-admitted response"
                    );
                }
                s = apply(&s, c);
                // TARGET_ADMITTED_BY_TARGET.
                if s.target_admitted {
                    prop_assert_eq!(s.target_admission_by, Authority::Target);
                }
                // RELAY_NO_PAYLOAD_ACCESS.
                prop_assert!(!s.relay_has_payload_access);
                // RETRY_DOES_NOT_WIDEN.
                prop_assert!(s.retry_handles.iter().all(|h| orig.contains(h)));
            }
        }
    }
}

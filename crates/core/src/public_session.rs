//! Public agent-hosting **session** lifecycle (SERVE-1) — one visit to a publicly
//! hosted, embedded agent, ported from `specs/models/public-session.qnt`.
//!
//! A thin lifecycle composing [[runtime-session]] / [[boundary]] / [[instance]]
//! (ADR 0050/0051), generalized over the **principal** (anonymous | authenticated) and
//! **retention**: `opened → active → expiring → tornDown`, idle-lease-driven and
//! resumable. The visitor is a principal *inside* the consultant's authority — never a
//! crossing (`INV-1`/`INV-21`). Discharges:
//! - `RETENTION_MATCHES_PRINCIPAL` — at teardown a durable end-user chat is persisted
//!   iff the principal is authenticated (anonymous discards).
//! - `DURABLE_CHAT_REQUIRES_IDENTITY` — no durable end-user-keyed chat without identity.
//! - `RESUME_REQUIRES_IDENTITY` — only an authenticated session resumes a prior chat.
//! - `TERMINAL_RETAINS_TRANSCRIPT` — teardown keeps the transcript handle + the one
//!   session-occurred fact (`INV-6` append-only, `INV-10` handle floor).
//! - `SESSION_OCCURRED_AT_TEARDOWN` — the session fact is recorded only at teardown.

use crate::Rejection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Phase {
    Init,
    Opened,
    Active,
    Expiring,
    TornDown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicSessionState {
    pub phase: Phase,
    /// The principal is an identified audience end-user (vs. anonymous). Fixed at open.
    pub authenticated: bool,
    /// This session resumed a prior durable chat (authenticated only).
    pub resumes_prior_chat: bool,
    /// The one durable session-occurred fact (billing/audit floor) — set at teardown.
    pub session_occurred: bool,
    /// The transcript is retained as content behind a handle — set at teardown.
    pub transcript_retained: bool,
    /// A durable, end-user-keyed chat was persisted — only when authenticated.
    pub durable_chat_persisted: bool,
}

impl Default for PublicSessionState {
    fn default() -> Self {
        Self {
            phase: Phase::Init,
            authenticated: false,
            resumes_prior_chat: false,
            session_occurred: false,
            transcript_retained: false,
            durable_chat_persisted: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicSessionCommand {
    /// Open an anonymous session (the 0050 floor).
    OpenAnonymous,
    /// Open an authenticated (audience) session.
    OpenAuthenticated,
    /// Open an authenticated session that resumes a prior durable chat. Authenticated
    /// by construction — anonymous conversations were discarded, so cannot resume.
    Resume,
    /// The microVM is warm / restored from the snapshot; the lease is live.
    Activate,
    /// The idle lease is timing out.
    IdleExpire,
    /// Activity in the grace window resets the lease (resumable).
    ResumeActivity,
    /// Release the lease (idle-driven). Records the session fact + retention.
    TearDown,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PublicSessionEvent {
    Opened { authenticated: bool, resuming: bool },
    Activated,
    IdleExpired,
    ActivityResumed,
    TornDown { durable_chat_persisted: bool },
}

fn reject(reason: &'static str) -> Result<Vec<PublicSessionEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(
    state: &PublicSessionState,
    command: PublicSessionCommand,
) -> Result<Vec<PublicSessionEvent>, Rejection> {
    use Phase::*;
    use PublicSessionCommand as C;
    use PublicSessionEvent as E;
    match command {
        C::OpenAnonymous => match state.phase {
            Init => Ok(vec![E::Opened {
                authenticated: false,
                resuming: false,
            }]),
            _ => reject("openAnonymous: session already opened"),
        },
        C::OpenAuthenticated => match state.phase {
            Init => Ok(vec![E::Opened {
                authenticated: true,
                resuming: false,
            }]),
            _ => reject("openAuthenticated: session already opened"),
        },
        // RESUME_REQUIRES_IDENTITY by construction: a resume is always authenticated.
        C::Resume => match state.phase {
            Init => Ok(vec![E::Opened {
                authenticated: true,
                resuming: true,
            }]),
            _ => reject("resume: session already opened"),
        },
        C::Activate => match state.phase {
            Opened => Ok(vec![E::Activated]),
            _ => reject("activate: only from opened"),
        },
        C::IdleExpire => match state.phase {
            Active => Ok(vec![E::IdleExpired]),
            _ => reject("idleExpire: only from active"),
        },
        C::ResumeActivity => match state.phase {
            Expiring => Ok(vec![E::ActivityResumed]),
            _ => reject("resumeActivity: only from expiring"),
        },
        // RETENTION_MATCHES_PRINCIPAL: persist a durable chat iff authenticated.
        C::TearDown => match state.phase {
            Opened | Active | Expiring => Ok(vec![E::TornDown {
                durable_chat_persisted: state.authenticated,
            }]),
            _ => reject("tearDown: only from a live (non-terminal) session"),
        },
    }
}

pub fn evolve(state: &PublicSessionState, event: PublicSessionEvent) -> PublicSessionState {
    use Phase::*;
    use PublicSessionEvent as E;
    let mut s = state.clone();
    match event {
        E::Opened {
            authenticated,
            resuming,
        } => {
            s.phase = Opened;
            s.authenticated = authenticated;
            s.resumes_prior_chat = resuming;
        }
        E::Activated => s.phase = Active,
        E::IdleExpired => s.phase = Expiring,
        E::ActivityResumed => s.phase = Active,
        E::TornDown {
            durable_chat_persisted,
        } => {
            s.phase = TornDown;
            // The append-only floor (INV-6/INV-10): always record the one session fact
            // and retain the transcript handle. "Discard" means no durable chat, never
            // erasing the transcript.
            s.session_occurred = true;
            s.transcript_retained = true;
            s.durable_chat_persisted = durable_chat_persisted;
        }
    }
    s
}

impl crate::Lifecycle for PublicSessionState {
    type State = PublicSessionState;
    type Command = PublicSessionCommand;
    type Event = PublicSessionEvent;
    const KIND: &'static str = "public_session";
    fn decide(
        state: &PublicSessionState,
        command: PublicSessionCommand,
    ) -> Result<Vec<PublicSessionEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &PublicSessionState, event: PublicSessionEvent) -> PublicSessionState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn apply(state: &PublicSessionState, command: PublicSessionCommand) -> PublicSessionState {
        match decide(state, command) {
            Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
            Err(_) => state.clone(),
        }
    }

    #[test]
    fn anonymous_discards_but_retains_transcript() {
        use PublicSessionCommand::*;
        let s = PublicSessionState::default();
        let s = apply(&s, OpenAnonymous);
        let s = apply(&s, Activate);
        let s = apply(&s, IdleExpire);
        let s = apply(&s, TearDown);
        assert_eq!(s.phase, Phase::TornDown);
        assert!(!s.durable_chat_persisted, "anonymous: no durable chat");
        assert!(
            s.transcript_retained && s.session_occurred,
            "floor retained"
        );
    }

    #[test]
    fn authenticated_persists_a_durable_chat() {
        use PublicSessionCommand::*;
        let s = PublicSessionState::default();
        let s = apply(&s, OpenAuthenticated);
        let s = apply(&s, Activate);
        let s = apply(&s, TearDown);
        assert!(s.durable_chat_persisted && s.session_occurred);
    }

    #[test]
    fn resume_is_authenticated_and_reactivates_the_lease() {
        use PublicSessionCommand::*;
        let s = PublicSessionState::default();
        let s = apply(&s, Resume);
        assert!(s.authenticated && s.resumes_prior_chat);
        let s = apply(&s, Activate);
        let s = apply(&s, IdleExpire);
        let s = apply(&s, ResumeActivity);
        assert_eq!(
            s.phase,
            Phase::Active,
            "activity in the grace window resumes"
        );
    }

    #[test]
    fn cannot_activate_before_opening_or_tear_down_twice() {
        use PublicSessionCommand::*;
        // Fail-closed: no activation from init.
        let s = PublicSessionState::default();
        assert!(decide(&s, Activate).is_err());
        // Terminal is terminal: no teardown after teardown.
        let s = apply(&s, OpenAnonymous);
        let s = apply(&s, TearDown);
        assert!(decide(&s, TearDown).is_err());
    }

    fn arb_command() -> impl Strategy<Value = PublicSessionCommand> {
        use PublicSessionCommand::*;
        prop_oneof![
            Just(OpenAnonymous),
            Just(OpenAuthenticated),
            Just(Resume),
            Just(Activate),
            Just(IdleExpire),
            Just(ResumeActivity),
            Just(TearDown),
        ]
    }

    proptest! {
        /// The public-session invariants hold over every reachable trace
        /// (mirrors public-session.qnt).
        #[test]
        fn public_session_invariants(commands in prop::collection::vec(arb_command(), 0..40)) {
            let mut s = PublicSessionState::default();
            for c in commands {
                s = apply(&s, c);
                // RETENTION_MATCHES_PRINCIPAL
                if s.phase == Phase::TornDown {
                    prop_assert_eq!(s.durable_chat_persisted, s.authenticated);
                }
                // DURABLE_CHAT_REQUIRES_IDENTITY
                if s.durable_chat_persisted {
                    prop_assert!(s.authenticated);
                }
                // RESUME_REQUIRES_IDENTITY
                if s.resumes_prior_chat {
                    prop_assert!(s.authenticated);
                }
                // TERMINAL_RETAINS_TRANSCRIPT
                if s.phase == Phase::TornDown {
                    prop_assert!(s.transcript_retained && s.session_occurred);
                }
                // SESSION_OCCURRED_AT_TEARDOWN
                if s.session_occurred {
                    prop_assert_eq!(s.phase, Phase::TornDown);
                }
            }
        }
    }
}

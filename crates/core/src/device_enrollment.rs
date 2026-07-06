//! Device enrollment — the `(decide, evolve)` reducer for one device joining an existing
//! account **root** (the keystone "this is also me" act, distinct from cross-party
//! federation). It is the Rust mirror of
//! [`device-enrollment.qnt`](../../../specs/models/device-enrollment.qnt) and implements
//! [`lifecycles/device-enrollment.md`](../../../specs/lifecycles/device-enrollment.md)
//! ([ADR 0055](../../../specs/decisions/0055-enrollment-handshake-protocol.md), `ACCT-1`).
//!
//! A new device mints a subkey and opens a pairing session over the dumb, untrusted
//! relay; a device holding the root authorizes it, issuing a root-signed **self**-delegation
//! (FED-5a — *not* an `INV-13` crossing) plus the account key **sealed** to the confirmed
//! subkey. The relay may **substitute** the presented subkey, so the load-bearing defense
//! is the out-of-band **SAS** comparison (`presented == requested`): the holder authorizes
//! only the subkey it confirmed. The reducer makes the safety properties enforced-by-`decide`,
//! so in every reachable state:
//! - `CHANNEL_BINDING_HOLDS` — an authorized/enrolled attempt had the SAS match
//!   (`presented_subkey == requested_subkey`); a relay-substituted subkey never authorizes.
//! - `NO_ATTACKER_ENROLLED` — the enrolled subkey is the genuine device's own
//!   (`delegated == requested`); the substituted subkey cannot reach `Enrolled`.
//! - `SELF_DELEGATION_ONLY` — authorize is refused unless the delegation is signed by the
//!   account root (a self-delegation; a foreign root is rejected, never admitted, `INV-13`).
//! - `KEY_SEALED_TO_DEVICE` — the account key must be sealed to the delegated subkey and
//!   never cross as plaintext (`INV-10`), else authorize is refused.
//! - `FAIL_CLOSED` — before a completed handshake nobody is enrolled (`INV-20`).
//!
//! The signature verification of the delegation and the SAS itself are the admission
//! shell's (`INV-21`, materialized before the command); the relay transport is
//! [`net_relay`](../../app)/needs-infra. This reducer owns the state machine only.

use crate::Rejection;

/// A device subkey id (abstract — the real subkey is a `PublicKey`, verified at the shell).
pub type Subkey = String;
/// An account root id (the pinned governance root the new device is joining).
pub type Root = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EnrollmentPhase {
    /// Pre-request genesis: no enrollment subject yet.
    Draft,
    /// A new device minted a subkey and opened a pairing session.
    Requested,
    /// A holder picked up the request; both ends derived a SAS for out-of-band compare.
    Challenged,
    /// The holder confirmed the SAS and issued a root-signed self-delegation + sealed key.
    Authorized,
    /// The new device verified the delegation under the pinned root and unsealed the key.
    /// Terminal (success).
    Enrolled,
    /// The holder declined (SAS mismatch or operator refusal). Terminal.
    Rejected,
    /// The pairing window lapsed before authorization. Terminal.
    Expired,
    /// Either side aborted before authorization. Terminal.
    Canceled,
}

/// A root-signed delegation binding a device `subkey` to an account `root` (FED-5a). The
/// shell verifies the signature; `decide` checks the *binding* (the claimed root is the
/// account root, and the subkey is the SAS-confirmed one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delegation {
    pub subkey: Subkey,
    pub root: Root,
}

/// The account key as it crosses to the new device: sealed (`SEC-4`) to `target_subkey`.
/// `plaintext` records whether it (wrongly) crossed unsealed — `decide` refuses that.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seal {
    pub target_subkey: Subkey,
    pub plaintext: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnrollmentState {
    pub phase: EnrollmentPhase,
    /// The account root being joined (the pinned root); set at request.
    pub account_root: Root,
    /// The genuine new device's subkey (what it actually minted); set at request.
    pub requested_subkey: Subkey,
    /// What the holder received over the relay — may differ under substitution; set at challenge.
    pub presented_subkey: Subkey,
    /// The subkey the issued delegation binds; set at authorize.
    pub delegated_subkey: Subkey,
    /// Terminal success flag.
    pub enrolled: bool,
}

impl Default for EnrollmentState {
    fn default() -> Self {
        Self {
            phase: EnrollmentPhase::Draft,
            account_root: String::new(),
            requested_subkey: String::new(),
            presented_subkey: String::new(),
            delegated_subkey: String::new(),
            enrolled: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnrollmentCommand {
    /// A new device opens the pairing session with its freshly-minted subkey.
    RequestEnrollment {
        account_root: Root,
        new_subkey: Subkey,
    },
    /// A holder picks up the request; `presented_subkey` is whatever the relay delivered.
    ChallengeEnrollment { presented_subkey: Subkey },
    /// The holder authorizes (only legitimately after confirming the SAS), issuing the
    /// delegation + sealed key.
    AuthorizeEnrollment { delegation: Delegation, seal: Seal },
    /// The new device verifies the delegation under the pinned root and unseals the key.
    CompleteEnrollment,
    /// The holder declines (SAS mismatch / operator refusal).
    RejectEnrollment,
    /// The pairing window lapsed.
    ExpireEnrollment,
    /// Either side aborts before authorization.
    CancelEnrollment,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EnrollmentEvent {
    EnrollmentRequested {
        account_root: Root,
        new_subkey: Subkey,
    },
    EnrollmentChallenged {
        presented_subkey: Subkey,
    },
    EnrollmentAuthorized {
        delegated_subkey: Subkey,
    },
    DeviceEnrolled,
    EnrollmentRejected,
    EnrollmentExpired,
    EnrollmentCanceled,
}

fn reject(reason: &'static str) -> Result<Vec<EnrollmentEvent>, Rejection> {
    Err(Rejection { reason })
}

pub fn decide(
    state: &EnrollmentState,
    command: EnrollmentCommand,
) -> Result<Vec<EnrollmentEvent>, Rejection> {
    use EnrollmentPhase::*;
    match command {
        EnrollmentCommand::RequestEnrollment {
            account_root,
            new_subkey,
        } => match state.phase {
            Draft => Ok(vec![EnrollmentEvent::EnrollmentRequested {
                account_root,
                new_subkey,
            }]),
            _ => reject("requestEnrollment: a request already exists or is terminal"),
        },
        EnrollmentCommand::ChallengeEnrollment { presented_subkey } => match state.phase {
            Requested => Ok(vec![EnrollmentEvent::EnrollmentChallenged {
                presented_subkey,
            }]),
            _ => reject("challengeEnrollment: no open request to challenge"),
        },
        // The keystone gate. A correct holder issues this only after the SAS matched; the
        // reducer enforces every safety property structurally, so a substituted subkey, a
        // foreign root, or a plaintext key can never reach Authorized (fail-closed).
        EnrollmentCommand::AuthorizeEnrollment { delegation, seal } => {
            if state.phase != Challenged {
                return reject("authorizeEnrollment: not in the challenged state");
            }
            // CHANNEL_BINDING: the holder may authorize only the subkey it confirmed
            // out-of-band — i.e. the one the new device actually minted. A relay that
            // substituted the presented subkey is caught here (SAS mismatch).
            if state.presented_subkey != state.requested_subkey {
                return reject(
                    "authorizeEnrollment: SAS mismatch — the presented subkey is not the \
                     device's own (relay substitution); reject instead",
                );
            }
            // The delegation must bind exactly the confirmed subkey.
            if delegation.subkey != state.presented_subkey {
                return reject(
                    "authorizeEnrollment: delegation does not bind the confirmed subkey",
                );
            }
            // SELF_DELEGATION_ONLY: signed by the account root, never a foreign party.
            if delegation.root != state.account_root {
                return reject("authorizeEnrollment: not a self-delegation by the account root");
            }
            // KEY_SEALED_TO_DEVICE: sealed to the delegated subkey, never plaintext (INV-10).
            if seal.target_subkey != delegation.subkey || seal.plaintext {
                return reject("authorizeEnrollment: account key not sealed to the device subkey");
            }
            Ok(vec![EnrollmentEvent::EnrollmentAuthorized {
                delegated_subkey: delegation.subkey,
            }])
        }
        // The new device verifies (shell: delegation chains to the pinned root, unexpired;
        // unseal succeeds) and completes only for *its own* subkey.
        EnrollmentCommand::CompleteEnrollment => {
            if state.phase == Authorized && state.delegated_subkey == state.requested_subkey {
                Ok(vec![EnrollmentEvent::DeviceEnrolled])
            } else {
                reject("completeEnrollment: not authorized for this device's subkey")
            }
        }
        EnrollmentCommand::RejectEnrollment => match state.phase {
            Challenged | Authorized => Ok(vec![EnrollmentEvent::EnrollmentRejected]),
            _ => reject("rejectEnrollment: nothing to reject"),
        },
        EnrollmentCommand::ExpireEnrollment => match state.phase {
            Requested | Challenged | Authorized => Ok(vec![EnrollmentEvent::EnrollmentExpired]),
            _ => reject("expireEnrollment: not in flight"),
        },
        EnrollmentCommand::CancelEnrollment => match state.phase {
            Requested | Challenged | Authorized => Ok(vec![EnrollmentEvent::EnrollmentCanceled]),
            _ => reject("cancelEnrollment: not in flight"),
        },
    }
}

pub fn evolve(state: &EnrollmentState, event: EnrollmentEvent) -> EnrollmentState {
    use EnrollmentPhase::*;
    let mut s = state.clone();
    match event {
        EnrollmentEvent::EnrollmentRequested {
            account_root,
            new_subkey,
        } => {
            s.phase = Requested;
            s.account_root = account_root;
            s.requested_subkey = new_subkey;
        }
        EnrollmentEvent::EnrollmentChallenged { presented_subkey } => {
            s.phase = Challenged;
            s.presented_subkey = presented_subkey;
        }
        EnrollmentEvent::EnrollmentAuthorized { delegated_subkey } => {
            s.phase = Authorized;
            s.delegated_subkey = delegated_subkey;
        }
        EnrollmentEvent::DeviceEnrolled => {
            s.phase = Enrolled;
            s.enrolled = true;
        }
        EnrollmentEvent::EnrollmentRejected => s.phase = Rejected,
        EnrollmentEvent::EnrollmentExpired => s.phase = Expired,
        EnrollmentEvent::EnrollmentCanceled => s.phase = Canceled,
    }
    s
}

impl crate::Lifecycle for EnrollmentState {
    type State = EnrollmentState;
    type Command = EnrollmentCommand;
    type Event = EnrollmentEvent;
    const KIND: &'static str = "device-enrollment";
    fn decide(
        state: &EnrollmentState,
        command: EnrollmentCommand,
    ) -> Result<Vec<EnrollmentEvent>, Rejection> {
        decide(state, command)
    }
    fn evolve(state: &EnrollmentState, event: EnrollmentEvent) -> EnrollmentState {
        evolve(state, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const ROOT: &str = "acct-root";
    const GENUINE: &str = "genuine-subkey";
    const ATTACKER: &str = "attacker-subkey";

    fn apply(state: &EnrollmentState, command: EnrollmentCommand) -> EnrollmentState {
        match decide(state, command) {
            Ok(events) => events
                .iter()
                .fold(state.clone(), |s, e| evolve(&s, e.clone())),
            Err(_) => state.clone(),
        }
    }

    fn requested() -> EnrollmentState {
        apply(
            &EnrollmentState::default(),
            EnrollmentCommand::RequestEnrollment {
                account_root: ROOT.into(),
                new_subkey: GENUINE.into(),
            },
        )
    }

    /// The good-faith authorize a correct holder issues after the SAS matched.
    fn good_authorize(subkey: &str) -> EnrollmentCommand {
        EnrollmentCommand::AuthorizeEnrollment {
            delegation: Delegation {
                subkey: subkey.into(),
                root: ROOT.into(),
            },
            seal: Seal {
                target_subkey: subkey.into(),
                plaintext: false,
            },
        }
    }

    #[test]
    fn happy_path_enrolls_the_genuine_device() {
        let s = requested();
        let s = apply(
            &s,
            EnrollmentCommand::ChallengeEnrollment {
                presented_subkey: GENUINE.into(),
            },
        );
        let s = apply(&s, good_authorize(GENUINE));
        assert_eq!(s.phase, EnrollmentPhase::Authorized);
        let s = apply(&s, EnrollmentCommand::CompleteEnrollment);
        assert_eq!(s.phase, EnrollmentPhase::Enrolled);
        assert!(s.enrolled);
        assert_eq!(s.delegated_subkey, GENUINE);
    }

    #[test]
    fn relay_substituted_subkey_cannot_be_authorized() {
        // The relay presents the attacker's subkey instead of the genuine one. The SAS
        // would not match, so a correct holder's authorize for the presented subkey is
        // refused (CHANNEL_BINDING / NO_ATTACKER_ENROLLED).
        let s = requested();
        let s = apply(
            &s,
            EnrollmentCommand::ChallengeEnrollment {
                presented_subkey: ATTACKER.into(),
            },
        );
        assert!(decide(&s, good_authorize(ATTACKER)).is_err());
        assert_ne!(s.phase, EnrollmentPhase::Authorized);
    }

    #[test]
    fn a_foreign_root_delegation_is_refused() {
        // SELF_DELEGATION_ONLY: even with a matched SAS, a delegation signed by a non-account
        // root is not an enrollment (that would be a cross-party federation, INV-13).
        let s = apply(
            &requested(),
            EnrollmentCommand::ChallengeEnrollment {
                presented_subkey: GENUINE.into(),
            },
        );
        let foreign = EnrollmentCommand::AuthorizeEnrollment {
            delegation: Delegation {
                subkey: GENUINE.into(),
                root: "some-other-root".into(),
            },
            seal: Seal {
                target_subkey: GENUINE.into(),
                plaintext: false,
            },
        };
        assert!(decide(&s, foreign).is_err());
    }

    #[test]
    fn a_plaintext_account_key_is_refused() {
        // KEY_SEALED_TO_DEVICE / INV-10: the account key must cross sealed, never plaintext.
        let s = apply(
            &requested(),
            EnrollmentCommand::ChallengeEnrollment {
                presented_subkey: GENUINE.into(),
            },
        );
        let plaintext = EnrollmentCommand::AuthorizeEnrollment {
            delegation: Delegation {
                subkey: GENUINE.into(),
                root: ROOT.into(),
            },
            seal: Seal {
                target_subkey: GENUINE.into(),
                plaintext: true,
            },
        };
        assert!(decide(&s, plaintext).is_err());
    }

    #[test]
    fn terminal_states_accept_nothing_further() {
        // Enrolled is terminal.
        let s = apply(
            &requested(),
            EnrollmentCommand::ChallengeEnrollment {
                presented_subkey: GENUINE.into(),
            },
        );
        let enrolled = apply(
            &apply(&s, good_authorize(GENUINE)),
            EnrollmentCommand::CompleteEnrollment,
        );
        assert_eq!(enrolled.phase, EnrollmentPhase::Enrolled);
        for c in [
            EnrollmentCommand::CompleteEnrollment,
            EnrollmentCommand::RejectEnrollment,
            EnrollmentCommand::ExpireEnrollment,
            EnrollmentCommand::CancelEnrollment,
        ] {
            assert!(decide(&enrolled, c).is_err(), "enrolled is terminal");
        }
    }

    // --- proptest: the qnt invariants hold over every reachable trace ----------------

    fn arb_command() -> impl Strategy<Value = EnrollmentCommand> {
        let subkey = prop_oneof![Just(GENUINE.to_string()), Just(ATTACKER.to_string())];
        let root = prop_oneof![Just(ROOT.to_string()), Just("foreign".to_string())];
        prop_oneof![
            Just(EnrollmentCommand::RequestEnrollment {
                account_root: ROOT.into(),
                new_subkey: GENUINE.into(),
            }),
            subkey
                .clone()
                .prop_map(|presented_subkey| EnrollmentCommand::ChallengeEnrollment {
                    presented_subkey
                }),
            // Adversarial-ish authorize: any subkey/root, sealed or plaintext. `decide`
            // must reject every unsafe combination, so the invariants below still hold.
            (subkey.clone(), root, subkey, any::<bool>()).prop_map(
                |(dsub, droot, tsub, plaintext)| EnrollmentCommand::AuthorizeEnrollment {
                    delegation: Delegation {
                        subkey: dsub,
                        root: droot,
                    },
                    seal: Seal {
                        target_subkey: tsub,
                        plaintext,
                    },
                }
            ),
            Just(EnrollmentCommand::CompleteEnrollment),
            Just(EnrollmentCommand::RejectEnrollment),
            Just(EnrollmentCommand::ExpireEnrollment),
            Just(EnrollmentCommand::CancelEnrollment),
        ]
    }

    proptest! {
        #[test]
        fn enrollment_invariants(commands in prop::collection::vec(arb_command(), 0..40)) {
            use EnrollmentPhase::*;
            let mut s = EnrollmentState::default();
            for c in commands {
                s = apply(&s, c);
                let authorized_or_enrolled = matches!(s.phase, Authorized | Enrolled);
                // CHANNEL_BINDING_HOLDS: reaching authorize/enroll required the SAS match.
                if authorized_or_enrolled {
                    prop_assert_eq!(
                        &s.presented_subkey, &s.requested_subkey,
                        "authorized without a matched SAS (channel binding broken)"
                    );
                }
                // NO_ATTACKER_ENROLLED: the enrolled subkey is the genuine device's own.
                if s.enrolled {
                    prop_assert_eq!(
                        &s.delegated_subkey, &s.requested_subkey,
                        "a non-genuine subkey was enrolled"
                    );
                    prop_assert_eq!(s.phase, Enrolled);
                }
                // FAIL_CLOSED: no enrollment outside the Enrolled state.
                if s.phase != Enrolled {
                    prop_assert!(!s.enrolled, "enrolled flag set outside Enrolled");
                }
            }
        }
    }
}

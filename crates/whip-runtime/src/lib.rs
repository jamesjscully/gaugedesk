//! GaugeDesk's side of the permanent WhippleScript runtime boundary.
//!
//! This crate deliberately depends on WhippleScript's published trust-boundary
//! types. GaugeDesk may produce product policy, but it must never reimplement the
//! envelope parser, attestation check, or IFC algebra it asks WhippleScript to
//! enforce (ADR 0080 / SUB-1).

use std::collections::BTreeSet;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use gaugewright_core::ids::{AuthorityId, PublicKey};
use gaugewright_core::signature::{verify_signature, Signature, SigningKey};
use gaugewright_harness::sandbox::Network;
use gaugewright_harness::{
    CredentialCapability, CredentialProbe, EgressGate, Harness, HarnessContinuitySpec,
    HarnessFactory, HarnessSpec, HumanPrompt, ImageContent, Observation, OutputFieldFlow, ToolInfo,
    TurnOutcome,
};
pub use whipplescript::gov::{
    external_signing_bytes, ExternalAttestation, GovernanceAttestationVerifier, SignedEnvelope,
};
pub use whipplescript::host_policy::{
    HostGovernancePolicy, PlacementPolicy as WhipplePlacementPolicy, ProviderBindingPolicy,
    ResourcePolicy,
};
pub use whipplescript::host_protocol::{
    AnswerHumanAskCommand, CredentialRef, EventPosition, ForkInstanceCommand, ForkedInstance,
    HumanAnswerReceipt, LabeledHumanAsk, LabeledRuntimeEvent, OpenInstanceCommand, OpenedInstance,
    PolicyEpochRef, ProtocolError, ProviderBindingRef, ResourceRef, RuntimeEvidencePointer,
    StartTurnCommand, TurnInput, TurnReceipt, TurnStatus, HOST_PROTOCOL,
};
pub use whipplescript::host_runtime::{
    native_workspace_tool_specs, native_workspace_tool_specs_with_capabilities,
    native_workspace_tool_specs_with_command, AdmittedCommand, AuthoredAgentPackage,
    CertifiedOutputFieldFlow, CommandExecutionOutput, CommandExecutor, GovernedHostRuntime,
    HostCancellationHandle, HostRuntimeError, HumanAnswerExecution, LabeledTurnOutput,
    ModelProvider, NativeCommandPolicy, NativeWorkspaceResolver, PackageResolver, PendingHumanTurn,
    ProjectedToolCall, ResolvedImage, ResolvedPackage, ResolvedProviderBinding, ResourceResolver,
    SecretResolver, ToolCall, TurnExecution,
};
use whipplescript::ifc::VerifiedEnvelope;

pub const GAUGEDESK_ATTESTATION_ALGORITHM: &str = "p256-sha256";

/// The pinned GaugeDesk governance root WhippleScript calls to verify an
/// externally signed policy envelope. Both the responsible authority identity
/// and its exact P-256 public key are bound; substituting either fails closed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GovernanceRootVerifier {
    expected_signer: AuthorityId,
    expected_key: PublicKey,
}

impl GovernanceRootVerifier {
    pub fn new(expected_signer: AuthorityId, expected_key: PublicKey) -> Self {
        Self {
            expected_signer,
            expected_key,
        }
    }

    pub fn expected_signer(&self) -> &AuthorityId {
        &self.expected_signer
    }

    pub fn expected_key(&self) -> &PublicKey {
        &self.expected_key
    }
}

impl GovernanceAttestationVerifier for GovernanceRootVerifier {
    fn verify(
        &self,
        signing_bytes: &[u8],
        attestation: &ExternalAttestation,
    ) -> Result<(), String> {
        if attestation.algorithm != GAUGEDESK_ATTESTATION_ALGORITHM {
            return Err("unsupported GaugeDesk governance signature algorithm".to_owned());
        }
        if attestation.key_id != self.expected_key.as_str() {
            return Err("governance attestation key does not match the pinned root".to_owned());
        }
        let bytes = hex::decode(&attestation.signature)
            .map_err(|_| "governance signature is not valid hex".to_owned())?;
        let signature = Signature::new(bytes);
        match verify_signature(signing_bytes, &signature, &self.expected_key) {
            Ok(true) => Ok(()),
            Ok(false) => Err("governance signature does not verify".to_owned()),
            Err(error) => Err(format!("invalid governance root: {}", error.reason)),
        }
    }
}

/// Compile and sign a WhippleScript governance envelope with GaugeDesk's
/// existing P-256 governance root. No environment variable or WhippleScript
/// admin mode participates; the matching [`GovernanceRootVerifier`] is the only
/// production verification path.
pub fn sign_policy_envelope(
    config_text: &str,
    signer: &AuthorityId,
    key: &SigningKey,
) -> Result<String, String> {
    let public_key = key.public_key();
    let signing_bytes = external_signing_bytes(
        config_text,
        signer.as_str(),
        GAUGEDESK_ATTESTATION_ALGORITHM,
        public_key.as_str(),
    )?;
    let signature = key.sign(&signing_bytes);
    SignedEnvelope::from_external_signature(
        config_text,
        signer.as_str(),
        GAUGEDESK_ATTESTATION_ALGORITHM,
        public_key.as_str(),
        &hex::encode(signature.as_bytes()),
    )
    .map(|envelope| envelope.to_json())
}

// The immediately preceding GaugeDesk-generated package. It remains resolvable
// only so an existing long-lived thread can make WhippleScript's explicit,
// position-preserving jump into its authored archetype package.
const GAUGEDESK_CHAT_PACKAGE: &str = r#"
file store project {
  root "."
  allow read ["**"]
  allow write ["**"]
}

workflow GaugeDeskChat {
  agent assistant {
    provider owned
    profile "repo-writer"
    capacity 1
  }

  rule converse
    when started
  => {
    tell assistant
      with access to project {
        read ["**"]
        write ["**"]
      }
      with access to command {
        run
      }
      with access to human {
        ask
      }
      "GaugeDesk host turn"
  }
}
"#;

// The immediately preceding immutable package. GaugeDesk keeps this resolver
// only to migrate an existing chat thread through WhippleScript's explicit
// cross-version fork; it is never selected for a new foreground turn.
const GAUGEDESK_CHAT_PACKAGE_COMMAND_V1: &str = r#"
file store project {
  root "."
  allow read ["**"]
  allow write ["**"]
}

workflow GaugeDeskChat {
  agent assistant {
    provider owned
    profile "repo-writer"
    capacity 1
  }

  rule converse
    when started
  => {
    tell assistant
      with access to project {
        read ["**"]
        write ["**"]
      }
      with access to command {
        run
      }
      "GaugeDesk host turn"
  }
}
"#;

const GAUGEDESK_EDITOR_MANIFEST: &str = r#"{
  "schema": "whipplescript.agent_package.v0",
  "source": "editor.whip",
  "workflow": "GaugeDeskEditor",
  "agent": "editor",
  "system_prompt": "editor.md",
  "capabilities": ["workspace.read", "workspace.write", "command.run", "human.ask"],
  "max_steps": 32
}"#;

const GAUGEDESK_EDITOR_SOURCE: &str = r#"
file store project {
  root "."
  allow read ["**"]
  allow write ["**"]
}

workflow GaugeDeskEditor {
  agent editor {
    provider owned
    profile "repo-writer"
    capacity 1
    capabilities ["workspace.read", "workspace.write", "command.run", "human.ask"]
  }

  rule edit
    when started
  => {
    tell editor requires ["workspace.read", "workspace.write", "command.run", "human.ask"]
      with access to project {
        read ["**"]
        write ["**"]
      }
      with access to command {
        run
      }
      with access to human {
        ask
      }
      "Edit the selected GaugeDesk method package."
  }
}
"#;

pub fn editor_package_capabilities() -> io::Result<BTreeSet<String>> {
    AuthoredAgentPackage::from_documents(
        GAUGEDESK_EDITOR_MANIFEST,
        GAUGEDESK_EDITOR_SOURCE,
        "GaugeDesk editor capability projection",
    )
    .map(|package| package.capabilities().iter().cloned().collect())
    .map_err(invalid_data)
}

/// Transitional implementation of GaugeDesk's neutral harness seam over the
/// permanent WhippleScript host protocol. GaugeDesk supplies its governance
/// root and state directory; package admission, IFC, transcript continuity,
/// tool execution, and the labeled output projection remain WhippleScript-owned.
#[derive(Clone)]
pub struct WhipHarnessFactory {
    authority: AuthorityId,
    signing_key: SigningKey,
    runtime_root: PathBuf,
}

impl WhipHarnessFactory {
    pub fn new(
        authority: AuthorityId,
        signing_key: SigningKey,
        runtime_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            authority,
            signing_key,
            runtime_root: runtime_root.into(),
        }
    }

    fn runtime_for_chat(
        &self,
        chat_id: &str,
        epoch: u64,
        signed_policy: &str,
    ) -> io::Result<GovernedHostRuntime> {
        let verifier =
            GovernanceRootVerifier::new(self.authority.clone(), self.signing_key.public_key());
        std::fs::create_dir_all(&self.runtime_root)?;
        GovernedHostRuntime::open_with_verifier(
            self.runtime_root
                .join(format!("{}.sqlite", hex::encode(chat_id.as_bytes()))),
            epoch,
            signed_policy,
            &verifier,
        )
        .map_err(invalid_data)
    }

    fn package_for(
        mode: gaugewright_harness::ChatMode,
        package_root: Option<&Path>,
        package_version_ref: Option<&str>,
        prompt_override: Option<&str>,
    ) -> io::Result<AuthoredAgentPackage> {
        match mode {
            gaugewright_harness::ChatMode::Use => {
                if prompt_override.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "a work chat cannot override its pinned package persona",
                    ));
                }
                let root = package_root.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "a work chat has no selected WhippleScript package root",
                    )
                })?;
                let expected = package_version_ref.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "a work chat has no pinned WhippleScript package reference",
                    )
                })?;
                let package = AuthoredAgentPackage::load(root).map_err(invalid_data)?;
                if package.version_ref() != expected {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "selected package bytes do not match the placement's pinned reference",
                    ));
                }
                Ok(package)
            }
            gaugewright_harness::ChatMode::Edit => AuthoredAgentPackage::from_documents(
                GAUGEDESK_EDITOR_MANIFEST,
                GAUGEDESK_EDITOR_SOURCE,
                prompt_override.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "an edit chat requires the GaugeDesk editor persona",
                    )
                })?,
            )
            .map_err(invalid_data),
        }
    }

    fn previous_package_for(
        worktree: &Path,
        mode: gaugewright_harness::ChatMode,
        prompt_override: Option<&str>,
    ) -> io::Result<StaticPackage> {
        let system_prompt = legacy_method_prompt(worktree, prompt_override)?;
        Ok(StaticPackage {
            version_ref: package_version_ref(mode, &system_prompt, "human-v1"),
            system_prompt,
            writable: true,
            human_interaction: true,
        })
    }

    fn open_request(
        chat_id: &str,
        package_version_ref: &str,
        policy: PolicyEpochRef,
    ) -> OpenInstanceCommand {
        OpenInstanceCommand {
            protocol: HOST_PROTOCOL.to_owned(),
            request_id: format!("gaugedesk:{chat_id}:{package_version_ref}"),
            package_version_ref: package_version_ref.to_owned(),
            policy,
        }
    }

    fn create_harness(&self, spec: &HarnessSpec) -> io::Result<WhipHarness> {
        let provider = ProviderConfig::from_spec(spec)?;
        let package = Self::package_for(
            spec.mode,
            spec.package_root.as_deref(),
            spec.package_version_ref.as_deref(),
            spec.system_prompt.as_deref(),
        )?;
        let previous =
            Self::previous_package_for(&spec.worktree, spec.mode, spec.system_prompt.as_deref())?;
        let packages = StaticPackages {
            current: package.clone(),
            previous,
        };
        let epoch = spec.policy_epoch.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "WhippleScript policy epoch is required",
            )
        })?;
        let signed_policy = spec.signed_policy_envelope.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "WhippleScript signed policy envelope is required",
            )
        })?;
        let mut runtime = self.runtime_for_chat(&spec.chat_id, epoch, signed_policy)?;
        let mut source_runtime = self.runtime_for_chat(&spec.chat_id, epoch, signed_policy)?;
        let source_open = Self::open_request(
            &spec.chat_id,
            packages.previous.version_ref.as_str(),
            source_runtime.policy_ref().clone(),
        );
        let source = source_runtime
            .open_instance(&source_open, &packages)
            .map_err(invalid_data)?;
        let source_position = source_runtime
            .current_position(&source.instance_ref)
            .map_err(invalid_data)?;
        let open = Self::open_request(
            &spec.chat_id,
            package.version_ref(),
            runtime.policy_ref().clone(),
        );
        let upgrade = ForkInstanceCommand {
            protocol: HOST_PROTOCOL.to_owned(),
            request_id: format!(
                "gaugedesk:package-upgrade:{}:{}:{}",
                spec.chat_id,
                packages.previous.version_ref,
                package.version_ref()
            ),
            source: source_position,
            target_request_id: open.request_id,
            package_version_ref: package.version_ref().to_owned(),
            policy: open.policy.clone(),
        };
        let instance = runtime
            .fork_instance_from(&source_runtime, &upgrade, &packages)
            .map(|fork| fork.target)
            .map_err(invalid_data)?;

        let read_only = spec
            .sandbox
            .read_only_roots
            .iter()
            .map(|path| {
                path.strip_prefix(&spec.worktree)
                    .map(Path::to_path_buf)
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "WhippleScript read-only root is outside the workspace capability",
                        )
                    })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let workspace = NativeWorkspaceResolver::new(&spec.worktree)
            .and_then(|resolver| resolver.read_only(read_only))
            .map_err(invalid_data)?
            .command_execution(
                NativeCommandPolicy::allow_any(Duration::from_secs(120)),
                Arc::new(GaugeDeskCommandExecutor {
                    sandbox: spec.sandbox.clone(),
                }),
            );

        Ok(WhipHarness {
            runtime,
            instance_ref: instance.instance_ref,
            policy: open.policy,
            package,
            provider,
            workspace,
            chat_id: spec.chat_id.clone(),
            provider_binding_ref: spec.provider_binding_ref.clone().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "provider binding ref is required",
                )
            })?,
            credential_ref: spec.credential_ref.clone().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "credential ref is required")
            })?,
            placement_ceiling_ref: spec.placement_ceiling_ref.clone().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "placement ceiling ref is required",
                )
            })?,
            respondent_ref: self.authority.as_str().to_owned(),
            turn_sequence: 0,
            cancellation: Arc::new(Mutex::new(None)),
        })
    }
}

impl HarnessFactory for WhipHarnessFactory {
    fn kind(&self) -> &'static str {
        "whip"
    }

    fn create(&self, spec: &HarnessSpec) -> io::Result<Box<dyn Harness>> {
        self.create_harness(spec)
            .map(|harness| Box::new(harness) as Box<dyn Harness>)
    }

    fn clone_continuity(
        &self,
        source: &HarnessContinuitySpec,
        target: &HarnessContinuitySpec,
    ) -> io::Result<()> {
        if source.policy_epoch.is_none() && source.signed_policy_envelope.is_none() {
            return Ok(());
        }
        if source.mode != target.mode {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "WhippleScript continuity fork cannot change chat mode",
            ));
        }
        let source_package = Self::package_for(
            source.mode,
            source.package_root.as_deref(),
            source.package_version_ref.as_deref(),
            source.system_prompt.as_deref(),
        )?;
        let target_package = Self::package_for(
            target.mode,
            target.package_root.as_deref(),
            target.package_version_ref.as_deref(),
            target.system_prompt.as_deref(),
        )?;
        if source_package.version_ref() != target_package.version_ref() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "WhippleScript continuity fork requires the same package identity",
            ));
        }

        let epoch = source.policy_epoch.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "WhippleScript source policy epoch is required for continuity",
            )
        })?;
        let signed_policy = source.signed_policy_envelope.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "WhippleScript source signed policy is required for continuity",
            )
        })?;
        let mut source_runtime = self.runtime_for_chat(&source.chat_id, epoch, signed_policy)?;
        let source_open = Self::open_request(
            &source.chat_id,
            source_package.version_ref(),
            source_runtime.policy_ref().clone(),
        );
        let source_instance = source_runtime
            .open_instance(&source_open, &source_package)
            .map_err(invalid_data)?;
        let source_position = source_runtime
            .current_position(&source_instance.instance_ref)
            .map_err(invalid_data)?;

        let mut target_runtime = self.runtime_for_chat(&target.chat_id, epoch, signed_policy)?;
        let target_open = Self::open_request(
            &target.chat_id,
            target_package.version_ref(),
            target_runtime.policy_ref().clone(),
        );
        let command = ForkInstanceCommand {
            protocol: HOST_PROTOCOL.to_owned(),
            request_id: format!(
                "gaugedesk:fork:{}:{}:{}",
                source.chat_id, target.chat_id, source_position.sequence
            ),
            source: source_position,
            target_request_id: target_open.request_id,
            package_version_ref: target_package.version_ref().to_owned(),
            policy: target_open.policy,
        };
        target_runtime
            .fork_instance_from(&source_runtime, &command, &target_package)
            .map(|_| ())
            .map_err(invalid_data)
    }

    fn credential_status(
        &self,
        provider: &str,
        capability: Option<&dyn CredentialCapability>,
    ) -> CredentialProbe {
        match capability {
            Some(capability) if !capability.credential_ref().is_empty() => CredentialProbe::Ready,
            _ if provider == "openai-codex" => CredentialProbe::Missing(
                "No GaugeDesk-owned Codex OAuth credential is linked. Open Account settings and connect ChatGPT."
                    .to_owned(),
            ),
            _ => CredentialProbe::Missing(format!(
                "WhippleScript has no admitted credential capability for provider `{provider}`"
            )),
        }
    }
}

struct WhipHarness {
    runtime: GovernedHostRuntime,
    instance_ref: String,
    policy: PolicyEpochRef,
    package: AuthoredAgentPackage,
    provider: ProviderConfig,
    workspace: NativeWorkspaceResolver,
    chat_id: String,
    provider_binding_ref: String,
    credential_ref: String,
    placement_ceiling_ref: String,
    respondent_ref: String,
    turn_sequence: u64,
    cancellation: Arc<Mutex<Option<HostCancellationHandle>>>,
}

impl Harness for WhipHarness {
    fn bind_authenticated_actor(&mut self, actor_ref: &str) {
        if !actor_ref.trim().is_empty() {
            self.respondent_ref = actor_ref.to_owned();
        }
    }

    fn run_turn(
        &mut self,
        _legacy_gate: &dyn EgressGate,
        prompt: &str,
        images: &[ImageContent],
        sink: &mut dyn FnMut(&Observation),
    ) -> io::Result<TurnOutcome> {
        self.turn_sequence += 1;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let resources = TurnResources {
            workspace: &self.workspace,
            images,
        };
        let pending = self
            .runtime
            .pending_human_turn(&self.instance_ref, &self.package)
            .map_err(invalid_data)?;
        let (command, execution, evidence_pointers) = if let Some(pending) = pending {
            if !images.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a human answer cannot attach new images to the suspended epoch",
                ));
            }
            let answer = AnswerHumanAskCommand {
                protocol: HOST_PROTOCOL.to_owned(),
                answer_id: format!(
                    "gaugedesk:answer:{}:{}:{nonce}",
                    self.chat_id, self.turn_sequence
                ),
                ask_ref: pending.ask.ask_ref,
                instance_ref: self.instance_ref.clone(),
                policy: pending.command.policy.clone(),
                respondent_ref: self.respondent_ref.clone(),
                answer: prompt.to_owned(),
            };
            answer.validate().map_err(invalid_data)?;
            self.install_cancellation(&pending.command);
            let resumed = self
                .runtime
                .answer_human_ask(&answer, &self.package, &self.provider, &resources)
                .map_err(invalid_data);
            self.clear_cancellation();
            let resumed = resumed?;
            resumed
                .answer_receipt
                .validate_for(&answer)
                .map_err(invalid_data)?;
            let evidence_pointers = resumed.evidence_pointers();
            (pending.command, resumed.turn, evidence_pointers)
        } else {
            let command = self.new_turn_command(prompt, images, nonce);
            self.install_cancellation(&command);
            let execution = self
                .runtime
                .run_turn(&command, &self.package, &self.provider, &resources)
                .map_err(invalid_data);
            self.clear_cancellation();
            let execution = execution?;
            let evidence_pointers = execution.evidence_pointers();
            (command, execution, evidence_pointers)
        };
        let mut outcome = project_turn_execution(execution, evidence_pointers, &command, sink)?;
        // DR-0036 §2 → ADR 0082 §5: attach the turn's certified dynamic
        // guarantee outcomes so the settle-time advancement policy can match
        // them by name. Best-effort by design — a runtime/report predating
        // DR-0036 yields nothing and consumers fall back to host-local truth;
        // a suspended turn has no terminal receipt yet, hence no report.
        if outcome.pending_human.is_none() {
            if let Ok(Some(report)) = self.runtime.turn_guarantee_report(&command) {
                outcome.guarantee_outcomes =
                    gaugewright_harness::GuaranteeOutcome::from_report(&report);
            }
        }
        Ok(outcome)
    }

    fn interrupt_handle(&self) -> Option<gaugewright_harness::InterruptHandle> {
        let cancellation = Arc::clone(&self.cancellation);
        Some(Arc::new(move || {
            if let Some(handle) = cancellation
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
            {
                let _ = handle.request();
            }
        }))
    }
}

impl WhipHarness {
    fn new_turn_command(
        &self,
        prompt: &str,
        images: &[ImageContent],
        nonce: u128,
    ) -> StartTurnCommand {
        let command_id = format!("gaugedesk:{}:{}:{nonce}", self.chat_id, self.turn_sequence);
        let has = |name: &str| {
            self.package
                .capabilities()
                .iter()
                .any(|capability| capability == name)
        };
        let mut resources = Vec::new();
        if has("workspace.read") || has("workspace.write") {
            resources.push(ResourceRef {
                handle: "project".to_owned(),
                kind: "file_store".to_owned(),
                selector: None,
            });
        }
        if has("command.run") {
            resources.push(ResourceRef {
                handle: "command".to_owned(),
                kind: "command".to_owned(),
                selector: None,
            });
        }
        if has("human.ask") {
            resources.push(ResourceRef {
                handle: "human".to_owned(),
                kind: "human".to_owned(),
                selector: None,
            });
        }
        StartTurnCommand {
            protocol: HOST_PROTOCOL.to_owned(),
            command_id: command_id.clone(),
            run_ref: format!("gaugedesk:run:{command_id}"),
            instance_ref: self.instance_ref.clone(),
            package_version_ref: self.package.version_ref().to_owned(),
            policy: self.policy.clone(),
            actor_ref: self.respondent_ref.clone(),
            input: TurnInput {
                text: prompt.to_owned(),
                images: images
                    .iter()
                    .enumerate()
                    .map(|(index, _)| ResourceRef {
                        handle: "turn_images".to_owned(),
                        kind: "image".to_owned(),
                        selector: Some(index.to_string()),
                    })
                    .collect(),
            },
            resources,
            provider_binding: ProviderBindingRef {
                binding_id: self.provider_binding_ref.clone(),
                credential: CredentialRef {
                    credential_id: self.credential_ref.clone(),
                },
            },
            placement_ceiling_ref: self.placement_ceiling_ref.clone(),
        }
    }

    fn install_cancellation(&self, command: &StartTurnCommand) {
        *self
            .cancellation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(
            self.runtime
                .cancellation_handle(&command.instance_ref, &command.command_id),
        );
    }

    fn clear_cancellation(&self) {
        self.cancellation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }
}

fn project_turn_execution(
    execution: TurnExecution,
    evidence_pointers: Vec<RuntimeEvidencePointer>,
    command: &StartTurnCommand,
    sink: &mut dyn FnMut(&Observation),
) -> io::Result<TurnOutcome> {
    let mut outcome = TurnOutcome {
        runtime_evidence_pointers: evidence_pointers
            .into_iter()
            .map(|pointer| serde_json::to_string(&pointer).map_err(invalid_data))
            .collect::<io::Result<Vec<_>>>()?,
        ..TurnOutcome::default()
    };
    if let Some(ask) = execution.pending_human_ask {
        if execution.receipt.is_some() {
            return Err(invalid_data(
                "WhippleScript returned both a human ask and a terminal receipt",
            ));
        }
        outcome.assistant_text = ask.question.clone();
        outcome.pending_human = Some(HumanPrompt {
            ask_ref: ask.ask_ref,
            question: ask.question.clone(),
            choices: ask.choices,
            freeform_allowed: ask.freeform_allowed,
            label_ref: ask.label_ref,
            evidence_ref: ask.evidence_ref,
        });
        let observation = Observation {
            kind: "human_ask",
            detail: ask.question,
            tool: None,
        };
        sink(&observation);
        outcome.observations.push(observation);
        return Ok(outcome);
    }

    let receipt = execution
        .receipt
        .ok_or_else(|| invalid_data("WhippleScript returned neither a terminal nor a human ask"))?;
    receipt.validate_for(command).map_err(invalid_data)?;
    if let Some(output) = execution.output {
        outcome.output_flow_signature = output
            .flow_signature
            .iter()
            .map(|flow| OutputFieldFlow {
                field: flow.field.clone(),
                read_handles: flow
                    .reads
                    .iter()
                    .map(|resource| resource.handle.clone())
                    .collect(),
            })
            .collect();
        outcome.assistant_text = output.assistant_text;
        for call in output.tool_calls {
            let target = tool_target(&call);
            let observation = Observation {
                kind: "tool_result",
                detail: format!("{} {}", call.name, target.as_deref().unwrap_or(""))
                    .trim()
                    .to_owned(),
                tool: Some(ToolInfo {
                    name: call.name.clone(),
                    call_id: call.call_id,
                    target,
                    args: call.arguments.to_string(),
                    ok: call.ok,
                    result: call.result,
                }),
            };
            sink(&observation);
            outcome.mediated_tool_calls.push(call.name);
            outcome.observations.push(observation);
        }
        if !outcome.assistant_text.is_empty() {
            sink(&Observation {
                kind: "text",
                detail: outcome.assistant_text.clone(),
                tool: None,
            });
        }
    }
    if receipt.status != TurnStatus::Completed {
        outcome.error = Some(format!("WhippleScript turn ended {:?}", receipt.status));
    }
    Ok(outcome)
}

#[derive(Clone)]
struct StaticPackage {
    version_ref: String,
    system_prompt: String,
    writable: bool,
    human_interaction: bool,
}

#[derive(Clone)]
struct StaticPackages {
    current: AuthoredAgentPackage,
    previous: StaticPackage,
}

const COMMAND_OUTPUT_LIMIT: usize = 1_000_000;

/// GaugeDesk realizes an already-admitted WhippleScript command inside the
/// product's OS boundary. It deliberately does not parse or reinterpret command
/// authority: WhippleScript owns the simple-command grammar, allow policy,
/// timeout ceiling, and output projection.
struct GaugeDeskCommandExecutor {
    sandbox: gaugewright_harness::sandbox::SandboxPolicy,
}

impl CommandExecutor for GaugeDeskCommandExecutor {
    fn execute(&self, admitted: &AdmittedCommand) -> Result<CommandExecutionOutput, String> {
        let policy = command_sandbox_policy(&self.sandbox, admitted);

        let args = vec!["-c".to_owned(), admitted.command.clone()];
        let mut command = gaugewright_harness::sandbox::wrap_strict(
            &policy,
            "/bin/sh",
            &args,
            Some(&admitted.workspace_root),
        )
        .map_err(|error| format!("cannot realize governed command: {error}"))?;
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot spawn governed command: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "governed command stdout was not captured".to_owned())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "governed command stderr was not captured".to_owned())?;
        let output_bytes = Arc::new(AtomicUsize::new(0));
        let readers_done = Arc::new(AtomicUsize::new(0));
        let stdout_reader =
            spawn_bounded_reader(stdout, Arc::clone(&output_bytes), Arc::clone(&readers_done));
        let stderr_reader =
            spawn_bounded_reader(stderr, Arc::clone(&output_bytes), Arc::clone(&readers_done));
        let started = Instant::now();
        let status = loop {
            match child
                .try_wait()
                .map_err(|error| format!("cannot observe governed command: {error}"))?
            {
                Some(status) => break status,
                None if output_bytes.load(Ordering::Relaxed) > COMMAND_OUTPUT_LIMIT => {
                    kill_governed_command(&mut child);
                    let _ = child.wait();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(format!(
                        "governed command exceeded the {} byte output limit",
                        COMMAND_OUTPUT_LIMIT
                    ));
                }
                None if started.elapsed() >= admitted.timeout => {
                    kill_governed_command(&mut child);
                    let _ = child.wait();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(format!(
                        "governed command exceeded its {} second timeout",
                        admitted.timeout.as_secs()
                    ));
                }
                None => thread::sleep(Duration::from_millis(10)),
            }
        };
        // A simple command may itself fork. The admitted invocation ends with
        // its foreground process; do not let any descendant retain workspace
        // authority or keep captured pipes alive past that boundary.
        let drain_deadline = Instant::now() + Duration::from_secs(1);
        while readers_done.load(Ordering::Relaxed) < 2 && Instant::now() < drain_deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if readers_done.load(Ordering::Relaxed) < 2 {
            kill_governed_command(&mut child);
        }
        let stdout = stdout_reader
            .join()
            .map_err(|_| "governed command stdout reader panicked".to_owned())??;
        let stderr = stderr_reader
            .join()
            .map_err(|_| "governed command stderr reader panicked".to_owned())??;
        if output_bytes.load(Ordering::Relaxed) > COMMAND_OUTPUT_LIMIT {
            return Err(format!(
                "governed command exceeded the {} byte output limit",
                COMMAND_OUTPUT_LIMIT
            ));
        }
        Ok(CommandExecutionOutput {
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            exit_code: status.code(),
        })
    }
}

fn kill_governed_command(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // SAFETY: the child was placed in a fresh process group whose id is its
        // pid immediately before spawn. A negative pid targets only that group.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

fn command_sandbox_policy(
    base: &gaugewright_harness::sandbox::SandboxPolicy,
    admitted: &AdmittedCommand,
) -> gaugewright_harness::sandbox::SandboxPolicy {
    let mut policy = base.clone();
    policy.writable_roots = vec![admitted.workspace_root.clone()];
    policy.read_only_roots = admitted.read_only_paths.clone();

    // The filtered provider route belongs to WhippleScript's in-process
    // provider connection, not arbitrary repository commands. Commands get no
    // network under that posture; only GaugeDesk's explicit unfiltered project
    // opt-in carries through as network authority.
    if policy.network == Network::Filtered {
        policy.network = Network::Deny;
    }
    policy
}

fn spawn_bounded_reader<R>(
    mut reader: R,
    output_bytes: Arc<AtomicUsize>,
    readers_done: Arc<AtomicUsize>,
) -> thread::JoinHandle<Result<Vec<u8>, String>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let result = (|| {
            let mut captured = Vec::new();
            let mut buffer = [0_u8; 8192];
            loop {
                let read = reader
                    .read(&mut buffer)
                    .map_err(|error| format!("cannot capture governed command output: {error}"))?;
                if read == 0 {
                    break;
                }
                let previous = output_bytes.fetch_add(read, Ordering::Relaxed);
                let remaining = COMMAND_OUTPUT_LIMIT.saturating_sub(previous);
                captured.extend_from_slice(&buffer[..read.min(remaining)]);
            }
            Ok(captured)
        })();
        readers_done.fetch_add(1, Ordering::Relaxed);
        result
    })
}

impl PackageResolver for StaticPackage {
    fn resolve_package(&self, version_ref: &str) -> Result<ResolvedPackage, String> {
        if version_ref != self.version_ref {
            return Err("package version ref does not match the GaugeDesk chat package".to_owned());
        }
        ResolvedPackage::compile(
            self.version_ref.clone(),
            if self.human_interaction {
                GAUGEDESK_CHAT_PACKAGE
            } else {
                GAUGEDESK_CHAT_PACKAGE_COMMAND_V1
            },
            Some("GaugeDeskChat"),
            "assistant",
            self.system_prompt.clone(),
            native_workspace_tool_specs_with_capabilities(
                self.writable,
                true,
                self.human_interaction,
            ),
            32,
        )
    }
}

impl PackageResolver for StaticPackages {
    fn resolve_package(&self, version_ref: &str) -> Result<ResolvedPackage, String> {
        if version_ref == self.current.version_ref() {
            self.current.resolve_package(version_ref)
        } else if version_ref == self.previous.version_ref {
            self.previous.resolve_package(version_ref)
        } else {
            Err("package version ref is outside the GaugeDesk migration set".to_owned())
        }
    }
}

struct ProviderConfig {
    provider: ModelProvider,
    model: String,
    base_url: String,
    codex_session_id: Option<String>,
    credential_ref: String,
    credential_capability: Arc<dyn CredentialCapability>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeProviderDescriptor {
    pub provider_name: String,
    pub model: String,
    pub base_url: String,
    pub endpoint_host: String,
    pub credential_env: &'static str,
}

pub fn native_provider_descriptor(
    provider_name: &str,
    model: Option<&str>,
) -> io::Result<NativeProviderDescriptor> {
    let (base_url, endpoint_host, credential_env, default_model) = match provider_name {
        "openai" => (
            "https://api.openai.com",
            "api.openai.com",
            "OPENAI_API_KEY",
            None,
        ),
        "anthropic" => (
            "https://api.anthropic.com",
            "api.anthropic.com",
            "ANTHROPIC_API_KEY",
            None,
        ),
        "openai-codex" => (
            "https://chatgpt.com",
            "chatgpt.com",
            "GAUGEDESK_CODEX_ACCESS_TOKEN",
            Some("gpt-5.5"),
        ),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("WhippleScript native provider `{provider_name}` is not supported"),
            ));
        }
    };
    let model = model.or(default_model).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "WhippleScript native API-key providers require an explicit model",
        )
    })?;
    Ok(NativeProviderDescriptor {
        provider_name: provider_name.to_owned(),
        model: model.to_owned(),
        base_url: base_url.to_owned(),
        endpoint_host: endpoint_host.to_owned(),
        credential_env,
    })
}

impl ProviderConfig {
    fn from_spec(spec: &HarnessSpec) -> io::Result<Self> {
        let provider_name = spec.provider.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "WhippleScript provider is required",
            )
        })?;
        let descriptor = native_provider_descriptor(provider_name, spec.model.as_deref())?;
        let provider = match provider_name {
            "openai" => ModelProvider::OpenAi,
            "anthropic" => ModelProvider::Anthropic,
            "openai-codex" => ModelProvider::Codex,
            _ => unreachable!("validated by native_provider_descriptor"),
        };
        if spec.sandbox.network == Network::Deny {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "GaugeDesk project policy denies provider network egress",
            ));
        }
        if !spec
            .sandbox
            .allowed_hosts
            .iter()
            .any(|host| host == &descriptor.endpoint_host)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "GaugeDesk project policy does not admit provider endpoint `{}`",
                    descriptor.endpoint_host
                ),
            ));
        }
        let credential_ref = spec.credential_ref.clone().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "credential ref is required")
        })?;
        let credential_capability = spec.credential_capability.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "GaugeDesk supplied no credential capability",
            )
        })?;
        if credential_capability.credential_ref() != credential_ref {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "credential capability does not match the policy reference",
            ));
        }
        Ok(Self {
            provider,
            model: descriptor.model,
            base_url: descriptor.base_url,
            codex_session_id: (provider == ModelProvider::Codex)
                .then(|| format!("gaugedesk-{}", hex::encode(spec.chat_id.as_bytes()))),
            credential_ref,
            credential_capability,
        })
    }
}

impl SecretResolver for ProviderConfig {
    fn resolve_provider(
        &self,
        binding: &ProviderBindingRef,
        placement_ceiling_ref: &str,
    ) -> Result<ResolvedProviderBinding, String> {
        if binding.binding_id != "model"
            || binding.credential.credential_id != self.credential_ref
            || placement_ceiling_ref != "local"
        {
            return Err(
                "provider binding does not match the admitted GaugeDesk placement".to_owned(),
            );
        }
        let material = self
            .credential_capability
            .resolve(&binding.credential.credential_id)
            .map_err(|error| format!("credential capability refused resolution: {error}"))?;
        if self.provider == ModelProvider::Codex {
            let account_id = material
                .account_id()
                .filter(|account_id| !account_id.is_empty())
                .ok_or_else(|| "GaugeDesk Codex capability has no account id".to_owned())?;
            return Ok(ResolvedProviderBinding::new_codex(
                material.secret().to_owned(),
                account_id.to_owned(),
                self.codex_session_id.clone().unwrap_or_default(),
                self.model.clone(),
                self.base_url.clone(),
                8_192,
                Duration::from_secs(120),
            ));
        }
        Ok(ResolvedProviderBinding::new(
            self.provider,
            material.secret().to_owned(),
            self.model.clone(),
            self.base_url.clone(),
            8_192,
            Duration::from_secs(120),
        ))
    }
}

struct TurnResources<'a> {
    workspace: &'a NativeWorkspaceResolver,
    images: &'a [ImageContent],
}

impl ResourceResolver for TurnResources<'_> {
    fn resolve_image(&self, image: &ResourceRef) -> Result<ResolvedImage, String> {
        if image.handle != "turn_images" || image.kind != "image" {
            return Err("image ref is outside the admitted turn-image capability".to_owned());
        }
        let index = image
            .selector
            .as_deref()
            .ok_or_else(|| "turn image ref has no selector".to_owned())?
            .parse::<usize>()
            .map_err(|_| "turn image selector is invalid".to_owned())?;
        let image = self
            .images
            .get(index)
            .ok_or_else(|| "turn image selector is out of range".to_owned())?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&image.data)
            .map_err(|_| "turn image is not valid base64".to_owned())?;
        Ok(ResolvedImage {
            media_type: image.mime_type.clone(),
            bytes,
        })
    }

    fn execute_tool(
        &self,
        admitted_resources: &[ResourceRef],
        call: &ToolCall,
    ) -> Result<String, String> {
        self.workspace.execute_tool(admitted_resources, call)
    }
}

fn legacy_method_prompt(worktree: &Path, prompt_override: Option<&str>) -> io::Result<String> {
    if let Some(prompt) = prompt_override {
        return Ok(prompt.to_owned());
    }
    for relative in [
        ".whipple/legacy-persona.md",
        ".whipple/versions/1/persona.md",
    ] {
        match std::fs::read_to_string(worktree.join(relative)) {
            Ok(text) if !text.trim().is_empty() => return Ok(text),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok("Work inside the admitted GaugeDesk project workspace.".to_owned())
}

fn package_version_ref(
    mode: gaugewright_harness::ChatMode,
    system_prompt: &str,
    revision: &str,
) -> String {
    let material = format!("{revision}\0{mode:?}\0{system_prompt}");
    format!("gaugedesk:chat-package:{}", stable_text_hash(&material))
}

fn stable_text_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(text.as_bytes()))
}

fn tool_target(call: &ProjectedToolCall) -> Option<String> {
    ["path", "command", "url", "query"]
        .into_iter()
        .find_map(|key| call.arguments.get(key).and_then(|value| value.as_str()))
        .map(str::to_owned)
}

fn invalid_data(error: impl fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

/// A monotonically increasing GaugeDesk policy epoch.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PolicyEpoch(u64);

impl PolicyEpoch {
    /// Epoch zero is reserved for "no admitted policy" and cannot identify a run.
    pub fn new(value: u64) -> Result<Self, PolicyAdmissionError> {
        if value == 0 {
            Err(PolicyAdmissionError::InvalidEpoch)
        } else {
            Ok(Self(value))
        }
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// A policy epoch only after WhippleScript has verified its signed envelope.
///
/// The verified envelope stays opaque. Callers may retain the stable identity
/// for commands and receipts, and may ask WhippleScript whether it governs a
/// resource, but cannot inspect or reinterpret WhippleScript's security model.
pub struct AdmittedPolicyEpoch {
    epoch: PolicyEpoch,
    policy_ref: PolicyEpochRef,
    envelope: VerifiedEnvelope,
}

impl AdmittedPolicyEpoch {
    /// Cross the production trust boundary. Unsigned, malformed, and tampered
    /// envelopes fail closed; the epoch is never admitted without an attestation.
    pub fn verify(epoch: PolicyEpoch, signed_envelope: &str) -> Result<Self, PolicyAdmissionError> {
        let envelope = VerifiedEnvelope::verify_signed_text(signed_envelope)
            .map_err(PolicyAdmissionError::EnvelopeRejected)?;
        let policy_ref = PolicyEpochRef::from_verified(epoch.get(), &envelope)
            .map_err(PolicyAdmissionError::Protocol)?;
        Ok(Self {
            epoch,
            policy_ref,
            envelope,
        })
    }

    /// Production embedding trust boundary: require a cryptographic GaugeDesk
    /// root attestation, then retain the exact WhippleScript policy identity.
    pub fn verify_with(
        epoch: PolicyEpoch,
        signed_envelope: &str,
        verifier: &GovernanceRootVerifier,
    ) -> Result<Self, PolicyAdmissionError> {
        let envelope = VerifiedEnvelope::verify_signed_text_with(signed_envelope, verifier)
            .map_err(PolicyAdmissionError::EnvelopeRejected)?;
        let policy_ref = PolicyEpochRef::from_verified(epoch.get(), &envelope)
            .map_err(PolicyAdmissionError::Protocol)?;
        Ok(Self {
            epoch,
            policy_ref,
            envelope,
        })
    }

    pub fn epoch(&self) -> PolicyEpoch {
        self.epoch
    }

    /// The canonical WhippleScript envelope hash to place on runtime commands and
    /// require back on evidence receipts.
    pub fn envelope_hash(&self) -> &str {
        &self.policy_ref.envelope_hash
    }

    /// The governance signer WhippleScript verified.
    pub fn signer(&self) -> &str {
        &self.policy_ref.signer
    }

    /// The cryptographic governance root bound to this epoch. `None` exists only
    /// for the legacy CLI hash-attestation path.
    pub fn key_id(&self) -> Option<&str> {
        self.policy_ref.key_id.as_deref()
    }

    /// The WhippleScript-owned identity placed unchanged on commands, events, and
    /// receipts. GaugeDesk does not define a parallel wire representation.
    pub fn protocol_ref(&self) -> &PolicyEpochRef {
        &self.policy_ref
    }

    /// Delegate resource-coverage questions to WhippleScript's verified model.
    pub fn governs(&self, resource: &str) -> bool {
        self.envelope.governs(resource)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyAdmissionError {
    InvalidEpoch,
    EnvelopeRejected(String),
    Protocol(ProtocolError),
}

impl fmt::Display for PolicyAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEpoch => formatter.write_str("policy epoch must be greater than zero"),
            Self::EnvelopeRejected(message) => {
                write!(
                    formatter,
                    "WhippleScript governance envelope rejected: {message}"
                )
            }
            Self::Protocol(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PolicyAdmissionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    #[derive(Debug)]
    struct TestCredentialCapability {
        credential_ref: String,
    }

    impl CredentialCapability for TestCredentialCapability {
        fn credential_ref(&self) -> &str {
            &self.credential_ref
        }

        fn resolve(
            &self,
            credential_ref: &str,
        ) -> io::Result<gaugewright_harness::CredentialMaterial> {
            if credential_ref != self.credential_ref {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "wrong ref"));
            }
            Ok(gaugewright_harness::CredentialMaterial::new(
                "test-key", None,
            ))
        }
    }

    fn test_credential_capability() -> Arc<dyn CredentialCapability> {
        Arc::new(TestCredentialCapability {
            credential_ref: "gaugedesk:credential:account:openai".to_owned(),
        })
    }

    fn signed_envelope() -> String {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("governance test env lock");
        std::env::set_var("WHIPPLESCRIPT_GOV_ADMIN", "test");
        let result = SignedEnvelope::sign(
            "grant file_store project -> file:/workspace readable by Operator from Operator\n",
            "gaugedesk-admin",
        )
        .expect("test governance agent signs")
        .to_json();
        std::env::remove_var("WHIPPLESCRIPT_GOV_ADMIN");
        drop(guard);
        result
    }

    fn signed_harness_policy() -> String {
        let principal = ResourcePolicy {
            principal: true,
            ..ResourcePolicy::default()
        };
        let ordinary = ResourcePolicy::default();
        let policy = HostGovernancePolicy {
            resources: std::collections::BTreeMap::from([
                ("file:workspace:chat-1".to_owned(), ordinary.clone()),
                ("memory:turn-images:chat-1".to_owned(), ordinary),
                ("command:workspace:chat-1".to_owned(), principal.clone()),
                ("human:owner".to_owned(), principal.clone()),
                ("provider:openai".to_owned(), principal.clone()),
                ("provider:owned".to_owned(), principal.clone()),
                ("placement:local".to_owned(), principal),
            ]),
            bindings: std::collections::BTreeMap::from([
                ("project".to_owned(), "file:workspace:chat-1".to_owned()),
                (
                    "turn_images".to_owned(),
                    "memory:turn-images:chat-1".to_owned(),
                ),
                ("command".to_owned(), "command:workspace:chat-1".to_owned()),
                ("human".to_owned(), "human:owner".to_owned()),
                ("model".to_owned(), "provider:openai".to_owned()),
                ("owned".to_owned(), "provider:owned".to_owned()),
                ("local".to_owned(), "placement:local".to_owned()),
            ]),
            capabilities: BTreeSet::from([
                "workspace.read".to_owned(),
                "workspace.write".to_owned(),
                "command.run".to_owned(),
                "human.ask".to_owned(),
            ]),
            provider_bindings: std::collections::BTreeMap::from([(
                "model".to_owned(),
                ProviderBindingPolicy {
                    provider: "openai".to_owned(),
                    model: "gpt-test".to_owned(),
                    base_url: "https://api.openai.com".to_owned(),
                    credential_ref: "gaugedesk:credential:account:openai".to_owned(),
                },
            )]),
            placements: std::collections::BTreeMap::from([(
                "local".to_owned(),
                WhipplePlacementPolicy {
                    kind: "local".to_owned(),
                    provider_bindings: BTreeSet::from(["model".to_owned()]),
                    command_network: false,
                },
            )]),
            ..HostGovernancePolicy::default()
        };
        let authority = AuthorityId::new("authority:owner");
        let key = SigningKey::from_seed(&[7u8; 32]).expect("key");
        sign_policy_envelope(&policy.to_json().expect("policy"), &authority, &key)
            .expect("signed harness policy")
    }

    #[test]
    fn admits_only_a_signed_whipplescript_envelope_and_keeps_its_identity() {
        let signed = signed_envelope();
        let admitted = AdmittedPolicyEpoch::verify(PolicyEpoch::new(7).expect("epoch"), &signed)
            .expect("signed envelope admits");
        assert_eq!(admitted.epoch().get(), 7);
        assert_eq!(admitted.signer(), "gaugedesk-admin");
        assert_eq!(admitted.envelope_hash().len(), 64);
        assert_eq!(admitted.protocol_ref().epoch, 7);
        assert!(admitted.governs("project"));
    }

    #[test]
    fn unsigned_tampered_and_zero_epoch_inputs_fail_closed() {
        assert_eq!(PolicyEpoch::new(0), Err(PolicyAdmissionError::InvalidEpoch));
        let epoch = PolicyEpoch::new(1).expect("epoch");
        assert!(AdmittedPolicyEpoch::verify(
            epoch,
            "grant file_store project -> file:/workspace public\n"
        )
        .is_err());

        let signed = signed_envelope();
        let tampered = signed.replace("file:/workspace", "file:/elsewhere");
        assert_ne!(tampered, signed);
        assert!(AdmittedPolicyEpoch::verify(epoch, &tampered).is_err());
    }

    #[test]
    fn gaugedesk_uses_whipplescripts_policy_bound_command_and_receipt_types() {
        let admitted =
            AdmittedPolicyEpoch::verify(PolicyEpoch::new(9).expect("epoch"), &signed_envelope())
                .expect("policy");
        let command = StartTurnCommand {
            protocol: HOST_PROTOCOL.to_owned(),
            command_id: "turn-command-9".to_owned(),
            run_ref: "gaugedesk:run:9".to_owned(),
            instance_ref: "whip:instance:9".to_owned(),
            package_version_ref: "whip:package-version:9".to_owned(),
            policy: admitted.protocol_ref().clone(),
            actor_ref: "authority:owner".to_owned(),
            input: TurnInput {
                text: "inspect the project".to_owned(),
                images: Vec::new(),
            },
            resources: vec![ResourceRef {
                handle: "gaugedesk:resource:project".to_owned(),
                kind: "file_store".to_owned(),
                selector: None,
            }],
            provider_binding: ProviderBindingRef {
                binding_id: "gaugedesk:provider:primary".to_owned(),
                credential: CredentialRef {
                    credential_id: "gaugedesk:credential:account:openai".to_owned(),
                },
            },
            placement_ceiling_ref: "gaugedesk:placement:local".to_owned(),
        };
        command.validate().expect("command");

        let receipt = TurnReceipt {
            protocol: HOST_PROTOCOL.to_owned(),
            command_id: command.command_id.clone(),
            run_ref: command.run_ref.clone(),
            instance_ref: command.instance_ref.clone(),
            policy: command.policy.clone(),
            terminal_position: EventPosition {
                instance_ref: command.instance_ref.clone(),
                sequence: 1,
            },
            status: TurnStatus::Completed,
            output_handle: Some("whip:output:9".to_owned()),
            usage_ref: "whip:evidence:usage:9".to_owned(),
            guarantee_report_ref: "whip:evidence:guarantee:9".to_owned(),
            workspace_cut_ref: None,
        };
        receipt.validate_for(&command).expect("receipt");
    }

    #[test]
    fn gaugedesk_root_signs_and_whipplescript_verifies_without_admin_env() {
        let authority = AuthorityId::new("authority:owner");
        let key = SigningKey::from_seed(&[7u8; 32]).expect("root key");
        let config = "grant file_store project -> file:/workspace readable by Operator\n";
        let signed = sign_policy_envelope(config, &authority, &key).expect("signed");
        let verifier = GovernanceRootVerifier::new(authority.clone(), key.public_key());
        let admitted = AdmittedPolicyEpoch::verify_with(
            PolicyEpoch::new(11).expect("epoch"),
            &signed,
            &verifier,
        )
        .expect("cryptographic policy admission");

        assert_eq!(admitted.signer(), authority.as_str());
        assert_eq!(admitted.key_id(), Some(key.public_key().as_str()));
        assert_eq!(
            admitted.protocol_ref().key_id,
            Some(key.public_key().to_string())
        );
        assert!(admitted.governs("project"));

        let other = SigningKey::from_seed(&[8u8; 32]).expect("other key");
        let wrong_root = GovernanceRootVerifier::new(authority, other.public_key());
        assert!(AdmittedPolicyEpoch::verify_with(
            PolicyEpoch::new(11).expect("epoch"),
            &signed,
            &wrong_root,
        )
        .is_err());
    }

    #[test]
    fn whip_harness_reopens_the_same_instance_and_owns_workspace_tools() {
        let root = tempfile::tempdir().expect("runtime root");
        let worktree = tempfile::tempdir().expect("worktree");
        let package_root = worktree.path().join(".whipple/versions/1");
        std::fs::create_dir_all(&package_root).expect("method dir");
        std::fs::write(
            package_root.join("package.json"),
            r#"{
  "schema":"whipplescript.agent_package.v0",
  "source":"method.whip",
  "workflow":"Method",
  "agent":"assistant",
  "system_prompt":"persona.md",
  "capabilities":["workspace.read","workspace.write","command.run","human.ask"],
  "max_steps":32
}"#,
        )
        .expect("manifest");
        std::fs::write(
            package_root.join("method.whip"),
            r#"
file store project { root "." allow read ["**"] allow write ["**"] }
workflow Method {
  agent assistant {
    provider owned
    profile "repo-writer"
    capacity 1
    capabilities ["workspace.read", "workspace.write", "command.run", "human.ask"]
  }
  rule converse when started => {
    tell assistant requires ["workspace.read", "workspace.write", "command.run", "human.ask"]
      with access to project { read ["**"] write ["**"] }
      with access to command { run }
      with access to human { ask }
      "Run."
  }
}
"#,
        )
        .expect("source");
        std::fs::write(package_root.join("persona.md"), "Use the project method.").expect("method");
        let package_ref = AuthoredAgentPackage::load(&package_root)
            .expect("package")
            .version_ref()
            .to_owned();
        let spec = HarnessSpec {
            chat_id: "chat-1".to_owned(),
            worktree: worktree.path().to_path_buf(),
            mode: gaugewright_harness::ChatMode::Use,
            package_root: Some(package_root.clone()),
            package_version_ref: Some(package_ref.clone()),
            policy_epoch: Some(1),
            signed_policy_envelope: Some(signed_harness_policy()),
            provider_binding_ref: Some("model".to_owned()),
            credential_ref: Some("gaugedesk:credential:account:openai".to_owned()),
            placement_ceiling_ref: Some("local".to_owned()),
            provider: Some("openai".to_owned()),
            model: Some("gpt-test".to_owned()),
            thinking: None,
            system_prompt: None,
            credential_capability: Some(test_credential_capability()),
            credentials: vec![("OPENAI_API_KEY".to_owned(), "test-key".to_owned())],
            sandbox: gaugewright_harness::sandbox::SandboxPolicy::new(vec![worktree
                .path()
                .to_path_buf()])
            .read_only(vec![worktree.path().join(".whipple")])
            .filter_egress(vec!["api.openai.com".to_owned()]),
        };
        let factory = WhipHarnessFactory::new(
            AuthorityId::new("authority:owner"),
            SigningKey::from_seed(&[7u8; 32]).expect("key"),
            root.path(),
        );
        let first = factory.create_harness(&spec).expect("first harness");
        assert_eq!(first.package.version_ref(), package_ref);
        assert!(first
            .package
            .capabilities()
            .iter()
            .any(|capability| capability == "human.ask"));
        assert!(
            native_workspace_tool_specs_with_capabilities(true, true, true)
                .iter()
                .any(|tool| tool.name == "ask_human")
        );
        assert!(first
            .new_turn_command("question", &[], 1)
            .resources
            .iter()
            .any(|resource| resource.kind == "human"));
        assert!(native_workspace_tool_specs(true)
            .iter()
            .any(|tool| tool.name == "write"));
        assert!(native_workspace_tool_specs_with_command(true, true)
            .iter()
            .any(|tool| tool.name == "bash"));
        let admitted_command = AdmittedCommand {
            command: "git status".to_owned(),
            workspace_root: worktree.path().to_path_buf(),
            read_only_paths: vec![worktree.path().join(".whipple")],
            timeout: Duration::from_secs(30),
        };
        let command_policy = command_sandbox_policy(&spec.sandbox, &admitted_command);
        assert_eq!(command_policy.network, Network::Deny);
        assert_eq!(
            command_policy.writable_roots,
            vec![worktree.path().to_path_buf()]
        );
        assert_eq!(
            command_policy.read_only_roots,
            vec![worktree.path().join(".whipple")]
        );
        assert!(first.interrupt_handle().is_some());
        let instance = first.instance_ref.clone();
        drop(first);
        let reopened = factory.create_harness(&spec).expect("reopened harness");
        assert_eq!(reopened.instance_ref, instance);

        let respondent = AuthorityId::new("authority:authenticated-member");
        let mut attributed = factory.create_harness(&spec).expect("attributed harness");
        attributed.bind_authenticated_actor(respondent.as_str());
        assert_eq!(attributed.respondent_ref, respondent.as_str());

        let target_worktree = tempfile::tempdir().expect("target worktree");
        let target_package_root = target_worktree.path().join(".whipple/versions/1");
        std::fs::create_dir_all(&target_package_root).expect("target package parent");
        for file in ["package.json", "method.whip", "persona.md"] {
            std::fs::copy(package_root.join(file), target_package_root.join(file))
                .expect("target package file");
        }
        let source_continuity = HarnessContinuitySpec {
            chat_id: spec.chat_id.clone(),
            worktree: spec.worktree.clone(),
            mode: spec.mode,
            package_root: Some(package_root),
            package_version_ref: Some(package_ref.clone()),
            system_prompt: None,
            policy_epoch: spec.policy_epoch,
            signed_policy_envelope: spec.signed_policy_envelope.clone(),
        };
        let target_continuity = HarnessContinuitySpec {
            chat_id: "chat-2".to_owned(),
            worktree: target_worktree.path().to_path_buf(),
            mode: spec.mode,
            package_root: Some(target_package_root),
            package_version_ref: Some(package_ref),
            system_prompt: None,
            policy_epoch: spec.policy_epoch,
            signed_policy_envelope: spec.signed_policy_envelope.clone(),
        };
        factory
            .clone_continuity(&source_continuity, &target_continuity)
            .expect("governed fork");
        factory
            .clone_continuity(&source_continuity, &target_continuity)
            .expect("governed fork replay");
        let target_spec = HarnessSpec {
            chat_id: target_continuity.chat_id.clone(),
            worktree: target_continuity.worktree.clone(),
            sandbox: gaugewright_harness::sandbox::SandboxPolicy::new(vec![target_continuity
                .worktree
                .clone()])
            .read_only(vec![target_continuity.worktree.join(".whipple")])
            .filter_egress(vec!["api.openai.com".to_owned()]),
            ..spec.clone()
        };
        let forked = factory
            .create_harness(&target_spec)
            .expect("forked harness reopens");
        assert_ne!(forked.instance_ref, instance);
        assert!(
            forked
                .runtime
                .current_position(&forked.instance_ref)
                .expect("fork position")
                .sequence
                >= 3
        );

        let mut isolated = spec.clone();
        isolated.sandbox.network = Network::Deny;
        let error = ProviderConfig::from_spec(&isolated)
            .err()
            .expect("isolation must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);

        let mut wrong_endpoint = spec;
        wrong_endpoint.sandbox.allowed_hosts = vec!["example.com".to_owned()];
        let error = ProviderConfig::from_spec(&wrong_endpoint)
            .err()
            .expect("provider endpoint must be explicitly admitted");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn whip_factory_requires_gaugedesk_owned_codex_material() {
        let factory = WhipHarnessFactory::new(
            AuthorityId::new("authority:owner"),
            SigningKey::from_seed(&[7u8; 32]).expect("key"),
            ".",
        );
        assert!(matches!(
            factory.credential_status("openai-codex", None),
            CredentialProbe::Missing(reason) if reason.contains("GaugeDesk-owned")
        ));
        assert_eq!(
            factory.credential_status("openai-codex", Some(test_credential_capability().as_ref())),
            CredentialProbe::Ready
        );
    }
}

//! Open library/workspace state helpers for agents, projects, placements, chats, search, and pairing.

use std::collections::BTreeMap;

use gaugewright_core::attestation::{AttestationQuote, CodeMeasurement};
use gaugewright_core::boundary_lifecycle::{
    BoundaryCommand, BoundaryPhase, BoundaryState, Operator, Placement, PlacementPolicy,
};
use gaugewright_core::ids::{BridgeGrantId, DeviceId};
use gaugewright_core::instance::{InstanceCommand, InstanceState};
use gaugewright_core::merge::MergeState;
use gaugewright_core::run::RunState;
use gaugewright_core::workstream::WorkstreamState;
use gaugewright_harness::HarnessFactory;
use gaugewright_store::{AdmitError, Store};
use gaugewright_workspace::{ChatWorkspace, Instance, MergeOutcome, Workspace, WorkspaceError};

use crate::attestation_verifier::{LoopbackVerifier, QuoteVerifier, RealQuoteVerifierError};
use crate::boundary_keeper::{accept_boundary_attested, AcceptError};
use crate::library::{
    Admission, AgentRecord, ChatRecord, InstanceKind, InstanceRecord, ProjectRecord, RecordOp,
    WorkstreamRecord, LIBRARY_SCOPE,
};
use crate::workbench_state::{provider_for, WorkspaceProviders};
use crate::{
    io, library, library_routes, AttestationMode, Workbench, DEFAULT_AGENT, DEFAULT_INSTANCE,
    DEFAULT_PLACEMENT, DEFAULT_PROJECT,
};

pub(crate) enum AgentDeleteError {
    DefaultAgent,
    NotFound,
    BoundElsewhere,
}

pub(crate) enum PullArchetypeError {
    NotFound,
    NotFork,
    SourceMissing,
    SourceNotOpen,
    ForkNotOpen,
    Workspace(WorkspaceError),
}

pub(crate) enum BindPlacementError {
    ProjectNotFound,
    AgentNotFound,
    Create(String),
}

pub(crate) enum UpgradePlacementError {
    PlacementNotFound,
    ArchetypeNotFound,
}

pub(crate) enum CreateArchetypeChatError {
    ArchetypeNotFound,
    Create(String),
}

pub(crate) struct CreatedArchetype {
    pub(crate) id: String,
    pub(crate) name: String,
}

pub(crate) enum CreateArchetypeError {
    Create(String),
}

pub(crate) enum ForkArchetypeError {
    NotFound,
    SourceNotOpen,
    Create(String),
}

pub(crate) struct ForkedChat {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) forked_from: String,
}

pub(crate) enum ForkChatError {
    NotFound,
    SourceNotLive,
    InstanceNotOpen,
    Create(String),
}

pub(crate) struct CreatedPairingRequest {
    pub(crate) pairing_id: String,
    pub(crate) device: String,
    pub(crate) bridge_grant: String,
    pub(crate) status: serde_json::Value,
}

pub(crate) struct BoundaryAttestationInput {
    pub(crate) measurement: String,
    pub(crate) nonce: String,
    pub(crate) expected_nonce: Option<String>,
    pub(crate) quote_bytes: Vec<u8>,
    pub(crate) vcek: Vec<u8>,
    pub(crate) sealed_key_id: Option<String>,
}

pub(crate) enum BoundaryAcceptError {
    PolicyRejected,
    Rejected(String),
    Store(AdmitError),
    QuoteRejected(String),
    MissingVcek,
    RealVerifierUnavailable,
    InvalidEndorsement(String),
}

pub(crate) struct StartupLibraryState {
    pub(crate) library: crate::library::Library,
    pub(crate) instances: BTreeMap<String, Box<dyn Workspace>>,
    pub(crate) engagements: BTreeMap<String, Box<dyn ChatWorkspace>>,
    pub(crate) engagement_index: BTreeMap<String, String>,
}

pub(crate) fn activate_instance(store: &mut Store, inst_id: &str) {
    let _ = store.admit::<InstanceState>(inst_id, InstanceCommand::PinVersion("v0".into()));
}

pub(crate) fn load_startup_library_state(
    store: &mut Store,
    instances_dir: &std::path::Path,
    providers: &WorkspaceProviders,
) -> std::io::Result<StartupLibraryState> {
    let mut library = crate::library::Library::rebuild(store).map_err(io)?;
    if library.is_empty() {
        seed_default_agent(store, &mut library, instances_dir, providers)?;
    }
    repair_legacy_default_instance(store, &library);
    ensure_default_project_and_placement(store, &mut library, instances_dir, providers)?;
    let (instances, engagements, engagement_index) =
        open_startup_instances(&library, instances_dir, providers)?;
    Ok(StartupLibraryState {
        library,
        instances,
        engagements,
        engagement_index,
    })
}

fn repair_legacy_default_instance(store: &mut Store, library: &crate::library::Library) {
    if library.instances.contains_key(DEFAULT_INSTANCE)
        && store
            .fold::<InstanceState>(DEFAULT_INSTANCE)
            .map(|s| s.pinned_version.is_none())
            .unwrap_or(false)
    {
        activate_instance(store, DEFAULT_INSTANCE);
    }
}

/// What startup reconciliation yields: open instances, live engagements, and the
/// chat-id → instance-id engagement index.
type StartupInstances = (
    BTreeMap<String, Box<dyn Workspace>>,
    BTreeMap<String, Box<dyn ChatWorkspace>>,
    BTreeMap<String, String>,
);

fn open_startup_instances(
    library: &crate::library::Library,
    instances_dir: &std::path::Path,
    providers: &WorkspaceProviders,
) -> std::io::Result<StartupInstances> {
    let mut instances = BTreeMap::new();
    let mut engagements = BTreeMap::new();
    let mut engagement_index = BTreeMap::new();
    let track_chats = !library.chats.is_empty();
    for inst_rec in library.instances.values() {
        let inst = provider_for(providers, &inst_rec.id).open_at(&instances_dir.join(&inst_rec.id));
        let existing = inst.reconcile_engagements().map_err(io)?;
        for (chat_id, eng) in existing {
            if track_chats && !library.chats.contains_key(&chat_id) {
                let _ = inst.remove_engagement(&chat_id);
                continue;
            }
            engagement_index.insert(chat_id.clone(), inst_rec.id.clone());
            engagements.insert(chat_id, eng);
        }
        instances.insert(inst_rec.id.clone(), inst);
    }
    Ok((instances, engagements, engagement_index))
}

/// Seed a fresh library (ADR 0035/0036): the default **archetype** + its authoring
/// repo, plus the hidden default **Personal project** and a default **placement**.
pub(crate) fn seed_default_agent(
    store: &mut Store,
    library: &mut crate::library::Library,
    instances_dir: &std::path::Path,
    providers: &WorkspaceProviders,
) -> std::io::Result<()> {
    let seed_repo = |id: &str| -> std::io::Result<()> {
        let inst = provider_for(providers, id)
            .init_at(&instances_dir.join(id))
            .map_err(io)?;
        let files = crate::app_support::default_agent_definition().seed_files();
        let files: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, content)| (path.as_str(), content.as_str()))
            .collect();
        inst.seed_main(&files).map_err(io)?;
        Ok(())
    };

    seed_repo(DEFAULT_INSTANCE)?;
    let inst_rec = InstanceRecord {
        id: DEFAULT_INSTANCE.into(),
        op: RecordOp::Upsert,
        kind: InstanceKind::Authoring,
        agent_id: DEFAULT_AGENT.into(),
        project_id: None,
        version: 1,
        admission: Admission::Active,
    };
    store
        .append_record(
            LIBRARY_SCOPE,
            "instance",
            &serde_json::to_string(&inst_rec).unwrap(),
        )
        .map_err(io)?;
    library.apply_instance(inst_rec);
    activate_instance(store, DEFAULT_INSTANCE);

    let agent = AgentRecord {
        id: DEFAULT_AGENT.into(),
        op: RecordOp::Upsert,
        name: "assistant".into(),
        instance_id: DEFAULT_INSTANCE.into(),
        config: "{}".into(),
        current_version: 1,
        auto_upgrade: false,
        forked_from: None,
    };
    store
        .append_record(
            LIBRARY_SCOPE,
            "agent",
            &serde_json::to_string(&agent).unwrap(),
        )
        .map_err(io)?;
    library.apply_agent(agent);

    let proj = ProjectRecord {
        id: DEFAULT_PROJECT.into(),
        op: RecordOp::Upsert,
        name: "Personal".into(),
        is_default: true,
        network_isolated: false,
        deployment_mode: None,
    };
    store
        .append_record(
            LIBRARY_SCOPE,
            "project",
            &serde_json::to_string(&proj).unwrap(),
        )
        .map_err(io)?;
    library.apply_project(proj);

    seed_repo(DEFAULT_PLACEMENT)?;
    let placement = InstanceRecord {
        id: DEFAULT_PLACEMENT.into(),
        op: RecordOp::Upsert,
        kind: InstanceKind::Using,
        agent_id: DEFAULT_AGENT.into(),
        project_id: Some(DEFAULT_PROJECT.into()),
        version: 1,
        admission: Admission::Active,
    };
    store
        .append_record(
            LIBRARY_SCOPE,
            "instance",
            &serde_json::to_string(&placement).unwrap(),
        )
        .map_err(io)?;
    library.apply_instance(placement);
    activate_instance(store, DEFAULT_PLACEMENT);
    Ok(())
}

/// Self-heal a store seeded before the ADR 0036 reset, which has the default
/// archetype but no hidden Personal project / default placement.
pub(crate) fn ensure_default_project_and_placement(
    store: &mut Store,
    library: &mut crate::library::Library,
    instances_dir: &std::path::Path,
    providers: &WorkspaceProviders,
) -> std::io::Result<()> {
    if !library.projects.contains_key(DEFAULT_PROJECT) {
        let proj = ProjectRecord {
            id: DEFAULT_PROJECT.into(),
            op: RecordOp::Upsert,
            name: "Personal".into(),
            is_default: true,
            network_isolated: false,
            deployment_mode: None,
        };
        store
            .append_record(
                LIBRARY_SCOPE,
                "project",
                &serde_json::to_string(&proj).unwrap(),
            )
            .map_err(io)?;
        library.apply_project(proj);
    }
    if !library.instances.contains_key(DEFAULT_PLACEMENT) {
        let dir = instances_dir.join(DEFAULT_PLACEMENT);
        let inst = provider_for(providers, DEFAULT_PLACEMENT)
            .init_at(&dir)
            .map_err(io)?;
        let files = crate::app_support::default_agent_definition().seed_files();
        let files: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, content)| (path.as_str(), content.as_str()))
            .collect();
        inst.seed_main(&files).map_err(io)?;
        let placement = InstanceRecord {
            id: DEFAULT_PLACEMENT.into(),
            op: RecordOp::Upsert,
            kind: InstanceKind::Using,
            agent_id: DEFAULT_AGENT.into(),
            project_id: Some(DEFAULT_PROJECT.into()),
            version: 1,
            admission: Admission::Active,
        };
        store
            .append_record(
                LIBRARY_SCOPE,
                "instance",
                &serde_json::to_string(&placement).unwrap(),
            )
            .map_err(io)?;
        library.apply_instance(placement);
        activate_instance(store, DEFAULT_PLACEMENT);
    } else if store
        .fold::<InstanceState>(DEFAULT_PLACEMENT)
        .map(|s| s.pinned_version.is_none())
        .unwrap_or(false)
    {
        activate_instance(store, DEFAULT_PLACEMENT);
    }
    Ok(())
}

impl Workbench {
    pub(crate) fn apply_startup_library_state(&mut self, state: StartupLibraryState) {
        self.instances = state.instances;
        self.engagements = state.engagements;
        self.engagement_index = state.engagement_index;
        self.library = state.library;
        self.default_instance = DEFAULT_PLACEMENT.to_string();
    }

    /// A workbench with one registered instance, made the default create target —
    /// the convenient shape for tests and the back-compat `POST /chats`.
    pub fn with_instance(inst_id: impl Into<String>, instance: Instance, store: Store) -> Self {
        let inst_id = inst_id.into();
        let mut wb = Self::new(store);
        // Anchor the instances root from the one registered instance (instances
        // live at `<instances-root>/<id>/repo`) — the test-constructor stand-in
        // for the build path, which records it from `prepare_workbench_root`.
        if let Some(root) = instance.repo().parent().and_then(|p| p.parent()) {
            wb.instances_root = root.to_path_buf();
        }
        wb.instances.insert(inst_id.clone(), Box::new(instance));
        wb.default_instance = inst_id;
        wb
    }

    /// Rebuild the cached library projection from the store. Federation handoff
    /// import paths call this after writing relocated library records so the
    /// project appears in this authority's local projection.
    pub fn rebuild_library(&mut self) {
        if let Ok(lib) = crate::library::Library::rebuild(self.store_ref()) {
            self.library = lib;
        }
    }

    pub(crate) fn library_project_display_name(&self, project_id: &str) -> String {
        self.library
            .projects
            .get(project_id)
            .map(|project| project.name.clone())
            .unwrap_or_else(|| project_id.to_string())
    }

    pub(crate) fn library_project_of_chat(&self, chat_id: &str) -> Option<String> {
        self.library.project_of_chat(chat_id).map(str::to_string)
    }

    pub(crate) fn library_chat_network_isolated(&self, chat_id: &str) -> bool {
        self.library.chat_network_isolated(chat_id)
    }

    pub(crate) fn library_has_instance_record(&self, id: &str) -> bool {
        self.library.instances.contains_key(id)
    }

    pub(crate) fn library_fork_forest(&self) -> Vec<crate::library::ForkNode> {
        self.library.fork_forest()
    }

    pub(crate) fn library_chat_mode(&self, chat_id: &str) -> crate::library::ChatMode {
        self.library
            .chats
            .get(chat_id)
            .and_then(|chat| self.library.instances.get(&chat.instance_id))
            .map(|instance| instance.kind.chat_mode())
            .unwrap_or_default()
    }

    pub(crate) fn library_project_relocation_content_bundles(
        &self,
        project: &str,
    ) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        for inst_rec in self.library.using_instances_of(project) {
            match self.instances.get(&inst_rec.id) {
                Some(inst) => match inst.export() {
                    Ok(export) => out.push((inst_rec.id.clone(), export.0)),
                    Err(e) => {
                        tracing::warn!("handoff: cannot bundle instance {}: {e}", inst_rec.id)
                    }
                },
                None => tracing::warn!("handoff: no live repo for instance {}", inst_rec.id),
            }
        }
        out
    }

    fn library_op_str(op: RecordOp) -> &'static str {
        match op {
            RecordOp::Upsert => "upsert",
            RecordOp::Tombstone => "tombstone",
        }
    }

    pub(crate) fn write_agent_record(&mut self, record: AgentRecord) -> i64 {
        let id = record.id.clone();
        let op = Self::library_op_str(record.op);
        let pos = self
            .store_mut()
            .append_record(
                LIBRARY_SCOPE,
                "agent",
                &serde_json::to_string(&record).unwrap(),
            )
            .unwrap_or(0);
        self.library.apply_agent(record);
        self.notify_library_changed("archetype", &id, op);
        pos
    }

    pub(crate) fn write_project_record(&mut self, record: ProjectRecord) {
        let id = record.id.clone();
        let op = Self::library_op_str(record.op);
        let _ = self.store_mut().append_record(
            LIBRARY_SCOPE,
            "project",
            &serde_json::to_string(&record).unwrap(),
        );
        self.library.apply_project(record);
        self.notify_library_changed("project", &id, op);
    }

    pub(crate) fn write_instance_record(&mut self, record: InstanceRecord) {
        let id = record.id.clone();
        let op = Self::library_op_str(record.op);
        let _ = self.store_mut().append_record(
            LIBRARY_SCOPE,
            "instance",
            &serde_json::to_string(&record).unwrap(),
        );
        self.library.apply_instance(record);
        self.notify_library_changed("placement", &id, op);
    }

    pub(crate) fn write_chat_record(&mut self, record: ChatRecord) {
        let id = record.id.clone();
        let op = Self::library_op_str(record.op);
        let _ = self.store_mut().append_record(
            LIBRARY_SCOPE,
            "chat",
            &serde_json::to_string(&record).unwrap(),
        );
        self.library.apply_chat(record);
        self.notify_library_changed("chat", &id, op);
    }

    pub(crate) fn write_created_chat_record(&mut self, record: ChatRecord) {
        let id = record.id.clone();
        let op = Self::library_op_str(record.op);
        let position = self
            .store_mut()
            .append_record(
                LIBRARY_SCOPE,
                "chat",
                &serde_json::to_string(&record).unwrap(),
            )
            .unwrap_or(0);
        self.library.apply_chat(ChatRecord {
            created_position: position,
            ..record
        });
        self.notify_library_changed("chat", &id, op);
    }

    pub(crate) fn library_workstream_ids(&self) -> Vec<String> {
        self.library.workstreams.keys().cloned().collect()
    }

    pub(crate) fn write_workstream_record(&mut self, record: WorkstreamRecord) -> i64 {
        let id = record.id.clone();
        let op = Self::library_op_str(record.op);
        let position = self
            .store_mut()
            .append_record(
                LIBRARY_SCOPE,
                "workstream",
                &serde_json::to_string(&record).unwrap(),
            )
            .unwrap_or(0);
        self.library.apply_workstream(record);
        self.notify_library_changed("workstream", &id, op);
        position
    }

    pub(crate) fn library_restamp_workstream_position(
        &mut self,
        workstream_id: &str,
        position: i64,
    ) {
        if let Some(record) = self.library.workstreams.get(workstream_id).cloned() {
            self.library.apply_workstream(WorkstreamRecord {
                created_position: position,
                ..record
            });
        }
    }

    pub(crate) fn library_workstreams_in(&self, instance_id: &str) -> Vec<&WorkstreamRecord> {
        self.library.workstreams_in(instance_id)
    }

    pub(crate) fn library_workstream(&self, workstream_id: &str) -> Option<WorkstreamRecord> {
        self.library.workstreams.get(workstream_id).cloned()
    }

    pub(crate) fn library_has_workstream(&self, workstream_id: &str) -> bool {
        self.library.workstreams.contains_key(workstream_id)
    }

    pub(crate) fn create_instance_workstream_ref(
        &self,
        instance_id: &str,
        workstream_id: &str,
    ) -> std::io::Result<()> {
        let Some(instance) = self.instances.get(instance_id) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no such placement",
            ));
        };
        instance.create_workstream(workstream_id).map_err(io)
    }

    pub(crate) fn promote_instance_workstream_ref_to_main(
        &self,
        workstream_id: &str,
        instance_id: &str,
    ) -> std::io::Result<MergeOutcome> {
        let Some(instance) = self.instances.get(instance_id) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "placement not open",
            ));
        };
        instance
            .promote_workstream_to_main(workstream_id)
            .map_err(io)
    }

    #[cfg(test)]
    pub(crate) fn seed_boundary_for_test(
        &mut self,
        boundary_id: &str,
        participants: std::collections::BTreeSet<String>,
        placement: Placement,
    ) -> Result<(), AdmitError> {
        self.store_mut()
            .admit::<BoundaryState>(boundary_id, BoundaryCommand::Propose(participants))?;
        self.store_mut()
            .admit::<BoundaryState>(boundary_id, BoundaryCommand::DeclareCeiling(placement))?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn seed_attested_boundary_release_for_test(
        &mut self,
        build_image: &str,
        build_version: &str,
        measurement: CodeMeasurement,
        sealed_key_id: &str,
        sealed_key: Vec<u8>,
    ) {
        self.measurements
            .register(crate::measurement_store::MeasurementRecord::new(
                crate::measurement_store::BuildId::new(build_image, build_version),
                measurement.clone(),
            ));
        self.sealed_keys
            .seal(gaugewright_core::key_release::SealedKeyRecord::new(
                sealed_key_id,
                measurement,
                sealed_key,
            ));
    }

    #[cfg(test)]
    pub(crate) fn seed_org_placement_policy_for_test(
        &mut self,
        policy: crate::org::PlacementPolicyRecord,
    ) -> Result<(), AdmitError> {
        self.store_mut().append_record(
            crate::org::ORG_SCOPE,
            "placement_policy",
            &serde_json::to_string(&policy).unwrap(),
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn write_chat_transcript_event(
        &mut self,
        chat_id: &str,
        event: crate::stream::ServerEvent,
    ) -> Result<(), AdmitError> {
        self.store_mut()
            .append_record(chat_id, "transcript", &event.to_json())?;
        Ok(())
    }

    pub(crate) fn instances_dir(&self) -> std::path::PathBuf {
        self.instances_root.clone()
    }

    pub(crate) fn destroy_chat(&mut self, chat_id: &str) {
        if let Some(proc) = self.sessions.remove(chat_id) {
            let _ = proc.shutdown();
        }
        if let Some(inst_id) = self.engagement_index.remove(chat_id) {
            if let Some(inst) = self.instances.get(&inst_id) {
                let _ = inst.remove_engagement(chat_id);
            }
        }
        self.engagements.remove(chat_id);
        self.streams.remove(chat_id);
        if let Some(existing) = self.library.chats.get(chat_id).cloned() {
            self.write_chat_record(ChatRecord {
                op: RecordOp::Tombstone,
                ..existing
            });
        }
    }

    pub(crate) fn destroy_instance(&mut self, inst_id: &str) {
        let chat_ids: Vec<String> = self
            .library
            .chats
            .values()
            .filter(|c| c.instance_id == inst_id)
            .map(|c| c.id.clone())
            .collect();
        for chat_id in chat_ids {
            self.destroy_chat(&chat_id);
        }
        let dir = self.instances_dir().join(inst_id);
        self.instances.remove(inst_id);
        let _ = std::fs::remove_dir_all(dir);
        if let Some(existing) = self.library.instances.get(inst_id).cloned() {
            self.write_instance_record(InstanceRecord {
                op: RecordOp::Tombstone,
                ..existing
            });
        }
    }

    fn merge_agent_config(base: &str, overlay: &str) -> String {
        let base_json = serde_json::from_str::<serde_json::Value>(base)
            .unwrap_or_else(|_| serde_json::json!({}));
        let overlay_json = serde_json::from_str::<serde_json::Value>(overlay)
            .unwrap_or_else(|_| serde_json::json!({}));
        match (base_json, overlay_json) {
            (serde_json::Value::Object(mut base_map), serde_json::Value::Object(overlay_map)) => {
                for (key, value) in overlay_map {
                    base_map.insert(key, value);
                }
                serde_json::to_string(&serde_json::Value::Object(base_map))
                    .unwrap_or_else(|_| base.to_string())
            }
            _ => base.to_string(),
        }
    }

    pub(crate) fn create_chat_in_instance(
        &mut self,
        inst_id: &str,
        title: &str,
    ) -> Result<serde_json::Value, String> {
        let Some(inst_rec) = self.library.instances.get(inst_id).cloned() else {
            return Err("no such instance".into());
        };
        // APPROVE-1 (ADR 0064): a placement hosts work chats only while active. A pending
        // placement (approved-but-not-yet-accepted under an approval-required policy) is
        // refused up front — fail closed until the project owner accepts it.
        if inst_rec.admission == Admission::Pending {
            return Err("placement is pending approval — accept it before starting a chat".into());
        }
        let kind = inst_rec.kind.chat_kind();
        if !self
            .store_ref()
            .fold::<InstanceState>(inst_id)
            .map(|s| s.runnable)
            .unwrap_or(false)
        {
            return Err("instance is not runnable (suspended or torn down)".into());
        }
        let base_cfg = self
            .library
            .agents
            .get(&inst_rec.agent_id)
            .map(|a| a.config.clone())
            .unwrap_or_else(|| "{}".into());
        let inst_state = self.store_ref().fold::<InstanceState>(inst_id).ok();
        let cfg = match inst_state
            .as_ref()
            .and_then(|s| s.local_config.as_deref())
            .filter(|c| !c.trim().is_empty())
        {
            Some(overlay) => Self::merge_agent_config(&base_cfg, overlay),
            None => base_cfg,
        };
        let notes = inst_state
            .as_ref()
            .and_then(|s| s.notes.clone())
            .unwrap_or_default();
        let Some(inst) = self.instances.get(inst_id) else {
            return Err("instance not open".into());
        };
        let chat_id = library::gen_id("chat");
        let eng = inst
            .create_engagement(&chat_id)
            .map_err(|e| e.to_string())?;
        let _ = eng.write_file(gaugewright_boundary::definition::CONFIG_PATH, &cfg);
        if !notes.trim().is_empty() {
            let _ = eng.write_file("CLAUDE.md", &notes);
        }
        let rec = ChatRecord {
            id: chat_id.clone(),
            op: RecordOp::Upsert,
            instance_id: inst_id.to_string(),
            title: title.to_string(),
            created_position: 0,
            forked_from: None,
        };
        let pos = self
            .store_mut()
            .append_record(LIBRARY_SCOPE, "chat", &serde_json::to_string(&rec).unwrap())
            .unwrap_or(0);
        let rec = ChatRecord {
            created_position: pos,
            ..rec
        };
        self.library.apply_chat(rec);
        self.notify_library_changed("chat", &chat_id, "upsert");
        self.register_engagement(chat_id.clone(), inst_id.to_string(), eng);
        Ok(serde_json::json!({ "id": chat_id, "title": title, "kind": kind }))
    }

    pub(crate) fn agent_record(&self, id: &str) -> Option<AgentRecord> {
        self.library.agents.get(id).cloned()
    }

    // `pub` for the hosted embed plane (`cloud/embed-host`): a public session's
    // engagement seeds the served placement's archetype config.
    pub fn agent_config_for_instance(&self, instance_id: &str) -> Option<String> {
        self.library
            .instances
            .get(instance_id)
            .and_then(|inst_rec| self.library.agents.get(&inst_rec.agent_id))
            .map(|agent| agent.config.clone())
    }

    pub(crate) fn update_agent_record(
        &mut self,
        id: &str,
        name: Option<String>,
        config: Option<String>,
    ) -> Option<AgentRecord> {
        let existing = self.library.agents.get(id).cloned()?;
        let updated = AgentRecord {
            name: name.unwrap_or(existing.name),
            config: config.unwrap_or(existing.config),
            ..existing
        };
        self.write_agent_record(updated.clone());
        Some(updated)
    }

    pub(crate) fn delete_agent_cascade(&mut self, id: &str) -> Result<(), AgentDeleteError> {
        if id == DEFAULT_AGENT {
            return Err(AgentDeleteError::DefaultAgent);
        }
        let Some(agent) = self.library.agents.get(id).cloned() else {
            return Err(AgentDeleteError::NotFound);
        };
        let bound_elsewhere = self.library.instances.values().any(|instance| {
            instance.agent_id == id
                && instance.kind == InstanceKind::Using
                && instance.project_id.as_deref() != Some(DEFAULT_PROJECT)
        });
        if bound_elsewhere {
            return Err(AgentDeleteError::BoundElsewhere);
        }
        let personal: Vec<String> = self
            .library
            .instances
            .values()
            .filter(|instance| {
                instance.agent_id == id
                    && instance.kind == InstanceKind::Using
                    && instance.project_id.as_deref() == Some(DEFAULT_PROJECT)
            })
            .map(|instance| instance.id.clone())
            .collect();
        for instance_id in personal {
            self.destroy_instance(&instance_id);
        }
        self.destroy_instance(&agent.instance_id);
        if let Some(existing) = self.library.agents.get(id).cloned() {
            self.write_agent_record(AgentRecord {
                op: RecordOp::Tombstone,
                ..existing
            });
        }
        Ok(())
    }

    pub(crate) fn pull_archetype_from_source(
        &mut self,
        id: &str,
    ) -> Result<MergeOutcome, PullArchetypeError> {
        let Some(fork) = self.library.agents.get(id).cloned() else {
            return Err(PullArchetypeError::NotFound);
        };
        let Some(source_id) = fork.forked_from.clone() else {
            return Err(PullArchetypeError::NotFork);
        };
        let Some(source) = self.library.agents.get(&source_id).cloned() else {
            return Err(PullArchetypeError::SourceMissing);
        };
        let Some(src) = self
            .instances
            .get(&source.instance_id)
            .map(|instance| instance.peer_source())
        else {
            return Err(PullArchetypeError::SourceNotOpen);
        };
        let Some(fork_inst) = self.instances.get(&fork.instance_id) else {
            return Err(PullArchetypeError::ForkNotOpen);
        };
        let outcome = fork_inst
            .pull_from(&src)
            .map_err(PullArchetypeError::Workspace)?;
        if matches!(outcome, MergeOutcome::Clean) {
            self.notify_library_changed("agent", id, "upsert");
        }
        Ok(outcome)
    }

    pub(crate) fn project_home_value(&self, id: &str) -> Option<serde_json::Value> {
        if !self.library.projects.contains_key(id) {
            return None;
        }
        let placements = self.library.using_instances_of(id).len();
        let chats = self.library.project_chats(id);
        let mut recent_runs = Vec::new();
        let mut outputs = Vec::new();
        let mut events_total = 0usize;
        for chat in chats.iter() {
            let run = self
                .store_ref()
                .fold::<RunState>(&chat.id)
                .unwrap_or_default();
            recent_runs.push(serde_json::json!({
                "chat": chat.id,
                "title": chat.title,
                "phase": run.phase,
                "ran": run.admitted_once,
            }));
            let merge = self
                .store_ref()
                .fold::<MergeState>(&chat.id)
                .unwrap_or_default();
            if !matches!(
                merge.phase,
                gaugewright_core::merge::MergePhase::Idle
                    | gaugewright_core::merge::MergePhase::Clean
            ) {
                outputs.push(serde_json::json!({
                    "chat": chat.id,
                    "title": chat.title,
                    "phase": merge.phase,
                }));
            }
            events_total += self
                .store_ref()
                .events(&chat.id)
                .map(|events| events.len())
                .unwrap_or(0);
        }
        recent_runs.sort_by(|left, right| {
            right["ran"]
                .as_u64()
                .unwrap_or(0)
                .cmp(&left["ran"].as_u64().unwrap_or(0))
        });
        Some(serde_json::json!({
            "project_id": id,
            "recent_runs": recent_runs,
            "outputs": outputs,
            "audit": {
                "placements": placements,
                "chats": chats.len(),
                "events": events_total,
            },
        }))
    }

    pub(crate) fn update_project_record(
        &mut self,
        id: &str,
        name: Option<String>,
        network_isolated: Option<bool>,
        deployment_mode: Option<Placement>,
    ) -> Option<ProjectRecord> {
        let existing = self.library.projects.get(id).cloned()?;
        let updated = ProjectRecord {
            name: name.unwrap_or_else(|| existing.name.clone()),
            network_isolated: network_isolated.unwrap_or(existing.network_isolated),
            deployment_mode: deployment_mode.or(existing.deployment_mode),
            ..existing
        };
        self.write_project_record(updated.clone());
        Some(updated)
    }

    pub(crate) fn delete_project_cascade(&mut self, id: &str) -> bool {
        let Some(project) = self.library.projects.get(id).cloned() else {
            return false;
        };
        let instance_ids: Vec<String> = self
            .library
            .using_instances_of(id)
            .iter()
            .map(|instance| instance.id.clone())
            .collect();
        for instance_id in instance_ids {
            self.destroy_instance(&instance_id);
        }
        self.write_project_record(ProjectRecord {
            op: RecordOp::Tombstone,
            ..project
        });
        true
    }

    pub(crate) fn create_archetype(
        &mut self,
        name: String,
    ) -> Result<CreatedArchetype, CreateArchetypeError> {
        let agent_id = library::gen_id("agent");
        let inst_id = library::gen_id("inst");
        let dir = self.instances_dir().join(&inst_id);
        let provider = self.workspace_provider(&inst_id);
        provider
            .init_at(&dir)
            .map_err(|error| CreateArchetypeError::Create(error.to_string()))?;
        self.instances
            .insert(inst_id.clone(), provider.open_at(&dir));
        self.write_instance_record(InstanceRecord {
            id: inst_id.clone(),
            op: RecordOp::Upsert,
            kind: InstanceKind::Authoring,
            agent_id: agent_id.clone(),
            project_id: None,
            version: 1,
            admission: Admission::Active,
        });
        activate_instance(self.store_mut(), &inst_id);
        self.write_agent_record(AgentRecord {
            id: agent_id.clone(),
            op: RecordOp::Upsert,
            name: name.clone(),
            instance_id: inst_id,
            config: "{}".into(),
            current_version: 1,
            auto_upgrade: false,
            forked_from: None,
        });
        let _ = self.place_archetype_on_project(
            DEFAULT_PROJECT,
            &agent_id,
            crate::library::Admission::Active,
        );
        Ok(CreatedArchetype { id: agent_id, name })
    }

    pub(crate) fn fork_archetype(
        &mut self,
        id: &str,
        name: Option<String>,
    ) -> Result<CreatedArchetype, ForkArchetypeError> {
        let Some(src) = self.library.agents.get(id).cloned() else {
            return Err(ForkArchetypeError::NotFound);
        };
        let Some(src_source) = self
            .instances
            .get(&src.instance_id)
            .map(|instance| instance.peer_source())
        else {
            return Err(ForkArchetypeError::SourceNotOpen);
        };
        let new_agent = library::gen_id("agent");
        let new_inst = library::gen_id("inst");
        let dir = self.instances_dir().join(&new_inst);
        let inst = self
            .workspace_provider(&new_inst)
            .fork_from_at(&dir, &src_source)
            .map_err(|error| ForkArchetypeError::Create(error.to_string()))?;
        self.instances.insert(new_inst.clone(), inst);
        self.write_instance_record(InstanceRecord {
            id: new_inst.clone(),
            op: RecordOp::Upsert,
            kind: InstanceKind::Authoring,
            agent_id: new_agent.clone(),
            project_id: None,
            version: 1,
            admission: Admission::Active,
        });
        activate_instance(self.store_mut(), &new_inst);
        let name = name.unwrap_or_else(|| format!("{} (fork)", src.name));
        self.write_agent_record(AgentRecord {
            id: new_agent.clone(),
            op: RecordOp::Upsert,
            name: name.clone(),
            instance_id: new_inst,
            config: src.config.clone(),
            current_version: 1,
            auto_upgrade: false,
            forked_from: Some(src.id.clone()),
        });
        let _ = self.place_archetype_on_project(
            DEFAULT_PROJECT,
            &new_agent,
            crate::library::Admission::Active,
        );
        Ok(CreatedArchetype {
            id: new_agent,
            name,
        })
    }

    pub(crate) fn place_archetype_on_project(
        &mut self,
        project_id: &str,
        agent_id: &str,
        admission: Admission,
    ) -> Result<String, String> {
        let inst_id = library::gen_id("inst");
        self.place_archetype_on_project_with_id(project_id, agent_id, &inst_id, admission)
    }

    pub(crate) fn place_archetype_on_project_with_id(
        &mut self,
        project_id: &str,
        agent_id: &str,
        inst_id: &str,
        admission: Admission,
    ) -> Result<String, String> {
        let inst_id = inst_id.to_string();
        let dir = self.instances_dir().join(&inst_id);
        let provider = self.workspace_provider(&inst_id);
        provider.init_at(&dir).map_err(|error| error.to_string())?;
        self.instances
            .insert(inst_id.clone(), provider.open_at(&dir));
        let version = self
            .library
            .agents
            .get(agent_id)
            .map(|agent| agent.current_version)
            .unwrap_or(1);
        self.write_instance_record(InstanceRecord {
            id: inst_id.clone(),
            op: RecordOp::Upsert,
            kind: InstanceKind::Using,
            agent_id: agent_id.to_string(),
            project_id: Some(project_id.to_string()),
            version,
            admission,
        });
        let _ = self
            .store_mut()
            .admit::<InstanceState>(&inst_id, InstanceCommand::PinVersion("v0".into()));
        Ok(inst_id)
    }

    /// Bind a library agent into a project as a placement (`APPROVE-1`, ADR 0064). The
    /// placement's admission is **policy-gated**: under an approval-required project
    /// policy it enters `Pending` (awaiting the owner's accept); by default it is
    /// frictionless and lands `Active` at once.
    pub(crate) fn bind_agent_to_project(
        &mut self,
        project_id: &str,
        agent_id: &str,
    ) -> Result<String, BindPlacementError> {
        if !self.library.projects.contains_key(project_id) {
            return Err(BindPlacementError::ProjectNotFound);
        }
        if !self.library.agents.contains_key(agent_id) {
            return Err(BindPlacementError::AgentNotFound);
        }
        let admission = if self.require_archetype_approval() {
            Admission::Pending
        } else {
            Admission::Active
        };
        self.place_archetype_on_project(project_id, agent_id, admission)
            .map_err(BindPlacementError::Create)
    }

    /// The effective per-org archetype-approval policy (`APPROVE-1`, ADR 0064): when set,
    /// an explicitly-placed archetype must be accepted by the project owner before it is
    /// usable. Read from the org projection; defaults to frictionless (`false`).
    pub(crate) fn require_archetype_approval(&self) -> bool {
        crate::org::Org::rebuild(self.store_ref())
            .map(|org| org.effective_require_archetype_approval())
            .unwrap_or(false)
    }

    /// **Accept** a pending placement (`APPROVE-1`, ADR 0064): the project owner's second
    /// explicit act flips it `Pending → Active`, so it can host work chats and appear in
    /// the chat picker. Accepting an already-active placement is idempotent. Returns
    /// `None` for an unknown placement.
    pub(crate) fn accept_placement(&mut self, inst_id: &str) -> Option<Admission> {
        let mut placement = self.library.instances.get(inst_id).cloned()?;
        placement.op = RecordOp::Upsert;
        placement.admission = Admission::Active;
        self.write_instance_record(placement);
        Some(Admission::Active)
    }

    pub(crate) fn publish_archetype_version(
        &mut self,
        id: &str,
        auto_upgrade: Option<bool>,
    ) -> Option<(u64, u64)> {
        let mut agent = self.library.agents.get(id).cloned()?;
        if let Some(auto_upgrade) = auto_upgrade {
            agent.auto_upgrade = auto_upgrade;
        }
        agent.op = RecordOp::Upsert;
        agent.current_version += 1;
        let new_version = agent.current_version;
        let owner_auto = agent.auto_upgrade;
        self.write_agent_record(agent);
        let org_allows = crate::org::Org::rebuild(self.store_ref())
            .map(|org| org.allow_auto_upgrade())
            .unwrap_or(false);
        let mut auto_upgraded = 0u64;
        if owner_auto && org_allows {
            let behind: Vec<InstanceRecord> = self
                .library
                .instances
                .values()
                .filter(|instance| {
                    instance.agent_id == id
                        && matches!(instance.kind, InstanceKind::Using)
                        && instance.version < new_version
                })
                .cloned()
                .collect();
            for mut placement in behind {
                placement.op = RecordOp::Upsert;
                placement.version = new_version;
                self.write_instance_record(placement);
                auto_upgraded += 1;
            }
        }
        Some((new_version, auto_upgraded))
    }

    pub(crate) fn upgrade_placement_version(
        &mut self,
        id: &str,
    ) -> Result<u64, UpgradePlacementError> {
        let Some(mut placement) = self.library.instances.get(id).cloned() else {
            return Err(UpgradePlacementError::PlacementNotFound);
        };
        let Some(agent) = self.library.agents.get(&placement.agent_id).cloned() else {
            return Err(UpgradePlacementError::ArchetypeNotFound);
        };
        placement.op = RecordOp::Upsert;
        placement.version = agent.current_version;
        let version = placement.version;
        self.write_instance_record(placement);
        Ok(version)
    }

    pub(crate) fn unbind_instance(&mut self, id: &str) -> bool {
        if !self.library.instances.contains_key(id) {
            return false;
        }
        self.destroy_instance(id);
        true
    }

    pub(crate) fn create_chat_under_agent(
        &mut self,
        agent_id: &str,
        title: &str,
    ) -> Result<serde_json::Value, CreateArchetypeChatError> {
        let Some(agent) = self.library.agents.get(agent_id).cloned() else {
            return Err(CreateArchetypeChatError::ArchetypeNotFound);
        };
        self.create_chat_in_instance(&agent.instance_id, title)
            .map_err(CreateArchetypeChatError::Create)
    }

    pub(crate) fn use_archetype_chat(
        &mut self,
        agent_id: &str,
        title: &str,
    ) -> Result<serde_json::Value, CreateArchetypeChatError> {
        if !self.library.agents.contains_key(agent_id) {
            return Err(CreateArchetypeChatError::ArchetypeNotFound);
        }
        let existing = self
            .library
            .instances
            .values()
            .find(|instance| {
                instance.kind == InstanceKind::Using
                    && instance.agent_id == agent_id
                    && instance.project_id.as_deref() == Some(DEFAULT_PROJECT)
            })
            .map(|instance| instance.id.clone());
        let placement_id = match existing {
            Some(placement_id) => placement_id,
            None => self
                .place_archetype_on_project(DEFAULT_PROJECT, agent_id, Admission::Active)
                .map_err(CreateArchetypeChatError::Create)?,
        };
        self.create_chat_in_instance(&placement_id, title)
            .map_err(CreateArchetypeChatError::Create)
    }

    pub(crate) fn fork_chat(&mut self, id: &str) -> Result<ForkedChat, ForkChatError> {
        let Some(src_chat) = self.library.chats.get(id).cloned() else {
            return Err(ForkChatError::NotFound);
        };
        let inst_id = src_chat.instance_id.clone();
        let (src_path, files): (std::path::PathBuf, Vec<(String, String)>) = {
            let Some(src_eng) = self.engagements.get(id) else {
                return Err(ForkChatError::SourceNotLive);
            };
            let files = src_eng
                .tree()
                .unwrap_or_default()
                .into_iter()
                .filter(|file| !file.is_dir)
                .filter_map(|file| src_eng.read_file(&file.path).ok().map(|c| (file.path, c)))
                .collect();
            (src_eng.path().to_path_buf(), files)
        };
        let new_id = library::gen_id("chat");
        let (new_eng, new_path) = {
            let Some(inst) = self.instances.get(&inst_id) else {
                return Err(ForkChatError::InstanceNotOpen);
            };
            let eng = inst
                .create_engagement(&new_id)
                .map_err(|error| ForkChatError::Create(error.to_string()))?;
            for (path, content) in &files {
                let _ = eng.write_file(path, content);
            }
            let _ = eng.commit_turn(&format!("forked from {id}"));
            let path = eng.path().to_path_buf();
            (eng, path)
        };
        // Continuity is harness state (its session sibling + path rebinds), so the
        // fork clones it through the factory hook. The CONFIGURED REAL factory is
        // called unconditionally — not `factory_for_turn()` — because today's copy
        // happens regardless of fake mode, and a fake-mode fork of a previously-real
        // chat must keep copying its session. Best-effort, exactly as before: a
        // fork never fails on continuity.
        let _ = gaugewright_pi_bridge::PiHarnessFactory
            .clone_continuity(id, &new_id, &src_path, &new_path);
        self.register_engagement(new_id.clone(), inst_id.clone(), new_eng);
        let title = format!("{} (fork)", src_chat.title);
        let rec = ChatRecord {
            id: new_id.clone(),
            op: RecordOp::Upsert,
            instance_id: inst_id,
            title: title.clone(),
            created_position: 0,
            forked_from: Some(id.to_string()),
        };
        let pos = self
            .store
            .append_record(LIBRARY_SCOPE, "chat", &serde_json::to_string(&rec).unwrap())
            .unwrap_or(0);
        self.library.apply_chat(ChatRecord {
            created_position: pos,
            ..rec
        });
        self.notify_library_changed("chat", &new_id, "upsert");
        Ok(ForkedChat {
            id: new_id,
            title,
            forked_from: id.to_string(),
        })
    }

    pub(crate) fn delete_chat_cascade(&mut self, id: &str) -> bool {
        if !self.engagement_index.contains_key(id) && !self.library.chats.contains_key(id) {
            return false;
        }
        // Capture the hosting instance before teardown drops the index entry, so we can
        // purge its git objects after the engagement's branch is gone (SECAUD-6).
        let inst_id = self.engagement_index.get(id).cloned();
        self.destroy_chat(id);
        self.crypto_erase_content(id);
        // SECAUD-6: crypto-erase the git-blob workspace content too — `destroy_chat` removed
        // the engagement branch, so its unique objects are now unreachable; prune them so the
        // deleted chat's workspace content is unrecoverable, matching the store crypto-erasure.
        if let Some(inst) = inst_id.and_then(|iid| self.instances.get(&iid)) {
            let _ = inst.purge_unreachable_objects();
        }
        true
    }

    pub(crate) fn rename_chat_record(&mut self, id: &str, title: String) -> Option<ChatRecord> {
        let existing = self.library.chats.get(id).cloned()?;
        let updated = ChatRecord { title, ..existing };
        self.write_chat_record(updated.clone());
        Some(updated)
    }

    fn library_chat_status(&self, chat_id: &str) -> (bool, bool) {
        if !self.engagement_index.contains_key(chat_id) {
            return (false, false);
        }
        let phase = self
            .store_ref()
            .fold::<MergeState>(chat_id)
            .map(|merge| merge.phase)
            .unwrap_or(gaugewright_core::merge::MergePhase::Idle);
        (
            phase == gaugewright_core::merge::MergePhase::Clean,
            phase == gaugewright_core::merge::MergePhase::Repairing,
        )
    }

    fn library_chat_json(
        &self,
        chat: &ChatRecord,
        chat_ws: &std::collections::BTreeMap<String, String>,
    ) -> serde_json::Value {
        let kind = self
            .library
            .instances
            .get(&chat.instance_id)
            .map(|instance| instance.kind.chat_kind())
            .unwrap_or("work");
        let (changes, conflict) = self.library_chat_status(&chat.id);
        serde_json::json!({
            "id": chat.id,
            "title": chat.title,
            "kind": kind,
            "forked_from": chat.forked_from,
            "placement": chat.instance_id,
            "workstream": chat_ws.get(&chat.id),
            "changes": changes,
            "conflict": conflict,
        })
    }

    pub(crate) fn workspace_value(&self) -> serde_json::Value {
        let lib = &self.library;
        let mut chat_ws: std::collections::BTreeMap<String, String> = Default::default();
        for workstream in lib.workstreams.values() {
            if let Ok(state) = self.store_ref().fold::<WorkstreamState>(&workstream.id) {
                for member in state.members {
                    chat_ws.insert(member, workstream.id.clone());
                }
            }
        }

        let archetypes: Vec<_> = lib
            .agents
            .values()
            .map(|agent| {
                serde_json::json!({
                    "id": agent.id,
                    "name": agent.name,
                    "instance_id": agent.instance_id,
                    "is_default": agent.id == DEFAULT_AGENT,
                    "forked_from": agent.forked_from,
                    "forked_from_name": agent.forked_from.as_ref().and_then(|src| lib.agents.get(src).map(|source| source.name.clone())),
                    "chats": lib.chats_in(&agent.instance_id).iter().map(|chat| self.library_chat_json(chat, &chat_ws)).collect::<Vec<_>>(),
                    "workstreams": lib.workstreams_in(&agent.instance_id).iter().map(|workstream| crate::workstream_routes::workstream_json(self, workstream)).collect::<Vec<_>>(),
                })
            })
            .collect();

        let projects: Vec<_> = lib
            .projects
            .values()
            .filter(|project| !project.is_default)
            .map(|project| {
                let placements: Vec<_> = lib
                    .using_instances_of(&project.id)
                    .iter()
                    .map(|instance| {
                        let archetype_name = lib
                            .agents
                            .get(&instance.agent_id)
                            .map(|agent| agent.name.clone())
                            .unwrap_or_default();
                        let inst_state = self.store_ref().fold::<InstanceState>(&instance.id).ok();
                        let pinned_version = inst_state
                            .as_ref()
                            .and_then(|state| state.pinned_version.clone());
                        let has_config = inst_state
                            .as_ref()
                            .map(|state| {
                                state
                                    .local_config
                                    .as_deref()
                                    .map(|config| !config.trim().is_empty())
                                    .unwrap_or(false)
                                    || state
                                        .notes
                                        .as_deref()
                                        .map(|notes| !notes.trim().is_empty())
                                        .unwrap_or(false)
                            })
                            .unwrap_or(false);
                        let current_version = lib
                            .agents
                            .get(&instance.agent_id)
                            .map(|agent| agent.current_version)
                            .unwrap_or(instance.version);
                        serde_json::json!({
                            "placement_id": instance.id,
                            "archetype_id": instance.agent_id,
                            "archetype_name": archetype_name,
                            "is_default": instance.id == library_routes::general_placement_id(&project.id),
                            "has_config": has_config,
                            "pinned_version": pinned_version,
                            "version": instance.version,
                            "current_version": current_version,
                            "upgrade_available": lib.upgrade_available(&instance.id),
                            // APPROVE-1 (ADR 0064): a pending placement is approved-but-not-yet-accepted
                            // under an approval-required policy — the nav flags it so the owner can accept.
                            "pending": instance.admission == Admission::Pending,
                            "chats": lib.chats_in(&instance.id).iter().map(|chat| self.library_chat_json(chat, &chat_ws)).collect::<Vec<_>>(),
                            "workstreams": lib.workstreams_in(&instance.id).iter().map(|workstream| crate::workstream_routes::workstream_json(self, workstream)).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                serde_json::json!({
                    "id": project.id,
                    "name": project.name,
                    "network_isolated": project.network_isolated,
                    "placements": placements,
                })
            })
            .collect();

        let mut recent: Vec<&ChatRecord> = lib.chats.values().collect();
        recent.sort_by_key(|chat| std::cmp::Reverse(chat.created_position));
        let recent: Vec<_> = recent
            .into_iter()
            .map(|chat| {
                let inst = lib.instances.get(&chat.instance_id);
                let archetype_name = inst
                    .and_then(|instance| lib.agents.get(&instance.agent_id))
                    .map(|agent| agent.name.clone())
                    .unwrap_or_default();
                let kind = inst
                    .map(|instance| instance.kind.chat_kind())
                    .unwrap_or("work");
                let (changes, conflict) = self.library_chat_status(&chat.id);
                serde_json::json!({
                    "id": chat.id,
                    "title": chat.title,
                    "archetype": archetype_name,
                    "kind": kind,
                    "forked_from": chat.forked_from,
                    "placement": chat.instance_id,
                    "workstream": chat_ws.get(&chat.id),
                    "changes": changes,
                    "conflict": conflict,
                })
            })
            .collect();

        let workstreams: Vec<_> = lib
            .workstreams
            .values()
            .map(|workstream| crate::workstream_routes::workstream_json(self, workstream))
            .collect();

        serde_json::json!({
            "archetypes": archetypes,
            "projects": projects,
            "recent": recent,
            "workstreams": workstreams,
            "personal_placement": DEFAULT_PLACEMENT,
        })
    }

    /// SEARCH-2 file-content walk bounds. A per-query worktree walk (NOT a persistent
    /// file index): the WhippleScript workspace swap (v0.5.0) brings the proper indexing
    /// primitive, so an index now would be throwaway migration — a bounded walk is correct
    /// at current scale (the SCALE-* items are deferred as "fine at current scale"). These
    /// caps keep the walk from being "materially heavier" than folding the chat log:
    /// at most `FILE_SEARCH_MAX_FILES` files per chat, `FILE_SEARCH_MAX_BYTES` read per file.
    pub(crate) const FILE_SEARCH_MAX_FILES: usize = 500;
    pub(crate) const FILE_SEARCH_MAX_BYTES: usize = 256 * 1024;

    /// The full content-search projection (`navigation.md` "Search scope and relevance"):
    /// the **chat-log** tier (tier 2, SEARCH-1) followed by the **file-content** tier
    /// (tier 3, SEARCH-2), each hit carrying its `tier` so the nav preserves title > log >
    /// file ordering. Both tiers are server projections (`INV-5`, projection-first): the
    /// client never folds transcripts nor walks worktrees. A chat that already matched in
    /// the log tier is not repeated as a file hit — the stronger (log) tier wins per chat.
    pub(crate) fn search_value(&self, query: &str) -> serde_json::Value {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return serde_json::json!({ "hits": [] });
        }
        let mut chats: Vec<&ChatRecord> = self.library.chats.values().collect();
        chats.sort_by_key(|chat| std::cmp::Reverse(chat.created_position));

        // Tier 2 — chat log: fold each chat's transcript records and substring-match.
        let mut hits = Vec::new();
        let mut logged: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for chat in &chats {
            let Ok(rows) = self.store_ref().records(&chat.id, "transcript") else {
                continue;
            };
            let mut hay = String::new();
            for row in &rows {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(row) else {
                    continue;
                };
                for key in ["text", "delta"] {
                    if let Some(text) = value.get(key).and_then(|item| item.as_str()) {
                        hay.push_str(text);
                        hay.push('\n');
                    }
                }
            }
            if let Some(index) = hay.to_lowercase().find(&needle) {
                logged.insert(chat.id.as_str());
                hits.push(serde_json::json!({
                    "id": chat.id,
                    "title": chat.title,
                    "snippet": Self::snippet_around(&hay, index, needle.len()),
                    "tier": "log",
                }));
            }
        }

        // Tier 3 — file content: a bounded walk of each chat's worktree (SEARCH-2),
        // ranked after the log tier. Chats that already matched in the log are skipped
        // so each surfaces once via its strongest tier.
        for chat in &chats {
            if logged.contains(chat.id.as_str()) {
                continue;
            }
            if let Some(hit) = self.search_engagement_files(chat, &needle) {
                hits.push(hit);
            }
        }
        serde_json::json!({ "hits": hits })
    }

    /// SEARCH-2 tier-3 walk for one chat: enumerate its live worktree (relative paths,
    /// `.git` already skipped by [`ChatWorkspace::tree`]) and case-insensitively match the
    /// first file whose content contains `needle`, returning its path + a one-line snippet.
    /// Bounded per [`FILE_SEARCH_MAX_FILES`](Self::FILE_SEARCH_MAX_FILES) /
    /// [`FILE_SEARCH_MAX_BYTES`](Self::FILE_SEARCH_MAX_BYTES); binary files are skipped
    /// (null-byte sniff in `read_file_capped`). All reads go through the workspace's
    /// path-confined API, so the walk can never read outside the chat's worktree.
    fn search_engagement_files(
        &self,
        chat: &ChatRecord,
        needle: &str,
    ) -> Option<serde_json::Value> {
        let eng = self.engagements.get(&chat.id)?;
        let entries = eng.tree().ok()?;
        let mut scanned = 0usize;
        for entry in entries {
            if entry.is_dir {
                continue;
            }
            if scanned >= Self::FILE_SEARCH_MAX_FILES {
                break;
            }
            scanned += 1;
            let Ok(Some(text)) = eng.read_file_capped(&entry.path, Self::FILE_SEARCH_MAX_BYTES)
            else {
                continue;
            };
            if let Some(index) = text.to_lowercase().find(needle) {
                let snippet = Self::snippet_around(&text, index, needle.len());
                return Some(serde_json::json!({
                    "id": chat.id,
                    "title": chat.title,
                    "path": entry.path,
                    // The nav renders one snippet per hit (id → snippet); lead a file
                    // snippet with its path so the row shows which file matched.
                    "snippet": format!("{}: {}", entry.path, snippet),
                    "tier": "file",
                }));
            }
        }
        None
    }

    fn snippet_around(text: &str, match_byte: usize, match_len: usize) -> String {
        const PAD: usize = 48;
        let clamp_down = |mut index: usize| {
            while index > 0 && !text.is_char_boundary(index) {
                index -= 1;
            }
            index
        };
        let clamp_up = |mut index: usize| {
            let len = text.len();
            while index < len && !text.is_char_boundary(index) {
                index += 1;
            }
            index.min(len)
        };
        let start = clamp_down(match_byte.saturating_sub(PAD));
        let end = clamp_up((match_byte + match_len + PAD).min(text.len()));
        let mut snippet = String::new();
        if start > 0 {
            snippet.push('…');
        }
        snippet.push_str(text[start..end].trim());
        if end < text.len() {
            snippet.push('…');
        }
        snippet.replace('\n', " ")
    }

    /// The unified task-bar projection (ADR 0075 §3/§5): onboarding checklist
    /// `issue` tasks from the account-global whip tracker, followed by the
    /// existing clean-merge `review` tasks. It owns no truth — it joins the whip
    /// issue (content) with the acting authority (the v1 assignee). Every task
    /// carries `kind` ∈ {`issue`, `review`} and an `assignee` authority.
    pub(crate) fn task_queue_value(&self) -> serde_json::Value {
        // Solo v1: everything is assigned to the boundary owner (the acting
        // authority). A future multi-user pass assigns per-authority and filters
        // (ADR 0075 §4, deferred).
        let assignee = self.authority.as_str();

        // Onboarding issues first — the active first-run guidance. `list_items`
        // returns them in filing order (WS-1, WS-2, …), which is checklist order.
        let mut tasks: Vec<serde_json::Value> = Vec::new();
        if let Some(tracker) = self
            .tracker_runtimes
            .get(crate::workbench_state::ACCOUNT_GLOBAL_BOUNDARY)
        {
            match tracker.list_items(Some(crate::onboarding::ONBOARDING_QUEUE), Some("open")) {
                Ok(items) => {
                    for item in items {
                        tasks.push(serde_json::json!({
                            "id": item.id,
                            "title": item.title,
                            "agent": "",
                            "kind": "issue",
                            "assignee": assignee,
                        }));
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "task queue: could not list onboarding items");
                }
            }
        }

        // Review tasks: clean-merge chats awaiting keep/reject, current-first.
        let mut reviews: Vec<(i64, serde_json::Value)> = Vec::new();
        for chat in self.library.chats.values() {
            if !self.engagement_index.contains_key(&chat.id) {
                continue;
            }
            let phase = self
                .store
                .fold::<MergeState>(&chat.id)
                .map(|merge| merge.phase)
                .unwrap_or(gaugewright_core::merge::MergePhase::Idle);
            if phase == gaugewright_core::merge::MergePhase::Clean {
                let agent = self
                    .library
                    .instances
                    .get(&chat.instance_id)
                    .and_then(|instance| self.library.agents.get(&instance.agent_id))
                    .map(|agent| agent.name.clone())
                    .unwrap_or_default();
                reviews.push((
                    chat.created_position,
                    serde_json::json!({
                        "id": chat.id,
                        "title": chat.title,
                        "agent": agent,
                        "kind": "review",
                        "assignee": assignee,
                    }),
                ));
            }
        }
        reviews.sort_by_key(|(position, _)| std::cmp::Reverse(*position));
        tasks.extend(reviews.into_iter().map(|(_, task)| task));

        serde_json::json!({ "tasks": tasks })
    }

    fn pairing_status_json(state: &BoundaryState) -> serde_json::Value {
        let bound = state.device_binding.as_ref().map(|(device, grant)| {
            serde_json::json!({ "device": device.as_str(), "bridge_grant": grant.as_str() })
        });
        serde_json::json!({
            "phase": format!("{:?}", state.phase),
            "bound": bound,
            "paired": state.active(),
            "ceiling": library::BoundaryProjection::from_state(state),
        })
    }

    pub(crate) fn create_pairing_request(
        &mut self,
        device: String,
        bridge_grant: Option<String>,
    ) -> Result<CreatedPairingRequest, AdmitError> {
        let pairing_id = library::gen_id("pairing");
        let device = DeviceId::new(device);
        let grant = BridgeGrantId::new(bridge_grant.unwrap_or_else(|| library::gen_id("grant")));
        let required = std::collections::BTreeSet::from([self.authority().as_str().to_string()]);
        self.store_mut()
            .admit::<BoundaryState>(&pairing_id, BoundaryCommand::Propose(required))?;
        self.store_mut().admit::<BoundaryState>(
            &pairing_id,
            BoundaryCommand::DeclareCeiling(Placement {
                operator: Operator::Local,
                attested: false,
            }),
        )?;
        let state = self.store_mut().admit::<BoundaryState>(
            &pairing_id,
            BoundaryCommand::BindDevice {
                device: device.clone(),
                bridge_grant: grant.clone(),
            },
        )?;
        Ok(CreatedPairingRequest {
            pairing_id,
            device: device.as_str().to_string(),
            bridge_grant: grant.as_str().to_string(),
            status: Self::pairing_status_json(&state),
        })
    }

    pub(crate) fn pairing_status_value(
        &self,
        pairing_id: &str,
    ) -> Result<Option<serde_json::Value>, AdmitError> {
        let state = self.store_ref().fold::<BoundaryState>(pairing_id)?;
        if state.phase == BoundaryPhase::Init {
            return Ok(None);
        }
        Ok(Some(Self::pairing_status_json(&state)))
    }

    pub(crate) fn issue_boundary_challenge(
        &mut self,
        boundary_id: &str,
        participant: &str,
    ) -> Result<String, AdmitError> {
        let nonce = crate::challenge::fresh_nonce();
        crate::challenge::issue(self.store_mut(), boundary_id, participant, &nonce)?;
        Ok(nonce)
    }

    fn boundary_accept_value(
        state: &BoundaryState,
        participant: &str,
        released: Option<bool>,
    ) -> serde_json::Value {
        let mut out = serde_json::json!({
            "accepted": state.accepted.contains(participant),
            "active": state.active(),
            "ceiling": library::BoundaryProjection::from_state(state),
        });
        if let Some(released) = released {
            out["released"] = serde_json::json!(released);
        }
        out
    }

    pub(crate) fn accept_boundary(
        &mut self,
        boundary_id: &str,
        participant: String,
        attestation: Option<BoundaryAttestationInput>,
    ) -> Result<serde_json::Value, BoundaryAcceptError> {
        let placement_policy = crate::org::Org::rebuild(self.store_ref())
            .map_err(BoundaryAcceptError::Store)?
            .effective_placement_policy();
        if placement_policy != PlacementPolicy::open()
            && !crate::boundary_keeper::pairing_policy_admits(
                self.store_ref(),
                boundary_id,
                &placement_policy,
                attestation.is_some(),
            )
        {
            return Err(BoundaryAcceptError::PolicyRejected);
        }

        let (state, released) = match attestation {
            None => {
                let state = self
                    .store_mut()
                    .admit::<BoundaryState>(
                        boundary_id,
                        BoundaryCommand::Accept {
                            participant: participant.clone(),
                            evidence: None,
                        },
                    )
                    .map_err(|error| match error {
                        AdmitError::Rejected(rejection) => {
                            BoundaryAcceptError::Rejected(rejection.reason.to_string())
                        }
                        other => BoundaryAcceptError::Store(other),
                    })?;
                (state, None)
            }
            Some(att) => {
                let measurement = CodeMeasurement::new(att.measurement);
                let quote = AttestationQuote::new(measurement, att.nonce.clone(), att.quote_bytes);
                let expected =
                    match crate::challenge::current(self.store_ref(), boundary_id, &participant) {
                        Ok(Some(issued)) => issued,
                        Ok(None) => att.expected_nonce.unwrap_or(att.nonce),
                        Err(error) => return Err(BoundaryAcceptError::Store(error)),
                    };
                let allow_list = self.measurements.allow_list();
                let verifier: Box<dyn QuoteVerifier> = match self.attestation_mode() {
                    AttestationMode::Loopback => Box::new(LoopbackVerifier::new(allow_list)),
                    AttestationMode::RealRequired => {
                        if att.vcek.is_empty() {
                            return Err(BoundaryAcceptError::MissingVcek);
                        }
                        match self.real_quote_verifier(&att.vcek, allow_list) {
                            Ok(verifier) => verifier,
                            Err(RealQuoteVerifierError::Unavailable) => {
                                return Err(BoundaryAcceptError::RealVerifierUnavailable)
                            }
                            Err(RealQuoteVerifierError::InvalidEndorsement(reason)) => {
                                return Err(BoundaryAcceptError::InvalidEndorsement(reason))
                            }
                        }
                    }
                };
                let entitlement =
                    crate::package_flow::attested_run_verdict(self.store_ref(), boundary_id)
                        .map_err(BoundaryAcceptError::Store)?;
                let (store, sealed_keys) = self.store_mut_and_sealed_keys();
                let out = accept_boundary_attested(
                    store,
                    boundary_id,
                    &participant,
                    quote,
                    &expected,
                    &*verifier,
                    sealed_keys,
                    entitlement,
                    att.sealed_key_id.as_deref(),
                )
                .map_err(|error| match error {
                    AcceptError::QuoteRejected(reason) => {
                        BoundaryAcceptError::QuoteRejected(format!("{reason:?}"))
                    }
                    AcceptError::Boundary(rejection) => {
                        BoundaryAcceptError::Rejected(rejection.reason.to_string())
                    }
                    AcceptError::Store(error) => BoundaryAcceptError::Store(error),
                })?;
                if let Some(evidence) = out.state.attestation_evidence.get(&participant).cloned() {
                    let _ = crate::resource_store::release_sealed_keys(
                        store,
                        boundary_id,
                        boundary_id,
                        &participant,
                        &evidence,
                        entitlement,
                        sealed_keys,
                    );
                }
                (
                    out.state,
                    out.release.map(|decision| decision.is_released()),
                )
            }
        };

        Ok(Self::boundary_accept_value(&state, &participant, released))
    }
}

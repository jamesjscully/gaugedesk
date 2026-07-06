//! The agent/project library — the ADR-0027 data model as durable **records**.
//!
//! Agents, projects, instances, and chats are not lifecycle state; they are
//! durable *declarations* whose current value is the source of truth
//! (`data.md`). We store them as append-only records in one reserved scope
//! (`"library"`) and fold them **latest-wins by id** into a [`Library`]
//! projection: an `Upsert` sets/overwrites, a `Tombstone` removes. Because the
//! log is ordered, rename/config-edit/delete all fall out of "append a newer
//! record" — and the full history is preserved, leaving a seam for an
//! `agent-version` facet later (M1).
//!
//! - **agent** = a library-level reusable definition (its own authoring instance).
//! - **instance** = `(bound agent, workspace repo)` — the repo-owning unit.
//! - **project** = a grouping of *using* instances (agents bound into the project).
//! - **chat** = an engagement: a worktree off an instance's `main`.

use std::collections::BTreeMap;

use gaugewright_core::boundary_lifecycle::{BoundaryPhase, BoundaryState, Operator, Placement};
use gaugewright_store::{AdmitError, Store};
use serde::{Deserialize, Serialize};

/// The reserved store scope holding every library record.
pub const LIBRARY_SCOPE: &str = "library";

/// A record either declares the current value (`Upsert`) or retracts the id
/// (`Tombstone`). Folded latest-wins, so the last write per id wins.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum RecordOp {
    #[default]
    Upsert,
    Tombstone,
}

/// Whether an instance's workspace is the agent's own definition repo
/// (`Authoring`) or a project binding (`Using`). Purpose is read from this,
/// not modeled as a separate kind (ADR 0027).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum InstanceKind {
    Authoring,
    Using,
}

/// A placement's **admission state** (`APPROVE-1`, [ADR 0064](../decisions/0064-archetype-approval-two-acts.md)):
/// whether it is admitted for use. A placement hosts work chats and is offered in the
/// project's chat picker **only while `Active`**. Under an approval-required project
/// policy, an *explicitly-placed* archetype starts **`Pending`** until the project owner
/// accepts it; the frictionless default (and the built-in general placement, the eager
/// Personal placement, and older records) is **`Active`**.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Admission {
    #[default]
    Active,
    Pending,
}

/// The chat kind lives in the harness seam crate (SUB-0) — the runtime adapter
/// needs it to key the membrane/persona — re-exported here at its old path so
/// existing callers keep compiling unchanged.
pub use gaugewright_harness::ChatMode;

impl InstanceKind {
    /// The chat kind a chat rooted on an instance of this kind takes (ADR 0035):
    /// an authoring instance (an archetype) ⇒ an **edit** chat; a using instance
    /// (a placement) ⇒ a **work** chat. The single source of edit-vs-work truth.
    pub fn chat_mode(self) -> ChatMode {
        match self {
            InstanceKind::Authoring => ChatMode::Edit,
            InstanceKind::Using => ChatMode::Use,
        }
    }

    /// The derived chat-kind label the projection emits (`"edit"` | `"work"`).
    pub fn chat_kind(self) -> &'static str {
        match self {
            InstanceKind::Authoring => "edit",
            InstanceKind::Using => "work",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AgentRecord {
    pub id: String,
    #[serde(default)]
    pub op: RecordOp,
    pub name: String,
    /// The agent's own authoring instance (its repo).
    pub instance_id: String,
    /// The raw `.agent-config.json` seeded into each chat's worktree. Stored
    /// verbatim (validated on write); empty config is `"{}"`.
    #[serde(default = "empty_config")]
    pub config: String,
    /// The archetype's **current published version** (`UX-9`, [ADR 0063]): a monotonic
    /// counter bumped on publish. A placement whose `version` is behind this has an upgrade
    /// available. Older records default to `1`.
    #[serde(default = "one")]
    pub current_version: u64,
    /// The **owner's** auto-upgrade preference (`UX-9`, [ADR 0063]): when set, placements of
    /// this archetype move to a newly-published version automatically — *but only where the
    /// hosting org also allows auto-updates* (`Org::allow_auto_upgrade`), else it falls back
    /// to manual. Default `false` (manual).
    #[serde(default)]
    pub auto_upgrade: bool,
    /// The source archetype this one was **forked from** (`Some(agent_id)`), or `None` for an
    /// original. A fork shares its source's git history, so it can later *pull* upstream
    /// improvements (ADR 0038). Older records default to `None`.
    #[serde(default)]
    pub forked_from: Option<String>,
}

fn empty_config() -> String {
    "{}".to_string()
}

/// The default version pointer (`1`) for archetypes/placements predating `UX-9` versioning,
/// so an un-versioned record reads as "current" rather than perpetually behind.
fn one() -> u64 {
    1
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectRecord {
    pub id: String,
    #[serde(default)]
    pub op: RecordOp,
    pub name: String,
    /// The hidden default "Personal" project (ADR 0036): a single trust boundary
    /// the solo "just start chatting" path roots its default placement on. Hidden
    /// from the project nav. Defaults to `false` for older records.
    #[serde(default)]
    pub is_default: bool,
    /// The project's network egress posture (RF-B3). The app ships **open** —
    /// chats in this project may reach the model (and, with no per-host proxy yet,
    /// any host) — and the operator *opts into* isolation per project. `true`
    /// re-imposes the fail-closed kernel network isolation (`--unshare-net`) the
    /// core [`SandboxPolicy`](gaugewright_harness::sandbox::SandboxPolicy) defaults to.
    /// Defaults to `false` (open) for older records.
    #[serde(default)]
    pub network_isolated: bool,
    /// The project's **deployment mode** (`DEPLOY-1`, [ADR 0059](../../../specs/decisions/0059-deployment-topology-headless-control-plane-policy-gated-pairing.md)):
    /// the `(operator, attested)` [`Placement`] the consultant declares for engagements on
    /// this project — the boundary `declareCeiling` input. `None` ⇒ the local default
    /// (`Placement::local`). Defaults to `None` for older records.
    #[serde(default)]
    pub deployment_mode: Option<Placement>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InstanceRecord {
    pub id: String,
    #[serde(default)]
    pub op: RecordOp,
    pub kind: InstanceKind,
    pub agent_id: String,
    /// `Some` for a using-instance (which project it's bound into); `None` for
    /// an authoring instance.
    #[serde(default)]
    pub project_id: Option<String>,
    /// The archetype **version this placement runs** (`UX-9`, [ADR 0063]). Set to the
    /// archetype's `current_version` at placement time and advanced by an upgrade; when it is
    /// behind the archetype's `current_version`, the placement has an upgrade available. Older
    /// records default to `1` (treated as current).
    #[serde(default = "one")]
    pub version: u64,
    /// The placement's **admission state** (`APPROVE-1`, [ADR 0064]). `Active` placements
    /// host work chats and appear in the chat picker; a `Pending` placement is
    /// approved-but-not-yet-accepted under an approval-required project policy. Older
    /// records and the frictionless default read as `Active`.
    #[serde(default)]
    pub admission: Admission,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatRecord {
    pub id: String,
    #[serde(default)]
    pub op: RecordOp,
    pub instance_id: String,
    pub title: String,
    /// The library-scope position at creation — drives "Recent" ordering.
    #[serde(default)]
    pub created_position: i64,
    /// The chat this one was forked from, if any (ADR 0038) — chats form a fork
    /// tree. `None` for an original chat.
    #[serde(default)]
    pub forked_from: Option<String>,
}

/// One node in the **fork forest** (`UX-8`): a chat plus its fork children, nested. A
/// derived projection (`INV-5`) over `ChatRecord.forked_from` — read-only, never stored.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ForkNode {
    pub id: String,
    pub title: String,
    pub children: Vec<ForkNode>,
}

/// A **workstream** declaration (`WS-A`/`WS-E`): a named shared auto-sync line within
/// one placement (its `instance_id`). This record carries only the stream's *existence*
/// for nav — its name and where it lives. The authoritative status (`active`/`archived`)
/// and **membership** live in the per-workstream [`WorkstreamState`] reducer
/// (`gaugewright_core::workstream`, scope = the workstream id), folded on demand; a chat's
/// homing is the in-memory [`gaugewright_workspace::Engagement::target`] cache rebuilt from it.
///
/// [`WorkstreamState`]: gaugewright_core::workstream::WorkstreamState
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorkstreamRecord {
    pub id: String,
    #[serde(default)]
    pub op: RecordOp,
    /// The placement (using-instance) this workstream's shared line lives in.
    pub instance_id: String,
    pub name: String,
    /// The library-scope position at creation — drives stable nav ordering.
    #[serde(default)]
    pub created_position: i64,
}

/// The current value of every library record id, folded latest-wins. Held in
/// the `Workbench` and mutated in place on each write (never re-folded on the
/// hot path); rebuilt from the log on startup.
#[derive(Default, Clone)]
pub struct Library {
    pub agents: BTreeMap<String, AgentRecord>,
    pub projects: BTreeMap<String, ProjectRecord>,
    pub instances: BTreeMap<String, InstanceRecord>,
    pub chats: BTreeMap<String, ChatRecord>,
    pub workstreams: BTreeMap<String, WorkstreamRecord>,
}

/// Apply one record to its map: `Tombstone` removes the id, `Upsert` sets it.
fn fold_one<T>(map: &mut BTreeMap<String, T>, id: &str, op: RecordOp, rec: T) {
    match op {
        RecordOp::Tombstone => {
            map.remove(id);
        }
        RecordOp::Upsert => {
            map.insert(id.to_string(), rec);
        }
    }
}

impl Library {
    /// Rebuild the projection by folding all library records in position order.
    pub fn rebuild(store: &Store) -> Result<Library, AdmitError> {
        let mut lib = Library::default();
        for row in store.records(LIBRARY_SCOPE, "agent")? {
            let r: AgentRecord = serde_json::from_str(&row)?;
            fold_one(&mut lib.agents, &r.id.clone(), r.op, r);
        }
        for row in store.records(LIBRARY_SCOPE, "project")? {
            let r: ProjectRecord = serde_json::from_str(&row)?;
            fold_one(&mut lib.projects, &r.id.clone(), r.op, r);
        }
        for row in store.records(LIBRARY_SCOPE, "instance")? {
            let r: InstanceRecord = serde_json::from_str(&row)?;
            fold_one(&mut lib.instances, &r.id.clone(), r.op, r);
        }
        for row in store.records(LIBRARY_SCOPE, "chat")? {
            let r: ChatRecord = serde_json::from_str(&row)?;
            fold_one(&mut lib.chats, &r.id.clone(), r.op, r);
        }
        for row in store.records(LIBRARY_SCOPE, "workstream")? {
            let r: WorkstreamRecord = serde_json::from_str(&row)?;
            fold_one(&mut lib.workstreams, &r.id.clone(), r.op, r);
        }
        Ok(lib)
    }

    /// First run: no agents declared yet (so we seed the default builder).
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty() && self.projects.is_empty()
    }

    /// In-memory apply mirrors of the four kinds, so a route appends a record
    /// then updates the projection without re-folding.
    pub fn apply_agent(&mut self, r: AgentRecord) {
        fold_one(&mut self.agents, &r.id.clone(), r.op, r);
    }
    pub fn apply_project(&mut self, r: ProjectRecord) {
        fold_one(&mut self.projects, &r.id.clone(), r.op, r);
    }
    pub fn apply_instance(&mut self, r: InstanceRecord) {
        fold_one(&mut self.instances, &r.id.clone(), r.op, r);
    }
    pub fn apply_chat(&mut self, r: ChatRecord) {
        fold_one(&mut self.chats, &r.id.clone(), r.op, r);
    }
    pub fn apply_workstream(&mut self, r: WorkstreamRecord) {
        fold_one(&mut self.workstreams, &r.id.clone(), r.op, r);
    }

    /// The deployment mode (`DEPLOY-1`) the consultant declared for `project_id` — the
    /// boundary `declareCeiling` input for engagements on it — or the **local default**
    /// (`Placement::local`) when unset or the project is unknown (fail-safe to the
    /// least-privileged placement).
    pub fn deployment_mode_of(&self, project_id: &str) -> Placement {
        self.projects
            .get(project_id)
            .and_then(|p| p.deployment_mode)
            .unwrap_or_else(Placement::local)
    }

    /// The archetype version a placement (using-instance) runs, and its archetype's current
    /// published version (`UX-9`, [ADR 0063]) — `None` if the instance/agent is unknown.
    pub fn placement_versions(&self, instance_id: &str) -> Option<(u64, u64)> {
        let inst = self.instances.get(instance_id)?;
        let agent = self.agents.get(&inst.agent_id)?;
        Some((inst.version, agent.current_version))
    }

    /// Whether a placement has an archetype **upgrade available** (`UX-9`): its version is
    /// behind the archetype's current published version. Fail-safe `false` when unknown.
    pub fn upgrade_available(&self, instance_id: &str) -> bool {
        self.placement_versions(instance_id)
            .map(|(on, current)| on < current)
            .unwrap_or(false)
    }

    /// Live (non-tombstoned) workstreams in a placement, stable order.
    pub fn workstreams_in(&self, instance_id: &str) -> Vec<&WorkstreamRecord> {
        let mut v: Vec<&WorkstreamRecord> = self
            .workstreams
            .values()
            .filter(|w| w.instance_id == instance_id)
            .collect();
        v.sort_by_key(|w| w.created_position);
        v
    }

    /// Live (non-tombstoned) chats in an instance.
    pub fn chats_in(&self, instance_id: &str) -> Vec<&ChatRecord> {
        let mut v: Vec<&ChatRecord> = self
            .chats
            .values()
            .filter(|c| c.instance_id == instance_id)
            .collect();
        v.sort_by_key(|c| c.created_position);
        v
    }

    /// The **fork forest** (`UX-8`): chats form a fork tree via `forked_from` (ADR 0038);
    /// this projects the live chats into nested roots → children. A root is a chat with no
    /// `forked_from`, or one whose parent is gone (an orphaned fork still surfaces). Pure,
    /// stable order (created-position), depth-guarded against any pathological cycle.
    pub fn fork_forest(&self) -> Vec<ForkNode> {
        let mut roots: Vec<&ChatRecord> = self
            .chats
            .values()
            .filter(|c| {
                c.forked_from
                    .as_ref()
                    .is_none_or(|f| !self.chats.contains_key(f))
            })
            .collect();
        roots.sort_by_key(|c| c.created_position);
        roots.into_iter().map(|c| self.fork_node(c, 64)).collect()
    }

    fn fork_node(&self, c: &ChatRecord, depth: usize) -> ForkNode {
        let children = if depth == 0 {
            Vec::new()
        } else {
            let mut kids: Vec<&ChatRecord> = self
                .chats
                .values()
                .filter(|k| k.forked_from.as_deref() == Some(c.id.as_str()))
                .collect();
            kids.sort_by_key(|k| k.created_position);
            kids.into_iter()
                .map(|k| self.fork_node(k, depth - 1))
                .collect()
        };
        ForkNode {
            id: c.id.clone(),
            title: c.title.clone(),
            children,
        }
    }

    /// All live chats across a **project's** placements (`UX-2`): the union of `chats_in` over
    /// the project's using-instances, **most-recent-first** (`created_position` desc; tie-break
    /// by id). The work chats whose lifecycle scopes the project-home rollup folds.
    pub fn project_chats(&self, project_id: &str) -> Vec<&ChatRecord> {
        let mut v: Vec<&ChatRecord> = self
            .using_instances_of(project_id)
            .into_iter()
            .flat_map(|i| self.chats_in(&i.id))
            .collect();
        v.sort_by(|a, b| {
            b.created_position
                .cmp(&a.created_position)
                .then_with(|| a.id.cmp(&b.id))
        });
        v
    }

    /// The network egress posture for a chat, resolved through its placement to
    /// its project (chat → instance → `project_id` → project). Defaults to **open**
    /// (`false`) when any hop is missing — an authoring/edit chat with no project,
    /// the hidden Personal default, or an unknown id — so the app's open-by-default
    /// posture holds and only an explicit per-project opt-in isolates.
    pub fn chat_network_isolated(&self, chat_id: &str) -> bool {
        self.chats
            .get(chat_id)
            .and_then(|c| self.instances.get(&c.instance_id))
            .and_then(|i| i.project_id.as_deref())
            .and_then(|pid| self.projects.get(pid))
            .map(|p| p.network_isolated)
            .unwrap_or(false)
    }

    /// The project a chat belongs to (`ENTSEC-2`): chat → its instance → the instance's
    /// `project_id`. `None` for an edit/authoring chat (no project), or any unknown id — the
    /// per-project scope gate then does not apply (the route is governed by membership alone).
    pub fn project_of_chat(&self, chat_id: &str) -> Option<&str> {
        self.chats
            .get(chat_id)
            .and_then(|c| self.instances.get(&c.instance_id))
            .and_then(|i| i.project_id.as_deref())
    }

    /// The project a using-instance (placement) is bound into (`ENTSEC-2`): its `project_id`.
    /// `None` for an authoring instance (an archetype's own repo) or an unknown id.
    pub fn project_of_instance(&self, instance_id: &str) -> Option<&str> {
        self.instances
            .get(instance_id)
            .and_then(|i| i.project_id.as_deref())
    }
    /// The using-instances bound into a project.
    pub fn using_instances_of(&self, project_id: &str) -> Vec<&InstanceRecord> {
        let mut v: Vec<&InstanceRecord> = self
            .instances
            .values()
            .filter(|i| {
                i.kind == InstanceKind::Using && i.project_id.as_deref() == Some(project_id)
            })
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }
}

/// The honest confidentiality ceiling of a boundary, as the API surfaces it
/// (ATTEST-14). Derived from the declared [`Placement`] and the attestation
/// evidence collected at acceptance — never asserted, always read back from the
/// reducer state so the value the client sees is the value the gate enforces.
///
/// `host_blind` is the one bit a client must trust: it is `true` *only* when the
/// placement is attested **and** every required participant presented trustworthy
/// [`AttestationEvidence`] (a verified quote over the very measurement it claimed,
/// `AttestationEvidence::is_trustworthy`). An attested placement whose evidence is
/// missing or failed verification is honestly reported as *not* host-blind: the
/// ceiling claim degrades to the unattested case rather than over-promising.
///
/// [`Placement`]: gaugewright_core::boundary_lifecycle::Placement
/// [`AttestationEvidence`]: gaugewright_core::attestation::AttestationEvidence
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct BoundaryProjection {
    pub phase: BoundaryPhase,
    /// `true` once the boundary is live (declared + every participant accepted).
    pub active: bool,
    /// `Some` once a ceiling is declared: who operates the host.
    pub operator: Option<Operator>,
    /// Whether the *declared* placement claims attested (TEE) execution.
    pub attested: bool,
    /// The honest ceiling: is the method hidden from the host? `true` only when
    /// attested **and** the collected evidence verifies (see type docs).
    pub host_blind: bool,
    /// A one-line, client-facing description of the ceiling.
    pub ceiling_description: String,
}

impl BoundaryProjection {
    /// Project a [`BoundaryState`] into the client-facing ceiling view (ATTEST-14).
    pub fn from_state(state: &BoundaryState) -> BoundaryProjection {
        let placement = state.placement;
        let attested = placement.map(|p| p.attested).unwrap_or(false);
        // host_blind is the honest ceiling, not the declared claim: an attested
        // placement is host-blind only once every required participant has
        // presented trustworthy evidence. Missing/failed evidence degrades the
        // claim rather than over-promising (the value the key-release gate, ATTEST-5,
        // also enforces).
        let evidence_complete = attested
            && !state.required.is_empty()
            && state.required.iter().all(|p| {
                state
                    .attestation_evidence
                    .get(p)
                    .map(|e| e.is_trustworthy())
                    .unwrap_or(false)
            });
        let host_blind = attested && evidence_complete;
        let operator = placement.map(|p| p.operator);
        let ceiling_description = match placement {
            None => "ceiling not yet declared".to_string(),
            Some(p) => {
                let host = match p.operator {
                    Operator::Local => "a host you operate",
                    Operator::Counterparty => "the counterparty's host",
                    Operator::Neutral => "a neutral third-party host",
                };
                if host_blind {
                    format!("host-blind: the method stays sealed from {host} (attested)")
                } else if attested {
                    // Declared attested but evidence is not (yet) complete/trustworthy.
                    format!("attestation pending: {host} could see the method until every party's quote verifies")
                } else {
                    format!("host-visible: the method is in plaintext to {host} (unattested)")
                }
            }
        };
        BoundaryProjection {
            phase: state.phase,
            active: state.active(),
            operator,
            attested,
            host_blind,
            ceiling_description,
        }
    }
}

/// Mint a globally-unique id: `prefix-<12 hex>`. Server-generated so chat ids
/// never collide across instances (the property the whole flat-map design rests
/// on).
pub fn gen_id(prefix: &str) -> String {
    let mut bytes = [0u8; 6];
    getrandom::getrandom(&mut bytes).expect("os rng");
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{prefix}-{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_mode_serializes_as_edit_and_reads_the_legacy_build_value() {
        // The mode renamed build→edit; records persisted before the rename store
        // "build" and must still deserialize (serde alias), so existing chats load.
        assert_eq!(serde_json::to_string(&ChatMode::Edit).unwrap(), "\"edit\"");
        assert_eq!(
            serde_json::from_str::<ChatMode>("\"edit\"").unwrap(),
            ChatMode::Edit
        );
        assert_eq!(
            serde_json::from_str::<ChatMode>("\"build\"").unwrap(),
            ChatMode::Edit
        );
        assert_eq!(
            serde_json::from_str::<ChatMode>("\"use\"").unwrap(),
            ChatMode::Use
        );
    }

    fn agent(store: &mut Store, id: &str, op: RecordOp, name: &str) {
        let r = AgentRecord {
            id: id.into(),
            op,
            name: name.into(),
            instance_id: format!("inst-{id}"),
            config: "{}".into(),
            current_version: 1,
            auto_upgrade: false,
            forked_from: None,
        };
        store
            .append_record(LIBRARY_SCOPE, "agent", &serde_json::to_string(&r).unwrap())
            .unwrap();
    }

    #[test]
    fn chat_network_posture_resolves_through_project_and_defaults_open() {
        // chat → instance → project. The app default is OPEN (false); only an
        // explicit per-project opt-in isolates. Build the projection by hand.
        let mut lib = Library::default();
        lib.apply_project(ProjectRecord {
            id: "p-iso".into(),
            op: RecordOp::Upsert,
            name: "Locked".into(),
            is_default: false,
            network_isolated: true,
            deployment_mode: None,
        });
        lib.apply_project(ProjectRecord {
            id: "p-open".into(),
            op: RecordOp::Upsert,
            name: "Open".into(),
            is_default: false,
            network_isolated: false,
            deployment_mode: None,
        });
        let bind = |lib: &mut Library, inst: &str, project: Option<&str>| {
            lib.apply_instance(InstanceRecord {
                id: inst.into(),
                op: RecordOp::Upsert,
                kind: InstanceKind::Using,
                agent_id: "a1".into(),
                project_id: project.map(str::to_string),
                version: 1,
                admission: Admission::Active,
            });
        };
        let chat = |lib: &mut Library, id: &str, inst: &str| {
            lib.apply_chat(ChatRecord {
                id: id.into(),
                op: RecordOp::Upsert,
                instance_id: inst.into(),
                title: id.into(),
                created_position: 0,
                forked_from: None,
            });
        };
        bind(&mut lib, "i-iso", Some("p-iso"));
        bind(&mut lib, "i-open", Some("p-open"));
        bind(&mut lib, "i-authoring", None); // an edit chat's instance — no project

        chat(&mut lib, "c-iso", "i-iso");
        chat(&mut lib, "c-open", "i-open");
        chat(&mut lib, "c-edit", "i-authoring");

        assert!(
            lib.chat_network_isolated("c-iso"),
            "isolated project isolates"
        );
        assert!(
            !lib.chat_network_isolated("c-open"),
            "open project stays open"
        );
        assert!(
            !lib.chat_network_isolated("c-edit"),
            "no project ⇒ open default"
        );
        assert!(
            !lib.chat_network_isolated("c-missing"),
            "unknown chat ⇒ open default"
        );
    }

    #[test]
    fn project_of_chat_and_instance_resolve_or_none() {
        // ENTSEC-2: chat → instance → project; None for an authoring chat / unknown id.
        let mut lib = Library::default();
        lib.apply_instance(InstanceRecord {
            id: "i-using".into(),
            op: RecordOp::Upsert,
            kind: InstanceKind::Using,
            agent_id: "a1".into(),
            project_id: Some("proj-acme".into()),
            version: 1,
            admission: Admission::Active,
        });
        lib.apply_instance(InstanceRecord {
            id: "i-authoring".into(),
            op: RecordOp::Upsert,
            kind: InstanceKind::Authoring,
            agent_id: "a1".into(),
            project_id: None,
            version: 1,
            admission: Admission::Active,
        });
        let chat = |lib: &mut Library, id: &str, inst: &str| {
            lib.apply_chat(ChatRecord {
                id: id.into(),
                op: RecordOp::Upsert,
                instance_id: inst.into(),
                title: id.into(),
                created_position: 0,
                forked_from: None,
            });
        };
        chat(&mut lib, "c-work", "i-using");
        chat(&mut lib, "c-edit", "i-authoring");

        assert_eq!(lib.project_of_chat("c-work"), Some("proj-acme"));
        assert_eq!(lib.project_of_chat("c-edit"), None); // authoring chat, no project
        assert_eq!(lib.project_of_chat("c-missing"), None); // unknown chat
        assert_eq!(lib.project_of_instance("i-using"), Some("proj-acme"));
        assert_eq!(lib.project_of_instance("i-authoring"), None);
        assert_eq!(lib.project_of_instance("i-missing"), None);
    }

    #[test]
    fn fork_forest_nests_chats_by_forked_from() {
        let mut lib = Library::default();
        let chat = |id: &str, pos: i64, from: Option<&str>| ChatRecord {
            id: id.into(),
            op: RecordOp::Upsert,
            instance_id: "p1".into(),
            title: format!("title {id}"),
            created_position: pos,
            forked_from: from.map(str::to_string),
        };
        // c1 (root) → c2 → c3 ; c4 (root) ; c5 forked from a missing parent ⇒ surfaces as root.
        lib.apply_chat(chat("c1", 1, None));
        lib.apply_chat(chat("c2", 2, Some("c1")));
        lib.apply_chat(chat("c3", 3, Some("c2")));
        lib.apply_chat(chat("c4", 4, None));
        lib.apply_chat(chat("c5", 5, Some("gone")));

        let forest = lib.fork_forest();
        let roots: Vec<&str> = forest.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            roots,
            vec!["c1", "c4", "c5"],
            "roots in created order, orphan surfaces"
        );

        let c1 = &forest[0];
        assert_eq!(c1.children.len(), 1);
        assert_eq!(c1.children[0].id, "c2");
        assert_eq!(c1.children[0].children[0].id, "c3"); // nested two deep
        assert!(forest[1].children.is_empty()); // c4 has no forks
    }

    #[test]
    fn deployment_mode_defaults_to_local_and_resolves_when_set() {
        let mut lib = Library::default();
        // Unset ⇒ the local default (least-privileged placement).
        lib.apply_project(ProjectRecord {
            id: "p-default".into(),
            op: RecordOp::Upsert,
            name: "Default".into(),
            is_default: false,
            network_isolated: false,
            deployment_mode: None,
        });
        assert_eq!(lib.deployment_mode_of("p-default"), Placement::local());
        // Unknown project ⇒ also the local default (fail-safe).
        assert_eq!(lib.deployment_mode_of("p-missing"), Placement::local());
        // An explicitly-declared counterparty-attested mode resolves through.
        let mode = Placement {
            operator: Operator::Counterparty,
            attested: true,
        };
        lib.apply_project(ProjectRecord {
            id: "p-attested".into(),
            op: RecordOp::Upsert,
            name: "Attested".into(),
            is_default: false,
            network_isolated: false,
            deployment_mode: Some(mode),
        });
        assert_eq!(lib.deployment_mode_of("p-attested"), mode);
    }

    #[test]
    fn folds_latest_wins_and_tombstones_disappear() {
        let mut store = Store::open_in_memory().unwrap();
        agent(&mut store, "a1", RecordOp::Upsert, "first");
        agent(&mut store, "a1", RecordOp::Upsert, "renamed"); // rename = newer upsert
        agent(&mut store, "a2", RecordOp::Upsert, "keep");
        agent(&mut store, "a2", RecordOp::Tombstone, "keep"); // delete a2

        let lib = Library::rebuild(&store).unwrap();
        assert_eq!(lib.agents.len(), 1);
        assert_eq!(lib.agents.get("a1").unwrap().name, "renamed");
        assert!(!lib.agents.contains_key("a2"), "tombstoned agent is gone");
    }

    #[test]
    fn created_position_orders_chats_and_using_instances_filter_by_project() {
        let mut store = Store::open_in_memory().unwrap();
        let mut lib = Library::default();
        for (id, pos) in [("chat-b", 5), ("chat-a", 2)] {
            let c = ChatRecord {
                id: id.into(),
                op: RecordOp::Upsert,
                instance_id: "inst-1".into(),
                title: id.into(),
                created_position: pos,
                forked_from: None,
            };
            store
                .append_record(LIBRARY_SCOPE, "chat", &serde_json::to_string(&c).unwrap())
                .unwrap();
            lib.apply_chat(c);
        }
        let ordered: Vec<&str> = lib
            .chats_in("inst-1")
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            ordered,
            vec!["chat-a", "chat-b"],
            "sorted by created_position"
        );

        lib.apply_instance(InstanceRecord {
            id: "inst-u".into(),
            op: RecordOp::Upsert,
            kind: InstanceKind::Using,
            agent_id: "a1".into(),
            project_id: Some("proj-1".into()),
            version: 1,
            admission: Admission::Active,
        });
        assert_eq!(lib.using_instances_of("proj-1").len(), 1);
        assert_eq!(lib.using_instances_of("proj-other").len(), 0);
    }

    #[test]
    fn project_chats_unions_placements_most_recent_first() {
        // UX-2: project_chats gathers chats across ALL the project's using-instances, newest
        // first, and is empty for an unknown project.
        let mut lib = Library::default();
        for (iid, pid) in [
            ("inst-x", "proj-1"),
            ("inst-y", "proj-1"),
            ("inst-z", "proj-2"),
        ] {
            lib.apply_instance(InstanceRecord {
                id: iid.into(),
                op: RecordOp::Upsert,
                kind: InstanceKind::Using,
                agent_id: "a1".into(),
                project_id: Some(pid.into()),
                version: 1,
                admission: Admission::Active,
            });
        }
        for (id, iid, pos) in [
            ("chat-old", "inst-x", 1),
            ("chat-new", "inst-y", 9),
            ("chat-mid", "inst-x", 4),
            ("chat-other", "inst-z", 7), // proj-2 — must not appear in proj-1's rollup
        ] {
            lib.apply_chat(ChatRecord {
                id: id.into(),
                op: RecordOp::Upsert,
                instance_id: iid.into(),
                title: id.into(),
                created_position: pos,
                forked_from: None,
            });
        }
        let ids: Vec<&str> = lib
            .project_chats("proj-1")
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["chat-new", "chat-mid", "chat-old"],
            "all proj-1 chats, newest first"
        );
        assert!(lib.project_chats("proj-unknown").is_empty());
    }

    #[test]
    fn gen_id_is_prefixed_and_unique() {
        let a = gen_id("chat");
        let b = gen_id("chat");
        assert!(a.starts_with("chat-") && a.len() == 17);
        assert_ne!(a, b);
    }

    mod ceiling_description {
        use super::super::*;
        use gaugewright_core::attestation::{
            AttestationEvidence, AttestationQuote, CodeMeasurement, QuoteRejection,
            QuoteVerificationResult,
        };
        use gaugewright_core::boundary_lifecycle::{
            decide, evolve, BoundaryCommand, BoundaryState, Operator, Placement,
        };

        /// Drive one command through the reducer (decide → evolve), as the store
        /// does — admitting only valid transitions (a rejection is a no-op).
        fn apply(state: &BoundaryState, command: BoundaryCommand) -> BoundaryState {
            match decide(state, command) {
                Ok(events) => events.into_iter().fold(state.clone(), |s, e| evolve(&s, e)),
                Err(_) => state.clone(),
            }
        }

        fn measurement() -> CodeMeasurement {
            CodeMeasurement::new("a".repeat(64))
        }

        /// Evidence that verifies the measurement it claims (trustworthy).
        fn good_evidence() -> AttestationEvidence {
            AttestationEvidence::new(
                AttestationQuote::new(measurement(), "nonce", vec![1, 2, 3]),
                QuoteVerificationResult::Verified {
                    measurement: measurement(),
                },
            )
        }

        /// Evidence carrying a rejected verdict (not trustworthy).
        fn bad_evidence() -> AttestationEvidence {
            AttestationEvidence::new(
                AttestationQuote::new(measurement(), "nonce", vec![1, 2, 3]),
                QuoteVerificationResult::Rejected {
                    reason: QuoteRejection::StaleNonce,
                },
            )
        }

        /// Drive a boundary to active via the reducer, declaring `placement` and
        /// accepting each participant with the given evidence.
        fn active_boundary(
            placement: Placement,
            participants: &[(&str, Option<AttestationEvidence>)],
        ) -> BoundaryState {
            let names = participants.iter().map(|(p, _)| p.to_string()).collect();
            let mut s = apply(&BoundaryState::default(), BoundaryCommand::Propose(names));
            s = apply(&s, BoundaryCommand::DeclareCeiling(placement));
            for (p, ev) in participants {
                s = apply(
                    &s,
                    BoundaryCommand::Accept {
                        participant: (*p).into(),
                        evidence: ev.clone(),
                    },
                );
            }
            s
        }

        #[test]
        fn undeclared_boundary_has_no_ceiling() {
            let proj = BoundaryProjection::from_state(&BoundaryState::default());
            assert!(!proj.attested && !proj.host_blind && proj.operator.is_none());
            assert_eq!(proj.ceiling_description, "ceiling not yet declared");
        }

        #[test]
        fn unattested_placement_is_host_visible() {
            let s = active_boundary(
                Placement {
                    operator: Operator::Counterparty,
                    attested: false,
                },
                &[("expert", None)],
            );
            let proj = BoundaryProjection::from_state(&s);
            assert!(proj.active && !proj.attested && !proj.host_blind);
            assert_eq!(proj.operator, Some(Operator::Counterparty));
            assert!(
                proj.ceiling_description.contains("host-visible")
                    && proj.ceiling_description.contains("counterparty"),
                "{}",
                proj.ceiling_description
            );
        }

        #[test]
        fn attested_with_trustworthy_evidence_is_host_blind() {
            let s = active_boundary(
                Placement {
                    operator: Operator::Counterparty,
                    attested: true,
                },
                &[("expert", Some(good_evidence()))],
            );
            let proj = BoundaryProjection::from_state(&s);
            assert!(proj.active && proj.attested && proj.host_blind);
            assert!(
                proj.ceiling_description.starts_with("host-blind"),
                "{}",
                proj.ceiling_description
            );
        }

        /// An attested placement whose evidence failed verification must NOT be
        /// reported host-blind — the projection degrades the claim honestly so the
        /// client never sees a stronger ceiling than the key-release gate enforces.
        #[test]
        fn attested_with_untrustworthy_evidence_is_not_host_blind() {
            let s = active_boundary(
                Placement {
                    operator: Operator::Neutral,
                    attested: true,
                },
                &[("expert", Some(bad_evidence()))],
            );
            let proj = BoundaryProjection::from_state(&s);
            assert!(proj.attested && !proj.host_blind);
            assert!(
                proj.ceiling_description.starts_with("attestation pending"),
                "{}",
                proj.ceiling_description
            );
        }

        /// Two required participants but only one presented trustworthy evidence:
        /// the ceiling is not yet host-blind (every party's quote must verify).
        #[test]
        fn attested_is_not_host_blind_until_every_participant_verifies() {
            // Declared + one of two accepted (still in Declared phase, not active).
            let s = active_boundary(
                Placement {
                    operator: Operator::Counterparty,
                    attested: true,
                },
                &[("expert", Some(good_evidence()))],
            );
            // `expert` accepted; `client` has not — drive a second required party in.
            let mut required = s.required.clone();
            required.insert("client".to_string());
            let mut s = apply(
                &BoundaryState::default(),
                BoundaryCommand::Propose(required),
            );
            s = apply(
                &s,
                BoundaryCommand::DeclareCeiling(Placement {
                    operator: Operator::Counterparty,
                    attested: true,
                }),
            );
            s = apply(
                &s,
                BoundaryCommand::Accept {
                    participant: "expert".into(),
                    evidence: Some(good_evidence()),
                },
            );
            let proj = BoundaryProjection::from_state(&s);
            assert!(
                proj.attested && !proj.host_blind,
                "client has not attested yet"
            );
            assert!(proj.ceiling_description.starts_with("attestation pending"));
        }
    }
}

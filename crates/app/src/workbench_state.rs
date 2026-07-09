//! Workbench state construction, accessors, and in-memory registration helpers.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use gaugewright_core::ids::AuthorityId;
use gaugewright_store::Store;
use gaugewright_tracker::WhipTrackerHandle;
use gaugewright_workspace::{ChatWorkspace, GitWorkspaceProvider, Workspace, WorkspaceProvider};
use tokio::sync::broadcast;

use crate::app_support::{attestation_enabled, attestation_mode_from_env, prepare_workbench_root};
use crate::boundary_keeper::LoopbackKeyReleaseService;
use crate::library::Library;
use crate::library_state;
use crate::measurement_store::MeasurementStore;
use crate::stream::ServerEvent;
use crate::{
    at_rest, audit, content_vault, federation, identity, key_store, throttle, AttestationMode,
    LOCAL_AUTHORITY,
};

/// The co-resident control-plane state. Holds many instances, the durable event
/// store, derived projections, live engagements, streams, and local/remote agent
/// sessions.
/// Workspace construction providers, keyed by substrate id.
pub(crate) type WorkspaceProviders = BTreeMap<&'static str, Arc<dyn WorkspaceProvider>>;

/// The git substrate's registry key.
const GIT_SUBSTRATE: &str = "git";

/// The account/global trust boundary — the one scope the v1 onboarding tracker
/// lives on (ADR 0075 §2): no project, no client-tainted data, bottom taint.
/// Per-project boundaries (which key by `project::<id>`) are deferred to Phase 4.
pub(crate) const ACCOUNT_GLOBAL_BOUNDARY: &str = "account::global";

/// The substrate id every instance resolves in SUB-0 — hardcoded, in-memory
/// only (the durable per-instance substrate stamp + migration of existing
/// instance dirs is SUB-2).
pub(crate) fn instance_substrate_id(_inst_id: &str) -> &'static str {
    GIT_SUBSTRATE
}

/// The default registry: the git provider under its substrate id. A future
/// substrate registers a second provider here, not new construction sites.
pub(crate) fn default_workspace_providers() -> WorkspaceProviders {
    BTreeMap::from([(
        GIT_SUBSTRATE,
        Arc::new(GitWorkspaceProvider) as Arc<dyn WorkspaceProvider>,
    )])
}

/// Resolve the provider that constructs/opens an instance's workspace. The
/// registry always carries every id `instance_substrate_id` mints.
pub(crate) fn provider_for(
    providers: &WorkspaceProviders,
    inst_id: &str,
) -> Arc<dyn WorkspaceProvider> {
    providers
        .get(instance_substrate_id(inst_id))
        .cloned()
        .expect("a workspace provider is registered for every substrate id")
}

pub struct Workbench {
    pub(crate) instances: BTreeMap<String, Box<dyn Workspace>>,
    /// Workspace construction providers, keyed by substrate id; instances
    /// resolve theirs via [`instance_substrate_id`].
    pub(crate) providers: WorkspaceProviders,
    /// Where the legacy `POST /chats` route creates (the seed builder live;
    /// the single registered instance in tests).
    pub(crate) default_instance: String,
    pub(crate) engagement_index: BTreeMap<String, String>, // chat id -> instance id
    pub(crate) library: Library,
    pub(crate) store: Store,
    pub(crate) engagements: BTreeMap<String, Box<dyn ChatWorkspace>>,
    pub(crate) streams: BTreeMap<String, broadcast::Sender<ServerEvent>>,
    /// One agent harness per engagement (ADR 0031).
    pub(crate) sessions: BTreeMap<String, Box<dyn gaugewright_harness::Harness>>,
    /// One remote harness per remotely placed engagement (ADR 0020/0031).
    pub(crate) remote_sessions: BTreeMap<String, Box<dyn gaugewright_harness::RemoteHarness>>,
    /// One embedded WhippleScript tracker runtime per trust boundary (ADR 0075),
    /// keyed by boundary id (`account::global` in v1). Spawned on demand and held
    /// for the workbench's lifetime, mirroring `sessions`. Structural isolation:
    /// each boundary gets its own store files under `<root>/trackers/<id>/`.
    pub(crate) tracker_runtimes: BTreeMap<String, WhipTrackerHandle>,
    /// The trusted reproducible-build measurement allow-list (ATTEST-10).
    pub(crate) measurements: MeasurementStore,
    /// The sealed-key release service (ATTEST-5/-6).
    pub(crate) sealed_keys: LoopbackKeyReleaseService,
    /// How attested-boundary acceptance verifies quotes before releasing sealed keys.
    pub(crate) attestation_mode: AttestationMode,
    /// Deployment-injected real quote verifier factory (ATTEST-15). `None` (the
    /// default) fails closed at attested acceptance; the private managed
    /// composition installs its factory at workbench open time.
    pub(crate) real_verifier_factory: Option<crate::attestation_verifier::RealQuoteVerifierFactory>,
    /// Whether the attested-specific operator surface is mounted.
    pub(crate) attestation_enabled: bool,
    /// The on-disk state root this workbench was opened from.
    pub(crate) root: std::path::PathBuf,
    /// Where instance state dirs live (`<instances_root>/<instance-id>`),
    /// recorded at build instead of reverse-derived from an open repo handle.
    pub(crate) instances_root: std::path::PathBuf,
    /// This control plane's network federation state (`SERVE-1`/D-REMOTE).
    pub(crate) federation: Option<federation::Federation>,
    /// This control plane's own authority identity (`SERVE-1`/D-REMOTE).
    pub(crate) authority: AuthorityId,
    /// The identity adapter that authenticates bearer credentials.
    pub(crate) idp: Option<Arc<dyn identity::IdentityProvider + Send + Sync>>,
    /// Optional streaming audit sink (`AUD-4`).
    pub(crate) audit_sink: Option<Arc<dyn audit::AuditSink>>,
    /// Governance key store used to sign audit checkpoints (`SECAUD-2`).
    pub(crate) audit_signer: Option<Arc<dyn key_store::KeyStore + Send + Sync>>,
    /// Per-scope content-encryption vault (`SECAUD-9/6`).
    pub(crate) content_vault: Option<Arc<content_vault::ContentVault>>,
    /// Whether sensitive reads are written to the audit trail (`SECAUD-4`).
    pub(crate) audit_reads: bool,
    /// In-process failed-attempt lockout for SCIM bearer checks (`SECAUD-8`).
    pub(crate) scim_throttle: Arc<throttle::Throttle>,
    /// In-process failed-attempt lockout for OIDC callback processing (`SECAUD-8`) — a
    /// per-tenant brute-force guard on the SSO callback, separate from SCIM's counter.
    pub(crate) oidc_throttle: Arc<throttle::Throttle>,
    /// Per-session activity ledger enforcing the org session-timeout policy (`SEC-2`).
    pub(crate) session_activity: Arc<crate::session_activity::SessionActivity>,
    /// Per-session pending device-enrollment legs (`ACCT-1`, the enrollment drive).
    /// An `Arc` so a route handler can clone the handle out under the lock and then
    /// run the broker legs (which await) without holding the workbench mutex.
    pub(crate) enroll_drive: Arc<crate::device_enroll_drive::EnrollDrive>,
    /// The rendezvous broker this workbench dials / advertises for enrollment
    /// (`ACCT-1`); `None` falls back to `GAUGEWRIGHT_BROKER_ADDR` / the default.
    pub(crate) enroll_broker: Option<String>,
    /// The account key a newly-enrolled device recovered over the handshake
    /// (`ACCT-1`), held in memory — never returned over HTTP (`INV-10`).
    pub(crate) recovered_account_key: Option<[u8; 32]>,
}

pub type SharedWorkbench = Arc<Mutex<Workbench>>;

/// Open (or initialize) the local workbench under `root`. Agents/projects/chats
/// are rehydrated from the library records + git (ADR 0027): for each instance
/// record we open its repo and reconcile its engagements. A fresh root is seeded
/// with a default agent so the user can chat immediately.
pub fn open_workbench(root: &std::path::Path) -> std::io::Result<SharedWorkbench> {
    let wb = build_workbench(root)?
        .with_attestation_mode(attestation_mode_from_env())
        .with_attestation_enabled(attestation_enabled());
    Ok(Arc::new(Mutex::new(wb)))
}

pub fn open_workbench_with_content_keywrap(
    root: &std::path::Path,
    content_keywrap: impl Fn(&std::path::Path) -> std::io::Result<Box<dyn at_rest::KeyWrap>>,
) -> std::io::Result<SharedWorkbench> {
    let wb = build_workbench_with_content_keywrap(root, content_keywrap)?
        .with_attestation_mode(attestation_mode_from_env())
        .with_attestation_enabled(attestation_enabled());
    Ok(Arc::new(Mutex::new(wb)))
}

/// Build a fresh [`Workbench`] **value** from an on-disk state root — opening the
/// store, rebuilding (or seeding) the library, and reconciling live engagements.
/// `open_workbench` wraps this in the shared mutex; the test-only reset route uses
/// it to rebuild a clean workbench in place after wiping the root.
pub(crate) fn build_workbench(root: &std::path::Path) -> std::io::Result<Workbench> {
    build_workbench_with_content_keywrap(root, at_rest::local_content_keywrap)
}

pub(crate) fn build_workbench_with_content_keywrap(
    root: &std::path::Path,
    content_keywrap: impl Fn(&std::path::Path) -> std::io::Result<Box<dyn at_rest::KeyWrap>>,
) -> std::io::Result<Workbench> {
    let (root, instances_dir) = prepare_workbench_root(root)?;

    let (mut store, content_vault) = content_vault::open_startup_store(&root, content_keywrap)?;
    let providers = default_workspace_providers();
    let startup_state =
        library_state::load_startup_library_state(&mut store, &instances_dir, &providers)?;

    let mut wb = Workbench::new(store);
    wb.providers = providers;
    wb.instances_root = instances_dir;
    wb.apply_startup_library_state(startup_state);
    wb.apply_startup_audit(&root);
    wb.apply_startup_content_vault(content_vault);
    wb.restore_startup_local_projections();
    wb.apply_startup_root(root);
    wb.activate_configured_authority();
    // Stand up + seed the account-global onboarding tracker (ADR 0075). Runs
    // after the root is applied so the tracker's store files resolve under it;
    // best-effort, so a tracker failure never aborts workbench startup.
    wb.ensure_onboarding_seeded();
    federation::activate_configured_federation(&mut wb)?;
    // Enterprise SSO activation (`ID-3`) moved with the ee band (`gaugewright-ee`,
    // SPLIT-1): the ee/hosted compositions call `activate_configured_idp` right
    // after workbench open, through the open `set_identity_provider` seam.
    Ok(wb)
}

impl Workbench {
    /// An empty workbench (no instances). Startup registers instances from the
    /// library; tests use [`Workbench::with_instance`].
    pub fn new(store: Store) -> Self {
        Self {
            instances: BTreeMap::new(),
            providers: default_workspace_providers(),
            default_instance: String::new(),
            engagement_index: BTreeMap::new(),
            library: Library::default(),
            store,
            engagements: BTreeMap::new(),
            streams: BTreeMap::new(),
            sessions: BTreeMap::new(),
            remote_sessions: BTreeMap::new(),
            tracker_runtimes: BTreeMap::new(),
            measurements: MeasurementStore::new(),
            sealed_keys: LoopbackKeyReleaseService::new(),
            attestation_mode: AttestationMode::RealRequired,
            real_verifier_factory: None,
            attestation_enabled: false,
            root: std::path::PathBuf::new(),
            // The bare-workbench default mirrors the old derived fallback; the
            // build path and `with_instance` record the real root.
            instances_root: std::path::PathBuf::from(".gaugewright/instances"),
            federation: None,
            authority: AuthorityId::new(LOCAL_AUTHORITY),
            idp: None,
            audit_sink: None,
            audit_signer: None,
            content_vault: None,
            audit_reads: false,
            // SECAUD-8: 10 failed SCIM auths within 60s locks the tenant's SCIM endpoint
            // for the rest of the window (defense-in-depth; edge is the primary control).
            scim_throttle: Arc::new(throttle::Throttle::new(10, 60_000)),
            // SECAUD-8: 10 failed OIDC callbacks within 60s locks the tenant's SSO callback
            // for the rest of the window (defense-in-depth behind the edge rate-limit).
            oidc_throttle: Arc::new(throttle::Throttle::new(10, 60_000)),
            // SEC-2: enforce the org session lifetime / idle-timeout policy on data routes.
            session_activity: Arc::new(crate::session_activity::SessionActivity::new()),
            // ACCT-1: the per-session device-enrollment drive; broker + recovered key
            // resolve lazily (env fallback) / on a successful handshake.
            enroll_drive: Arc::new(crate::device_enroll_drive::EnrollDrive::new()),
            enroll_broker: None,
            recovered_account_key: None,
        }
    }

    /// The provider that constructs/opens this instance's workspace.
    pub(crate) fn workspace_provider(&self, inst_id: &str) -> Arc<dyn WorkspaceProvider> {
        provider_for(&self.providers, inst_id)
    }

    /// Get (spawning on first use) the embedded whip tracker for `boundary_id`.
    /// Lazy, mirroring the `sessions` harness map: the store files under
    /// `<root>/trackers/<boundary_id>/` are created on first touch and the handle
    /// is held for the workbench's lifetime. Structural isolation is the only
    /// isolation (ADR 0075 §1); callers must pass a boundary the acting authority
    /// owns.
    pub(crate) fn tracker_for_boundary(
        &mut self,
        boundary_id: &str,
    ) -> gaugewright_tracker::TrackerResult<&mut WhipTrackerHandle> {
        if !self.tracker_runtimes.contains_key(boundary_id) {
            let handle = WhipTrackerHandle::open(&self.root, boundary_id)?;
            self.tracker_runtimes.insert(boundary_id.to_owned(), handle);
        }
        Ok(self
            .tracker_runtimes
            .get_mut(boundary_id)
            .expect("tracker just inserted"))
    }

    /// The v1 account-global onboarding tracker (ADR 0075 §2).
    pub(crate) fn account_tracker(
        &mut self,
    ) -> gaugewright_tracker::TrackerResult<&mut WhipTrackerHandle> {
        self.tracker_for_boundary(ACCOUNT_GLOBAL_BOUNDARY)
    }
}

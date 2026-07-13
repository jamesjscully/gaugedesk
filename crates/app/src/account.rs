//! The **account** — the person behind the placements/devices (ACCT-1). Identity is
//! the governance root keypair; this module is the durable **account state** that
//! follows the person: the **device registry**, **settings**, and **sealed
//! linked-credentials**. See [`specs/primitives/account.md`](../../../specs/primitives/account.md)
//! and [ADR 0053](../../../specs/decisions/0053-account-root-identity-and-device-enrollment.md).
//!
//! Records folded latest-wins by id (`data.md`, `INV-5`/`INV-6`) in a reserved
//! `account` scope — the same discipline as [`crate::org`] / [`crate::library`]. The
//! linked credential is **sealed at rest** via the `SEC-4` [`Encryptor`](crate::at_rest::Encryptor):
//! the stored record is ciphertext, decrypted only when the local runtime needs it,
//! never crossing as payload (`INV-10`). Adds no protection invariant (ADR 0020).

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use gaugewright_harness::{CredentialCapability, CredentialMaterial};
use gaugewright_store::{AdmitError, Store};

use crate::at_rest::{Encryptor, LocalAeadEncryptor};
pub use crate::library::RecordOp;
use crate::Workbench;

/// The reserved store scope holding the person's account state.
pub const ACCOUNT_SCOPE: &str = "account";

/// A device's standing in the registry.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus {
    #[default]
    Active,
    Revoked,
}

/// One enrolled device (the "trusted devices" surface). Durable, auditable
/// (`INV-5`/`INV-6`); revoke flips `status`, it does not erase history.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DeviceRecord {
    pub id: String,
    #[serde(default)]
    pub op: RecordOp,
    #[serde(default)]
    pub label: String,
    /// Hex of the device subkey's public key (FED-5a).
    #[serde(default)]
    pub subkey_pubkey: String,
    #[serde(default)]
    pub status: DeviceStatus,
}

/// A latest-wins preference (`id` = the setting key).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SettingRecord {
    pub id: String,
    #[serde(default)]
    pub op: RecordOp,
    #[serde(default)]
    pub value: String,
}

/// A linked provider credential (`id` = provider, e.g. `openai`). The token is stored
/// **only** as `SEC-4` ciphertext (hex); the plaintext never lives at rest here.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CredentialRecord {
    pub id: String,
    #[serde(default)]
    pub op: RecordOp,
    /// Hex-encoded `SEC-4` ciphertext of the OAuth token.
    #[serde(default)]
    pub sealed_token: String,
}

/// The folded account projection (derived, rebuildable — `INV-5`).
#[derive(Default, Clone, Debug)]
pub struct Account {
    pub devices: BTreeMap<String, DeviceRecord>,
    pub settings: BTreeMap<String, SettingRecord>,
    pub credentials: BTreeMap<String, CredentialRecord>,
}

fn fold<T>(map: &mut BTreeMap<String, T>, id: String, op: RecordOp, rec: T) {
    match op {
        RecordOp::Tombstone => {
            map.remove(&id);
        }
        RecordOp::Upsert => {
            map.insert(id, rec);
        }
    }
}

/// The store scope holding **one person's** account state (`ADR 0077`). The single-user desktop
/// (and every internal, non-per-request caller) uses the fixed [`ACCOUNT_SCOPE`]; the hosted hub
/// keys each person's devices/settings/credentials/facilities under `account::<root>` so
/// authenticated callers are isolated (`INV-1`) and never share one scope. An empty `person`
/// (solo / no session) collapses to [`ACCOUNT_SCOPE`], so the desktop path is unchanged.
pub fn account_scope(person: &str) -> String {
    if person.is_empty() {
        ACCOUNT_SCOPE.to_string()
    } else {
        format!("{ACCOUNT_SCOPE}::{person}")
    }
}

impl Account {
    /// Rebuild by folding the default [`ACCOUNT_SCOPE`] — the solo / single-user path.
    pub fn rebuild(store: &Store) -> Result<Account, AdmitError> {
        Self::rebuild_in(store, ACCOUNT_SCOPE)
    }

    /// Rebuild **one person's** account by folding `scope`'s records in position order. Pass
    /// [`ACCOUNT_SCOPE`] for the default tenant-of-one / desktop, or [`account_scope`]`(person)`
    /// for a hosted person. Scope-isolated (`INV-1`).
    pub fn rebuild_in(store: &Store, scope: &str) -> Result<Account, AdmitError> {
        let mut acct = Account::default();
        for row in store.records(scope, "device")? {
            let r: DeviceRecord = serde_json::from_str(&row)?;
            fold(&mut acct.devices, r.id.clone(), r.op, r);
        }
        for row in store.records(scope, "setting")? {
            let r: SettingRecord = serde_json::from_str(&row)?;
            fold(&mut acct.settings, r.id.clone(), r.op, r);
        }
        for row in store.records(scope, "credential")? {
            let r: CredentialRecord = serde_json::from_str(&row)?;
            fold(&mut acct.credentials, r.id.clone(), r.op, r);
        }
        Ok(acct)
    }

    /// The active (non-revoked) devices, stable order.
    pub fn active_devices(&self) -> Vec<&DeviceRecord> {
        self.devices
            .values()
            .filter(|d| d.status == DeviceStatus::Active)
            .collect()
    }
}

// --- the account encryption key + sealing (SEC-4) -------------------------------

/// Derive the account encryption key from the governance root **seed** (ADR 0053 §4).
/// The seed is the *private* key material behind the recovery code — unlike the public
/// authority id it is secret, so this key is secret too. Restoring the seed
/// ([`gaugewright_core::recovery`]) re-derives the **same** account key, so seed recovery
/// restores access to all sealed account state. The `v2` domain tag marks the deliberate
/// break from the old public-id "loopback double" (`v1`), which **anyone** who knew your
/// public root id could derive — the vulnerability this closes. Callers never derive this
/// themselves; the one resolver is [`Workbench::account_key`], which also returns a
/// device's *recovered* key (enrollment) ahead of the seed path.
pub fn account_key_from_seed(seed: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"gaugewright-account-key:v2:");
    h.update(seed);
    h.finalize().into()
}

/// The account [`Encryptor`] for a resolved account `key`.
pub fn account_encryptor(key: [u8; 32]) -> LocalAeadEncryptor {
    LocalAeadEncryptor::new(key)
}

/// Seal a plaintext token → hex ciphertext for storage, under the account `key`.
pub fn seal_token(key: [u8; 32], token: &str) -> Option<String> {
    account_encryptor(key)
        .encrypt(token.as_bytes())
        .ok()
        .map(hex::encode)
}

/// Unseal a stored hex ciphertext → the plaintext token under the account `key` (None on
/// any failure — fail-closed; a token sealed under a different key does not open).
pub fn unseal_token(key: [u8; 32], sealed_hex: &str) -> Option<String> {
    let ct = hex::decode(sealed_hex).ok()?;
    let pt = account_encryptor(key).decrypt(&ct).ok()?;
    String::from_utf8(pt).ok()
}

/// Resolve the **plaintext** linked token for `provider` under the account `key`,
/// decrypting the sealed record — the internal API the local runtime uses (never exposed
/// over HTTP). `None` if no credential is linked or it fails to unseal.
pub fn resolve_token(store: &Store, key: [u8; 32], provider: &str) -> Option<String> {
    let acct = Account::rebuild(store).ok()?;
    let rec = acct.credentials.get(provider)?;
    unseal_token(key, &rec.sealed_token)
}

/// The provider → API-key env-var mapping — the ONE map (the engine's fail-closed
/// BYOK precheck keys off it too, so "which providers are BYOK" is answered here
/// only). Only mapped BYOK providers use the transitional env-shaped resolver;
/// OAuth providers travel through their separate GaugeDesk-owned resolved
/// binding path.
pub(crate) fn provider_env_var(provider: &str) -> Option<&'static str> {
    match provider {
        "openai" => Some("OPENAI_API_KEY"),
        "anthropic" => Some("ANTHROPIC_API_KEY"),
        _ => None,
    }
}

/// The coordination scope holding a **project's** per-project overrides (`LLM-2`,
/// [ADR 0062]): the sealed credential a project pins to beat the account default for
/// chats in that project. The same `project::<id>` coordination scope the federation
/// relocation treats as project-owned, so a project's credential pin travels with it.
pub fn project_scope(project_id: &str) -> String {
    format!("project::{project_id}")
}

/// Fold the sealed `credential` records held in an arbitrary `scope` (latest-wins by
/// provider id) — the scope-parametric core under both the account store and the
/// project-scope override (`LLM-2`).
pub fn credentials_in_scope(store: &Store, scope: &str) -> BTreeMap<String, CredentialRecord> {
    let mut map = BTreeMap::new();
    if let Ok(rows) = store.records(scope, "credential") {
        for row in rows {
            if let Ok(r) = serde_json::from_str::<CredentialRecord>(&row) {
                fold(&mut map, r.id.clone(), r.op, r);
            }
        }
    }
    map
}

/// Env vars carrying the **resolved** (decrypted) linked-credential tokens held in
/// `scope`, for providers with a known API-key env mapping. Sealed to `authority`;
/// an entry that fails to unseal is skipped (fail-closed).
pub fn credential_envs_in_scope(
    store: &Store,
    scope: &str,
    key: [u8; 32],
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (provider, rec) in credentials_in_scope(store, scope) {
        let Some(var) = provider_env_var(&provider) else {
            continue;
        };
        if let Some(token) = unseal_token(key, &rec.sealed_token) {
            out.push((var.to_string(), token));
        }
    }
    out
}

/// Env vars carrying the **resolved** (decrypted) account-linked credential tokens —
/// injected into the runtime so a linked account is actually *used* (ACCT-1 → the agent
/// harness). The account-scope tier of [`resolved_credential_envs`].
pub fn credential_envs(store: &Store, key: [u8; 32]) -> Vec<(String, String)> {
    credential_envs_in_scope(store, ACCOUNT_SCOPE, key)
}

/// The runtime credential env vars for a turn, applying **nearest-scope-wins** (`LLM-2`,
/// [ADR 0062]): the account default is the base; a credential pinned in the chat's
/// `project` overrides it **per provider** (so a project may redirect one provider while
/// inheriting the rest). `project = None` (an authoring/edit chat, the hidden Personal
/// default, or an unknown chat) resolves to the account default alone.
pub fn resolved_credential_envs(
    store: &Store,
    key: [u8; 32],
    project: Option<&str>,
) -> Vec<(String, String)> {
    let mut envs: BTreeMap<String, String> = credential_envs(store, key).into_iter().collect();
    if let Some(project) = project {
        for (var, token) in credential_envs_in_scope(store, &project_scope(project), key) {
            envs.insert(var, token); // project pin beats the account default for this provider
        }
    }
    envs.into_iter().collect()
}

impl Workbench {
    /// The **account encryption key** this workbench seals/unseals account state with
    /// (ADR 0053 §4). Resolution order: the key this device **recovered** over enrollment
    /// (a device that joined someone else's root holds *their* account key), else the key
    /// derived from this account's own governance root **seed**. Never the retired
    /// public-id loopback double — see [`account_key_from_seed`].
    pub(crate) fn account_key(&self) -> [u8; 32] {
        if let Some(k) = self.recovered_account_key() {
            return k;
        }
        account_key_from_seed(&self.governance_seed())
    }

    /// Seal an account-scoped secret without exposing the resolved account key to a
    /// band-specific crate. Hosted and enterprise shells use this for credentials such as
    /// refresh tokens; key resolution remains centralized in [`Workbench::account_key`].
    pub fn seal_account_secret(&self, value: &str) -> Option<String> {
        seal_token(self.account_key(), value)
    }

    /// Open an account-scoped secret sealed by [`Workbench::seal_account_secret`].
    /// Returns `None` on malformed ciphertext or a non-matching account key.
    pub fn unseal_account_secret(&self, sealed: &str) -> Option<String> {
        unseal_token(self.account_key(), sealed)
    }

    /// This account's governance root **seed** — the private key material the account key
    /// and the recovered-key wrap both derive from. Resolved through the file-backed key
    /// store when a real state root exists (persisted on first use, stable across restarts);
    /// a **bare/test workbench with no root** derives it purely via the loopback double
    /// *without touching disk*, so credential resolution never writes a key store into the
    /// process CWD. The private half never leaves this in-process value.
    pub(crate) fn governance_seed(&self) -> [u8; 32] {
        use crate::key_store::{FileKeyStore, KeyStore, LoopbackKeyStore};
        let root = self.root_path();
        if root.as_os_str().is_empty() {
            LoopbackKeyStore
                .signing_key(self.authority())
                .to_seed_bytes()
        } else {
            FileKeyStore::new(root.join("keys"))
                .signing_key(self.authority())
                .to_seed_bytes()
        }
    }

    /// Stable, non-secret identity for the credential selected by the same
    /// nearest-scope-wins rule as the ephemeral secret resolver.
    pub(crate) fn credential_ref_for_chat(&self, chat_id: &str, provider: &str) -> String {
        let project = self.library_project_of_chat(chat_id);
        let scope = project
            .as_deref()
            .filter(|project_id| {
                credentials_in_scope(self.store_ref(), &project_scope(project_id))
                    .contains_key(provider)
            })
            .map(|project_id| format!("project:{project_id}"))
            .unwrap_or_else(|| "account".to_owned());
        format!("gaugedesk:credential:{scope}:{provider}")
    }

    /// Resolve the nearest-scope BYOK credential into an exact-reference
    /// capability. The secret remains private to this in-memory object and is
    /// released only after WhippleScript admits the matching reference.
    pub(crate) fn credential_capability_for_chat(
        &self,
        chat_id: &str,
        provider: &str,
    ) -> Option<Arc<dyn CredentialCapability>> {
        let project = self.library_project_of_chat(chat_id);
        let project_record = project.as_deref().and_then(|project_id| {
            credentials_in_scope(self.store_ref(), &project_scope(project_id)).remove(provider)
        });
        let record = project_record
            .or_else(|| credentials_in_scope(self.store_ref(), ACCOUNT_SCOPE).remove(provider))?;
        let secret = unseal_token(self.account_key(), &record.sealed_token)?;
        Some(resolved_credential_capability(
            self.credential_ref_for_chat(chat_id, provider),
            secret,
            None,
        ))
    }

    /// Provider ids pinned as BYOK credentials in one project's coordination scope.
    pub fn project_credential_providers(&self, project_id: &str) -> Vec<String> {
        let scope = project_scope(project_id);
        credentials_in_scope(self.store_ref(), &scope)
            .into_keys()
            .collect()
    }

    /// Persist a sealed BYOK credential pin for one project.
    pub fn upsert_project_credential(
        &mut self,
        project_id: &str,
        provider: String,
        sealed_token: String,
    ) -> Result<(), AdmitError> {
        let scope = project_scope(project_id);
        let record = CredentialRecord {
            id: provider,
            op: RecordOp::Upsert,
            sealed_token,
        };
        self.store_mut().append_record(
            &scope,
            "credential",
            &serde_json::to_string(&record).unwrap(),
        )?;
        self.notify_library_changed(&scope, &record.id, "upsert");
        Ok(())
    }

    /// Tombstone a project's BYOK credential pin.
    pub fn tombstone_project_credential(
        &mut self,
        project_id: &str,
        provider: String,
    ) -> Result<(), AdmitError> {
        let scope = project_scope(project_id);
        let record = CredentialRecord {
            id: provider,
            op: RecordOp::Tombstone,
            sealed_token: String::new(),
        };
        self.store_mut().append_record(
            &scope,
            "credential",
            &serde_json::to_string(&record).unwrap(),
        )?;
        self.notify_library_changed(&scope, &record.id, "upsert");
        Ok(())
    }

    /// Folded trusted-device records for the local account.
    pub fn account_devices(&self) -> Result<Vec<DeviceRecord>, AdmitError> {
        self.account_devices_in(ACCOUNT_SCOPE)
    }

    /// Trusted devices in `scope` (the caller's account, `ADR 0077`).
    pub fn account_devices_in(&self, scope: &str) -> Result<Vec<DeviceRecord>, AdmitError> {
        Ok(Account::rebuild_in(self.store_ref(), scope)?
            .devices
            .into_values()
            .collect())
    }

    /// Folded settings for the local account (default scope).
    pub fn account_settings(&self) -> Result<BTreeMap<String, String>, AdmitError> {
        self.account_settings_in(ACCOUNT_SCOPE)
    }

    /// Folded settings for the account in `scope` (the caller's account, `ADR 0077`).
    pub fn account_settings_in(&self, scope: &str) -> Result<BTreeMap<String, String>, AdmitError> {
        Ok(Account::rebuild_in(self.store_ref(), scope)?
            .settings
            .into_values()
            .map(|setting| (setting.id, setting.value))
            .collect())
    }

    /// Provider ids linked as sealed local account credentials (default scope).
    pub fn account_credential_providers(&self) -> Result<Vec<String>, AdmitError> {
        self.account_credential_providers_in(ACCOUNT_SCOPE)
    }

    /// Provider ids linked in `scope` (the caller's account).
    pub fn account_credential_providers_in(&self, scope: &str) -> Result<Vec<String>, AdmitError> {
        Ok(Account::rebuild_in(self.store_ref(), scope)?
            .credentials
            .into_keys()
            .collect())
    }

    /// Persist one account record into `scope` (one person's account scope, `ADR 0077`) and
    /// publish the change ref. `pub` so the per-request account routes write to the caller's
    /// [`account_scope`] rather than the shared default.
    pub fn write_account_record_in<T: serde::Serialize>(
        &mut self,
        scope: &str,
        kind: &str,
        id: &str,
        record: &T,
    ) -> Result<(), AdmitError> {
        self.store_mut()
            .append_record(scope, kind, &serde_json::to_string(record).unwrap())?;
        self.notify_library_changed("account", id, "upsert");
        Ok(())
    }

    /// Enroll or update one trusted device record (default scope).
    pub fn upsert_account_device(&mut self, record: &DeviceRecord) -> Result<(), AdmitError> {
        self.upsert_account_device_in(ACCOUNT_SCOPE, record)
    }

    /// Enroll or update one trusted device in `scope` (the caller's account).
    pub fn upsert_account_device_in(
        &mut self,
        scope: &str,
        record: &DeviceRecord,
    ) -> Result<(), AdmitError> {
        self.write_account_record_in(scope, "device", &record.id, record)
    }

    /// Mark one trusted device revoked in `scope`, preserving its label/subkey metadata.
    pub fn revoke_account_device_in(
        &mut self,
        scope: &str,
        device_id: &str,
    ) -> Result<Option<DeviceRecord>, AdmitError> {
        let account = Account::rebuild_in(self.store_ref(), scope)?;
        let Some(existing) = account.devices.get(device_id) else {
            return Ok(None);
        };
        let mut record = existing.clone();
        record.op = RecordOp::Upsert;
        record.status = DeviceStatus::Revoked;
        let id = record.id.clone();
        self.write_account_record_in(scope, "device", &id, &record)?;
        Ok(Some(record))
    }

    /// Mark one trusted device revoked (default scope).
    pub fn revoke_account_device(
        &mut self,
        device_id: &str,
    ) -> Result<Option<DeviceRecord>, AdmitError> {
        self.revoke_account_device_in(ACCOUNT_SCOPE, device_id)
    }

    /// Persist one account setting (default scope).
    pub fn upsert_account_setting(&mut self, record: &SettingRecord) -> Result<(), AdmitError> {
        self.upsert_account_setting_in(ACCOUNT_SCOPE, record)
    }

    /// Persist one account setting in `scope` (the caller's account).
    pub fn upsert_account_setting_in(
        &mut self,
        scope: &str,
        record: &SettingRecord,
    ) -> Result<(), AdmitError> {
        self.write_account_record_in(scope, "setting", &record.id, record)
    }

    /// Persist one sealed account credential (default scope).
    pub fn upsert_account_credential(
        &mut self,
        provider: String,
        sealed_token: String,
    ) -> Result<(), AdmitError> {
        self.upsert_account_credential_in(ACCOUNT_SCOPE, provider, sealed_token)
    }

    /// Persist one sealed account credential in `scope` (the caller's account).
    pub fn upsert_account_credential_in(
        &mut self,
        scope: &str,
        provider: String,
        sealed_token: String,
    ) -> Result<(), AdmitError> {
        let record = CredentialRecord {
            id: provider,
            op: RecordOp::Upsert,
            sealed_token,
        };
        self.write_account_record_in(scope, "credential", &record.id, &record)
    }

    /// Tombstone one account credential (default scope).
    pub fn tombstone_account_credential(&mut self, provider: String) -> Result<(), AdmitError> {
        self.tombstone_account_credential_in(ACCOUNT_SCOPE, provider)
    }

    /// Tombstone one account credential in `scope` (the caller's account).
    pub fn tombstone_account_credential_in(
        &mut self,
        scope: &str,
        provider: String,
    ) -> Result<(), AdmitError> {
        let record = CredentialRecord {
            id: provider,
            op: RecordOp::Tombstone,
            sealed_token: String::new(),
        };
        self.write_account_record_in(scope, "credential", &record.id, &record)
    }

    // --- facilities + tenancy (ADR 0077) -------------------------------------

    /// The account-level facilities in `scope` (the caller's account, `ADR 0077` §7) — the ones
    /// that follow the person into every tenant (e.g. library sync).
    pub fn account_facilities_in(
        &self,
        scope: &str,
    ) -> Result<crate::facility::Facilities, AdmitError> {
        crate::facility::Facilities::rebuild_in(self.store_ref(), scope)
    }

    /// The account-level facilities (default scope).
    pub fn account_facilities(&self) -> Result<crate::facility::Facilities, AdmitError> {
        self.account_facilities_in(ACCOUNT_SCOPE)
    }

    /// Attach or update one account-level facility in `scope` (the caller's account).
    pub fn upsert_account_facility_in(
        &mut self,
        scope: &str,
        record: &crate::facility::FacilityRecord,
    ) -> Result<(), AdmitError> {
        self.write_account_record_in(scope, crate::facility::FACILITY_KIND, &record.id, record)
    }

    /// Attach or update one account-level facility (default scope).
    pub fn upsert_account_facility(
        &mut self,
        record: &crate::facility::FacilityRecord,
    ) -> Result<(), AdmitError> {
        self.upsert_account_facility_in(ACCOUNT_SCOPE, record)
    }

    /// Detach (tombstone) one account-level facility in `scope`, if present — future-only
    /// revocation (`INV-18`). Returns the removed record, or `None`.
    pub fn revoke_account_facility_in(
        &mut self,
        scope: &str,
        id: &str,
    ) -> Result<Option<crate::facility::FacilityRecord>, AdmitError> {
        let facilities = crate::facility::Facilities::rebuild_in(self.store_ref(), scope)?;
        let Some(existing) = facilities.get(id) else {
            return Ok(None);
        };
        let mut record = existing.clone();
        record.op = RecordOp::Tombstone;
        let id = record.id.clone();
        self.write_account_record_in(scope, crate::facility::FACILITY_KIND, &id, &record)?;
        Ok(Some(record))
    }

    /// Detach (tombstone) one account-level facility (default scope).
    pub fn revoke_account_facility(
        &mut self,
        id: &str,
    ) -> Result<Option<crate::facility::FacilityRecord>, AdmitError> {
        self.revoke_account_facility_in(ACCOUNT_SCOPE, id)
    }

    /// The person's tenant switcher in `scope` (the caller's account, `ADR 0077` §9).
    pub fn account_tenancy_in(&self, scope: &str) -> Result<crate::tenancy::Tenancy, AdmitError> {
        crate::tenancy::Tenancy::rebuild_in(self.store_ref(), scope)
    }

    /// The person's tenant switcher (default scope). Empty on the solo desktop path.
    pub fn account_tenancy(&self) -> Result<crate::tenancy::Tenancy, AdmitError> {
        self.account_tenancy_in(ACCOUNT_SCOPE)
    }
}

#[derive(Clone)]
struct ResolvedCredentialCapability {
    credential_ref: String,
    material: CredentialMaterial,
}

impl std::fmt::Debug for ResolvedCredentialCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedCredentialCapability")
            .field("credential_ref", &self.credential_ref)
            .field("material", &self.material)
            .finish()
    }
}

impl CredentialCapability for ResolvedCredentialCapability {
    fn credential_ref(&self) -> &str {
        &self.credential_ref
    }

    fn resolve(&self, credential_ref: &str) -> io::Result<CredentialMaterial> {
        if credential_ref != self.credential_ref {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "credential capability reference mismatch",
            ));
        }
        Ok(self.material.clone())
    }
}

pub(crate) fn resolved_credential_capability(
    credential_ref: String,
    secret: String,
    account_id: Option<String>,
) -> Arc<dyn CredentialCapability> {
    Arc::new(ResolvedCredentialCapability {
        credential_ref,
        material: CredentialMaterial::new(secret, account_id),
    })
}

// --- ACCT-2 core: the sealed sync blob + the readable directory record ----------
//
// The blind directory ([ADR 0054](../../../specs/decisions/0054-account-directory-and-sealed-sync.md))
// holds exactly two things: a **sealed account blob** (opaque to it; your devices
// decrypt) and a **readable directory record** (pubkeys + addresses, never secrets).
// These are the *data model*; the always-on directory *service* is needs-infra.

/// The syncable account metadata, sealed into one blob for the directory. Carries the
/// device registry + settings + the (already-sealed) credentials — never your work.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountBlob {
    pub settings: BTreeMap<String, String>,
    pub devices: Vec<DeviceRecord>,
    pub credentials: Vec<CredentialRecord>,
}

/// Project the folded account to the syncable blob shape.
pub fn account_blob(acct: &Account) -> AccountBlob {
    AccountBlob {
        settings: acct
            .settings
            .values()
            .map(|s| (s.id.clone(), s.value.clone()))
            .collect(),
        devices: acct.devices.values().cloned().collect(),
        credentials: acct.credentials.values().cloned().collect(),
    }
}

/// Seal the account blob (hex ciphertext) for the directory to store opaquely, under the
/// account `key`.
pub fn seal_account_blob(key: [u8; 32], acct: &Account) -> Option<String> {
    let bytes = serde_json::to_vec(&account_blob(acct)).ok()?;
    account_encryptor(key).encrypt(&bytes).ok().map(hex::encode)
}

/// Open a sealed account blob back into its metadata — only the matching account `key`
/// can (fail-closed). This is what a device does after fetching the blob from the
/// directory.
pub fn open_account_blob(key: [u8; 32], sealed_hex: &str) -> Option<AccountBlob> {
    let ct = hex::decode(sealed_hex).ok()?;
    let pt = account_encryptor(key).decrypt(&ct).ok()?;
    serde_json::from_slice(&pt).ok()
}

/// The **readable** record the blind directory holds for routing/identity (`INV-10`:
/// pubkeys + addresses only, never secrets). It is what lets any device prove who you
/// are, see your devices, and find your placements — even with your machine off.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct DirectoryRecord {
    /// The account root public key (the self-certifying identity).
    pub root_pubkey: String,
    /// The active device subkey public keys (which devices are yours).
    pub device_pubkeys: Vec<String>,
    /// Rendezvous pointers to reach your placements (addresses, not content).
    pub placement_pointers: Vec<String>,
}

/// Build the directory record from the account + your placement pointers. Carries
/// **no** secrets: only the root pubkey, your active devices' pubkeys, and addresses.
pub fn directory_record(
    root_pubkey: &str,
    acct: &Account,
    placement_pointers: Vec<String>,
) -> DirectoryRecord {
    DirectoryRecord {
        root_pubkey: root_pubkey.to_string(),
        device_pubkeys: acct
            .active_devices()
            .iter()
            .map(|d| d.subkey_pubkey.clone())
            .collect(),
        placement_pointers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed account key for the sealing tests (stands in for the seed-derived key the
    /// [`Workbench::account_key`] resolver hands the sealing primitives at runtime).
    const KEY: [u8; 32] = [7u8; 32];
    /// A *different* key — the "not your key" negative in the fail-closed cases.
    const OTHER: [u8; 32] = [9u8; 32];

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn account_scope_isolates_state_per_person() {
        // ADR 0077: the hosted hub keys each person's account under account::<root>, so one
        // person's devices/settings/credentials never fold into another's (INV-1). The empty
        // person collapses to the shared default scope (desktop unchanged).
        assert_eq!(account_scope(""), ACCOUNT_SCOPE);
        assert_ne!(account_scope("alice"), account_scope("bob"));
        assert_ne!(account_scope("alice"), ACCOUNT_SCOPE);

        let mut s = store();
        let setting = |v: &str| {
            serde_json::to_string(&SettingRecord {
                id: "theme".into(),
                op: RecordOp::Upsert,
                value: v.into(),
            })
            .unwrap()
        };
        s.append_record(&account_scope("alice"), "setting", &setting("dark"))
            .unwrap();

        // alice sees her setting; bob and the default scope see nothing.
        assert_eq!(
            Account::rebuild_in(&s, &account_scope("alice"))
                .unwrap()
                .settings
                .len(),
            1
        );
        assert!(Account::rebuild_in(&s, &account_scope("bob"))
            .unwrap()
            .settings
            .is_empty());
        assert!(Account::rebuild(&s).unwrap().settings.is_empty());
    }

    #[test]
    fn devices_settings_credentials_fold_latest_wins() {
        let mut s = store();
        let dev = DeviceRecord {
            id: "phone".into(),
            op: RecordOp::Upsert,
            label: "My phone".into(),
            subkey_pubkey: "abcd".into(),
            status: DeviceStatus::Active,
        };
        s.append_record(
            ACCOUNT_SCOPE,
            "device",
            &serde_json::to_string(&dev).unwrap(),
        )
        .unwrap();
        let setting = SettingRecord {
            id: "theme".into(),
            op: RecordOp::Upsert,
            value: "dark".into(),
        };
        s.append_record(
            ACCOUNT_SCOPE,
            "setting",
            &serde_json::to_string(&setting).unwrap(),
        )
        .unwrap();

        let acct = Account::rebuild(&s).unwrap();
        assert_eq!(acct.devices.len(), 1);
        assert_eq!(acct.settings.get("theme").unwrap().value, "dark");
        assert_eq!(acct.active_devices().len(), 1);
    }

    #[test]
    fn revoking_a_device_keeps_the_record() {
        let mut s = store();
        let mut dev = DeviceRecord {
            id: "old".into(),
            op: RecordOp::Upsert,
            label: "Old laptop".into(),
            subkey_pubkey: "ff".into(),
            status: DeviceStatus::Active,
        };
        s.append_record(
            ACCOUNT_SCOPE,
            "device",
            &serde_json::to_string(&dev).unwrap(),
        )
        .unwrap();
        dev.status = DeviceStatus::Revoked;
        s.append_record(
            ACCOUNT_SCOPE,
            "device",
            &serde_json::to_string(&dev).unwrap(),
        )
        .unwrap();

        let acct = Account::rebuild(&s).unwrap();
        // Record preserved (INV-6), but no longer active.
        assert_eq!(acct.devices.len(), 1);
        assert!(acct.active_devices().is_empty());
        assert_eq!(
            acct.devices.get("old").unwrap().status,
            DeviceStatus::Revoked
        );
    }

    #[test]
    fn token_seals_and_unseals_and_is_never_plaintext_at_rest() {
        let token = "sk-oauth-secret-12345";
        let sealed = seal_token(KEY, token).unwrap();
        assert!(
            !sealed.contains("secret"),
            "ciphertext is not the plaintext"
        );
        assert_eq!(unseal_token(KEY, &sealed).unwrap(), token);
        // A different authority's key cannot unseal it.
        assert_eq!(unseal_token(OTHER, &sealed), None);
    }

    fn seeded_account() -> Account {
        let mut acct = Account::default();
        acct.settings.insert(
            "theme".into(),
            SettingRecord {
                id: "theme".into(),
                op: RecordOp::Upsert,
                value: "dark".into(),
            },
        );
        acct.devices.insert(
            "phone".into(),
            DeviceRecord {
                id: "phone".into(),
                op: RecordOp::Upsert,
                label: "Phone".into(),
                subkey_pubkey: "dev-pub-1".into(),
                status: DeviceStatus::Active,
            },
        );
        acct.credentials.insert(
            "openai".into(),
            CredentialRecord {
                id: "openai".into(),
                op: RecordOp::Upsert,
                sealed_token: seal_token(KEY, "tok").unwrap(),
            },
        );
        acct
    }

    #[test]
    fn credential_envs_maps_linked_providers_to_resolved_tokens() {
        let mut s = store();
        for (provider, token) in [
            ("openai", "tok-oai"),
            ("anthropic", "tok-ant"),
            ("unknown", "x"),
        ] {
            let rec = CredentialRecord {
                id: provider.into(),
                op: RecordOp::Upsert,
                sealed_token: seal_token(KEY, token).unwrap(),
            };
            s.append_record(
                ACCOUNT_SCOPE,
                "credential",
                &serde_json::to_string(&rec).unwrap(),
            )
            .unwrap();
        }
        let envs = credential_envs(&s, KEY);
        // Known providers map to their env var with the decrypted token; unknown skipped.
        assert!(envs.contains(&("OPENAI_API_KEY".to_string(), "tok-oai".to_string())));
        assert!(envs.contains(&("ANTHROPIC_API_KEY".to_string(), "tok-ant".to_string())));
        assert_eq!(envs.len(), 2);
    }

    #[test]
    fn account_blob_seals_and_opens_only_with_your_key() {
        let acct = seeded_account();
        let sealed = seal_account_blob(KEY, &acct).unwrap();
        // The directory stores opaque ciphertext — no readable settings/devices.
        assert!(!sealed.contains("dark") && !sealed.contains("phone"));
        // Your key opens it back to the same metadata.
        let blob = open_account_blob(KEY, &sealed).unwrap();
        assert_eq!(blob, account_blob(&acct));
        assert_eq!(blob.settings.get("theme").unwrap(), "dark");
        // A different key cannot (fail-closed).
        assert_eq!(open_account_blob(OTHER, &sealed), None);
    }

    #[test]
    fn directory_record_is_routing_only_no_secrets() {
        let acct = seeded_account();
        let rec = directory_record("root-pub", &acct, vec!["relay://abc".into()]);
        assert_eq!(rec.root_pubkey, "root-pub");
        assert_eq!(rec.device_pubkeys, vec!["dev-pub-1".to_string()]);
        assert_eq!(rec.placement_pointers, vec!["relay://abc".to_string()]);
        // No secret ever appears in the readable directory record.
        let raw = serde_json::to_string(&rec).unwrap();
        assert!(!raw.contains("tok") && !raw.contains("sealed") && !raw.contains("dark"));
    }

    #[test]
    fn resolve_token_decrypts_the_stored_credential() {
        let mut s = store();
        let cred = CredentialRecord {
            id: "openai".into(),
            op: RecordOp::Upsert,
            sealed_token: seal_token(KEY, "tok-abc").unwrap(),
        };
        s.append_record(
            ACCOUNT_SCOPE,
            "credential",
            &serde_json::to_string(&cred).unwrap(),
        )
        .unwrap();

        // The stored record is ciphertext, not the token…
        let acct = Account::rebuild(&s).unwrap();
        assert!(!acct
            .credentials
            .get("openai")
            .unwrap()
            .sealed_token
            .contains("tok-abc"));
        // …but the runtime resolves the plaintext.
        assert_eq!(resolve_token(&s, KEY, "openai").as_deref(), Some("tok-abc"));
        assert_eq!(resolve_token(&s, KEY, "anthropic"), None);
    }

    /// LLM-2 (ADR 0062): a credential pinned in the chat's project overrides the account
    /// default **per provider** (nearest-scope-wins); providers the project does not pin
    /// fall through to the account; `project = None` resolves to the account alone.
    #[test]
    fn project_pin_beats_account_per_provider() {
        let mut s = store();
        let seal = |tok: &str| seal_token(KEY, tok).unwrap();
        let put = |s: &mut Store, scope: &str, provider: &str, tok: &str| {
            let rec = CredentialRecord {
                id: provider.into(),
                op: RecordOp::Upsert,
                sealed_token: seal(tok),
            };
            s.append_record(scope, "credential", &serde_json::to_string(&rec).unwrap())
                .unwrap();
        };
        // Account links both providers; the project re-pins only openai.
        put(&mut s, ACCOUNT_SCOPE, "openai", "acct-openai");
        put(&mut s, ACCOUNT_SCOPE, "anthropic", "acct-anthropic");
        put(&mut s, &project_scope("proj-1"), "openai", "proj-openai");

        // No project → account default for both.
        let base: BTreeMap<_, _> = resolved_credential_envs(&s, KEY, None)
            .into_iter()
            .collect();
        assert_eq!(
            base.get("OPENAI_API_KEY").map(String::as_str),
            Some("acct-openai")
        );
        assert_eq!(
            base.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("acct-anthropic")
        );

        // In proj-1: openai is overridden, anthropic inherited from the account.
        let scoped: BTreeMap<_, _> = resolved_credential_envs(&s, KEY, Some("proj-1"))
            .into_iter()
            .collect();
        assert_eq!(
            scoped.get("OPENAI_API_KEY").map(String::as_str),
            Some("proj-openai")
        );
        assert_eq!(
            scoped.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("acct-anthropic")
        );

        // A project with no pin resolves exactly to the account default.
        let other = resolved_credential_envs(&s, KEY, Some("proj-other"));
        assert_eq!(other, resolved_credential_envs(&s, KEY, None));
    }

    #[test]
    fn resolved_credential_capability_is_exact_reference_bound_and_redacted() {
        let capability = resolved_credential_capability(
            "gaugedesk:credential:account:openai".to_owned(),
            "sk-secret-value".to_owned(),
            None,
        );
        assert!(capability
            .resolve("gaugedesk:credential:project:other:openai")
            .is_err());
        let material = capability
            .resolve("gaugedesk:credential:account:openai")
            .expect("exact ref resolves");
        assert_eq!(material.secret(), "sk-secret-value");
        assert!(!format!("{capability:?}").contains("sk-secret-value"));
        assert!(!format!("{material:?}").contains("sk-secret-value"));
    }
}

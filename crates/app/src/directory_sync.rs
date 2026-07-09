//! Library sync — the account-side **blind-directory client** (`ADR 0054` / the `library_sync`
//! facility, `ADR 0077`). The device holding the account root key publishes its **sealed account
//! blob** + a **readable directory record** to the always-on blind directory
//! ([`crate::account::AccountBlob`] / [`crate::account::DirectoryRecord`]), so the person's other
//! (enrolled) devices can pull their account state — devices, settings, the sealed linked model
//! key — even with the first machine off. The directory stores the blob **opaquely** (`INV-10`);
//! only a device holding the shared account key can open it.
//!
//! This is the **client** half of the deployed directory *service* (`gaugewright-directory`,
//! `PUT/GET /directory/:root`): it signs the publish with the root key (only the root may
//! overwrite its own record) and fetches the opaque record back. The wire shapes
//! ([`DirectoryEntry`] / [`SignedDirectoryPut`] / [`signing_bytes`]) mirror the service exactly —
//! same [`DirectoryRecord`] + `serde_json`, so the signatures agree — and are the canonical
//! definitions the service should import (a byte-compatible unify is owed).
//!
//! It runs where the **root key lives** — the desktop workbench (the hosted hub authenticates a
//! person via OIDC and holds no sovereign keypair, so it does not publish). The facility flag in
//! the person's account scope says whether sync is on.

use serde::{Deserialize, Serialize};

use gaugewright_core::ids::PublicKey;
use gaugewright_core::signature::{verify_signature, Signature, SigningKey};

use crate::account::{directory_record, seal_account_blob, Account, DirectoryRecord};
use crate::key_store::KeyStore; // brings the `signing_key` trait method into scope
use crate::net_http::HttpClient;

/// What the blind directory holds for one account root: the readable routing record + the opaque
/// sealed [`AccountBlob`](crate::account::AccountBlob) (hex ciphertext the directory never opens).
/// The wire shape the directory service stores; identical field order → identical [`signing_bytes`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub directory: DirectoryRecord,
    /// Opaque hex ciphertext (`INV-10`) — the sealed account blob. Never opened by the directory.
    pub sealed_blob: String,
}

/// The canonical bytes a root signs to authorize publishing `entry` — the same function feeds the
/// signer here and the service's verifier, so they always agree.
pub fn signing_bytes(entry: &DirectoryEntry) -> Vec<u8> {
    serde_json::to_vec(entry).unwrap_or_default()
}

/// A signed publish request: the entry + the root key's signature over [`signing_bytes`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedDirectoryPut {
    pub entry: DirectoryEntry,
    pub signature: Signature,
}

/// Build the signed publish for the account rooted at `signing_key`: seal the account blob under
/// the account `key`, assemble the readable directory record (root pubkey + active device
/// pubkeys + placement pointers), and sign it with the root key. Pure (no I/O), so it is testable
/// against the same [`verify_signature`] the service runs. `None` if the blob fails to seal.
pub fn signed_put(
    signing_key: &SigningKey,
    key: [u8; 32],
    acct: &Account,
    placement_pointers: Vec<String>,
) -> Option<SignedDirectoryPut> {
    let root_pubkey = signing_key.public_key().as_str().to_string();
    let entry = DirectoryEntry {
        directory: directory_record(&root_pubkey, acct, placement_pointers),
        sealed_blob: seal_account_blob(key, acct)?,
    };
    let signature = signing_key.sign(&signing_bytes(&entry));
    Some(SignedDirectoryPut { entry, signature })
}

/// Whether `put` verifies under its own claimed root pubkey — the exact check the directory service
/// performs at `PUT` (`verify_signature` over [`signing_bytes`] against `entry.directory.root_pubkey`).
/// The publish path uses this to fail fast; tests use it to prove service-compatibility.
pub fn put_verifies(put: &SignedDirectoryPut) -> bool {
    let pubkey = PublicKey::new(put.entry.directory.root_pubkey.clone());
    verify_signature(&signing_bytes(&put.entry), &put.signature, &pubkey).unwrap_or(false)
}

/// Publish a signed record to the blind directory (`PUT {base}/directory/:root`). `base` is the
/// directory service origin (e.g. `https://…:7901`). A non-2xx status is an error.
pub fn publish(http: &HttpClient, base: &str, put: &SignedDirectoryPut) -> Result<(), String> {
    let root = &put.entry.directory.root_pubkey;
    let url = format!("{}/directory/{}", base.trim_end_matches('/'), root);
    let body = serde_json::to_string(put).map_err(|e| format!("serialize: {e}"))?;
    let (status, resp) = http.put_json(&url, &body)?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(format!("directory publish HTTP {status}: {resp}"))
    }
}

/// Fetch the readable record + opaque sealed blob for `root` (`GET {base}/directory/:root`).
/// `Ok(None)` when the directory has nothing for `root` (404). The blob stays sealed — the caller
/// opens it with [`crate::account::open_account_blob`] under the account key.
pub fn fetch(http: &HttpClient, base: &str, root: &str) -> Result<Option<DirectoryEntry>, String> {
    let url = format!("{}/directory/{}", base.trim_end_matches('/'), root);
    match http.get_string(&url) {
        Ok(body) => {
            let entry = serde_json::from_str(&body).map_err(|e| format!("parse entry: {e}"))?;
            Ok(Some(entry))
        }
        // get_string errors on non-2xx; a 404 (no record yet) is not a failure.
        Err(e) if e.contains("HTTP 404") => Ok(None),
        Err(e) => Err(e),
    }
}

impl crate::Workbench {
    /// Whether the `library_sync` facility is active for this account (`ADR 0077`). The store-side
    /// gate the publish/pull halves check; sync is off unless the person attached it.
    pub fn library_sync_active(&self) -> bool {
        crate::facility::Facilities::rebuild_account(self.store_ref())
            .map(|f| f.has_active(crate::facility::FacilityKind::LibrarySync))
            .unwrap_or(false)
    }

    /// The account root pubkey this workbench publishes under (its governance key) — the directory
    /// path/identity for [`fetch`].
    pub fn library_sync_root(&self) -> String {
        self.governance_public_key().as_str().to_string()
    }

    /// The signed publish for the current account state, **iff** library sync is active — built
    /// under the workbench lock (store + root key), so the caller can then publish it over the
    /// network *off* the lock. `None` when sync is off or the blob fails to seal.
    pub fn library_sync_signed_put(&self) -> Option<SignedDirectoryPut> {
        if !self.library_sync_active() {
            return None;
        }
        let signing_key = crate::key_store::FileKeyStore::new(self.root_path().join("keys"))
            .signing_key(self.authority());
        let acct = Account::rebuild(self.store_ref()).ok()?;
        signed_put(&signing_key, self.account_key(), &acct, vec![])
    }

    /// Merge a fetched directory entry's sealed blob into the local account scope (the pull half):
    /// open it under this account's key and upsert its devices/settings/credentials (latest-wins
    /// fold). Returns how many records merged; errors if the blob does not open (a foreign key).
    pub fn library_sync_apply(&mut self, entry: &DirectoryEntry) -> Result<usize, String> {
        use crate::account::{open_account_blob, ACCOUNT_SCOPE};
        let blob = open_account_blob(self.account_key(), &entry.sealed_blob)
            .ok_or_else(|| "sealed blob did not open under this account key".to_string())?;
        let mut n = 0usize;
        for d in &blob.devices {
            if self
                .write_account_record_in(ACCOUNT_SCOPE, "device", &d.id, d)
                .is_ok()
            {
                n += 1;
            }
        }
        for c in &blob.credentials {
            if self
                .write_account_record_in(ACCOUNT_SCOPE, "credential", &c.id, c)
                .is_ok()
            {
                n += 1;
            }
        }
        for (id, value) in &blob.settings {
            let rec = crate::account::SettingRecord {
                id: id.clone(),
                op: crate::account::RecordOp::Upsert,
                value: value.clone(),
            };
            if self
                .write_account_record_in(ACCOUNT_SCOPE, "setting", id, &rec)
                .is_ok()
            {
                n += 1;
            }
        }
        Ok(n)
    }
}

/// The blind-directory service origin from `GAUGEWRIGHT_DIRECTORY_URL` (e.g. the deployed
/// `https://…:7901`); `None` when unset (sync is then a no-op — the mechanism needs the service).
pub fn directory_url_from_env() -> Option<String> {
    std::env::var("GAUGEWRIGHT_DIRECTORY_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{open_account_blob, DeviceRecord, DeviceStatus, RecordOp, SettingRecord};

    fn seeded_account() -> Account {
        let mut a = Account::default();
        a.devices.insert(
            "phone".into(),
            DeviceRecord {
                id: "phone".into(),
                op: RecordOp::Upsert,
                label: "My phone".into(),
                subkey_pubkey: "dev-pub-1".into(),
                status: DeviceStatus::Active,
            },
        );
        a.settings.insert(
            "theme".into(),
            SettingRecord {
                id: "theme".into(),
                op: RecordOp::Upsert,
                value: "dark".into(),
            },
        );
        a
    }

    fn key() -> SigningKey {
        SigningKey::from_seed(&[7u8; 32]).unwrap()
    }

    /// A fixed account (sealing) key for these tests — distinct from the signing key.
    const AKEY: [u8; 32] = [11u8; 32];
    const OTHER_AKEY: [u8; 32] = [22u8; 32];

    #[test]
    fn signed_put_verifies_under_its_own_root_key() {
        // The signature the client produces passes the exact check the directory service runs at
        // PUT — so a real publish would be accepted (and a forged one rejected).
        let k = key();
        let put = signed_put(&k, AKEY, &seeded_account(), vec![]).expect("seals");
        assert_eq!(put.entry.directory.root_pubkey, k.public_key().as_str());
        assert!(put_verifies(&put), "verifies under its own root key");

        // Tampering with the entry (a different device pubkey) breaks the signature (fail-closed).
        let mut forged = put.clone();
        forged.entry.directory.device_pubkeys = vec!["attacker".into()];
        assert!(!put_verifies(&forged));
    }

    #[test]
    fn the_directory_record_carries_no_secrets_and_the_blob_round_trips() {
        // The readable record is routing-only (root + device pubkeys); the settings/credentials
        // live only inside the sealed blob, which opens only under the same account key.
        let put =
            signed_put(&key(), AKEY, &seeded_account(), vec!["relay://x".into()]).expect("seals");
        assert_eq!(
            put.entry.directory.device_pubkeys,
            vec!["dev-pub-1".to_string()]
        );
        assert_eq!(
            put.entry.directory.placement_pointers,
            vec!["relay://x".to_string()]
        );
        // the sealed blob is opaque hex, not the plaintext settings.
        assert!(!put.entry.sealed_blob.contains("dark"));
        // the same account key opens it back to the account metadata.
        let blob = open_account_blob(AKEY, &put.entry.sealed_blob).expect("opens");
        assert_eq!(blob.settings.get("theme").map(String::as_str), Some("dark"));
        assert_eq!(blob.devices.len(), 1);
        // a different account key cannot open it (fail-closed).
        assert!(open_account_blob(OTHER_AKEY, &put.entry.sealed_blob).is_none());
    }
}

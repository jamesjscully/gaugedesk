//! `KeyStore` — where an authority's / device's P-256 **governance signing key**
//! lives (D-CRYPTO / ADR 0042). The pure core does the crypto
//! ([`gaugewright_core::signature`]); *storage* is impure and lives here behind a trait,
//! so the loopback double, a file-backed dev impl, and a future secure-enclave /
//! TPM impl are interchangeable with no change to the signing path.

use std::path::PathBuf;

use gaugewright_core::ids::{AuthorityId, PublicKey};
use gaugewright_core::signature::SigningKey;
use sha2::{Digest, Sha256};

use crate::workbench_state::Workbench;

/// Resolve the signing key for an authority. Real signing paths (the relay
/// constructing a federated envelope, the challenge/response handshake) take a
/// `&dyn KeyStore` and never see raw key material beyond the `SigningKey`.
pub trait KeyStore {
    /// The authority's signing key, deriving + persisting one on first use.
    fn signing_key(&self, authority: &AuthorityId) -> SigningKey;
}

/// Loopback/dev double: derive a deterministic key from the authority id (SHA-256
/// → seed), with **no storage**. This makes the loopback bridge sign and verify
/// with real P-256 — but it is **not secure** (the key is derivable from the
/// public id). Real deployments enroll keys in [`FileKeyStore`] (or a secure
/// enclave) instead of deriving them.
#[derive(Default, Clone, Copy)]
pub struct LoopbackKeyStore;

impl KeyStore for LoopbackKeyStore {
    fn signing_key(&self, authority: &AuthorityId) -> SigningKey {
        SigningKey::from_seed(&seed_from(authority.as_str()))
            .expect("a SHA-256 digest is a valid P-256 scalar")
    }
}

/// File-backed dev store: persists each authority's 32-byte seed under
/// `dir/<authority>.key`, deriving + writing one on first use. The on-disk format
/// is the bare seed (the secure-enclave impl replaces *where* the bytes live, not
/// the trait). Not for production secrets — a real install seals these.
#[derive(Clone)]
pub struct FileKeyStore {
    dir: PathBuf,
}

impl FileKeyStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, authority: &AuthorityId) -> PathBuf {
        // authority ids are scope segments (no path separators); hex-namespace to
        // be safe against odd characters.
        self.dir
            .join(format!("{}.key", hex::encode(authority.as_str())))
    }

    /// Enroll a specific key for an authority (overwrites). Used by tests and by
    /// out-of-band key import.
    pub fn enroll(&self, authority: &AuthorityId, key: &SigningKey) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.path(authority), key.to_seed_bytes())
    }
}

impl KeyStore for FileKeyStore {
    fn signing_key(&self, authority: &AuthorityId) -> SigningKey {
        if let Ok(bytes) = std::fs::read(self.path(authority)) {
            if let Ok(seed) = <[u8; 32]>::try_from(bytes.as_slice()) {
                if let Ok(key) = SigningKey::from_seed(&seed) {
                    return key;
                }
            }
        }
        // First use (or unreadable): derive deterministically and persist it, so
        // the same authority gets a stable key across restarts.
        let key = LoopbackKeyStore.signing_key(authority);
        let _ = self.enroll(authority, &key);
        key
    }
}

fn seed_from(s: &str) -> [u8; 32] {
    let digest = Sha256::digest(s.as_bytes());
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&digest);
    seed
}

impl Workbench {
    /// This authority's P-256 governance **public** key, resolved (deriving +
    /// persisting on first use) through the file-backed [`FileKeyStore`] under
    /// `root/keys`. This is the value pinned in a peer's bridge grant at pairing
    /// (`SERVE-1`/ADR 0042). The private half never leaves the key store.
    pub fn governance_public_key(&self) -> PublicKey {
        FileKeyStore::new(self.root_path().join("keys"))
            .signing_key(self.authority())
            .public_key()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_keys_are_stable_per_authority_and_distinct_across() {
        let ks = LoopbackKeyStore;
        let a = ks.signing_key(&AuthorityId::new("acme"));
        let a2 = ks.signing_key(&AuthorityId::new("acme"));
        let b = ks.signing_key(&AuthorityId::new("other"));
        assert_eq!(
            a.to_seed_bytes(),
            a2.to_seed_bytes(),
            "stable per authority"
        );
        assert_ne!(
            a.to_seed_bytes(),
            b.to_seed_bytes(),
            "distinct across authorities"
        );
    }

    #[test]
    fn signed_then_verified_through_the_store() {
        use gaugewright_core::signature::verify_signature;
        let ks = LoopbackKeyStore;
        let key = ks.signing_key(&AuthorityId::new("acme"));
        let sig = key.sign(b"challenge");
        assert_eq!(
            verify_signature(b"challenge", &sig, &key.public_key()),
            Ok(true)
        );
    }

    #[test]
    fn file_store_persists_and_reloads_the_same_key() {
        let dir =
            std::env::temp_dir().join(format!("gaugewright-keystore-test-{}", std::process::id()));
        let ks = FileKeyStore::new(&dir);
        let authority = AuthorityId::new("acme");
        let first = ks.signing_key(&authority).to_seed_bytes();
        // a fresh store over the same dir reloads the persisted key, not a new one.
        let reloaded = FileKeyStore::new(&dir)
            .signing_key(&authority)
            .to_seed_bytes();
        assert_eq!(first, reloaded);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

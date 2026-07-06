//! Sovereign-peer recovery (`remote-substrate.md` "Recovery"): export the root
//! key's 32-byte seed as a transcribable, checksummed **recovery code**, and restore
//! it on a new device. This is the **baseline self-custody backup** — *restore ==
//! recover* — so a lost sovereign peer re-establishes the **same** root identity and
//! its peers keep the keys they pinned. (Social / multi-device recovery is the
//! deferred upgrade; re-enrollment under a *fresh* root is the fallback when no
//! backup exists — the seedless client persona's primary path, ADR 0044.)
//!
//! The code is the seed in human-transferable form. It is therefore **as sensitive
//! as the private key**: whoever holds it controls the authority. It carries a small
//! integrity checksum so a transcription typo is caught on import rather than
//! silently restoring a wrong (or invalid) key.

use crate::signature::SigningKey;

const SEED_LEN: usize = 32;

/// Why a recovery code could not be imported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryError {
    /// The code did not decode to the expected length (not a recovery code).
    Malformed,
    /// The checksum did not match — a transcription error in the code.
    Checksum,
    /// The decoded seed is not a valid P-256 signing key.
    BadSeed,
}

/// A non-cryptographic 2-byte integrity check (FNV-1a, truncated): it catches
/// transcription typos, nothing more. The seed itself is the secret — this guards
/// integrity, not confidentiality (so no crypto hash / extra dependency is needed).
fn checksum(bytes: &[u8]) -> [u8; 2] {
    let mut h: u32 = 0x811c_9dc5;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    [(h >> 8) as u8, h as u8]
}

/// Export `key`'s seed as a grouped, checksummed recovery code (upper-hex in 4-char
/// groups). **SENSITIVE** — this *is* the private key in transcribable form; treat it
/// like the seed it encodes.
pub fn export_recovery(key: &SigningKey) -> String {
    let seed = key.to_seed_bytes();
    let mut buf = seed.to_vec();
    buf.extend_from_slice(&checksum(&seed));
    hex::encode_upper(&buf)
        .as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("-")
}

/// Restore a [`SigningKey`] from a recovery code, ignoring dashes / whitespace /
/// case and verifying the checksum. The recovered key has the **same** public
/// identity as the one exported — restore is recovery.
pub fn import_recovery(code: &str) -> Result<SigningKey, RecoveryError> {
    let cleaned: String = code.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let bytes = hex::decode(&cleaned).map_err(|_| RecoveryError::Malformed)?;
    if bytes.len() != SEED_LEN + 2 {
        return Err(RecoveryError::Malformed);
    }
    let (seed, sum) = bytes.split_at(SEED_LEN);
    if checksum(seed) != sum {
        return Err(RecoveryError::Checksum);
    }
    let mut s = [0u8; SEED_LEN];
    s.copy_from_slice(seed);
    SigningKey::from_seed(&s).map_err(|_| RecoveryError::BadSeed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SigningKey {
        SigningKey::from_seed(&[5u8; 32]).unwrap()
    }

    #[test]
    fn a_recovery_code_round_trips_to_the_same_identity() {
        let original = key();
        let code = export_recovery(&original);
        // The code is grouped + upper-hex, so it is transcribable.
        assert!(code.contains('-'));
        let restored = import_recovery(&code).expect("a genuine code restores");
        assert_eq!(
            restored.to_seed_bytes(),
            original.to_seed_bytes(),
            "restore recovers the same seed"
        );
        assert_eq!(
            restored.public_key(),
            original.public_key(),
            "and therefore the same public identity"
        );
    }

    #[test]
    fn import_tolerates_formatting_noise() {
        let code = export_recovery(&key());
        // Lower-cased, spaces, missing dashes — all tolerated (only hex digits count).
        let messy = code.to_lowercase().replace('-', "  ");
        assert_eq!(
            import_recovery(&messy).unwrap().public_key(),
            key().public_key()
        );
    }

    #[test]
    fn a_transcription_typo_is_caught_by_the_checksum() {
        let code = export_recovery(&key());
        // Flip one hex digit (0<->1) somewhere in the body.
        let typo: String = {
            let mut done = false;
            code.chars()
                .map(|c| {
                    if !done && c == '0' {
                        done = true;
                        '1'
                    } else if !done && c == '1' {
                        done = true;
                        '0'
                    } else {
                        c
                    }
                })
                .collect()
        };
        assert_ne!(typo, code, "the test actually changed a digit");
        assert!(matches!(
            import_recovery(&typo),
            Err(RecoveryError::Checksum)
        ));
    }

    #[test]
    fn a_too_short_code_is_malformed() {
        assert!(matches!(
            import_recovery("DEAD-BEEF"),
            Err(RecoveryError::Malformed)
        ));
    }
}

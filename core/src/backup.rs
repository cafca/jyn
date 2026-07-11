//! Identity backup and encrypted state snapshots.
//!
//! Everything hangs off the 32-byte identity seed in `node.key`: the X25519
//! key-agreement secret and the friends-space id derive from it (see
//! `crate::spaces`), so a 24-word BIP39 phrase over that seed recovers the
//! full encryption identity.
//!
//! The state snapshot covers what cannot be re-derived or re-synced with
//! certainty: the domain store (operations, group-encryption state, key
//! secrets — all opaque, per the spec a first-class backup concern) and the
//! profile-data store (private posts, keeps, outgoing requests). It is
//! encrypted to a key derived from the identity seed, so possession of the
//! archive alone reveals nothing; restore needs archive + seed phrase.
//!
//! Blob bytes are not yet included (spec phase 2); after a restore, media
//! re-fetches from peers that still hold it.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use p2panda_core::SigningKey;
use p2panda_encryption::crypto::aead::{aead_decrypt, aead_encrypt, AeadKey, AeadNonce};
use p2panda_encryption::crypto::hkdf::hkdf;
use p2panda_encryption::Rng;
use serde::{Deserialize, Serialize};

use crate::data_schema::DATA_SCHEMA_VERSION;

const MAGIC: &[u8; 8] = b"JYNBAK01";
const NODE_KEY_FILE: &str = "node.key";
const SCHEMA_VERSION_FILE: &str = "schema.version";

/// Files snapshotted into the archive, relative to the data dir. Sqlite
/// stores are snapshotted via `VACUUM INTO` (consistent while live), the
/// rest byte-for-byte.
const PLAIN_FILES: &[&str] = &["settings.json"];

#[derive(Serialize, Deserialize)]
struct BackupPayload {
    /// Data-schema version the stores were written by; restoring into a
    /// different app version would hand incompatible stores to the wipe.
    schema_version: u32,
    /// File name → contents.
    files: Vec<(String, serde_bytes::ByteBuf)>,
}

/// The 24-word BIP39 phrase encoding the identity seed.
pub fn seed_phrase(private_key: &SigningKey) -> Result<String> {
    let mnemonic = bip39::Mnemonic::from_entropy(private_key.as_bytes())
        .context("failed to encode identity key as seed phrase")?;
    Ok(mnemonic.to_string())
}

/// Recovers the identity key from a seed phrase.
pub fn key_from_seed_phrase(phrase: &str) -> Result<SigningKey> {
    let mnemonic: bip39::Mnemonic = phrase
        .trim()
        .parse()
        .context("that doesn't look like a valid recovery phrase")?;
    let (entropy, len) = mnemonic.to_entropy_array();
    anyhow::ensure!(
        len == 32,
        "recovery phrase must encode a 32-byte key (24 words)"
    );
    let seed: [u8; 32] = entropy[..32].try_into().expect("checked length");
    Ok(SigningKey::from_bytes(&seed))
}

fn backup_key(private_key: &SigningKey) -> Result<AeadKey> {
    hkdf::<32>(b"jyn/backup/v1", private_key.as_bytes(), None)
        .map_err(|err| anyhow::anyhow!("failed to derive backup key: {err}"))
}

/// Seals collected files into an encrypted archive at `dest`.
pub fn write_archive(
    private_key: &SigningKey,
    files: Vec<(String, Vec<u8>)>,
    dest: &Path,
) -> Result<()> {
    let payload = BackupPayload {
        schema_version: DATA_SCHEMA_VERSION,
        files: files
            .into_iter()
            .map(|(name, bytes)| (name, serde_bytes::ByteBuf::from(bytes)))
            .collect(),
    };
    let plaintext =
        p2panda_core::cbor::encode_cbor(&payload).context("failed to encode backup payload")?;
    let key = backup_key(private_key)?;
    let nonce: AeadNonce = Rng::default()
        .random_array()
        .map_err(|err| anyhow::anyhow!("failed to draw backup nonce: {err}"))?;
    let ciphertext = aead_encrypt(&key, &plaintext, nonce, Some(MAGIC))
        .map_err(|err| anyhow::anyhow!("failed to encrypt backup: {err}"))?;

    let mut out = Vec::with_capacity(MAGIC.len() + nonce.len() + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    fs::write(dest, out).with_context(|| format!("failed to write backup {}", dest.display()))?;
    Ok(())
}

fn read_archive(private_key: &SigningKey, archive: &Path) -> Result<BackupPayload> {
    let bytes = fs::read(archive)
        .with_context(|| format!("failed to read backup {}", archive.display()))?;
    anyhow::ensure!(
        bytes.len() > MAGIC.len() + 12 && &bytes[..MAGIC.len()] == MAGIC,
        "this is not a jyn backup file"
    );
    let nonce: AeadNonce = bytes[MAGIC.len()..MAGIC.len() + 12]
        .try_into()
        .expect("checked length");
    let key = backup_key(private_key)?;
    let plaintext = aead_decrypt(&key, &bytes[MAGIC.len() + 12..], nonce, Some(MAGIC))
        .map_err(|_| anyhow::anyhow!("backup does not match this recovery phrase"))?;
    p2panda_core::cbor::decode_cbor(&plaintext[..]).context("failed to decode backup payload")
}

/// Names of the sqlite stores included in a snapshot, in `VACUUM INTO`
/// snapshot order.
pub const SQLITE_STORES: &[&str] = &["domain.sqlite3", "profile-store.sqlite3"];

/// Collects the non-sqlite files for the archive from a data dir.
pub fn collect_plain_files(data_dir: &Path) -> Vec<(String, Vec<u8>)> {
    PLAIN_FILES
        .iter()
        .filter_map(|name| {
            fs::read(data_dir.join(name))
                .ok()
                .map(|bytes| (name.to_string(), bytes))
        })
        .collect()
}

/// Restores a backup into a (fresh or existing) data dir: writes the
/// identity key from the phrase, decrypts the archive with it, materializes
/// the store files and stamps the schema version so startup doesn't wipe
/// them. Must run before the node starts.
pub fn restore_backup(data_dir: &Path, archive: &Path, phrase: &str) -> Result<()> {
    let private_key = key_from_seed_phrase(phrase)?;
    let payload = read_archive(&private_key, archive)?;
    anyhow::ensure!(
        payload.schema_version == DATA_SCHEMA_VERSION,
        "this backup was written by an incompatible app version (data schema {} vs {})",
        payload.schema_version,
        DATA_SCHEMA_VERSION
    );

    fs::create_dir_all(data_dir).with_context(|| {
        format!(
            "failed to create data directory {} for restore",
            data_dir.display()
        )
    })?;
    // Restoring over live stores would corrupt them; the caller guarantees
    // the node is not running. Leftover -wal/-shm from a previous run would
    // shadow the restored db files, so clear them.
    for name in &payload.files {
        for suffix in ["-wal", "-shm"] {
            let _ = fs::remove_file(data_dir.join(format!("{}{}", name.0, suffix)));
        }
    }

    fs::write(data_dir.join(NODE_KEY_FILE), private_key.as_bytes())
        .context("failed to write restored identity key")?;
    for (name, bytes) in &payload.files {
        fs::write(data_dir.join(name), bytes)
            .with_context(|| format!("failed to restore {name}"))?;
    }
    fs::write(
        data_dir.join(SCHEMA_VERSION_FILE),
        format!("{DATA_SCHEMA_VERSION}\n"),
    )
    .context("failed to stamp restored data schema")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_phrase_round_trips_the_identity_key() -> Result<()> {
        let key = SigningKey::generate();
        let phrase = seed_phrase(&key)?;
        assert_eq!(phrase.split_whitespace().count(), 24);
        let recovered = key_from_seed_phrase(&phrase)?;
        assert_eq!(recovered.as_bytes(), key.as_bytes());
        Ok(())
    }

    #[test]
    fn archive_round_trips_and_rejects_wrong_phrase() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let key = SigningKey::generate();
        let archive = dir.path().join("jyn.backup");
        write_archive(
            &key,
            vec![("domain.sqlite3".into(), b"store-bytes".to_vec())],
            &archive,
        )?;

        let payload = read_archive(&key, &archive)?;
        assert_eq!(payload.schema_version, DATA_SCHEMA_VERSION);
        assert_eq!(payload.files[0].1.as_ref(), b"store-bytes");

        let wrong = SigningKey::generate();
        assert!(read_archive(&wrong, &archive).is_err());
        Ok(())
    }

    #[test]
    fn restore_materializes_identity_stores_and_schema_stamp() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let key = SigningKey::generate();
        let phrase = seed_phrase(&key)?;
        let archive = dir.path().join("jyn.backup");
        write_archive(
            &key,
            vec![
                ("domain.sqlite3".into(), b"domain".to_vec()),
                ("profile-store.sqlite3".into(), b"profile".to_vec()),
            ],
            &archive,
        )?;

        let restored = dir.path().join("restored");
        restore_backup(&restored, &archive, &phrase)?;

        assert_eq!(fs::read(restored.join("node.key"))?, key.as_bytes());
        assert_eq!(fs::read(restored.join("domain.sqlite3"))?, b"domain");
        assert_eq!(
            fs::read(restored.join("profile-store.sqlite3"))?,
            b"profile"
        );
        assert_eq!(
            fs::read_to_string(restored.join("schema.version"))?.trim(),
            DATA_SCHEMA_VERSION.to_string()
        );
        Ok(())
    }
}

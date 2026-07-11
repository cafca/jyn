//! Per-blob encryption for attachments on non-public posts.
//!
//! Each blob gets its own random ChaCha20-Poly1305 key + nonce; the pair
//! travels as `MediaAttachment::blob_secret` *inside* the group-encrypted
//! post payload, so only the post's audience can decrypt. The blob store and
//! the network only ever see ciphertext, and the content address is the
//! ciphertext hash — identical media in different posts yields unrelated
//! blob ids (no equality leak; see the spec's rejection of convergent
//! encryption).
//!
//! Whole files are processed in memory; the post-time size caps
//! ([`super::max_bytes_for_kind`]) bound this. Chunked streaming is a later
//! optimization.

use anyhow::{Context, Result};
use p2panda_encryption::crypto::aead::{aead_decrypt, aead_encrypt, AeadKey, AeadNonce};
use p2panda_encryption::Rng;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// Encrypts blob bytes under a fresh random key. Returns the ciphertext and
/// the opaque secret (key || nonce) to embed in the post payload.
pub fn encrypt_blob(plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let rng = Rng::default();
    let key: AeadKey = rng
        .random_array()
        .map_err(|err| anyhow::anyhow!("failed to draw blob key: {err}"))?;
    let nonce: AeadNonce = rng
        .random_array()
        .map_err(|err| anyhow::anyhow!("failed to draw blob nonce: {err}"))?;
    let ciphertext = aead_encrypt(&key, plaintext, nonce, None)
        .map_err(|err| anyhow::anyhow!("failed to encrypt blob: {err}"))?;
    let mut secret = Vec::with_capacity(KEY_LEN + NONCE_LEN);
    secret.extend_from_slice(&key);
    secret.extend_from_slice(&nonce);
    Ok((ciphertext, secret))
}

/// Decrypts blob bytes with a secret from a post payload.
pub fn decrypt_blob(ciphertext: &[u8], secret: &[u8]) -> Result<Vec<u8>> {
    anyhow::ensure!(
        secret.len() == KEY_LEN + NONCE_LEN,
        "blob secret has invalid length {}",
        secret.len()
    );
    let key: AeadKey = secret[..KEY_LEN].try_into().context("bad blob key")?;
    let nonce: AeadNonce = secret[KEY_LEN..].try_into().context("bad blob nonce")?;
    aead_decrypt(&key, ciphertext, nonce, None)
        .map_err(|err| anyhow::anyhow!("failed to decrypt blob: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_unique_ciphertexts() -> Result<()> {
        let data = b"the same picture twice";
        let (cipher_a, secret_a) = encrypt_blob(data)?;
        let (cipher_b, secret_b) = encrypt_blob(data)?;
        // Random per-blob keys: identical plaintext must not produce
        // identical ciphertext (that equality leak is why convergent
        // encryption was rejected).
        assert_ne!(cipher_a, cipher_b);
        assert_ne!(secret_a, secret_b);
        assert_eq!(decrypt_blob(&cipher_a, &secret_a)?, data.to_vec());
        assert_eq!(decrypt_blob(&cipher_b, &secret_b)?, data.to_vec());
        Ok(())
    }

    #[test]
    fn wrong_secret_fails() -> Result<()> {
        let (cipher, _) = encrypt_blob(b"secret media")?;
        let (_, other_secret) = encrypt_blob(b"other")?;
        assert!(decrypt_blob(&cipher, &other_secret).is_err());
        Ok(())
    }
}

//! Friend codes: the out-of-band introduction ritual.
//!
//! A friend code carries everything needed to reach someone who can't see
//! you yet — their identity key, how to bootstrap a connection, and a name
//! to show while the request is pending. Handed over any channel the two
//! people already trust; entering one sends a friendship request.

use anyhow::{ensure, Context, Result};
use data_encoding::BASE32_NOPAD;
use p2panda_core::VerifyingKey;
use serde::{Deserialize, Serialize};

const FRIEND_CODE_PREFIX: &str = "jyn-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendCode {
    /// The profile's identity key (also its node id in v1).
    pub profile_id: [u8; 32],
    pub relay_url: Option<String>,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EncodedFriendCode {
    #[serde(with = "serde_bytes")]
    profile_id: Vec<u8>,
    #[serde(default)]
    relay_url: Option<String>,
    #[serde(default)]
    display_name: String,
}

impl FriendCode {
    pub fn new(
        profile_id: VerifyingKey,
        relay_url: Option<String>,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            profile_id: *profile_id.as_bytes(),
            relay_url,
            display_name: display_name.into(),
        }
    }

    pub fn encode(&self) -> Result<String> {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&EncodedFriendCode::from(self), &mut bytes)
            .context("failed to encode friend code as CBOR")?;
        Ok(format!(
            "{FRIEND_CODE_PREFIX}{}",
            BASE32_NOPAD.encode(&bytes)
        ))
    }

    pub fn decode(encoded: &str) -> Result<Self> {
        let encoded = encoded.trim();
        ensure!(
            encoded.starts_with(FRIEND_CODE_PREFIX),
            "friend codes start with {FRIEND_CODE_PREFIX}"
        );

        let payload = &encoded[FRIEND_CODE_PREFIX.len()..];
        let bytes = BASE32_NOPAD
            .decode(payload.as_bytes())
            .map_err(|err| anyhow::anyhow!("invalid base32 friend code: {err}"))?;

        let decoded: EncodedFriendCode =
            ciborium::de::from_reader(bytes.as_slice()).context("malformed CBOR friend code")?;
        decoded.try_into()
    }

    pub fn verifying_key(&self) -> Result<VerifyingKey> {
        VerifyingKey::from_bytes(&self.profile_id).context("invalid identity key in friend code")
    }

    pub fn profile_id_string(&self) -> Result<String> {
        Ok(self.verifying_key()?.to_string())
    }
}

impl From<&FriendCode> for EncodedFriendCode {
    fn from(value: &FriendCode) -> Self {
        Self {
            profile_id: value.profile_id.to_vec(),
            relay_url: value.relay_url.clone(),
            display_name: value.display_name.clone(),
        }
    }
}

impl TryFrom<EncodedFriendCode> for FriendCode {
    type Error = anyhow::Error;

    fn try_from(value: EncodedFriendCode) -> Result<Self> {
        let profile_id: [u8; 32] = value
            .profile_id
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("friend code identity key has invalid length"))?;
        Ok(Self {
            profile_id,
            relay_url: value.relay_url,
            display_name: value.display_name,
        })
    }
}

#[cfg(test)]
mod tests {
    use p2panda_core::SigningKey;

    use super::*;

    #[test]
    fn encode_decode_roundtrip_preserves_all_fields() {
        let key = SigningKey::generate().verifying_key();
        let code = FriendCode::new(key, Some("https://relay.example.com".into()), "Mira");
        let encoded = code.encode().unwrap();
        assert!(encoded.starts_with("jyn-"));

        let decoded = FriendCode::decode(&encoded).unwrap();
        assert_eq!(decoded, code);
        assert_eq!(decoded.verifying_key().unwrap(), key);
        assert_eq!(decoded.display_name, "Mira");
    }

    #[test]
    fn decode_rejects_missing_prefix_and_garbage() {
        assert!(FriendCode::decode("p2p-ABCDEF").is_err());
        assert!(FriendCode::decode("jyn-!!!not-base32!!!").is_err());
        assert!(FriendCode::decode("jyn-MFRGG").is_err());
    }

    #[test]
    fn decode_tolerates_surrounding_whitespace() {
        let key = SigningKey::generate().verifying_key();
        let code = FriendCode::new(key, None, "Bo");
        let encoded = format!("  {}\n", code.encode().unwrap());
        assert_eq!(FriendCode::decode(&encoded).unwrap(), code);
    }
}

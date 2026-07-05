//! The local user's identity: a keypair born on this machine, a chosen name
//! and face, and the profile defaults every new post starts from.
//!
//! The profile is stored locally in the profile data store; its replicated
//! form is the `ProfileUpdated` domain operation published via the sync
//! service whenever it changes.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use p2panda_core::identity::SIGNING_KEY_LEN;
use p2panda_core::SigningKey;
use p2panda_store::sqlite::SqlitePool;
use serde::{Deserialize, Serialize};

use crate::domain::Visibility;
use crate::profile_data::{load_json_key, open_profile_data_store, write_json_key};

const USER_PROFILE_KEY: &str = "user-profile-v1";
const NODE_KEY_FILE: &str = "node.key";

/// Default lifetime pre-filled into the composer: 36 hours, matching the
/// design handoff's `◔ ebbs 36h` pill.
pub const DEFAULT_LIFETIME_SECS: u64 = 36 * 3600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    pub version: u8,
    /// Hex-encoded public key; the identity itself.
    pub profile_id: String,
    pub display_name: String,
    #[serde(default)]
    pub bio: String,
    #[serde(default)]
    pub default_visibility: Visibility,
    #[serde(default = "default_lifetime")]
    pub default_lifetime_secs: Option<u64>,
    /// Whether the owner has been through the first-hour flow.
    #[serde(default)]
    pub onboarded: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

fn default_lifetime() -> Option<u64> {
    Some(DEFAULT_LIFETIME_SECS)
}

impl UserProfile {
    fn new(profile_id: String) -> Self {
        let now = now_unix_secs();
        Self {
            version: 1,
            display_name: generated_display_name(&profile_id),
            profile_id,
            bio: String::new(),
            default_visibility: Visibility::Friends,
            default_lifetime_secs: Some(DEFAULT_LIFETIME_SECS),
            onboarded: false,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug)]
pub struct ProfileStore {
    pool: SqlitePool,
    profile: UserProfile,
}

impl ProfileStore {
    /// Loads the profile, creating one (with a generated name) on first run.
    /// The identity keypair is `node.key`, shared with the network node.
    pub fn load_or_create(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        let store = open_profile_data_store(data_dir)
            .context("failed to open profile data store for user profile")?;

        if let Some(profile) = load_json_key::<UserProfile>(&store.pool, USER_PROFILE_KEY)? {
            return Ok(Self {
                pool: store.pool,
                profile,
            });
        }

        let private_key = load_or_create_private_key(data_dir)?;
        let profile = UserProfile::new(private_key.verifying_key().to_string());
        write_json_key(&store.pool, USER_PROFILE_KEY, &profile)?;
        Ok(Self {
            pool: store.pool,
            profile,
        })
    }

    pub fn profile(&self) -> &UserProfile {
        &self.profile
    }

    pub fn update(
        &mut self,
        display_name: impl Into<String>,
        bio: impl Into<String>,
        default_visibility: Visibility,
        default_lifetime_secs: Option<u64>,
    ) -> Result<&UserProfile> {
        self.profile.display_name = display_name.into();
        self.profile.bio = bio.into();
        self.profile.default_visibility = default_visibility;
        self.profile.default_lifetime_secs = default_lifetime_secs;
        self.profile.updated_at = now_unix_secs();
        self.save()?;
        Ok(&self.profile)
    }

    pub fn mark_onboarded(&mut self) -> Result<()> {
        if !self.profile.onboarded {
            self.profile.onboarded = true;
            self.profile.updated_at = now_unix_secs();
            self.save()?;
        }
        Ok(())
    }

    fn save(&self) -> Result<()> {
        write_json_key(&self.pool, USER_PROFILE_KEY, &self.profile)
    }
}

pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Loads the node's signing key, which doubles as the profile identity.
pub fn load_private_key_from_data_dir(data_dir: impl AsRef<Path>) -> Result<SigningKey> {
    let path = data_dir.as_ref().join(NODE_KEY_FILE);
    let bytes = fs::read(&path)
        .with_context(|| format!("failed to read node key file {}", path.display()))?;
    let bytes: [u8; SIGNING_KEY_LEN] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("node key file {} has invalid length", path.display()))?;
    Ok(SigningKey::from_bytes(&bytes))
}

fn load_or_create_private_key(data_dir: &Path) -> Result<SigningKey> {
    let path: PathBuf = data_dir.join(NODE_KEY_FILE);
    if path.exists() {
        return load_private_key_from_data_dir(data_dir);
    }
    fs::create_dir_all(data_dir).with_context(|| {
        format!(
            "failed to create data directory {} for node key",
            data_dir.display()
        )
    })?;
    let key = SigningKey::generate();
    fs::write(&path, key.as_bytes())
        .with_context(|| format!("failed to write node key file {}", path.display()))?;
    Ok(key)
}

/// A deterministic, friendly placeholder name derived from the public key,
/// used until the owner names themselves during onboarding.
fn generated_display_name(profile_id: &str) -> String {
    const ADJECTIVES: &[&str] = &[
        "Amber", "Briny", "Coral", "Drifting", "Ebbing", "Foamy", "Glassy", "Hidden", "Inky",
        "Jade", "Kelp", "Lunar", "Misty", "Nacre", "Opal", "Pearl", "Quiet", "Rippling", "Silty",
        "Tidal", "Velvet", "Wading", "Winding", "Yonder",
    ];
    const WATERS: &[&str] = &[
        "Bay", "Brook", "Cove", "Creek", "Delta", "Eddy", "Fjord", "Gulf", "Inlet", "Lagoon",
        "Marsh", "Oxbow", "Pond", "Pool", "Rapids", "Reef", "Shoal", "Sound", "Spring", "Strait",
        "Tarn", "Tide", "Wake", "Well",
    ];

    let bytes = profile_id.as_bytes();
    let first = bytes.first().copied().unwrap_or(0) as usize;
    let second = bytes.get(1).copied().unwrap_or(0) as usize;
    format!(
        "{} {}",
        ADJECTIVES[first % ADJECTIVES.len()],
        WATERS[second % WATERS.len()]
    )
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn profile_is_created_once_and_persists_updates() -> Result<()> {
        let dir = tempdir()?;

        let mut store = ProfileStore::load_or_create(dir.path())?;
        let generated_name = store.profile().display_name.clone();
        let profile_id = store.profile().profile_id.clone();
        assert!(!store.profile().onboarded);
        assert_eq!(
            store.profile().default_lifetime_secs,
            Some(DEFAULT_LIFETIME_SECS)
        );

        store.update(
            "Mira",
            "casts fragments",
            Visibility::Friends,
            Some(12 * 3600),
        )?;
        store.mark_onboarded()?;
        drop(store);

        let store = ProfileStore::load_or_create(dir.path())?;
        assert_eq!(store.profile().profile_id, profile_id);
        assert_eq!(store.profile().display_name, "Mira");
        assert_ne!(store.profile().display_name, generated_name);
        assert_eq!(store.profile().bio, "casts fragments");
        assert_eq!(store.profile().default_lifetime_secs, Some(12 * 3600));
        assert!(store.profile().onboarded);

        Ok(())
    }

    #[test]
    fn profile_identity_matches_node_key() -> Result<()> {
        let dir = tempdir()?;
        let store = ProfileStore::load_or_create(dir.path())?;
        let key = load_private_key_from_data_dir(dir.path())?;
        assert_eq!(store.profile().profile_id, key.verifying_key().to_string());
        Ok(())
    }
}

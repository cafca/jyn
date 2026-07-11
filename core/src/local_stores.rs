//! Local-only stores that never replicate.
//!
//! `PrivatePostsStore` holds posts with `Visibility::Private` — they are
//! structurally unleakable because they are never encoded into a
//! `DomainOperation` (the domain additionally rejects them, see
//! `JynOperationDomain::append_operation`).
//!
//! `KeepsStore` holds the reader's kept copies. A keep is a lease, not a
//! possession: it dies when the ephemeral post's lifetime ends and when the
//! author deletes the post — permanent posts included.

use std::path::Path;

use anyhow::{Context, Result};
use p2panda_store::sqlite::SqlitePool;
use serde::{Deserialize, Serialize};

use crate::domain::{ReducedPost, Visibility};
use crate::profile_data::{load_json_key, open_profile_data_store, write_json_key};

const PRIVATE_POSTS_KEY: &str = "private-posts-v1";
const KEEPS_KEY: &str = "keeps-v1";
const OUTGOING_REQUESTS_KEY: &str = "outgoing-friend-requests-v1";

/// A kept copy of someone's post, subordinate to the author's intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeepRecord {
    pub post_id: String,
    pub author_profile_id: String,
    pub snapshot: ReducedPost,
    pub kept_at: u64,
}

#[derive(Debug, Clone)]
pub struct PrivatePostsStore {
    pool: SqlitePool,
}

impl PrivatePostsStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let store = open_profile_data_store(data_dir.as_ref())
            .context("failed to open profile data store for private posts")?;
        Ok(Self { pool: store.pool })
    }

    pub fn with_pool(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Snapshots the whole profile-data store (private posts, keeps,
    /// outgoing requests share one database) into `dest` via `VACUUM INTO`,
    /// which is consistent while the store is live.
    pub async fn snapshot_into(&self, dest: &Path) -> Result<()> {
        let dest = dest.to_string_lossy().replace('\'', "''");
        sqlx::query(&format!("VACUUM INTO '{dest}'"))
            .execute(&self.pool)
            .await
            .context("failed to snapshot profile data store")?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<ReducedPost>> {
        Ok(load_json_key(&self.pool, PRIVATE_POSTS_KEY)?.unwrap_or_default())
    }

    /// Inserts or replaces a private post (keyed by `post_id`).
    pub fn upsert(&self, post: ReducedPost) -> Result<()> {
        anyhow::ensure!(
            post.visibility == Visibility::Private,
            "only private posts belong in the private posts store"
        );
        let mut posts = self.list()?;
        posts.retain(|existing| existing.post_id != post.post_id);
        posts.push(post);
        posts.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.post_id.cmp(&right.post_id))
        });
        self.save(&posts)
    }

    pub fn edit(
        &self,
        post_id: &str,
        body: impl Into<String>,
        media: Vec<crate::domain::MediaAttachment>,
    ) -> Result<bool> {
        let mut posts = self.list()?;
        let Some(post) = posts.iter_mut().find(|post| post.post_id == post_id) else {
            return Ok(false);
        };
        post.body = body.into();
        post.media = media;
        post.edited = true;
        self.save(&posts)?;
        Ok(true)
    }

    /// Promote (`None`) or demote (`Some`) a private post's lifetime.
    pub fn set_lifetime(&self, post_id: &str, expires_at: Option<u64>) -> Result<bool> {
        let mut posts = self.list()?;
        let Some(post) = posts.iter_mut().find(|post| post.post_id == post_id) else {
            return Ok(false);
        };
        post.expires_at = expires_at;
        self.save(&posts)?;
        Ok(true)
    }

    pub fn remove(&self, post_id: &str) -> Result<bool> {
        let mut posts = self.list()?;
        let before = posts.len();
        posts.retain(|post| post.post_id != post_id);
        let removed = posts.len() != before;
        if removed {
            self.save(&posts)?;
        }
        Ok(removed)
    }

    /// Removes and returns all posts whose lifetime has ended at `now`.
    pub fn drain_expired(&self, now: u64) -> Result<Vec<ReducedPost>> {
        let posts = self.list()?;
        let (expired, alive): (Vec<_>, Vec<_>) =
            posts.into_iter().partition(|post| post.is_expired(now));
        if !expired.is_empty() {
            self.save(&alive)?;
        }
        Ok(expired)
    }

    fn save(&self, posts: &[ReducedPost]) -> Result<()> {
        write_json_key(&self.pool, PRIVATE_POSTS_KEY, &posts.to_vec())
    }
}

#[derive(Debug, Clone)]
pub struct KeepsStore {
    pool: SqlitePool,
}

impl KeepsStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let store = open_profile_data_store(data_dir.as_ref())
            .context("failed to open profile data store for keeps")?;
        Ok(Self { pool: store.pool })
    }

    pub fn with_pool(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn list(&self) -> Result<Vec<KeepRecord>> {
        Ok(load_json_key(&self.pool, KEEPS_KEY)?.unwrap_or_default())
    }

    /// Keeps a post (idempotent per author + post id).
    pub fn keep(&self, record: KeepRecord) -> Result<()> {
        let mut keeps = self.list()?;
        keeps.retain(|existing| {
            (
                existing.author_profile_id.as_str(),
                existing.post_id.as_str(),
            ) != (record.author_profile_id.as_str(), record.post_id.as_str())
        });
        keeps.push(record);
        keeps.sort_by(|left, right| {
            right
                .kept_at
                .cmp(&left.kept_at)
                .then_with(|| left.post_id.cmp(&right.post_id))
        });
        self.save(&keeps)
    }

    pub fn release(&self, author_profile_id: &str, post_id: &str) -> Result<bool> {
        let mut keeps = self.list()?;
        let before = keeps.len();
        keeps.retain(|keep| {
            (keep.author_profile_id.as_str(), keep.post_id.as_str()) != (author_profile_id, post_id)
        });
        let removed = keeps.len() != before;
        if removed {
            self.save(&keeps)?;
        }
        Ok(removed)
    }

    /// Enforces the lease: removes and returns keeps whose snapshot expired
    /// at `now` or whose post the author has since deleted.
    pub fn prune_dead(
        &self,
        now: u64,
        is_tombstoned: impl Fn(&str, &str) -> bool,
    ) -> Result<Vec<KeepRecord>> {
        let keeps = self.list()?;
        let (dead, alive): (Vec<_>, Vec<_>) = keeps.into_iter().partition(|keep| {
            keep.snapshot.is_expired(now) || is_tombstoned(&keep.author_profile_id, &keep.post_id)
        });
        if !dead.is_empty() {
            self.save(&alive)?;
        }
        Ok(dead)
    }

    fn save(&self, keeps: &[KeepRecord]) -> Result<()> {
        write_json_key(&self.pool, KEEPS_KEY, &keeps.to_vec())
    }
}

/// Profiles we have asked for friendship and are still waiting on. Purely a
/// local bookkeeping list so their topics get re-joined after a restart —
/// the request itself lives in the replicated log on the target's topic.
#[derive(Debug, Clone)]
pub struct OutgoingRequestsStore {
    pool: SqlitePool,
}

impl OutgoingRequestsStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let store = open_profile_data_store(data_dir.as_ref())
            .context("failed to open profile data store for outgoing requests")?;
        Ok(Self { pool: store.pool })
    }

    pub fn list(&self) -> Result<Vec<String>> {
        Ok(load_json_key(&self.pool, OUTGOING_REQUESTS_KEY)?.unwrap_or_default())
    }

    pub fn add(&self, profile_id: &str) -> Result<()> {
        let mut requests = self.list()?;
        if !requests.iter().any(|existing| existing == profile_id) {
            requests.push(profile_id.to_owned());
            self.save(&requests)?;
        }
        Ok(())
    }

    pub fn remove(&self, profile_id: &str) -> Result<bool> {
        let mut requests = self.list()?;
        let before = requests.len();
        requests.retain(|existing| existing != profile_id);
        let removed = requests.len() != before;
        if removed {
            self.save(&requests)?;
        }
        Ok(removed)
    }

    fn save(&self, requests: &[String]) -> Result<()> {
        write_json_key(&self.pool, OUTGOING_REQUESTS_KEY, &requests.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use tempfile::tempdir;

    use super::*;

    fn post(post_id: &str, visibility: Visibility, expires_at: Option<u64>) -> ReducedPost {
        ReducedPost {
            profile_id: "author".into(),
            post_id: post_id.into(),
            body: "body".into(),
            media: Vec::new(),
            visibility,
            expires_at,
            created_at: 10,
            edited: false,
        }
    }

    #[test]
    fn private_posts_round_trip_edit_promote_and_drain() -> Result<()> {
        let dir = tempdir()?;
        let store = PrivatePostsStore::open(dir.path())?;

        store.upsert(post("p-1", Visibility::Private, Some(100)))?;
        store.upsert(post("p-2", Visibility::Private, None))?;
        assert_eq!(store.list()?.len(), 2);

        assert!(store.edit("p-1", "revised", vec![])?);
        assert!(store.set_lifetime("p-2", Some(50))?);
        let posts = store.list()?;
        let p1 = posts.iter().find(|p| p.post_id == "p-1").unwrap();
        assert_eq!(p1.body, "revised");
        assert!(p1.edited);
        assert!(p1.media.is_empty());

        // Draining at 50 removes p-2 (expires_at 50) but keeps p-1 (expires 100).
        let drained = store.drain_expired(50)?;
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].post_id, "p-2");
        assert_eq!(store.list()?.len(), 1);

        assert!(store.remove("p-1")?);
        assert!(store.list()?.is_empty());

        Ok(())
    }

    #[test]
    fn non_private_posts_are_rejected() -> Result<()> {
        let dir = tempdir()?;
        let store = PrivatePostsStore::open(dir.path())?;
        assert!(store
            .upsert(post("p-1", Visibility::Friends, None))
            .is_err());
        Ok(())
    }

    #[test]
    fn keeps_are_leases_dying_on_expiry_and_tombstone() -> Result<()> {
        let dir = tempdir()?;
        let store = KeepsStore::open(dir.path())?;

        // An ephemeral keep, a permanent keep, and a permanent keep whose
        // author will delete the post.
        store.keep(KeepRecord {
            post_id: "ephemeral".into(),
            author_profile_id: "anna".into(),
            snapshot: post("ephemeral", Visibility::Friends, Some(100)),
            kept_at: 10,
        })?;
        store.keep(KeepRecord {
            post_id: "permanent".into(),
            author_profile_id: "anna".into(),
            snapshot: post("permanent", Visibility::Friends, None),
            kept_at: 11,
        })?;
        store.keep(KeepRecord {
            post_id: "regretted".into(),
            author_profile_id: "bob".into(),
            snapshot: post("regretted", Visibility::Friends, None),
            kept_at: 12,
        })?;

        // Nothing dies while everything is alive and undeleted.
        assert!(store.prune_dead(50, |_, _| false)?.is_empty());
        assert_eq!(store.list()?.len(), 3);

        // The ephemeral keep dies with its post's lifetime; bob's delete
        // reaches into the kept copy of a *permanent* post.
        let dead = store.prune_dead(100, |author, post_id| {
            author == "bob" && post_id == "regretted"
        })?;
        let mut dead_ids: Vec<_> = dead.iter().map(|keep| keep.post_id.clone()).collect();
        dead_ids.sort();
        assert_eq!(
            dead_ids,
            vec!["ephemeral".to_owned(), "regretted".to_owned()]
        );

        let remaining = store.list()?;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].post_id, "permanent");

        // Keeping is idempotent per (author, post); releasing removes.
        store.keep(KeepRecord {
            post_id: "permanent".into(),
            author_profile_id: "anna".into(),
            snapshot: post("permanent", Visibility::Friends, None),
            kept_at: 20,
        })?;
        assert_eq!(store.list()?.len(), 1);
        assert!(store.release("anna", "permanent")?);
        assert!(store.list()?.is_empty());

        Ok(())
    }
}

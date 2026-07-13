//! Jyn's operation domain: posts with author-set lifetimes, named hearts,
//! flat comments, consented friendship — all on p2panda append-only logs with
//! hybrid-logical-clock ordering.
//!
//! Logs are co-deletion units keyed by expiry (ADR-0016): a [`DomainLogId`]
//! is an opaque per-author handle naming a bundle of operations that live and
//! die together, not a semantic stream. Fixed low ids address the author's
//! singleton logs (profile, contacts, spaces control); everything dynamic —
//! expiry buckets, permanent-month buckets, per-target request logs — draws
//! from a monotonic, never-reused counter whose `bucket → log id` mapping is
//! local authoring state (`jyn_log_registry`). Readers never need the
//! mapping: reduction folds every log associated with a topic and reads
//! `expires_at` straight from each payload.
//!
//! Every profile has one sync topic, derived from the header's `audience`
//! field (not from the log id). All logs on a topic are normally authored by
//! the profile owner; the one deliberate exception is
//! [`DomainOperation::FriendshipRequested`], which is authored by the
//! *requester* but lives on the *target's* topic so requests reach their
//! target through normal topic sync. Reduction therefore enforces authorship:
//! owner-signed operations shape the profile, requester-signed operations can
//! only ever surface as pending friendship requests.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use p2panda_core::cbor::{decode_cbor, encode_cbor};
use p2panda_core::timestamp::HybridTimestamp;
use p2panda_core::Topic;
use p2panda_core::{Body, Extension, Hash, Header, Operation, SeqNum, SigningKey, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::operations::OperationStore;
use p2panda_store::topics::TopicStore;
use p2panda_store::{SqliteStore, Transaction};
use p2panda_stream::ingest::ingest_operation;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::placement::LogBucket;
use crate::profile::now_unix_secs;

// v2: the group-encryption flag day. Old plaintext clients stay on v1 topics
// and never exchange operations with encrypted ones.
// v3: the co-deletion-logs flag day (ADR-0016). Log addressing and the header
// extension layout changed incompatibly; pre-v3 clients must never exchange
// operations with v3 ones.
const DOMAIN_TOPIC_NAMESPACE: &[u8] = b"jyn/domain/v3";

/// Reach of a post, chosen by its author.
///
/// In v1 replication is pure friend-circle: `Public` and `Circles` are stored
/// for forward compatibility but replicate exactly like `Friends`. `Private`
/// posts are structurally local-only — they must never be encoded into a
/// [`DomainOperation`] (see `PrivatePostsStore`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Circles,
    #[default]
    Friends,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Photo,
    Audio,
    Video,
    File,
}

/// A media file attached to a post, referenced by blob hash and fetched
/// on demand from the author (or any peer that holds it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaAttachment {
    pub kind: MediaKind,
    pub blob_hash: String,
    pub byte_len: u64,
    pub mime: String,
    /// Duration for audio/video, so cards can render it before the blob arrives.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Peak buckets for audio waveforms, rendered before the blob arrives.
    #[serde(default)]
    pub waveform: Option<Vec<u8>>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// Original file name, used by the external-open fallback card.
    #[serde(default)]
    pub file_name: Option<String>,
    /// Per-blob AEAD key + nonce (32 + 12 bytes) when the blob replicates as
    /// ciphertext. Only ever present inside encrypted post payloads, so the
    /// key is protected by the group encryption around it. `None` = plaintext
    /// blob (public posts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob_secret: Option<Vec<u8>>,
}

/// Opaque handle for one co-deletion log (ADR-0016).
///
/// The id deliberately encodes nothing: what a log's operations are *about*
/// lives in the header's `audience` field, and what makes them co-deletable
/// (their expiry bucket) lives only in the author's local registry. On the
/// wire an arbitrary integer therefore reveals neither audience nor expiry.
///
/// Ids are scoped to their author (`operations_v1` is keyed
/// `(verifying_key, log_id, seq_num)`), so two authors using the same number
/// never collide. Within one author: `0..1000` are reserved fixed ids for
/// singleton logs known at compile time, everything from
/// [`Self::FIRST_DYNAMIC`] up is allocated by a monotonic counter and *never
/// reused* — a fresh log at a recycled id would restart at `seq_num 0` and be
/// rejected by any peer still holding the old log's head.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DomainLogId(pub u64);

impl DomainLogId {
    /// The author's own profile log (display name, bio, defaults).
    pub const PROFILE: Self = Self(0);
    /// Follows and friendship responses.
    pub const CONTACTS: Self = Self(1);
    /// Group-encryption control traffic: key bundles and membership
    /// messages (see `crate::spaces`). Encrypted *application* payloads are
    /// placed into expiry buckets instead, like the posts they carry.
    pub const SPACES_CONTROL: Self = Self(2);
    /// First id the dynamic allocator hands out (expiry buckets,
    /// permanent-month buckets, per-target request logs).
    pub const FIRST_DYNAMIC: u64 = 1000;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainExtensions {
    pub log_id: DomainLogId,
    /// The audience whose sync topic carries this operation — a profile id
    /// today; an opaque shared handle once members-only contexts blind their
    /// topic (ADR-0016). The topic derives from this field, never from the
    /// log id.
    pub audience: String,
    #[serde(default = "HybridTimestamp::now")]
    pub ordering_timestamp: HybridTimestamp,
}

impl Extension<DomainLogId> for DomainExtensions {
    fn extract(header: &Header<Self>) -> Option<DomainLogId> {
        Some(header.extensions.log_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DomainOperation {
    ProfileUpdated {
        profile_id: String,
        display_name: String,
        #[serde(default)]
        bio: String,
        #[serde(default)]
        default_visibility: Visibility,
        #[serde(default)]
        default_lifetime_secs: Option<u64>,
        created_at: u64,
        updated_at: u64,
    },
    ContactFollowChanged {
        profile_id: String,
        followed_profile_id: String,
        recorded_at: u64,
        active: bool,
    },
    /// Authored by the requester, but carried on the *target's* topic
    /// ([`DomainOperation::profile_id`] returns the target).
    FriendshipRequested {
        requester_profile_id: String,
        target_profile_id: String,
        requester_display_name: String,
        #[serde(default)]
        greeting: Option<String>,
        recorded_at: u64,
    },
    /// The target's answer, on the target's own topic. Accepting is always
    /// accompanied by a `ContactFollowChanged { active: true }`.
    FriendshipResponded {
        target_profile_id: String,
        requester_profile_id: String,
        accepted: bool,
        recorded_at: u64,
    },
    /// A post — either its first publication or a self-contained re-home
    /// snapshot after a lifetime change (ADR-0016). A snapshot carries the
    /// post's complete current state (body, media, visibility, expiry) and
    /// supersedes every earlier copy by `ordering_timestamp`, so the post
    /// survives the eventual GC of every earlier bucket it passed through.
    PostPublished {
        profile_id: String,
        post_id: String,
        body: String,
        #[serde(default)]
        media: Vec<MediaAttachment>,
        visibility: Visibility,
        /// `None` = permanent; otherwise unix seconds after which the post
        /// (and any kept copies) drains everywhere.
        expires_at: Option<u64>,
        created_at: u64,
        /// Whether the post had been edited when this copy was written; a
        /// re-home snapshot collapses prior edits but keeps the badge.
        #[serde(default)]
        edited: bool,
    },
    PostEdited {
        profile_id: String,
        post_id: String,
        body: String,
        /// `None` (legacy ops) leaves attachments untouched; `Some`
        /// replaces the full list.
        #[serde(default)]
        media: Option<Vec<MediaAttachment>>,
        edited_at: u64,
    },
    /// Marks this log's copy of a post as superseded by a re-home snapshot
    /// living in another bucket (lifetime change, ADR-0016). A pure GC
    /// marker: reduction ignores it — the snapshot itself outranks the stale
    /// copy by `ordering_timestamp` — but GC can drop the old bucket knowing
    /// its copy was disowned rather than left to shadow.
    PostRehomed {
        profile_id: String,
        post_id: String,
        moved_at: u64,
    },
    /// Tombstone. Reaches into readers' kept copies.
    PostDeleted {
        profile_id: String,
        post_id: String,
        deleted_at: u64,
    },
    /// A named heart on someone's post, living in the *hearter's* log.
    HeartChanged {
        profile_id: String,
        post_author_profile_id: String,
        post_id: String,
        active: bool,
        recorded_at: u64,
    },
    /// A flat-thread comment on someone's post, living in the commenter's log.
    CommentPublished {
        profile_id: String,
        comment_id: String,
        post_author_profile_id: String,
        post_id: String,
        body: String,
        created_at: u64,
    },
    /// Group-encryption traffic authored by this profile: an opaque
    /// CBOR-encoded `SpacesArgs` (key bundle, membership control message or
    /// encrypted application payload). Encrypted payloads decrypt back into a
    /// regular [`DomainOperation`] which reduction picks up in place of this
    /// wrapper; control messages and not-yet-decryptable payloads are skipped.
    Spaces {
        profile_id: String,
        #[serde(with = "serde_bytes")]
        args: Vec<u8>,
    },
}

impl DomainOperation {
    /// The profile whose topic carries this operation. For friendship
    /// requests this is the *target*, not the (requester) author.
    fn profile_id(&self) -> &str {
        match self {
            Self::ProfileUpdated { profile_id, .. }
            | Self::ContactFollowChanged { profile_id, .. }
            | Self::PostPublished { profile_id, .. }
            | Self::PostEdited { profile_id, .. }
            | Self::PostRehomed { profile_id, .. }
            | Self::PostDeleted { profile_id, .. }
            | Self::HeartChanged { profile_id, .. }
            | Self::CommentPublished { profile_id, .. }
            | Self::Spaces { profile_id, .. } => profile_id,
            Self::FriendshipRequested {
                target_profile_id, ..
            } => target_profile_id,
            Self::FriendshipResponded {
                target_profile_id, ..
            } => target_profile_id,
        }
    }

    /// The post whose readable body this operation carries, if any. Only the
    /// content-bearing post ops qualify: teardown erases these — both the
    /// decrypted cache row and the encrypted payload — so an expired or deleted
    /// post's content cannot be recovered from disk. Tombstones and re-home
    /// markers are deliberately excluded so their bodies survive and reduction
    /// still sees the delete after teardown.
    fn content_post_id(&self) -> Option<&str> {
        match self {
            Self::PostPublished { post_id, .. } | Self::PostEdited { post_id, .. } => Some(post_id),
            _ => None,
        }
    }

    /// The `(post_author, post_id)` a reaction or comment is *on*, if this
    /// operation is one. Used by GC to reap a reaction reactively once its
    /// target post is gone (ADR-0016: "a reaction lives exactly as long as the
    /// post it is on").
    fn reaction_target(&self) -> Option<(&str, &str)> {
        match self {
            Self::HeartChanged {
                post_author_profile_id,
                post_id,
                ..
            }
            | Self::CommentPublished {
                post_author_profile_id,
                post_id,
                ..
            } => Some((post_author_profile_id, post_id)),
            _ => None,
        }
    }

    /// Blob hashes this operation's payload pins, so GC can reclaim media when
    /// the operation is dropped.
    fn media_blob_hashes(&self) -> Vec<String> {
        match self {
            Self::PostPublished { media, .. } => {
                media.iter().map(|m| m.blob_hash.clone()).collect()
            }
            Self::PostEdited {
                media: Some(media), ..
            } => media.iter().map(|m| m.blob_hash.clone()).collect(),
            _ => Vec::new(),
        }
    }
}

/// A post as reconstructed from a profile's operation history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReducedPost {
    pub profile_id: String,
    pub post_id: String,
    pub body: String,
    pub media: Vec<MediaAttachment>,
    pub visibility: Visibility,
    pub expires_at: Option<u64>,
    pub created_at: u64,
    pub edited: bool,
}

impl ReducedPost {
    pub fn is_expired(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|expires_at| expires_at <= now)
    }
}

/// An active heart cast by this profile on someone's post.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartRef {
    pub post_author_profile_id: String,
    pub post_id: String,
    pub recorded_at: u64,
}

/// A comment written by this profile on someone's post.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReducedComment {
    pub comment_id: String,
    pub post_author_profile_id: String,
    pub post_id: String,
    pub body: String,
    pub created_at: u64,
}

/// A friendship request awaiting the profile owner's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingFriendRequest {
    pub requester_profile_id: String,
    pub requester_display_name: String,
    pub greeting: Option<String>,
    pub recorded_at: u64,
}

/// Everything a profile's operation history reduces to.
///
/// Expired posts are *not* filtered here — reduction stays deterministic and
/// restart-safe. Callers filter at read time via [`ReducedProfileState::active_posts`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReducedProfileState {
    pub profile_id: String,
    pub display_name: Option<String>,
    pub bio: String,
    pub default_visibility: Visibility,
    pub default_lifetime_secs: Option<u64>,
    pub posts: Vec<ReducedPost>,
    pub followed_profile_ids: Vec<String>,
    pub hearts: Vec<HeartRef>,
    pub comments: Vec<ReducedComment>,
    pub pending_requests: Vec<PendingFriendRequest>,
    /// Post ids the author has deleted; used to kill kept copies.
    pub tombstoned_post_ids: Vec<String>,
}

impl ReducedProfileState {
    /// Posts that are still alive at `now` (unix seconds), newest first.
    pub fn active_posts(&self, now: u64) -> impl Iterator<Item = &ReducedPost> {
        self.posts.iter().filter(move |post| !post.is_expired(now))
    }

    pub fn is_tombstoned(&self, post_id: &str) -> bool {
        self.tombstoned_post_ids
            .iter()
            .any(|tombstoned| tombstoned == post_id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredDomainOperation {
    pub author: VerifyingKey,
    pub log_id: DomainLogId,
    pub header: Header<DomainExtensions>,
    pub operation: DomainOperation,
}

#[derive(Debug, Clone)]
pub struct JynOperationDomain {
    store: SqliteStore,
}

impl JynOperationDomain {
    pub fn new(store: SqliteStore) -> Self {
        Self { store }
    }

    pub fn into_store(self) -> SqliteStore {
        self.store
    }

    /// Resolves the co-deletion log an operation belongs in (ADR-0016).
    ///
    /// - Singleton streams use their reserved fixed ids.
    /// - A post is placed by its own expiry; its edits, tombstone and re-home
    ///   marker follow the log its current copy lives in, so the whole set
    ///   co-deletes. The one call-order invariant: a [`DomainOperation::PostRehomed`]
    ///   marker must be appended *before* its snapshot, while the old copy is
    ///   still the post's current one.
    /// - A heart or comment is placed by the *target post's* expiry at write
    ///   time, so in the common case it drains with the same bucket; keeping a
    ///   reaction alive exactly as long as its post is GC's job, not placement's.
    /// - A `Spaces` wrapper defaults to the control log; the forge overrides
    ///   this for encrypted application payloads via
    ///   [`Self::append_operation_in_log`], since the wrapper is opaque here.
    ///
    /// Fallback for reactions and post ops whose target is unknown locally:
    /// the permanent-month bucket of the operation's own timestamp — never
    /// wrong to over-retain, and GC still finds it by reading payloads.
    pub(crate) async fn log_id_for(&self, operation: &DomainOperation) -> Result<DomainLogId> {
        let now = now_unix_secs();
        match operation {
            DomainOperation::ProfileUpdated { .. } => Ok(DomainLogId::PROFILE),
            DomainOperation::ContactFollowChanged { .. }
            | DomainOperation::FriendshipResponded { .. } => Ok(DomainLogId::CONTACTS),
            DomainOperation::Spaces { .. } => Ok(DomainLogId::SPACES_CONTROL),
            // One log per request target: a request log is associated with the
            // target's topic, so sharing one log across targets would leak
            // every request to every target.
            DomainOperation::FriendshipRequested {
                target_profile_id, ..
            } => {
                self.log_id_for_context(&format!("requests/{target_profile_id}"))
                    .await
            }
            DomainOperation::PostPublished {
                expires_at,
                created_at,
                ..
            } => {
                let bucket = LogBucket::place(*expires_at, *created_at, now);
                self.log_id_for_context(&bucket.context_key()).await
            }
            DomainOperation::PostEdited {
                profile_id,
                post_id,
                edited_at: at,
                ..
            }
            | DomainOperation::PostRehomed {
                profile_id,
                post_id,
                moved_at: at,
            }
            | DomainOperation::PostDeleted {
                profile_id,
                post_id,
                deleted_at: at,
            } => match self.current_post_log(profile_id, post_id).await? {
                Some(log_id) => Ok(log_id),
                None => {
                    let bucket = LogBucket::place(None, *at, now);
                    self.log_id_for_context(&bucket.context_key()).await
                }
            },
            // A reaction is placed by its *own* creation month, never the
            // target post's expiry: the post's deadline is a foreign, mutable
            // value, so keying on it would go stale on promotion and force a
            // roll-forward. GC enforces "a reaction lives as long as its post"
            // reactively instead (see `reap_reactions_for_dead_targets`).
            DomainOperation::HeartChanged {
                recorded_at: at, ..
            }
            | DomainOperation::CommentPublished {
                created_at: at, ..
            } => {
                self.log_id_for_context(&LogBucket::place_reaction(*at).context_key())
                    .await
            }
        }
    }

    /// The log holding a post's current (newest) published copy, from the
    /// author's own stored operations. `None` when the post is unknown or its
    /// content was already erased.
    async fn current_post_log(
        &self,
        profile_id: &str,
        post_id: &str,
    ) -> Result<Option<DomainLogId>> {
        let operations = self.operations_for_profile(profile_id).await?;
        Ok(operations
            .into_iter()
            .filter(|op| {
                op.author.to_string() == profile_id
                    && matches!(
                        &op.operation,
                        DomainOperation::PostPublished { post_id: id, .. } if id == post_id
                    )
            })
            .max_by(|left, right| {
                left.header
                    .extensions
                    .ordering_timestamp
                    .cmp(&right.header.extensions.ordering_timestamp)
            })
            .map(|op| op.log_id))
    }

    /// Looks up (or allocates) the dynamic log id for a placement context.
    ///
    /// The `context → log id` mapping is local authoring state: only this
    /// device's GC ever needs to know which bucket a log id means. Ids come
    /// from a dedicated counter that only increments, so a retired id is
    /// structurally never reused even after GC deletes registry rows. A racing
    /// allocation for the same context wastes an id (harmless — u64s are free)
    /// and both callers converge on whichever mapping won the insert.
    async fn log_id_for_context(&self, context: &str) -> Result<DomainLogId> {
        ensure_domain_log_tables(&self.store).await?;
        if let Some(existing) = self.registered_log_id(context).await? {
            return Ok(existing);
        }
        let (allocated,): (i64,) = sqlx::query_as(
            "UPDATE jyn_log_allocator SET next_log_id = next_log_id + 1 WHERE id = 0
             RETURNING next_log_id - 1",
        )
        .fetch_one(self.store.pool())
        .await
        .context("failed to allocate domain log id")?;
        sqlx::query("INSERT OR IGNORE INTO jyn_log_registry (context, log_id) VALUES (?, ?)")
            .bind(context)
            .bind(allocated)
            .execute(self.store.pool())
            .await
            .context("failed to register domain log id")?;
        self.registered_log_id(context)
            .await?
            .context("domain log registry lost a just-inserted context")
    }

    async fn registered_log_id(&self, context: &str) -> Result<Option<DomainLogId>> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT log_id FROM jyn_log_registry WHERE context = ?")
                .bind(context)
                .fetch_optional(self.store.pool())
                .await
                .context("failed to read domain log registry")?;
        Ok(row.map(|(id,)| DomainLogId(id as u64)))
    }

    pub async fn append_operation(
        &mut self,
        private_key: &SigningKey,
        operation: DomainOperation,
    ) -> Result<Header<DomainExtensions>> {
        let log_id = self.log_id_for(&operation).await?;
        self.append_operation_in_log(private_key, operation, log_id)
            .await
    }

    /// Appends into an explicitly chosen log. Only the spaces forge should
    /// need this: an encrypted application payload's placement is computed
    /// from its *inner* operation, which is opaque by the time the wrapper
    /// reaches [`Self::append_operation`].
    pub(crate) async fn append_operation_in_log(
        &mut self,
        private_key: &SigningKey,
        operation: DomainOperation,
        log_id: DomainLogId,
    ) -> Result<Header<DomainExtensions>> {
        if let DomainOperation::PostPublished { visibility, .. } = &operation {
            // Private posts are local-only by construction; encoding one into
            // a replicated operation would be a privacy bug, not a feature.
            anyhow::ensure!(
                *visibility != Visibility::Private,
                "private posts must never enter the replicated operation log"
            );
        }

        let profile_id = operation.profile_id().to_owned();
        let body_bytes = encode_cbor(&operation).context("failed to encode domain body")?;
        let body = Body::from(body_bytes.clone());
        let latest: Option<Operation<DomainExtensions>> = self
            .store
            .get_latest_entry(&private_key.verifying_key(), &log_id)
            .await
            .map_err(|err| anyhow::anyhow!("failed to load latest domain operation: {err}"))?;
        let (seq_num, backlink) = latest
            .as_ref()
            .map(|operation| (operation.header.seq_num + 1, Some(operation.hash)))
            .unwrap_or((0, None));

        // Chain the ordering timestamp off the newest operation known for this profile (from any
        // author and device) so new operations always sort after everything they were created in
        // response to, even when wall clocks are skewed or frozen.
        let previous_ordering = self
            .operations_for_profile(&profile_id)
            .await?
            .into_iter()
            .map(|operation| operation.header.extensions.ordering_timestamp)
            .max();
        let ordering_timestamp = next_ordering_timestamp(previous_ordering);
        let mut header = Header {
            version: 1,
            verifying_key: private_key.verifying_key(),
            signature: None,
            payload_size: body.size(),
            payload_hash: Some(body.hash()),
            seq_num,
            backlink,
            extensions: DomainExtensions {
                log_id,
                audience: profile_id.clone(),
                ordering_timestamp,
            },
        };
        header.sign(private_key);

        let operation = Operation {
            hash: header.hash(),
            header: header.clone(),
            body: Some(body),
        };
        let topic = profile_sync_topic(&profile_id);
        ingest_operation(&self.store, &operation, &log_id, &topic, false)
            .await
            .map_err(|err| anyhow::anyhow!("failed to ingest domain operation: {err}"))?;

        Ok(header)
    }

    pub async fn ingest_remote_operation(
        &mut self,
        operation: Operation<DomainExtensions>,
    ) -> Result<()> {
        let log_id = operation.header.extensions.log_id;
        // The topic derives from the header's audience field, not the log id
        // (ADR-0016). A forged audience only mis-shelves the author's own
        // operations: reduction drops foreign-authored operations anyway.
        let topic = profile_sync_topic(&operation.header.extensions.audience);

        ingest_operation(&self.store, &operation, &log_id, &topic, false)
            .await
            .map_err(|err| {
                anyhow::anyhow!("failed to ingest replicated domain operation: {err}")
            })?;

        Ok(())
    }

    /// Reduces a profile's full operation history to its current state.
    ///
    /// Authorship rules: operations only count when their author is the
    /// profile owner (the key `profile_id` names) — except friendship
    /// requests, which must be authored by the requester they claim.
    pub async fn read_profile_state(
        &self,
        profile_id: &str,
    ) -> Result<Option<ReducedProfileState>> {
        let mut operations = self.operations_for_profile(profile_id).await?;
        if operations.is_empty() {
            return Ok(None);
        }
        sort_for_reduction(&mut operations);

        let mut display_name = None;
        let mut bio = String::new();
        let mut default_visibility = Visibility::default();
        let mut default_lifetime_secs = None;
        let mut posts = HashMap::<String, ReducedPost>::new();
        let mut tombstones = HashSet::<String>::new();
        let mut follows = HashMap::<String, bool>::new();
        let mut hearts = HashMap::<(String, String), Option<u64>>::new();
        let mut comments = HashMap::<String, ReducedComment>::new();
        let mut requests = HashMap::<String, PendingFriendRequest>::new();
        let mut responded = HashSet::<String>::new();

        for op in operations {
            let author_id = op.author.to_string();
            let author_is_owner = author_id == profile_id;

            match op.operation {
                DomainOperation::FriendshipRequested {
                    requester_profile_id,
                    requester_display_name,
                    greeting,
                    recorded_at,
                    ..
                } => {
                    // A request must be signed by the requester it claims and
                    // asking for friendship with yourself is meaningless.
                    if author_id == requester_profile_id && requester_profile_id != profile_id {
                        requests.insert(
                            requester_profile_id.clone(),
                            PendingFriendRequest {
                                requester_profile_id,
                                requester_display_name,
                                greeting,
                                recorded_at,
                            },
                        );
                    }
                    continue;
                }
                operation if !author_is_owner => {
                    // Only the profile owner shapes the profile itself. A
                    // foreign-signed post/follow/etc. on this topic is either
                    // a sync artifact or an attempted forgery; drop it.
                    warn!(
                        author = %author_id,
                        profile = %profile_id,
                        "ignoring foreign-authored operation during reduction: {operation:?}"
                    );
                    continue;
                }
                DomainOperation::ProfileUpdated {
                    display_name: next_display_name,
                    bio: next_bio,
                    default_visibility: next_visibility,
                    default_lifetime_secs: next_lifetime,
                    ..
                } => {
                    display_name = Some(next_display_name);
                    bio = next_bio;
                    default_visibility = next_visibility;
                    default_lifetime_secs = next_lifetime;
                }
                DomainOperation::ContactFollowChanged {
                    followed_profile_id,
                    active,
                    ..
                } => {
                    follows.insert(followed_profile_id, active);
                }
                DomainOperation::FriendshipResponded {
                    requester_profile_id,
                    ..
                } => {
                    responded.insert(requester_profile_id);
                }
                DomainOperation::PostPublished {
                    profile_id: author_profile_id,
                    post_id,
                    body,
                    media,
                    visibility,
                    expires_at,
                    created_at,
                    edited,
                } => {
                    // A tombstone is the last word on a post: a re-home
                    // snapshot sorting after the delete (crash between the
                    // two, device races) must not resurrect it.
                    if tombstones.contains(&post_id) {
                        continue;
                    }
                    posts.insert(
                        post_id.clone(),
                        ReducedPost {
                            profile_id: author_profile_id,
                            post_id,
                            body,
                            media,
                            visibility,
                            expires_at,
                            created_at,
                            edited,
                        },
                    );
                }
                DomainOperation::PostEdited {
                    post_id,
                    body,
                    media,
                    ..
                } => {
                    if let Some(post) = posts.get_mut(&post_id) {
                        post.body = body;
                        if let Some(media) = media {
                            post.media = media;
                        }
                        post.edited = true;
                    }
                }
                DomainOperation::PostRehomed { .. } => {
                    // A pure GC marker for the old bucket's copy; the re-home
                    // snapshot itself carries the post's new state and outranks
                    // the stale copy by ordering, so reduction has nothing to do.
                }
                DomainOperation::PostDeleted { post_id, .. } => {
                    posts.remove(&post_id);
                    tombstones.insert(post_id);
                }
                DomainOperation::HeartChanged {
                    post_author_profile_id,
                    post_id,
                    active,
                    recorded_at,
                    ..
                } => {
                    hearts.insert(
                        (post_author_profile_id, post_id),
                        active.then_some(recorded_at),
                    );
                }
                DomainOperation::CommentPublished {
                    comment_id,
                    post_author_profile_id,
                    post_id,
                    body,
                    created_at,
                    ..
                } => {
                    comments.insert(
                        comment_id.clone(),
                        ReducedComment {
                            comment_id,
                            post_author_profile_id,
                            post_id,
                            body,
                            created_at,
                        },
                    );
                }
                DomainOperation::Spaces { .. } => {
                    // Spaces wrappers are substituted by their decrypted inner
                    // operation in `operations_for_profile`; one reaching
                    // reduction is a control message or undecryptable payload.
                }
            }
        }

        let mut posts = posts.into_values().collect::<Vec<_>>();
        posts.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.post_id.cmp(&right.post_id))
        });

        let mut followed_profile_ids = follows
            .iter()
            .filter(|(_, active)| **active)
            .map(|(profile_id, _)| profile_id.clone())
            .collect::<Vec<_>>();
        followed_profile_ids.sort();

        let mut hearts = hearts
            .into_iter()
            .filter_map(|((post_author_profile_id, post_id), recorded_at)| {
                recorded_at.map(|recorded_at| HeartRef {
                    post_author_profile_id,
                    post_id,
                    recorded_at,
                })
            })
            .collect::<Vec<_>>();
        hearts.sort_by(|left, right| {
            left.recorded_at
                .cmp(&right.recorded_at)
                .then_with(|| left.post_id.cmp(&right.post_id))
        });

        let mut comments = comments.into_values().collect::<Vec<_>>();
        comments.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.comment_id.cmp(&right.comment_id))
        });

        // A request stays pending until the owner responded or already follows
        // the requester (e.g. friendship formed through a crossed request).
        let mut pending_requests = requests
            .into_values()
            .filter(|request| {
                !responded.contains(&request.requester_profile_id)
                    && follows
                        .get(&request.requester_profile_id)
                        .copied()
                        .unwrap_or(false)
                        .eq(&false)
            })
            .collect::<Vec<_>>();
        pending_requests.sort_by(|left, right| {
            left.recorded_at
                .cmp(&right.recorded_at)
                .then_with(|| left.requester_profile_id.cmp(&right.requester_profile_id))
        });

        let mut tombstoned_post_ids = tombstones.into_iter().collect::<Vec<_>>();
        tombstoned_post_ids.sort();

        Ok(Some(ReducedProfileState {
            profile_id: profile_id.to_owned(),
            display_name,
            bio,
            default_visibility,
            default_lifetime_secs,
            posts,
            followed_profile_ids,
            hearts,
            comments,
            pending_requests,
            tombstoned_post_ids,
        }))
    }

    pub async fn operations_for_profile(
        &self,
        profile_id: &str,
    ) -> Result<Vec<StoredDomainOperation>> {
        let topic = profile_sync_topic(profile_id);
        let associations =
            TopicStore::<Topic, VerifyingKey, DomainLogId>::resolve(&self.store, &topic)
                .await
                .map_err(|err| {
                    anyhow::anyhow!("failed to resolve domain topic associations: {err}")
                })?;

        let mut operations = Vec::new();
        for (author, log_ids) in associations {
            for log_id in log_ids {
                let entries = self
                    .store
                    .get_log_entries(&author, &log_id, None, None)
                    .await
                    .map_err(|err| anyhow::anyhow!("failed to load domain log: {err}"))?
                    .unwrap_or_default();

                for (operation, _header_bytes) in entries {
                    let operation: Operation<DomainExtensions> = operation;
                    // A body-less operation is a valid, modeled state: teardown
                    // deletes the ciphertext payload of an expired or deleted
                    // post's content op, leaving a header-only record. It carries
                    // nothing to reduce (its decrypted cache row is purged in the
                    // same teardown), so skip it rather than erroring on the
                    // missing body.
                    let Some(body) = operation.body else {
                        continue;
                    };
                    let mut domain_operation =
                        decode_cbor::<DomainOperation, _>(&body.to_bytes()[..])
                            .context("failed to decode domain body")?;
                    if let DomainOperation::Spaces { .. } = &domain_operation {
                        // Substitute the wrapper with its decrypted inner
                        // operation; control messages and payloads we cannot
                        // (yet) decrypt stay out of reduction entirely.
                        match self.decrypted_inner_operation(&operation.hash).await? {
                            Some(inner) if !matches!(inner, DomainOperation::Spaces { .. }) => {
                                domain_operation = inner;
                            }
                            _ => continue,
                        }
                    }
                    operations.push(StoredDomainOperation {
                        author,
                        log_id,
                        header: operation.header,
                        operation: domain_operation,
                    });
                }
            }
        }

        Ok(operations)
    }

    /// All raw operations on a profile's topic, without decoding bodies or
    /// substituting decrypted payloads. Used by the spaces service to find
    /// unprocessed spaces messages after a restart.
    pub async fn operations_for_profile_raw(
        &self,
        profile_id: &str,
    ) -> Result<Vec<Operation<DomainExtensions>>> {
        let topic = profile_sync_topic(profile_id);
        let associations =
            TopicStore::<Topic, VerifyingKey, DomainLogId>::resolve(&self.store, &topic)
                .await
                .map_err(|err| {
                    anyhow::anyhow!("failed to resolve domain topic associations: {err}")
                })?;

        let mut operations = Vec::new();
        for (author, log_ids) in associations {
            for log_id in log_ids {
                let entries = self
                    .store
                    .get_log_entries(&author, &log_id, None, None)
                    .await
                    .map_err(|err| anyhow::anyhow!("failed to load domain log: {err}"))?
                    .unwrap_or_default();
                operations.extend(entries.into_iter().map(|(operation, _)| operation));
            }
        }
        Ok(operations)
    }

    /// The decrypted inner operation for a `DomainOperation::Spaces` wrapper,
    /// stored by the spaces service once the payload could be decrypted.
    pub async fn decrypted_inner_operation(
        &self,
        op_hash: &Hash,
    ) -> Result<Option<DomainOperation>> {
        let row: Option<(Vec<u8>,)> =
            sqlx::query_as("SELECT inner_body FROM jyn_spaces_decrypted WHERE op_hash = ?")
                .bind(op_hash.to_string())
                .fetch_optional(self.store.pool())
                .await
                .context("failed to read decrypted spaces operation")?;
        row.map(|(body,)| {
            decode_cbor::<DomainOperation, _>(&body[..])
                .context("failed to decode decrypted spaces operation")
        })
        .transpose()
    }

    pub async fn store_decrypted_inner_operation(
        &self,
        op_hash: &Hash,
        inner: &DomainOperation,
    ) -> Result<()> {
        let body = encode_cbor(inner).context("failed to encode decrypted spaces operation")?;
        sqlx::query(
            "INSERT OR REPLACE INTO jyn_spaces_decrypted (op_hash, inner_body) VALUES (?, ?)",
        )
        .bind(op_hash.to_string())
        .bind(body)
        .execute(self.store.pool())
        .await
        .context("failed to store decrypted spaces operation")?;
        Ok(())
    }

    /// Erases the content of an expired or deleted non-public post's
    /// content-bearing operations so it cannot be recovered from local storage:
    /// for each `PostPublished`/`PostEdited` wrapper op it deletes both the
    /// locally-cached decrypted row *and* the encrypted payload (body) of the
    /// stored operation.
    ///
    /// Dropping the payload is chain-safe from any position in the log: the
    /// header — carrying the `payload_hash`, backlink and signature — is kept,
    /// so backlink validation and sync are unaffected, and a body-less operation
    /// is a modeled state that reduction skips (see `operations_for_profile`).
    /// This is distinct from removing a whole operation (header included), which
    /// would break the next operation's backlink and is never done. What remains
    /// after teardown is header-only metadata, no content and no ciphertext.
    ///
    /// Tombstone and lifetime-change wrappers are deliberately left whole — both
    /// their decrypted rows and their payloads — so reduction still reflects the
    /// delete/promote after teardown. Idempotent (re-running skips ops whose
    /// payload is already gone) — returns how many ops it erased. `profile_id`
    /// scopes the walk to that author's own logs.
    pub async fn erase_post_content(&self, profile_id: &str, post_id: &str) -> Result<usize> {
        self.erase_content_where(profile_id, |op| op.content_post_id() == Some(post_id))
            .await
    }

    /// Payload-erases every stored operation of `profile_id` whose (plaintext
    /// or decrypted) content matches `predicate`: drops the operation body and,
    /// for `Spaces` wrappers, the decrypted-plaintext cache row too. Keeps the
    /// header, so the backlink chain and sync stay intact and reduction simply
    /// skips the now body-less op. Idempotent (already body-less ops are
    /// skipped); returns how many ops it erased. This is the shared primitive
    /// behind both post teardown ([`Self::erase_post_content`]) and reaction
    /// reaping ([`Self::reap_reactions_for_dead_targets`]).
    async fn erase_content_where(
        &self,
        profile_id: &str,
        predicate: impl Fn(&DomainOperation) -> bool,
    ) -> Result<usize> {
        let mut erased = 0;
        for operation in self.operations_for_profile_raw(profile_id).await? {
            // An already-erased op is body-less and has nothing left to strip.
            let Some(body) = &operation.body else {
                continue;
            };
            let Ok(decoded) = decode_cbor::<DomainOperation, _>(&body.to_bytes()[..]) else {
                continue;
            };
            let (matches, is_wrapper) = match &decoded {
                // A non-public post/reaction rides an encrypted wrapper whose
                // real content is the cached decrypted inner op.
                DomainOperation::Spaces { .. } => {
                    match self.decrypted_inner_operation(&operation.hash).await? {
                        Some(inner) => (predicate(&inner), true),
                        None => (false, true),
                    }
                }
                // A public post/reaction is a plaintext op; its body itself
                // is the content to strip.
                other => (predicate(other), false),
            };
            if !matches {
                continue;
            }
            if is_wrapper {
                self.delete_decrypted_inner_operation(&operation.hash).await?;
            }
            <SqliteStore as OperationStore<Operation<DomainExtensions>, Hash>>::delete_operation_payload(
                &self.store,
                &operation.hash,
            )
            .await
            .map_err(|err| anyhow::anyhow!("failed to delete operation payload: {err}"))?;
            erased += 1;
        }
        Ok(erased)
    }

    async fn delete_decrypted_inner_operation(&self, op_hash: &Hash) -> Result<()> {
        sqlx::query("DELETE FROM jyn_spaces_decrypted WHERE op_hash = ?")
            .bind(op_hash.to_string())
            .execute(self.store.pool())
            .await
            .context("failed to delete decrypted spaces operation")?;
        Ok(())
    }

    /// The `(post_author, post_id)` pairs among `profile_ids` whose posts are
    /// **dead** at `now` — tombstoned or expired. This is GC's authority on
    /// which posts a reaction may outlive (ADR-0016): a reaction on a dead
    /// target is reaped. Reads reduced state, so it reflects the author's
    /// current view, including a promotion that kept a post alive.
    pub async fn dead_post_targets(
        &self,
        profile_ids: &[String],
        now: u64,
    ) -> Result<HashSet<(String, String)>> {
        let mut dead = HashSet::new();
        for profile_id in profile_ids {
            let Some(state) = self.read_profile_state(profile_id).await? else {
                continue;
            };
            for post_id in &state.tombstoned_post_ids {
                dead.insert((profile_id.clone(), post_id.clone()));
            }
            for post in &state.posts {
                if post.is_expired(now) {
                    dead.insert((profile_id.clone(), post.post_id.clone()));
                }
            }
        }
        Ok(dead)
    }

    /// Reactively reaps this holder's reactions/comments whose target post is
    /// dead: payload-erases them (content gone, header kept), so a reaction
    /// never outlives the post it is on even though it lives in the reactor's
    /// own month bucket rather than the post's expiry bucket (ADR-0016). Runs
    /// on the local profile and on each synced contact, so both our own
    /// reactions and those we received leave once their target is gone.
    /// Idempotent; returns how many it reaped.
    pub async fn reap_reactions_for_dead_targets(
        &self,
        holder_profile_id: &str,
        dead: &HashSet<(String, String)>,
    ) -> Result<usize> {
        self.erase_content_where(holder_profile_id, |op| {
            op.reaction_target()
                .is_some_and(|(author, post_id)| {
                    dead.contains(&(author.to_owned(), post_id.to_owned()))
                })
        })
        .await
    }

    /// Drops every fully-drained dynamic bucket log associated with a topic
    /// (ADR-0016's payoff): once a log holds nothing live, GC deletes the whole
    /// log — operations *and* headers — with `prune_entries` and un-associates
    /// it so it stops being announced and synced. This is safe precisely
    /// because placement made each bucket a co-deletion unit: an expiry bucket
    /// empties wholesale when its window passes, a re-homed post's old copy is
    /// disowned by its marker, and a reaction bucket empties as its reactions
    /// are reaped. Reserved singleton logs and any log still holding a live
    /// post, unresolved request, or undecryptable op are left untouched.
    ///
    /// Works for our own topic and for a contact's: `prune_entries` targets a
    /// specific `(author, log_id)`, so a recipient reclaims a contact's expired
    /// bucket locally too — the mechanism that lets expired content leave the
    /// network, not just the author. Returns the post ids and blob hashes freed
    /// so the caller can reclaim their pins/cache. Best-effort per log: one
    /// log's failure is logged and skipped, never aborting the rest.
    ///
    /// `local_profile_id` names this device's own author; when a dropped log is
    /// ours, its registry context is forgotten so a later post in that context
    /// allocates a *fresh* id rather than resurrecting the retired one.
    pub async fn drop_drained_buckets(
        &self,
        topic_profile_id: &str,
        local_profile_id: &str,
        now: u64,
        dead: &HashSet<(String, String)>,
    ) -> Result<DrainedContent> {
        let topic = profile_sync_topic(topic_profile_id);
        let associations =
            TopicStore::<Topic, VerifyingKey, DomainLogId>::resolve(&self.store, &topic)
                .await
                .map_err(|err| anyhow::anyhow!("failed to resolve topic for GC: {err}"))?;

        let mut freed = DrainedContent::default();
        for (author, log_ids) in associations {
            for log_id in log_ids {
                // Reserved singleton logs (profile, contacts, spaces control)
                // are never co-deletion buckets.
                if log_id.0 < DomainLogId::FIRST_DYNAMIC {
                    continue;
                }
                let entries: Vec<(Operation<DomainExtensions>, Vec<u8>)> = self
                    .store
                    .get_log_entries(&author, &log_id, None, None)
                    .await
                    .map_err(|err| anyhow::anyhow!("failed to load log for GC: {err}"))?
                    .unwrap_or_default();
                if entries.is_empty() {
                    continue;
                }

                // Decode each op with its ordering timestamp, and track — per
                // post id — the newest `PostPublished` and the latest re-home
                // marker. A post's *current* copy is its newest publication; a
                // re-home marker that sorts after it means the live copy moved
                // to another bucket (same post id and blob hash), so this log's
                // copy is a disowned stale one.
                let mut effective = Vec::with_capacity(entries.len());
                let mut max_seq = 0;
                let mut newest_pub: HashMap<String, (HybridTimestamp, Option<u64>, String)> =
                    HashMap::new();
                let mut latest_marker: HashMap<String, HybridTimestamp> = HashMap::new();
                for (operation, _) in &entries {
                    max_seq = max_seq.max(operation.header.seq_num);
                    let ts = operation.header.extensions.ordering_timestamp;
                    let op = self.effective_operation(operation).await?;
                    match &op {
                        EffectiveOp::Decoded(DomainOperation::PostPublished {
                            post_id,
                            profile_id,
                            expires_at,
                            ..
                        }) => {
                            let newer = newest_pub
                                .get(post_id)
                                .is_none_or(|(seen, _, _)| ts > *seen);
                            if newer {
                                newest_pub.insert(
                                    post_id.clone(),
                                    (ts, *expires_at, profile_id.clone()),
                                );
                            }
                        }
                        EffectiveOp::Decoded(DomainOperation::PostRehomed { post_id, .. }) => {
                            let latest = latest_marker.entry(post_id.clone()).or_insert(ts);
                            if ts > *latest {
                                *latest = ts;
                            }
                        }
                        _ => {}
                    }
                    effective.push(op);
                }

                // Classify each post by its current copy: `live` (keeps the log
                // alive), `reclaimable` (dead by expiry/tombstone — its media
                // may be freed), or moved-out (disowned; its media stays because
                // the same blob lives in the destination bucket).
                let mut live_posts = HashSet::new();
                let mut reclaimable_posts = HashSet::new();
                for (post_id, (pub_ts, expires_at, profile_id)) in &newest_pub {
                    let disowned = latest_marker.get(post_id).is_some_and(|m| m > pub_ts);
                    let gone = expires_at.is_some_and(|at| at <= now)
                        || dead.contains(&(profile_id.clone(), post_id.clone()));
                    if !disowned && !gone {
                        live_posts.insert(post_id.clone());
                    } else if !disowned {
                        reclaimable_posts.insert(post_id.clone());
                    }
                }

                // The log drains only if nothing live remains: no live post,
                // every reaction's target dead, and no undecryptable op.
                let mut drainable = live_posts.is_empty();
                if drainable {
                    for op in &effective {
                        let alive = match op {
                            EffectiveOp::Erased => false,
                            EffectiveOp::Opaque => true,
                            EffectiveOp::Decoded(decoded) => match decoded {
                                // Post ops are accounted for by `live_posts`
                                // above; if any post were live we'd have bailed.
                                DomainOperation::PostPublished { .. }
                                | DomainOperation::PostEdited { .. }
                                | DomainOperation::PostRehomed { .. }
                                | DomainOperation::PostDeleted { .. } => false,
                                DomainOperation::HeartChanged {
                                    post_author_profile_id,
                                    post_id,
                                    ..
                                }
                                | DomainOperation::CommentPublished {
                                    post_author_profile_id,
                                    post_id,
                                    ..
                                } => !dead
                                    .contains(&(post_author_profile_id.clone(), post_id.clone())),
                                // Profile/contact/request/spaces-control content
                                // is never a droppable bucket's tenant.
                                _ => true,
                            },
                        };
                        if alive {
                            drainable = false;
                            break;
                        }
                    }
                }
                if !drainable {
                    continue;
                }

                // Reclaim pins/blobs only for genuinely-dead posts — never a
                // moved-out one, whose blob the live copy still references.
                for op in &effective {
                    if let EffectiveOp::Decoded(
                        decoded @ (DomainOperation::PostPublished { post_id, .. }
                        | DomainOperation::PostEdited { post_id, .. }),
                    ) = op
                    {
                        if reclaimable_posts.contains(post_id) {
                            freed.post_ids.insert(post_id.clone());
                            freed.blob_hashes.extend(decoded.media_blob_hashes());
                        }
                    }
                }

                if let Err(err) = self.prune_and_unassociate(&topic, &author, &log_id, max_seq).await
                {
                    warn!(log_id = log_id.0, "failed to drop drained log: {err:#}");
                    continue;
                }
                // A dropped log that is ours frees its registry context, so the
                // next post there allocates a fresh, never-reused id (ADR-0016).
                // Gated to our own author: a contact's log id can numerically
                // collide with one of our live contexts.
                if author.to_string() == local_profile_id {
                    if let Err(err) = self.forget_log_registry(&log_id).await {
                        warn!(log_id = log_id.0, "failed to forget drained log context: {err:#}");
                    }
                }
            }
        }
        Ok(freed)
    }

    /// Prunes a whole log (operations and headers) then un-associates it from
    /// its topic so it stops being announced. Prune first — content gone is the
    /// priority; a dangling association left by a later failure is harmless
    /// (the now-empty log is skipped next pass). `remove` runs in a transaction.
    async fn prune_and_unassociate(
        &self,
        topic: &Topic,
        author: &VerifyingKey,
        log_id: &DomainLogId,
        max_seq: SeqNum,
    ) -> Result<()> {
        <SqliteStore as LogStore<
            Operation<DomainExtensions>,
            VerifyingKey,
            DomainLogId,
            SeqNum,
            Hash,
        >>::prune_entries(&self.store, author, log_id, &(max_seq + 1))
        .await
        .map_err(|err| anyhow::anyhow!("failed to prune drained log: {err}"))?;

        let permit = self
            .store
            .begin()
            .await
            .map_err(|err| anyhow::anyhow!("failed to begin GC transaction: {err}"))?;
        match TopicStore::<Topic, VerifyingKey, DomainLogId>::remove(
            &self.store,
            topic,
            author,
            log_id,
        )
        .await
        {
            Ok(_) => self
                .store
                .commit(permit)
                .await
                .map_err(|err| anyhow::anyhow!("failed to commit GC transaction: {err}")),
            Err(err) => {
                self.store.rollback(permit).await.ok();
                Err(anyhow::anyhow!("failed to un-associate drained log: {err}"))
            }
        }
    }

    /// Forgets a log's `context → id` registry row after GC dropped the log, so
    /// a later post in that context allocates a fresh monotonic id instead of
    /// resurrecting the retired one. Only ever called for our own logs.
    async fn forget_log_registry(&self, log_id: &DomainLogId) -> Result<()> {
        sqlx::query("DELETE FROM jyn_log_registry WHERE log_id = ?")
            .bind(log_id.0 as i64)
            .execute(self.store.pool())
            .await
            .context("failed to forget drained log registry row")?;
        Ok(())
    }

    /// Classifies a stored op for GC (see [`EffectiveOp`]): body-less ops are
    /// `Erased`, undecryptable `Spaces` wrappers are `Opaque` (we hold the
    /// ciphertext but aren't its audience), everything else is `Decoded` with
    /// its plaintext or decrypted-inner operation.
    async fn effective_operation(
        &self,
        operation: &Operation<DomainExtensions>,
    ) -> Result<EffectiveOp> {
        let Some(body) = &operation.body else {
            return Ok(EffectiveOp::Erased);
        };
        let Ok(decoded) = decode_cbor::<DomainOperation, _>(&body.to_bytes()[..]) else {
            return Ok(EffectiveOp::Opaque);
        };
        match decoded {
            DomainOperation::Spaces { .. } => {
                Ok(match self.decrypted_inner_operation(&operation.hash).await? {
                    Some(inner) => EffectiveOp::Decoded(inner),
                    None => EffectiveOp::Opaque,
                })
            }
            other => Ok(EffectiveOp::Decoded(other)),
        }
    }
}

/// A stored op as GC sees it when deciding whether its bucket has drained.
enum EffectiveOp {
    /// Payload already erased (body-less): its content is gone.
    Erased,
    /// An undecryptable `Spaces` wrapper — we can't classify it, so it must
    /// keep its log alive rather than have GC drop content it can't read.
    Opaque,
    /// A plaintext op or the decrypted inner of a wrapper.
    Decoded(DomainOperation),
}

/// Post ids and blob hashes freed by [`JynOperationDomain::drop_drained_buckets`],
/// so the caller can reclaim their blob pins and materialized cache files.
#[derive(Debug, Default)]
pub struct DrainedContent {
    pub post_ids: HashSet<String>,
    pub blob_hashes: HashSet<String>,
}

/// Creates the local log-placement bookkeeping (ADR-0016): the `context →
/// log id` registry and the monotonic id allocator. Both are pure authoring
/// state — never replicated. The allocator starts at [`DomainLogId::FIRST_DYNAMIC`]
/// and only ever increments, so a retired id is never handed out again.
/// Idempotent; safe to call on every store open and lazily before allocation.
pub async fn ensure_domain_log_tables(store: &SqliteStore) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS jyn_log_registry (
            context TEXT PRIMARY KEY,
            log_id INTEGER NOT NULL
        )",
    )
    .execute(store.pool())
    .await
    .context("failed to create jyn log registry table")?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS jyn_log_allocator (
            id INTEGER PRIMARY KEY CHECK (id = 0),
            next_log_id INTEGER NOT NULL
        )",
    )
    .execute(store.pool())
    .await
    .context("failed to create jyn log allocator table")?;
    sqlx::query("INSERT OR IGNORE INTO jyn_log_allocator (id, next_log_id) VALUES (0, ?)")
        .bind(DomainLogId::FIRST_DYNAMIC as i64)
        .execute(store.pool())
        .await
        .context("failed to seed jyn log allocator")?;
    Ok(())
}

/// Creates jyn's own bookkeeping tables for group encryption (decrypted
/// payload cache, processed-message set, space ownership) next to the
/// p2panda-store tables in the same database.
pub async fn ensure_spaces_tables(store: &SqliteStore) -> Result<()> {
    for ddl in [
        "CREATE TABLE IF NOT EXISTS jyn_spaces_decrypted (
            op_hash TEXT PRIMARY KEY,
            inner_body BLOB NOT NULL
        )",
        "CREATE TABLE IF NOT EXISTS jyn_spaces_processed (
            op_hash TEXT PRIMARY KEY
        )",
        "CREATE TABLE IF NOT EXISTS jyn_spaces_owner (
            space_id TEXT PRIMARY KEY,
            owner_profile_id TEXT NOT NULL,
            kind TEXT
        )",
        "CREATE TABLE IF NOT EXISTS jyn_spaces_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
    ] {
        sqlx::query(ddl)
            .execute(store.pool())
            .await
            .context("failed to create jyn spaces table")?;
    }

    // Additive migration for stores created before circles existed. If the
    // column is new, every existing row is a friends space (the only kind
    // back then) — backfill exactly once; later rows start as NULL until a
    // decrypted post settles their kind.
    if sqlx::query("ALTER TABLE jyn_spaces_owner ADD COLUMN kind TEXT")
        .execute(store.pool())
        .await
        .is_ok()
    {
        sqlx::query("UPDATE jyn_spaces_owner SET kind = 'friends' WHERE kind IS NULL")
            .execute(store.pool())
            .await
            .context("failed to backfill space kinds")?;
    }
    Ok(())
}

fn sort_for_reduction(operations: &mut [StoredDomainOperation]) {
    operations.sort_by(|left, right| {
        left.header
            .extensions
            .ordering_timestamp
            .cmp(&right.header.extensions.ordering_timestamp)
            .then_with(|| left.log_id.cmp(&right.log_id))
            .then_with(|| left.author.to_string().cmp(&right.author.to_string()))
            .then_with(|| left.header.seq_num.cmp(&right.header.seq_num))
    });
}

pub fn profile_sync_topic(profile_id: &str) -> Topic {
    let mut bytes = Vec::with_capacity(DOMAIN_TOPIC_NAMESPACE.len() + profile_id.len() + 1);
    bytes.extend_from_slice(DOMAIN_TOPIC_NAMESPACE);
    bytes.push(b'/');
    bytes.extend_from_slice(profile_id.as_bytes());
    Hash::digest(&bytes).into()
}

/// Returns a hybrid timestamp strictly after `previous` (when given) and never behind the local
/// wall clock.
///
/// Unlike `HybridTimestamp::increment` this never moves backwards when the local clock lags
/// behind a previously observed timestamp; the logical clock component is bumped instead.
fn next_ordering_timestamp(previous: Option<HybridTimestamp>) -> HybridTimestamp {
    let now = HybridTimestamp::now();
    match previous {
        Some(previous) if now <= previous => {
            let (wall, logical) = previous.to_parts();
            HybridTimestamp::from_parts(wall, logical.increment())
        }
        _ => now,
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    fn text_post(
        profile_id: &str,
        post_id: &str,
        body: &str,
        expires_at: Option<u64>,
        created_at: u64,
    ) -> DomainOperation {
        DomainOperation::PostPublished {
            profile_id: profile_id.to_owned(),
            post_id: post_id.to_owned(),
            body: body.to_owned(),
            media: Vec::new(),
            visibility: Visibility::Friends,
            expires_at,
            created_at,
            edited: false,
        }
    }

    #[tokio::test]
    async fn appending_associates_every_authored_log_with_the_profile_topic() -> Result<()> {
        // Placement scatters a profile's operations across several logs
        // (profile, contacts, posts-by-bucket); every one must land on the
        // owner's sync topic so reduction folds them all back together.
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        let mut domain = JynOperationDomain::new(store.clone());

        domain
            .append_operation(
                &key,
                DomainOperation::ProfileUpdated {
                    profile_id: profile_id.clone(),
                    display_name: "Reed".into(),
                    bio: String::new(),
                    default_visibility: Visibility::Friends,
                    default_lifetime_secs: None,
                    created_at: 10,
                    updated_at: 10,
                },
            )
            .await?;
        domain
            .append_operation(&key, text_post(&profile_id, "post-a", "ebbing", Some(100), 10))
            .await?;
        domain
            .append_operation(&key, text_post(&profile_id, "post-b", "settled", None, 20))
            .await?;

        let topic = profile_sync_topic(&profile_id);
        let logs = TopicStore::<Topic, VerifyingKey, DomainLogId>::resolve(&store, &topic).await?;
        let resolved = logs
            .get(&key.verifying_key())
            .cloned()
            .unwrap_or_default();
        // Profile log (reserved id 0), one ephemeral bucket, one permanent
        // bucket — three distinct logs, all on the one topic.
        assert_eq!(resolved.len(), 3);
        assert!(resolved.contains(&DomainLogId::PROFILE));

        Ok(())
    }

    #[tokio::test]
    async fn singleton_streams_use_reserved_fixed_ids() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let domain = JynOperationDomain::new(SqliteStore::temporary().await);

        let profile = domain
            .log_id_for(&DomainOperation::ProfileUpdated {
                profile_id: profile_id.clone(),
                display_name: "R".into(),
                bio: String::new(),
                default_visibility: Visibility::Friends,
                default_lifetime_secs: None,
                created_at: 1,
                updated_at: 1,
            })
            .await?;
        let contacts = domain
            .log_id_for(&DomainOperation::ContactFollowChanged {
                profile_id: profile_id.clone(),
                followed_profile_id: "someone".into(),
                recorded_at: 1,
                active: true,
            })
            .await?;
        let spaces = domain
            .log_id_for(&DomainOperation::Spaces {
                profile_id: profile_id.clone(),
                args: vec![1, 2, 3],
            })
            .await?;

        assert_eq!(profile, DomainLogId::PROFILE);
        assert_eq!(contacts, DomainLogId::CONTACTS);
        assert_eq!(spaces, DomainLogId::SPACES_CONTROL);
        // Reserved ids never touch the dynamic allocator.
        assert!(profile.0 < DomainLogId::FIRST_DYNAMIC);
        assert!(contacts.0 < DomainLogId::FIRST_DYNAMIC);
        assert!(spaces.0 < DomainLogId::FIRST_DYNAMIC);

        Ok(())
    }

    #[tokio::test]
    async fn posts_of_one_expiry_window_share_a_log_across_permanence_boundary() -> Result<()> {
        let profile_id = SigningKey::generate().verifying_key().to_string();
        let domain = JynOperationDomain::new(SqliteStore::temporary().await);

        // Two ephemeral posts with the same expiry share a bucket regardless
        // of post id or creation time (ephemeral placement keys on expiry only).
        let a = domain
            .log_id_for(&text_post(&profile_id, "a", "", Some(5_000_000), 1))
            .await?;
        let b = domain
            .log_id_for(&text_post(&profile_id, "b", "", Some(5_000_000), 999))
            .await?;
        assert_eq!(a, b);

        // A permanent post lives in a different (post-month) bucket.
        let permanent = domain
            .log_id_for(&text_post(&profile_id, "c", "", None, 1))
            .await?;
        assert_ne!(a, permanent);

        // Every dynamically allocated id is in the monotonic range and never a
        // reserved one.
        for id in [a, permanent] {
            assert!(id.0 >= DomainLogId::FIRST_DYNAMIC);
        }

        Ok(())
    }

    #[tokio::test]
    async fn dynamic_ids_are_monotonic_and_stable_per_context() -> Result<()> {
        let profile_id = SigningKey::generate().verifying_key().to_string();
        let domain = JynOperationDomain::new(SqliteStore::temporary().await);

        let first = domain
            .log_id_for(&text_post(&profile_id, "a", "", Some(1_000), 1))
            .await?;
        let second = domain
            .log_id_for(&text_post(&profile_id, "b", "", None, 1))
            .await?;
        // Distinct contexts get distinct, increasing ids starting at the base.
        assert_eq!(first, DomainLogId(DomainLogId::FIRST_DYNAMIC));
        assert_eq!(second, DomainLogId(DomainLogId::FIRST_DYNAMIC + 1));
        // Re-resolving a known context returns its existing id, never a new one.
        let first_again = domain
            .log_id_for(&text_post(&profile_id, "z", "", Some(1_000), 42))
            .await?;
        assert_eq!(first_again, first);

        Ok(())
    }

    #[tokio::test]
    async fn friendship_requests_to_different_targets_use_different_logs() -> Result<()> {
        let requester = SigningKey::generate().verifying_key().to_string();
        let domain = JynOperationDomain::new(SqliteStore::temporary().await);

        let request_to = |target: &str| DomainOperation::FriendshipRequested {
            requester_profile_id: requester.clone(),
            target_profile_id: target.to_owned(),
            requester_display_name: "R".into(),
            greeting: None,
            recorded_at: 1,
        };
        let to_alice = domain.log_id_for(&request_to("alice")).await?;
        let to_bob = domain.log_id_for(&request_to("bob")).await?;
        let to_alice_again = domain.log_id_for(&request_to("alice")).await?;

        // A request log rides the target's topic, so mixing targets would leak
        // one target's request onto another's topic — each target gets its own.
        assert_ne!(to_alice, to_bob);
        assert_eq!(to_alice, to_alice_again);

        Ok(())
    }

    #[tokio::test]
    async fn reactions_are_placed_by_their_own_month_not_the_target_posts_expiry() -> Result<()> {
        let author_key = SigningKey::generate();
        let author_id = author_key.verifying_key().to_string();
        let reactor_id = SigningKey::generate().verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        // The target post lives in its own expiry bucket.
        let post = text_post(&author_id, "post-1", "hi", Some(9_000_000), 5);
        let post_log = domain.log_id_for(&post).await?;
        domain.append_operation(&author_key, post).await?;

        let heart = |month_secs: u64| DomainOperation::HeartChanged {
            profile_id: reactor_id.clone(),
            post_author_profile_id: author_id.clone(),
            post_id: "post-1".into(),
            active: true,
            recorded_at: month_secs,
        };
        // A reaction is placed by its own creation month — never the post's
        // bucket — so a later lifetime change to the post can't strand it.
        let same_month = domain.log_id_for(&heart(crate::placement::MONTH_SECS + 10)).await?;
        let same_month_again = domain
            .log_id_for(&heart(crate::placement::MONTH_SECS + 20))
            .await?;
        let next_month = domain
            .log_id_for(&heart(2 * crate::placement::MONTH_SECS + 5))
            .await?;

        assert_ne!(same_month, post_log);
        assert_eq!(same_month, same_month_again);
        assert_ne!(same_month, next_month);

        Ok(())
    }

    #[tokio::test]
    async fn edits_tombstones_and_rehome_markers_follow_the_posts_current_log() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        // An ephemeral post establishes the "current" bucket for its post id.
        let post = text_post(&profile_id, "post-1", "body", Some(7_000_000), 5);
        let post_log = domain.log_id_for(&post).await?;
        domain.append_operation(&key, post).await?;

        let edit_log = domain
            .log_id_for(&DomainOperation::PostEdited {
                profile_id: profile_id.clone(),
                post_id: "post-1".into(),
                body: "revised".into(),
                media: None,
                edited_at: 6,
            })
            .await?;
        let rehome_log = domain
            .log_id_for(&DomainOperation::PostRehomed {
                profile_id: profile_id.clone(),
                post_id: "post-1".into(),
                moved_at: 7,
            })
            .await?;
        let delete_log = domain
            .log_id_for(&DomainOperation::PostDeleted {
                profile_id: profile_id.clone(),
                post_id: "post-1".into(),
                deleted_at: 8,
            })
            .await?;

        // Edit, re-home marker and tombstone all co-locate with the live copy,
        // so the whole set of a post's operations deletes together.
        assert_eq!(edit_log, post_log);
        assert_eq!(rehome_log, post_log);
        assert_eq!(delete_log, post_log);

        // The re-home *snapshot* itself (a new PostPublished, now permanent)
        // lands in the destination bucket, distinct from the old one.
        let snapshot_log = domain
            .log_id_for(&DomainOperation::PostPublished {
                profile_id: profile_id.clone(),
                post_id: "post-1".into(),
                body: "revised".into(),
                media: Vec::new(),
                visibility: Visibility::Friends,
                expires_at: None,
                created_at: 5,
                edited: true,
            })
            .await?;
        assert_ne!(snapshot_log, post_log);

        Ok(())
    }

    #[tokio::test]
    async fn reducer_rebuilds_posts_with_edit_promote_and_delete() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        domain
            .append_operation(
                &key,
                text_post(&profile_id, "post-a", "first", Some(100), 10),
            )
            .await?;
        domain
            .append_operation(
                &key,
                text_post(&profile_id, "post-b", "second", Some(200), 20),
            )
            .await?;
        // Edit post-a.
        domain
            .append_operation(
                &key,
                DomainOperation::PostEdited {
                    profile_id: profile_id.clone(),
                    post_id: "post-a".into(),
                    body: "first, revised".into(),
                    media: Some(vec![MediaAttachment {
                        kind: MediaKind::Photo,
                        blob_hash: "blob-1".into(),
                        byte_len: 1,
                        mime: "image/png".into(),
                        duration_ms: None,
                        waveform: None,
                        width: None,
                        height: None,
                        file_name: None,
                        blob_secret: None,
                    }]),
                    edited_at: 30,
                },
            )
            .await?;
        // Promote post-a to permanent: a lifetime change re-homes the post as
        // a self-contained snapshot (collapsing the edit above) into the new
        // bucket, and disowns the old copy with a re-home marker (ADR-0016).
        domain
            .append_operation(
                &key,
                DomainOperation::PostRehomed {
                    profile_id: profile_id.clone(),
                    post_id: "post-a".into(),
                    moved_at: 40,
                },
            )
            .await?;
        domain
            .append_operation(
                &key,
                DomainOperation::PostPublished {
                    profile_id: profile_id.clone(),
                    post_id: "post-a".into(),
                    body: "first, revised".into(),
                    media: vec![MediaAttachment {
                        kind: MediaKind::Photo,
                        blob_hash: "blob-1".into(),
                        byte_len: 1,
                        mime: "image/png".into(),
                        duration_ms: None,
                        waveform: None,
                        width: None,
                        height: None,
                        file_name: None,
                        blob_secret: None,
                    }],
                    visibility: Visibility::Friends,
                    expires_at: None,
                    created_at: 10,
                    edited: true,
                },
            )
            .await?;
        // Delete post-b.
        domain
            .append_operation(
                &key,
                DomainOperation::PostDeleted {
                    profile_id: profile_id.clone(),
                    post_id: "post-b".into(),
                    deleted_at: 50,
                },
            )
            .await?;

        let state = domain
            .read_profile_state(&profile_id)
            .await?
            .expect("state exists");

        assert_eq!(state.posts.len(), 1);
        let post = &state.posts[0];
        assert_eq!(post.post_id, "post-a");
        assert_eq!(post.body, "first, revised");
        assert!(post.edited);
        // The edit's Some(media) replaced the attachment list.
        assert_eq!(post.media.len(), 1);
        assert_eq!(post.media[0].blob_hash, "blob-1");
        assert_eq!(post.expires_at, None);
        assert_eq!(state.tombstoned_post_ids, vec!["post-b".to_owned()]);
        assert!(state.is_tombstoned("post-b"));

        Ok(())
    }

    #[tokio::test]
    async fn tombstone_beats_later_edit_and_rehome_snapshot() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        domain
            .append_operation(&key, text_post(&profile_id, "post-a", "body", None, 10))
            .await?;
        domain
            .append_operation(
                &key,
                DomainOperation::PostDeleted {
                    profile_id: profile_id.clone(),
                    post_id: "post-a".into(),
                    deleted_at: 20,
                },
            )
            .await?;
        // An edit arriving after the tombstone is a no-op.
        domain
            .append_operation(
                &key,
                DomainOperation::PostEdited {
                    profile_id: profile_id.clone(),
                    post_id: "post-a".into(),
                    body: "necromancy".into(),
                    media: None,
                    edited_at: 30,
                },
            )
            .await?;
        // A re-home snapshot (a full PostPublished) that sorts after the
        // tombstone must NOT resurrect the post — the tombstone is final.
        domain
            .append_operation(
                &key,
                DomainOperation::PostPublished {
                    profile_id: profile_id.clone(),
                    post_id: "post-a".into(),
                    body: "necromancy".into(),
                    media: Vec::new(),
                    visibility: Visibility::Friends,
                    expires_at: None,
                    created_at: 10,
                    edited: true,
                },
            )
            .await?;

        let state = domain
            .read_profile_state(&profile_id)
            .await?
            .expect("state exists");
        assert!(state.posts.is_empty());
        assert!(state.is_tombstoned("post-a"));

        Ok(())
    }

    #[tokio::test]
    async fn erasing_content_removes_a_non_public_post_from_disk() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        ensure_spaces_tables(&store).await?;
        let mut domain = JynOperationDomain::new(store.clone());

        // A non-public post arrives as an encrypted Spaces wrapper whose
        // decrypted inner op the spaces service cached so reduction can read it.
        let wrapper = domain
            .append_operation(
                &key,
                DomainOperation::Spaces {
                    profile_id: profile_id.clone(),
                    args: vec![0xde, 0xad],
                },
            )
            .await?;
        let wrapper_hash = wrapper.hash();
        domain
            .store_decrypted_inner_operation(
                &wrapper_hash,
                &text_post(&profile_id, "post-x", "sealed words", Some(100), 10),
            )
            .await?;

        // Before teardown: reduction reconstructs the post through the cached
        // decrypted row, and the row is retrievable.
        let state = domain
            .read_profile_state(&profile_id)
            .await?
            .expect("state exists");
        assert!(state
            .posts
            .iter()
            .any(|post| post.post_id == "post-x" && post.body == "sealed words"));
        assert!(domain
            .decrypted_inner_operation(&wrapper_hash)
            .await?
            .is_some());

        // Teardown erases both the readable text and the encrypted payload.
        let erased = domain.erase_post_content(&profile_id, "post-x").await?;
        assert_eq!(erased, 1);

        // The decrypted inner op is gone, so the post can no longer be
        // reconstructed from disk.
        assert!(domain
            .decrypted_inner_operation(&wrapper_hash)
            .await?
            .is_none());

        // The ciphertext payload is gone too: the wrapper op's body is now None,
        // leaving only header-only metadata in the log.
        let raw = domain.operations_for_profile_raw(&profile_id).await?;
        let torn_down = raw
            .iter()
            .find(|op| op.hash == wrapper_hash)
            .expect("wrapper op still present as a header");
        assert!(
            torn_down.body.is_none(),
            "the encrypted payload should be deleted"
        );

        // Reduction tolerates the body-less op (skips it) rather than erroring,
        // and the post is absent from reconstructed state.
        let state = domain.read_profile_state(&profile_id).await?;
        assert!(state.is_none_or(|state| state
            .posts
            .iter()
            .all(|post| post.post_id != "post-x")));

        // Idempotent: a second teardown erases nothing (the op is already
        // body-less and skipped).
        assert_eq!(domain.erase_post_content(&profile_id, "post-x").await?, 0);

        Ok(())
    }

    #[tokio::test]
    async fn expiry_is_a_read_time_filter_not_a_reduction_effect() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        domain
            .append_operation(
                &key,
                text_post(&profile_id, "post-a", "ebbing", Some(100), 10),
            )
            .await?;
        domain
            .append_operation(&key, text_post(&profile_id, "post-b", "settled", None, 20))
            .await?;

        let state = domain
            .read_profile_state(&profile_id)
            .await?
            .expect("state exists");

        // Reduction keeps expired posts (deterministic, restart-safe)...
        assert_eq!(state.posts.len(), 2);
        // ...and read-time filtering drains them.
        let alive_before: Vec<_> = state.active_posts(99).map(|p| p.post_id.clone()).collect();
        assert_eq!(alive_before, vec!["post-b".to_owned(), "post-a".to_owned()]);
        let alive_after: Vec<_> = state.active_posts(100).map(|p| p.post_id.clone()).collect();
        assert_eq!(alive_after, vec!["post-b".to_owned()]);

        Ok(())
    }

    #[tokio::test]
    async fn heart_toggling_reduces_to_final_state() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let friend_id = SigningKey::generate().verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        let heart = |active: bool, recorded_at: u64| DomainOperation::HeartChanged {
            profile_id: profile_id.clone(),
            post_author_profile_id: friend_id.clone(),
            post_id: "their-post".into(),
            active,
            recorded_at,
        };
        domain.append_operation(&key, heart(true, 10)).await?;
        domain.append_operation(&key, heart(false, 20)).await?;
        domain.append_operation(&key, heart(true, 30)).await?;

        let state = domain
            .read_profile_state(&profile_id)
            .await?
            .expect("state exists");
        assert_eq!(
            state.hearts,
            vec![HeartRef {
                post_author_profile_id: friend_id,
                post_id: "their-post".into(),
                recorded_at: 30,
            }]
        );

        Ok(())
    }

    #[tokio::test]
    async fn comments_dedupe_by_id_and_sort_by_time() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let friend_id = SigningKey::generate().verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        let comment = |id: &str, body: &str, created_at: u64| DomainOperation::CommentPublished {
            profile_id: profile_id.clone(),
            comment_id: id.to_owned(),
            post_author_profile_id: friend_id.clone(),
            post_id: "their-post".into(),
            body: body.to_owned(),
            created_at,
        };
        domain
            .append_operation(&key, comment("c-2", "second", 20))
            .await?;
        domain
            .append_operation(&key, comment("c-1", "first", 10))
            .await?;
        domain
            .append_operation(&key, comment("c-1", "first, revised", 10))
            .await?;

        let state = domain
            .read_profile_state(&profile_id)
            .await?
            .expect("state exists");
        assert_eq!(state.comments.len(), 2);
        assert_eq!(state.comments[0].comment_id, "c-1");
        assert_eq!(state.comments[0].body, "first, revised");
        assert_eq!(state.comments[1].comment_id, "c-2");

        Ok(())
    }

    #[tokio::test]
    async fn friendship_requests_reduce_to_pending_until_answered() -> Result<()> {
        let owner_key = SigningKey::generate();
        let owner_id = owner_key.verifying_key().to_string();
        let requester_key = SigningKey::generate();
        let requester_id = requester_key.verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        // The requester writes onto the owner's topic, signed by the requester.
        domain
            .append_operation(
                &requester_key,
                DomainOperation::FriendshipRequested {
                    requester_profile_id: requester_id.clone(),
                    target_profile_id: owner_id.clone(),
                    requester_display_name: "Wen Li".into(),
                    greeting: Some("river sent me".into()),
                    recorded_at: 10,
                },
            )
            .await?;

        let state = domain
            .read_profile_state(&owner_id)
            .await?
            .expect("state exists");
        assert_eq!(state.pending_requests.len(), 1);
        assert_eq!(state.pending_requests[0].requester_profile_id, requester_id);
        assert_eq!(
            state.pending_requests[0].greeting.as_deref(),
            Some("river sent me")
        );

        // Owner accepts: respond + follow.
        domain
            .append_operation(
                &owner_key,
                DomainOperation::FriendshipResponded {
                    target_profile_id: owner_id.clone(),
                    requester_profile_id: requester_id.clone(),
                    accepted: true,
                    recorded_at: 20,
                },
            )
            .await?;
        domain
            .append_operation(
                &owner_key,
                DomainOperation::ContactFollowChanged {
                    profile_id: owner_id.clone(),
                    followed_profile_id: requester_id.clone(),
                    recorded_at: 20,
                    active: true,
                },
            )
            .await?;

        let state = domain
            .read_profile_state(&owner_id)
            .await?
            .expect("state exists");
        assert!(state.pending_requests.is_empty());
        assert_eq!(state.followed_profile_ids, vec![requester_id]);

        Ok(())
    }

    #[tokio::test]
    async fn spoofed_requests_and_foreign_posts_are_ignored() -> Result<()> {
        let owner_key = SigningKey::generate();
        let owner_id = owner_key.verifying_key().to_string();
        let attacker_key = SigningKey::generate();
        let innocent_id = SigningKey::generate().verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        // A request claiming to come from someone other than its signer.
        domain
            .append_operation(
                &attacker_key,
                DomainOperation::FriendshipRequested {
                    requester_profile_id: innocent_id.clone(),
                    target_profile_id: owner_id.clone(),
                    requester_display_name: "Not Really Them".into(),
                    greeting: None,
                    recorded_at: 10,
                },
            )
            .await?;
        // A post signed by a foreign key claiming to be the owner's.
        domain
            .append_operation(
                &attacker_key,
                text_post(&owner_id, "forged-post", "not yours", None, 20),
            )
            .await?;
        // A foreign-signed follow "from" the owner.
        domain
            .append_operation(
                &attacker_key,
                DomainOperation::ContactFollowChanged {
                    profile_id: owner_id.clone(),
                    followed_profile_id: innocent_id.clone(),
                    recorded_at: 30,
                    active: true,
                },
            )
            .await?;

        let state = domain
            .read_profile_state(&owner_id)
            .await?
            .expect("state exists");
        assert!(state.pending_requests.is_empty());
        assert!(state.posts.is_empty());
        assert!(state.followed_profile_ids.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn private_posts_are_rejected_by_the_domain() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        let result = domain
            .append_operation(
                &key,
                DomainOperation::PostPublished {
                    profile_id: profile_id.clone(),
                    post_id: "private-post".into(),
                    body: "only for me".into(),
                    media: Vec::new(),
                    visibility: Visibility::Private,
                    expires_at: None,
                    created_at: 10,
                    edited: false,
                },
            )
            .await;

        assert!(result.is_err());
        assert!(domain.read_profile_state(&profile_id).await?.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn ingest_remote_operation_is_idempotent_for_duplicate_delivery() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let operation = text_post(&profile_id, "post-a", "hello", None, 10);

        let mut source = JynOperationDomain::new(SqliteStore::temporary().await);
        let header = source.append_operation(&key, operation.clone()).await?;
        let replicated = Operation {
            hash: header.hash(),
            header,
            body: Some(Body::from(encode_cbor(&operation)?)),
        };

        let mut target = JynOperationDomain::new(SqliteStore::temporary().await);
        target.ingest_remote_operation(replicated.clone()).await?;
        target.ingest_remote_operation(replicated).await?;

        let operations = target.operations_for_profile(&profile_id).await?;
        assert_eq!(operations.len(), 1);
        assert_eq!(
            target
                .read_profile_state(&profile_id)
                .await?
                .expect("state exists")
                .posts
                .len(),
            1
        );

        Ok(())
    }

    #[tokio::test]
    async fn profile_defaults_reduce_from_latest_update() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        domain
            .append_operation(
                &key,
                DomainOperation::ProfileUpdated {
                    profile_id: profile_id.clone(),
                    display_name: "Amber Apple".into(),
                    bio: String::new(),
                    default_visibility: Visibility::Friends,
                    default_lifetime_secs: Some(36 * 3600),
                    created_at: 10,
                    updated_at: 10,
                },
            )
            .await?;
        domain
            .append_operation(
                &key,
                DomainOperation::ProfileUpdated {
                    profile_id: profile_id.clone(),
                    display_name: "Velvet Pear".into(),
                    bio: "casts fragments".into(),
                    default_visibility: Visibility::Circles,
                    default_lifetime_secs: None,
                    created_at: 10,
                    updated_at: 20,
                },
            )
            .await?;

        let state = domain
            .read_profile_state(&profile_id)
            .await?
            .expect("state exists");
        assert_eq!(state.display_name.as_deref(), Some("Velvet Pear"));
        assert_eq!(state.bio, "casts fragments");
        assert_eq!(state.default_visibility, Visibility::Circles);
        assert_eq!(state.default_lifetime_secs, None);

        Ok(())
    }

    /// Log ids currently associated with a profile's topic, for GC assertions.
    async fn topic_log_ids(store: &SqliteStore, profile_id: &str) -> Result<Vec<DomainLogId>> {
        let topic = profile_sync_topic(profile_id);
        let logs = TopicStore::<Topic, VerifyingKey, DomainLogId>::resolve(store, &topic).await?;
        Ok(logs.into_values().flatten().collect())
    }

    const FUTURE: u64 = 10_000_000_000;

    #[tokio::test]
    async fn a_fully_expired_bucket_log_is_pruned_and_un_announced() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        let mut domain = JynOperationDomain::new(store.clone());

        // A post whose expiry is already in the past: its whole bucket window
        // has closed, so GC may drop the entire log.
        domain
            .append_operation(&key, text_post(&profile_id, "post-a", "ebbing", Some(50), 10))
            .await?;
        let before = topic_log_ids(&store, &profile_id).await?;
        assert_eq!(before.len(), 1, "the post's bucket log is associated");

        let dead = HashSet::new();
        let freed = domain
            .drop_drained_buckets(&profile_id, &profile_id, FUTURE, &dead)
            .await?;

        // The bucket log is gone from the store and from the topic (so it stops
        // being announced/synced), and its post is reported freed for reclaim.
        assert!(topic_log_ids(&store, &profile_id).await?.is_empty());
        assert!(domain.operations_for_profile(&profile_id).await?.is_empty());
        assert!(freed.post_ids.contains("post-a"));

        Ok(())
    }

    #[tokio::test]
    async fn an_expired_public_post_is_reaped_like_any_other() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        let mut domain = JynOperationDomain::new(store.clone());

        // A public post the author gave a lifetime is ephemeral by their
        // choice; GC reclaims its bucket on expiry just like a non-public post.
        domain
            .append_operation(
                &key,
                DomainOperation::PostPublished {
                    profile_id: profile_id.clone(),
                    post_id: "shout".into(),
                    body: "read me".into(),
                    media: Vec::new(),
                    visibility: Visibility::Public,
                    expires_at: Some(50),
                    created_at: 10,
                    edited: false,
                },
            )
            .await?;

        let dead = domain
            .dead_post_targets(std::slice::from_ref(&profile_id), FUTURE)
            .await?;
        assert!(dead.contains(&(profile_id.clone(), "shout".to_owned())));

        let freed = domain
            .drop_drained_buckets(&profile_id, &profile_id, FUTURE, &dead)
            .await?;
        // Its expiry bucket is pruned and un-announced, and the post is freed.
        assert!(topic_log_ids(&store, &profile_id).await?.is_empty());
        assert!(freed.post_ids.contains("shout"));

        Ok(())
    }

    #[tokio::test]
    async fn a_permanent_bucket_is_never_window_dropped() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        let mut domain = JynOperationDomain::new(store.clone());

        domain
            .append_operation(&key, text_post(&profile_id, "keeper", "settled", None, 10))
            .await?;

        let dead = HashSet::new();
        domain
            .drop_drained_buckets(&profile_id, &profile_id, FUTURE, &dead)
            .await?;

        // A permanent post's bucket stays — permanent content leaves only by
        // individual tombstone, never a window drop.
        assert_eq!(topic_log_ids(&store, &profile_id).await?.len(), 1);
        assert_eq!(domain.operations_for_profile(&profile_id).await?.len(), 1);

        Ok(())
    }

    #[tokio::test]
    async fn a_rehomed_posts_old_bucket_drops_while_the_post_survives() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        let mut domain = JynOperationDomain::new(store.clone());

        // Publish ephemeral, then re-home to permanent: marker into the old
        // bucket, self-contained snapshot into the new (permanent) one.
        domain
            .append_operation(&key, text_post(&profile_id, "post-a", "body", Some(50), 10))
            .await?;
        domain
            .append_operation(
                &key,
                DomainOperation::PostRehomed {
                    profile_id: profile_id.clone(),
                    post_id: "post-a".into(),
                    moved_at: 60,
                },
            )
            .await?;
        domain
            .append_operation(&key, text_post(&profile_id, "post-a", "body", None, 10))
            .await?;
        assert_eq!(topic_log_ids(&store, &profile_id).await?.len(), 2);

        let dead = HashSet::new();
        domain
            .drop_drained_buckets(&profile_id, &profile_id, FUTURE, &dead)
            .await?;

        // The old (disowned) bucket is gone; the permanent snapshot's bucket
        // stays, and the post still reduces from it.
        assert_eq!(topic_log_ids(&store, &profile_id).await?.len(), 1);
        let state = domain
            .read_profile_state(&profile_id)
            .await?
            .expect("state exists");
        assert_eq!(state.posts.len(), 1);
        assert_eq!(state.posts[0].post_id, "post-a");
        assert_eq!(state.posts[0].expires_at, None);

        Ok(())
    }

    #[tokio::test]
    async fn a_reaction_is_reaped_when_its_target_dies_then_its_bucket_drops() -> Result<()> {
        let key = SigningKey::generate();
        let reactor_id = key.verifying_key().to_string();
        let author_id = SigningKey::generate().verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        let mut domain = JynOperationDomain::new(store.clone());

        domain
            .append_operation(
                &key,
                DomainOperation::CommentPublished {
                    profile_id: reactor_id.clone(),
                    comment_id: "c-1".into(),
                    post_author_profile_id: author_id.clone(),
                    post_id: "post-x".into(),
                    body: "nice".into(),
                    created_at: 10,
                },
            )
            .await?;

        // While the target post is alive, the reaction is left untouched.
        let alive: HashSet<(String, String)> = HashSet::new();
        assert_eq!(
            domain
                .reap_reactions_for_dead_targets(&reactor_id, &alive)
                .await?,
            0
        );
        assert_eq!(
            domain
                .read_profile_state(&reactor_id)
                .await?
                .expect("state")
                .comments
                .len(),
            1
        );

        // Once the target post is dead, the reaction is reaped (content erased)
        // and its now-empty month bucket drops wholesale.
        let dead: HashSet<(String, String)> =
            [(author_id.clone(), "post-x".to_owned())].into_iter().collect();
        assert_eq!(
            domain
                .reap_reactions_for_dead_targets(&reactor_id, &dead)
                .await?,
            1
        );
        // With its only op now body-less, the reactor reduces to no state at
        // all — the comment is gone, not merely hidden.
        assert!(domain
            .read_profile_state(&reactor_id)
            .await?
            .is_none_or(|state| state.comments.is_empty()));

        domain
            .drop_drained_buckets(&reactor_id, &reactor_id, FUTURE, &dead)
            .await?;
        assert!(topic_log_ids(&store, &reactor_id).await?.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn a_request_log_is_never_dropped_by_gc() -> Result<()> {
        let requester_key = SigningKey::generate();
        let requester_id = requester_key.verifying_key().to_string();
        let target_id = SigningKey::generate().verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        let mut domain = JynOperationDomain::new(store.clone());

        // A request rides the *target's* topic, in a dynamic request log.
        domain
            .append_operation(
                &requester_key,
                DomainOperation::FriendshipRequested {
                    requester_profile_id: requester_id.clone(),
                    target_profile_id: target_id.clone(),
                    requester_display_name: "R".into(),
                    greeting: None,
                    recorded_at: 10,
                },
            )
            .await?;

        let dead = HashSet::new();
        domain.drop_drained_buckets(&target_id, &requester_id, FUTURE, &dead).await?;

        // GC leaves it alone — a pending request isn't co-deletion content.
        assert_eq!(topic_log_ids(&store, &target_id).await?.len(), 1);
        assert_eq!(
            domain
                .read_profile_state(&target_id)
                .await?
                .expect("state")
                .pending_requests
                .len(),
            1
        );

        Ok(())
    }

    #[tokio::test]
    async fn a_dropped_logs_context_reallocates_a_fresh_id_never_the_retired_one() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        let mut domain = JynOperationDomain::new(store.clone());

        // A permanent post and its bucket's log id (context "bucket/perm/0").
        let first = text_post(&profile_id, "post-a", "gone soon", None, 10);
        let retired_id = domain.log_id_for(&first).await?;
        domain.append_operation(&key, first).await?;
        domain
            .append_operation(
                &key,
                DomainOperation::PostDeleted {
                    profile_id: profile_id.clone(),
                    post_id: "post-a".into(),
                    deleted_at: 20,
                },
            )
            .await?;

        // Delete drains the bucket; GC drops the whole log.
        let dead: HashSet<(String, String)> =
            [(profile_id.clone(), "post-a".to_owned())].into_iter().collect();
        domain
            .drop_drained_buckets(&profile_id, &profile_id, FUTURE, &dead)
            .await?;
        assert!(topic_log_ids(&store, &profile_id).await?.is_empty());

        // A new permanent post in the same month resolves the same context —
        // but must get a FRESH id, never the retired one (a recycled id would
        // restart at seq 0 and be rejected by peers still holding the old log).
        let second = text_post(&profile_id, "post-b", "new one", None, 15);
        let reused_id = domain.log_id_for(&second).await?;
        assert_ne!(reused_id, retired_id);
        assert!(reused_id.0 > retired_id.0, "the allocator only ever moves forward");

        Ok(())
    }

    #[tokio::test]
    async fn dropping_a_re_homed_posts_old_bucket_does_not_reclaim_its_still_live_media() -> Result<()>
    {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        let mut domain = JynOperationDomain::new(store.clone());

        let with_media = |post_id: &str, expires_at: Option<u64>| DomainOperation::PostPublished {
            profile_id: profile_id.clone(),
            post_id: post_id.to_owned(),
            body: "body".into(),
            media: vec![MediaAttachment {
                kind: MediaKind::Photo,
                blob_hash: "blob-1".into(),
                byte_len: 1,
                mime: "image/png".into(),
                duration_ms: None,
                waveform: None,
                width: None,
                height: None,
                file_name: None,
                blob_secret: None,
            }],
            visibility: Visibility::Friends,
            expires_at,
            created_at: 10,
            edited: false,
        };

        // Publish ephemeral with media, then promote: the snapshot re-uses the
        // same post id and blob hash and lands in a new (permanent) bucket.
        domain
            .append_operation(&key, with_media("post-a", Some(50)))
            .await?;
        domain
            .append_operation(
                &key,
                DomainOperation::PostRehomed {
                    profile_id: profile_id.clone(),
                    post_id: "post-a".into(),
                    moved_at: 60,
                },
            )
            .await?;
        domain
            .append_operation(&key, with_media("post-a", None))
            .await?;

        let dead = HashSet::new();
        let freed = domain
            .drop_drained_buckets(&profile_id, &profile_id, FUTURE, &dead)
            .await?;

        // The old bucket is dropped, but its media must NOT be reported freed —
        // the live copy in the new bucket still references that blob.
        assert!(!freed.post_ids.contains("post-a"));
        assert!(!freed.blob_hashes.contains("blob-1"));
        // The post is still alive, in its new permanent bucket.
        let state = domain
            .read_profile_state(&profile_id)
            .await?
            .expect("state");
        assert_eq!(state.posts.len(), 1);
        assert_eq!(state.posts[0].media[0].blob_hash, "blob-1");

        Ok(())
    }

    #[tokio::test]
    async fn a_re_home_landing_in_the_same_log_keeps_the_live_snapshot() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        let mut domain = JynOperationDomain::new(store.clone());

        // Force old copy, re-home marker, and new snapshot into ONE log (as a
        // lifetime change within the same bucket window would). The new
        // snapshot is still live (expiry far in the future).
        let log = DomainLogId(DomainLogId::FIRST_DYNAMIC);
        domain
            .append_operation_in_log(&key, text_post(&profile_id, "post-a", "old", Some(100), 10), log)
            .await?;
        domain
            .append_operation_in_log(
                &key,
                DomainOperation::PostRehomed {
                    profile_id: profile_id.clone(),
                    post_id: "post-a".into(),
                    moved_at: 20,
                },
                log,
            )
            .await?;
        domain
            .append_operation_in_log(
                &key,
                text_post(&profile_id, "post-a", "new", Some(5_000_000_000), 10),
                log,
            )
            .await?;

        // At now=1000 the new snapshot is alive, so the log must NOT be dropped
        // even though a re-home marker disowns an *earlier* copy in it.
        let dead = HashSet::new();
        domain
            .drop_drained_buckets(&profile_id, &profile_id, 1000, &dead)
            .await?;

        assert_eq!(topic_log_ids(&store, &profile_id).await?, vec![log]);
        let state = domain
            .read_profile_state(&profile_id)
            .await?
            .expect("state");
        assert_eq!(state.posts.len(), 1);
        assert_eq!(state.posts[0].body, "new");

        Ok(())
    }
}

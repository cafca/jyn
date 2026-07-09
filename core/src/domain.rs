//! Jyn's operation domain: posts with author-set lifetimes, named hearts,
//! flat comments, consented friendship — all on p2panda append-only logs with
//! hybrid-logical-clock ordering.
//!
//! Every profile has one sync topic and five logs (see [`DomainLogKind`]).
//! All logs on a topic are normally authored by the profile owner; the one
//! deliberate exception is [`DomainOperation::FriendshipRequested`], which is
//! authored by the *requester* but lives on the *target's* topic so requests
//! reach their target through normal topic sync. Reduction therefore enforces
//! authorship: owner-signed operations shape the profile, requester-signed
//! operations can only ever surface as pending friendship requests.
//!
//! Replaces the file-sharing `operation_domain` module, which is kept around
//! only until its remaining dependents are ported.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use p2panda_core::cbor::{decode_cbor, encode_cbor};
use p2panda_core::timestamp::HybridTimestamp;
use p2panda_core::Topic;
use p2panda_core::{Body, Extension, Hash, Header, Operation, SigningKey, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::topics::TopicStore;
use p2panda_store::{SqliteError, SqliteStore, Transaction};
use p2panda_stream::ingest::ingest_operation;
use serde::{Deserialize, Serialize};
use tracing::warn;

const DOMAIN_TOPIC_NAMESPACE: &[u8] = b"jyn/domain/v1";
const REDUCED_PROFILE_STATE_VERSION: u8 = 1;
const DOMAIN_OPERATION_CACHE_VERSION: u8 = 1;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainLogKind {
    Profile,
    Posts,
    Contacts,
    Interactions,
    Requests,
    /// Group-encryption traffic: key bundles, membership control messages and
    /// encrypted application payloads (see `crate::spaces`).
    Spaces,
}

impl DomainLogKind {
    const fn rank(self) -> u8 {
        match self {
            Self::Profile => 0,
            Self::Posts => 1,
            Self::Contacts => 2,
            Self::Interactions => 3,
            Self::Requests => 4,
            Self::Spaces => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DomainLogId {
    pub profile_id: String,
    pub kind: DomainLogKind,
}

impl DomainLogId {
    pub fn new(profile_id: impl Into<String>, kind: DomainLogKind) -> Self {
        Self {
            profile_id: profile_id.into(),
            kind,
        }
    }

    pub fn all_for_profile(profile_id: &str) -> Vec<Self> {
        [
            DomainLogKind::Profile,
            DomainLogKind::Posts,
            DomainLogKind::Contacts,
            DomainLogKind::Interactions,
            DomainLogKind::Requests,
            DomainLogKind::Spaces,
        ]
        .into_iter()
        .map(|kind| Self::new(profile_id, kind))
        .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainExtensions {
    pub log_id: DomainLogId,
    #[serde(default = "HybridTimestamp::now")]
    pub ordering_timestamp: HybridTimestamp,
}

impl Extension<DomainLogId> for DomainExtensions {
    fn extract(header: &Header<Self>) -> Option<DomainLogId> {
        Some(header.extensions.log_id.clone())
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
    /// Promote (`expires_at: None`) or let it go again (`Some`).
    PostLifetimeChanged {
        profile_id: String,
        post_id: String,
        expires_at: Option<u64>,
        changed_at: u64,
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
            | Self::PostLifetimeChanged { profile_id, .. }
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

    fn log_kind(&self) -> DomainLogKind {
        match self {
            Self::ProfileUpdated { .. } => DomainLogKind::Profile,
            Self::PostPublished { .. }
            | Self::PostEdited { .. }
            | Self::PostLifetimeChanged { .. }
            | Self::PostDeleted { .. } => DomainLogKind::Posts,
            Self::ContactFollowChanged { .. } | Self::FriendshipResponded { .. } => {
                DomainLogKind::Contacts
            }
            Self::HeartChanged { .. } | Self::CommentPublished { .. } => {
                DomainLogKind::Interactions
            }
            Self::FriendshipRequested { .. } => DomainLogKind::Requests,
            Self::Spaces { .. } => DomainLogKind::Spaces,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedReducedProfileState {
    version: u8,
    state: ReducedProfileState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PersistedDomainOperations {
    version: u8,
    operations: Vec<StoredRawDomainOperation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredRawDomainOperation {
    header: Header<DomainExtensions>,
    #[serde(with = "serde_bytes")]
    body: Vec<u8>,
}

/// Profile-oriented view onto the store's topic associations.
#[derive(Clone, Debug)]
pub struct JynTopicMap {
    store: SqliteStore,
}

impl JynTopicMap {
    pub fn new(store: SqliteStore) -> Self {
        Self { store }
    }

    pub async fn register_profile(&self, profile_id: &str) -> Topic {
        profile_sync_topic(profile_id)
    }

    pub async fn register_profile_author(&self, profile_id: &str, author: VerifyingKey) -> Topic {
        let topic = profile_sync_topic(profile_id);
        if let Err(err) = self
            .associate_profile_logs(profile_id, &topic, &author)
            .await
        {
            warn!("failed to associate domain logs with topic: {err}");
        }
        topic
    }

    /// Associates all domain logs of a profile with its sync topic in one store transaction.
    async fn associate_profile_logs(
        &self,
        profile_id: &str,
        topic: &Topic,
        author: &VerifyingKey,
    ) -> Result<(), SqliteError> {
        let permit = self.store.begin().await?;
        for log_id in DomainLogId::all_for_profile(profile_id) {
            if let Err(err) = TopicStore::<Topic, VerifyingKey, DomainLogId>::associate(
                &self.store,
                topic,
                author,
                &log_id,
            )
            .await
            {
                self.store.rollback(permit).await?;
                return Err(err);
            }
        }
        self.store.commit(permit).await?;
        Ok(())
    }

    pub async fn known_authors(&self, profile_id: &str) -> Vec<VerifyingKey> {
        let topic = profile_sync_topic(profile_id);
        let associations =
            TopicStore::<Topic, VerifyingKey, DomainLogId>::resolve(&self.store, &topic)
                .await
                .unwrap_or_default();
        let mut authors = associations.into_keys().collect::<Vec<_>>();
        authors.sort_by_key(|author| author.to_string());
        authors
    }
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

    pub fn topic_map(&self) -> JynTopicMap {
        JynTopicMap::new(self.store.clone())
    }

    pub async fn append_operation(
        &mut self,
        private_key: &SigningKey,
        operation: DomainOperation,
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
        let log_id = DomainLogId::new(&profile_id, operation.log_kind());
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
                log_id: log_id.clone(),
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
        let log_id = operation.header.extensions.log_id.clone();
        let topic = profile_sync_topic(&log_id.profile_id);

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
                } => {
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
                            edited: false,
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
                DomainOperation::PostLifetimeChanged {
                    post_id,
                    expires_at,
                    ..
                } => {
                    if let Some(post) = posts.get_mut(&post_id) {
                        post.expires_at = expires_at;
                    }
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
                    let body = operation
                        .body
                        .context("domain operation payload is missing")?;
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
                        log_id: log_id.clone(),
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
            owner_profile_id TEXT NOT NULL
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
    Ok(())
}

fn sort_for_reduction(operations: &mut [StoredDomainOperation]) {
    operations.sort_by(|left, right| {
        left.header
            .extensions
            .ordering_timestamp
            .cmp(&right.header.extensions.ordering_timestamp)
            .then_with(|| left.log_id.kind.rank().cmp(&right.log_id.kind.rank()))
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

pub fn write_reduced_profile_state_to_path(
    path: impl AsRef<Path>,
    state: &ReducedProfileState,
) -> Result<()> {
    let persisted = PersistedReducedProfileState {
        version: REDUCED_PROFILE_STATE_VERSION,
        state: state.clone(),
    };
    write_json_atomic(path.as_ref(), &persisted, "reduced profile cache")
}

pub fn load_reduced_profile_state_from_path(path: impl AsRef<Path>) -> Result<ReducedProfileState> {
    let path = path.as_ref();
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read reduced profile cache {}", path.display()))?;
    let persisted: PersistedReducedProfileState = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse reduced profile cache {}", path.display()))?;
    if persisted.version != REDUCED_PROFILE_STATE_VERSION {
        anyhow::bail!(
            "unsupported reduced profile cache version {} in {}",
            persisted.version,
            path.display()
        );
    }
    Ok(persisted.state)
}

pub fn load_raw_domain_operations_from_path(
    path: impl AsRef<Path>,
) -> Result<Vec<Operation<DomainExtensions>>> {
    let path = path.as_ref();
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read domain operation cache {}", path.display()))?;
    let persisted: PersistedDomainOperations = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse domain operation cache {}", path.display()))?;
    if persisted.version != DOMAIN_OPERATION_CACHE_VERSION {
        anyhow::bail!(
            "unsupported domain operation cache version {} in {}",
            persisted.version,
            path.display()
        );
    }
    persisted
        .operations
        .into_iter()
        .map(|operation| {
            let validated = Operation {
                hash: operation.header.hash(),
                header: operation.header,
                body: Some(Body::from(operation.body)),
            };
            p2panda_core::validate_operation(&validated)
                .context("cached domain operation validation failed")?;
            Ok(validated)
        })
        .collect()
}

pub fn write_raw_domain_operations_to_path(
    path: impl AsRef<Path>,
    operations: impl IntoIterator<Item = Operation<DomainExtensions>>,
) -> Result<()> {
    let operations = operations
        .into_iter()
        .map(|operation| {
            let body = operation
                .body
                .context("domain operation is missing a body while persisting cache")?;
            Ok(StoredRawDomainOperation {
                header: operation.header,
                body: body.to_bytes(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let persisted = PersistedDomainOperations {
        version: DOMAIN_OPERATION_CACHE_VERSION,
        operations,
    };
    write_json_atomic(path.as_ref(), &persisted, "domain operation cache")
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T, label: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create parent directory for {} at {}",
                label,
                parent.display()
            )
        })?;
    }

    let bytes =
        serde_json::to_vec_pretty(value).with_context(|| format!("failed to serialize {label}"))?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, bytes).with_context(|| {
        format!(
            "failed to write temporary {} file {}",
            label,
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "failed to atomically move temporary {} file {} to {}",
            label,
            tmp_path.display(),
            path.display()
        )
    })?;
    Ok(())
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
        }
    }

    #[tokio::test]
    async fn topic_map_resolves_profile_topic_to_all_domain_logs() -> Result<()> {
        let private_key = SigningKey::generate();
        let profile_id = private_key.verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        let topic_map = JynTopicMap::new(store.clone());
        let topic = topic_map
            .register_profile_author(&profile_id, private_key.verifying_key())
            .await;

        let logs = TopicStore::<Topic, VerifyingKey, DomainLogId>::resolve(&store, &topic).await?;
        let mut resolved = logs
            .get(&private_key.verifying_key())
            .cloned()
            .unwrap_or_default();
        resolved.sort();
        let mut expected = DomainLogId::all_for_profile(&profile_id);
        expected.sort();
        assert_eq!(resolved, expected);
        assert_eq!(expected.len(), 6);

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
        // Promote post-a to permanent.
        domain
            .append_operation(
                &key,
                DomainOperation::PostLifetimeChanged {
                    profile_id: profile_id.clone(),
                    post_id: "post-a".into(),
                    expires_at: None,
                    changed_at: 40,
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
    async fn tombstone_beats_later_edit_and_lifetime_change() -> Result<()> {
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
        // Edits and lifetime changes arriving after the tombstone are no-ops.
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
        domain
            .append_operation(
                &key,
                DomainOperation::PostLifetimeChanged {
                    profile_id: profile_id.clone(),
                    post_id: "post-a".into(),
                    expires_at: None,
                    changed_at: 40,
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
}

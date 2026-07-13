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

use crate::groups::{GroupContentMode, GroupDiscoverability, GroupGovernanceAction, GroupJoinMode};

// v2: the group-encryption flag day. Old plaintext clients stay on v1 topics
// and never exchange operations with encrypted ones.
// v3: the Groups flag day. `GroupMembershipAdvertised` is a new operation
// variant on the shared Contacts topic; released v2 clients hard-error on an
// unknown variant and would drop the whole author's reduction, so the topic
// namespace moves again and old and new clients never share a topic.
const DOMAIN_TOPIC_NAMESPACE: &[u8] = b"jyn/domain/v3";
/// Each Group is its own replication topic derived from its GroupId
/// (ADR-0007) — a replication axis alongside the per-profile topics.
const GROUP_TOPIC_NAMESPACE: &[u8] = b"jyn/groups/v1";
/// Log-context prefix for group genesis ops. The GroupId is the hash *of*
/// the genesis op, so the genesis cannot live on a log named after it; it
/// gets a one-op log under a unique context instead.
pub const GROUP_GENESIS_CONTEXT_PREFIX: &str = "jyn/group-genesis/";
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
    /// Group-context traffic (see `crate::groups`): genesis, governance,
    /// join requests, group posts and interactions, and the group's own
    /// auth/encryption control messages. Lives on group topics only — never
    /// part of a profile's log set ([`DomainLogId::all_for_profile`]).
    Groups,
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
            Self::Groups => 6,
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
    ///
    /// A heart on a post in a **public + listed** Group is additionally
    /// published on the hearter's profile log with the group context set, so
    /// friends' rivers can surface a named discovery card pointing into the
    /// group (ADR-0009). Hearts in any other group stay in-group only.
    HeartChanged {
        profile_id: String,
        post_author_profile_id: String,
        post_id: String,
        active: bool,
        recorded_at: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group_name: Option<String>,
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
    /// The genesis op of a Group. The GroupId is this op's hash (ADR-0006),
    /// so it lives on a one-op log under a unique
    /// [`GROUP_GENESIS_CONTEXT_PREFIX`] context, associated with the group
    /// topic once the hash is known. Content mode is fixed here forever.
    GroupCreated {
        creator_profile_id: String,
        name: String,
        content_mode: GroupContentMode,
        join_mode: GroupJoinMode,
        #[serde(default)]
        discoverability: GroupDiscoverability,
        created_at: u64,
    },
    /// A governance action on a Group, authored by the member holding
    /// `Manage` at that point in the log (validated during reduction).
    GroupGoverned {
        group_id: String,
        actor_profile_id: String,
        action: GroupGovernanceAction,
        recorded_at: u64,
    },
    /// A join request, authored by the requester on the *group's* topic
    /// (the same foreign-author pattern as [`Self::FriendshipRequested`]).
    /// In Open join mode the Owner's node auto-accepts it; in Request mode
    /// it stays pending until the Owner answers (ADR-0005).
    GroupJoinRequested {
        group_id: String,
        requester_profile_id: String,
        requester_display_name: String,
        #[serde(default)]
        greeting: Option<String>,
        recorded_at: u64,
    },
    /// A member leaving, self-authored — effective immediately, no Owner
    /// liveness needed (ADR-0003).
    GroupLeft {
        group_id: String,
        member_profile_id: String,
        recorded_at: u64,
    },
    /// Membership advertisement (ADR-0008): a member disclosing their *own*
    /// membership edge ("I'm in G") to their *own* friends, riding the same
    /// friend-visible profile state that carries follow lists. Published for
    /// `listed` groups only; retracted (`active: false`) on leave, removal,
    /// or the group going `unlisted`. Distinct from roster visibility.
    GroupMembershipAdvertised {
        profile_id: String,
        group_id: String,
        group_name: String,
        active: bool,
        recorded_at: u64,
    },
}

impl DomainOperation {
    /// The profile whose topic carries this operation. For friendship
    /// requests this is the *target*, not the (requester) author. Group
    /// operations have no carrier profile — they live on group topics via
    /// [`JynOperationDomain::append_group_operation`].
    fn profile_id(&self) -> Option<&str> {
        match self {
            Self::ProfileUpdated { profile_id, .. }
            | Self::ContactFollowChanged { profile_id, .. }
            | Self::PostPublished { profile_id, .. }
            | Self::PostEdited { profile_id, .. }
            | Self::PostLifetimeChanged { profile_id, .. }
            | Self::PostDeleted { profile_id, .. }
            | Self::HeartChanged { profile_id, .. }
            | Self::CommentPublished { profile_id, .. }
            | Self::Spaces { profile_id, .. } => Some(profile_id),
            Self::FriendshipRequested {
                target_profile_id, ..
            } => Some(target_profile_id),
            Self::FriendshipResponded {
                target_profile_id, ..
            } => Some(target_profile_id),
            Self::GroupMembershipAdvertised { profile_id, .. } => Some(profile_id),
            Self::GroupCreated { .. }
            | Self::GroupGoverned { .. }
            | Self::GroupJoinRequested { .. }
            | Self::GroupLeft { .. } => None,
        }
    }

    fn log_kind(&self) -> DomainLogKind {
        match self {
            Self::ProfileUpdated { .. } => DomainLogKind::Profile,
            Self::PostPublished { .. }
            | Self::PostEdited { .. }
            | Self::PostLifetimeChanged { .. }
            | Self::PostDeleted { .. } => DomainLogKind::Posts,
            Self::ContactFollowChanged { .. }
            | Self::FriendshipResponded { .. }
            | Self::GroupMembershipAdvertised { .. } => DomainLogKind::Contacts,
            Self::HeartChanged { .. } | Self::CommentPublished { .. } => {
                DomainLogKind::Interactions
            }
            Self::FriendshipRequested { .. } => DomainLogKind::Requests,
            Self::Spaces { .. } => DomainLogKind::Spaces,
            Self::GroupCreated { .. }
            | Self::GroupGoverned { .. }
            | Self::GroupJoinRequested { .. }
            | Self::GroupLeft { .. } => DomainLogKind::Groups,
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

/// An active heart cast by this profile on someone's post. Group context is
/// set only for hearts on public + listed group posts (ADR-0009) — the data
/// a friend's river needs to build the discovery card into the group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartRef {
    pub post_author_profile_id: String,
    pub post_id: String,
    pub recorded_at: u64,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub group_name: Option<String>,
}

/// A group membership this profile advertises to its friends (ADR-0008).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvertisedGroup {
    pub group_id: String,
    pub group_name: String,
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
    /// Group memberships this profile advertises to its friends
    /// (`listed` groups only, ADR-0008).
    #[serde(default)]
    pub advertised_groups: Vec<AdvertisedGroup>,
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

        let profile_id = operation
            .profile_id()
            .context("group operations belong on a group log; use append_group_operation")?
            .to_owned();
        let log_id = DomainLogId::new(&profile_id, operation.log_kind());
        let topic = profile_sync_topic(&profile_id);
        // Chain ordering over *every* stored op's header, including ops whose
        // body this binary can't decode (a newer peer's variant). The
        // ordering timestamp lives in the header, so a header-only read keeps
        // "new ops sort after everything they respond to" intact across
        // version skew — which `operations_for_profile` (body-decoding, and
        // now skip-on-failure) would silently break.
        let previous_ordering = self
            .operations_for_profile_raw(&profile_id)
            .await?
            .into_iter()
            .map(|operation| operation.header.extensions.ordering_timestamp)
            .max();
        self.append_to_log(private_key, log_id, topic, previous_ordering, operation)
            .await
    }

    /// Appends an operation to the author's log for a group context and
    /// associates it with the group's replication topic — never with any
    /// profile topic, so group content stays exclusively in the group
    /// (ADR-0007).
    ///
    /// `context_id` is the GroupId — except for the genesis op, whose
    /// context is a fresh [`GROUP_GENESIS_CONTEXT_PREFIX`] nonce (the
    /// GroupId is the genesis op's own hash) and whose topic is derived from
    /// that hash after signing.
    pub async fn append_group_operation(
        &mut self,
        private_key: &SigningKey,
        context_id: &str,
        topic_group_id: Option<&str>,
        operation: DomainOperation,
    ) -> Result<Header<DomainExtensions>> {
        let log_id = DomainLogId::new(context_id, DomainLogKind::Groups);
        let is_genesis = matches!(operation, DomainOperation::GroupCreated { .. });
        anyhow::ensure!(
            is_genesis == topic_group_id.is_none(),
            "genesis ops derive their topic from their own hash; all other group ops name their group"
        );
        // Genesis-ness must be readable from the log context alone: remote
        // ingest routes on this prefix without decoding the body (see
        // `ingest_remote_operation`), so the two sides must agree.
        anyhow::ensure!(
            is_genesis == context_id.starts_with(GROUP_GENESIS_CONTEXT_PREFIX),
            "genesis ops must use a genesis-context log; group ops must use the GroupId context"
        );
        // Header-only read: chaining must survive ops whose body this binary
        // can't decode (see the note in `append_operation`).
        let previous_ordering = match topic_group_id {
            Some(group_id) => self
                .operations_for_group_raw(group_id)
                .await?
                .into_iter()
                .map(|operation| operation.header.extensions.ordering_timestamp)
                .max(),
            None => None,
        };
        // For the genesis the topic is unknowable pre-hash; pass a
        // placeholder derived after signing inside append_to_log via the
        // two-step below.
        match topic_group_id {
            Some(group_id) => {
                let topic = group_sync_topic(group_id);
                self.append_to_log(private_key, log_id, topic, previous_ordering, operation)
                    .await
            }
            None => {
                let header = self
                    .sign_header(private_key, &log_id, previous_ordering, &operation)
                    .await?;
                let group_id = header.hash().to_string();
                let topic = group_sync_topic(&group_id);
                self.ingest_signed(&header, &log_id, topic, &operation)
                    .await?;
                Ok(header)
            }
        }
    }

    async fn append_to_log(
        &mut self,
        private_key: &SigningKey,
        log_id: DomainLogId,
        topic: Topic,
        previous_ordering: Option<HybridTimestamp>,
        operation: DomainOperation,
    ) -> Result<Header<DomainExtensions>> {
        let header = self
            .sign_header(private_key, &log_id, previous_ordering, &operation)
            .await?;
        self.ingest_signed(&header, &log_id, topic, &operation)
            .await?;
        Ok(header)
    }

    async fn sign_header(
        &self,
        private_key: &SigningKey,
        log_id: &DomainLogId,
        previous_ordering: Option<HybridTimestamp>,
        operation: &DomainOperation,
    ) -> Result<Header<DomainExtensions>> {
        let body_bytes = encode_cbor(operation).context("failed to encode domain body")?;
        let body = Body::from(body_bytes);
        let latest: Option<Operation<DomainExtensions>> = self
            .store
            .get_latest_entry(&private_key.verifying_key(), log_id)
            .await
            .map_err(|err| anyhow::anyhow!("failed to load latest domain operation: {err}"))?;
        let (seq_num, backlink) = latest
            .as_ref()
            .map(|operation| (operation.header.seq_num + 1, Some(operation.hash)))
            .unwrap_or((0, None));

        // Chain the ordering timestamp off the newest operation known for this context (from any
        // author and device) so new operations always sort after everything they were created in
        // response to, even when wall clocks are skewed or frozen.
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
        Ok(header)
    }

    async fn ingest_signed(
        &mut self,
        header: &Header<DomainExtensions>,
        log_id: &DomainLogId,
        topic: Topic,
        operation: &DomainOperation,
    ) -> Result<()> {
        let body_bytes = encode_cbor(operation).context("failed to encode domain body")?;
        let operation = Operation {
            hash: header.hash(),
            header: header.clone(),
            body: Some(Body::from(body_bytes)),
        };
        ingest_operation(&self.store, &operation, log_id, &topic, false)
            .await
            .map_err(|err| anyhow::anyhow!("failed to ingest domain operation: {err}"))?;
        Ok(())
    }

    pub async fn ingest_remote_operation(
        &mut self,
        operation: Operation<DomainExtensions>,
    ) -> Result<()> {
        let log_id = operation.header.extensions.log_id.clone();
        let topic = if log_id.kind == DomainLogKind::Groups {
            // Group logs associate with the group topic. A genesis op names
            // its group by its own hash; every other group op's log context
            // *is* the GroupId. Genesis-ness is carried by the signed log
            // context (a `GROUP_GENESIS_CONTEXT_PREFIX` nonce), not the body —
            // so routing never depends on decoding a body this binary may not
            // understand (a newer client's genesis must still land on the
            // right topic; see `append_group_operation`).
            if log_id.profile_id.starts_with(GROUP_GENESIS_CONTEXT_PREFIX) {
                group_sync_topic(&operation.hash.to_string())
            } else {
                group_sync_topic(&log_id.profile_id)
            }
        } else {
            profile_sync_topic(&log_id.profile_id)
        };

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
        let mut hearts =
            HashMap::<(String, String), Option<(u64, Option<String>, Option<String>)>>::new();
        let mut comments = HashMap::<String, ReducedComment>::new();
        let mut requests = HashMap::<String, PendingFriendRequest>::new();
        let mut responded = HashSet::<String>::new();
        let mut advertised = HashMap::<String, Option<AdvertisedGroup>>::new();

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
                    group_id,
                    group_name,
                    ..
                } => {
                    hearts.insert(
                        (post_author_profile_id, post_id),
                        active.then_some((recorded_at, group_id, group_name)),
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
                DomainOperation::GroupMembershipAdvertised {
                    group_id,
                    group_name,
                    active,
                    recorded_at,
                    ..
                } => {
                    advertised.insert(
                        group_id.clone(),
                        active.then_some(AdvertisedGroup {
                            group_id,
                            group_name,
                            recorded_at,
                        }),
                    );
                }
                DomainOperation::GroupCreated { .. }
                | DomainOperation::GroupGoverned { .. }
                | DomainOperation::GroupJoinRequested { .. }
                | DomainOperation::GroupLeft { .. } => {
                    // Group operations live on group topics; one showing up
                    // in a profile reduction is a routing bug or a forgery.
                    warn!(
                        profile = %profile_id,
                        "ignoring group operation during profile reduction"
                    );
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
            .filter_map(|((post_author_profile_id, post_id), heart)| {
                heart.map(|(recorded_at, group_id, group_name)| HeartRef {
                    post_author_profile_id,
                    post_id,
                    recorded_at,
                    group_id,
                    group_name,
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

        let mut advertised_groups = advertised.into_values().flatten().collect::<Vec<_>>();
        advertised_groups.sort_by(|left, right| {
            left.recorded_at
                .cmp(&right.recorded_at)
                .then_with(|| left.group_id.cmp(&right.group_id))
        });

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
            advertised_groups,
        }))
    }

    pub async fn operations_for_profile(
        &self,
        profile_id: &str,
    ) -> Result<Vec<StoredDomainOperation>> {
        self.operations_for_topic(profile_sync_topic(profile_id))
            .await
    }

    /// All decoded operations on a group's topic: the genesis, governance,
    /// join requests, and every member's group posts and interactions.
    pub async fn operations_for_group(&self, group_id: &str) -> Result<Vec<StoredDomainOperation>> {
        self.operations_for_topic(group_sync_topic(group_id)).await
    }

    async fn operations_for_topic(&self, topic: Topic) -> Result<Vec<StoredDomainOperation>> {
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
                    let Some(body) = operation.body else {
                        warn!("skipping domain operation without a payload");
                        continue;
                    };
                    // An undecodable body is an operation from a newer op
                    // set (or a corrupt one); it must not poison the whole
                    // context's reduction.
                    let mut domain_operation =
                        match decode_cbor::<DomainOperation, _>(&body.to_bytes()[..]) {
                            Ok(operation) => operation,
                            Err(err) => {
                                warn!("skipping undecodable domain operation: {err}");
                                continue;
                            }
                        };
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
        self.operations_for_topic_raw(profile_sync_topic(profile_id))
            .await
    }

    /// Like [`Self::operations_for_profile_raw`], for a group's topic.
    pub async fn operations_for_group_raw(
        &self,
        group_id: &str,
    ) -> Result<Vec<Operation<DomainExtensions>>> {
        self.operations_for_topic_raw(group_sync_topic(group_id))
            .await
    }

    async fn operations_for_topic_raw(
        &self,
        topic: Topic,
    ) -> Result<Vec<Operation<DomainExtensions>>> {
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

pub(crate) fn sort_for_reduction(operations: &mut [StoredDomainOperation]) {
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

/// The replication topic of a Group, derived from its GroupId (ADR-0007).
pub fn group_sync_topic(group_id: &str) -> Topic {
    let mut bytes = Vec::with_capacity(GROUP_TOPIC_NAMESPACE.len() + group_id.len() + 1);
    bytes.extend_from_slice(GROUP_TOPIC_NAMESPACE);
    bytes.push(b'/');
    bytes.extend_from_slice(group_id.as_bytes());
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
            group_id: None,
            group_name: None,
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
                group_id: None,
                group_name: None,
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
    async fn membership_advertisements_reduce_to_the_active_set() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        let advertise = |group_id: &str, name: &str, active: bool, at: u64| {
            DomainOperation::GroupMembershipAdvertised {
                profile_id: profile_id.clone(),
                group_id: group_id.to_owned(),
                group_name: name.to_owned(),
                active,
                recorded_at: at,
            }
        };
        domain
            .append_operation(&key, advertise("g-1", "reading circle", true, 10))
            .await?;
        domain
            .append_operation(&key, advertise("g-2", "casting club", true, 11))
            .await?;
        // g-1 renamed: re-advertised under the new name.
        domain
            .append_operation(&key, advertise("g-1", "evening reading circle", true, 20))
            .await?;
        // g-2 left (or went unlisted): retracted.
        domain
            .append_operation(&key, advertise("g-2", "casting club", false, 21))
            .await?;

        let state = domain
            .read_profile_state(&profile_id)
            .await?
            .expect("state exists");
        assert_eq!(state.advertised_groups.len(), 1);
        assert_eq!(state.advertised_groups[0].group_id, "g-1");
        assert_eq!(
            state.advertised_groups[0].group_name,
            "evening reading circle"
        );
        Ok(())
    }

    #[tokio::test]
    async fn hearts_keep_their_group_context_through_reduction() -> Result<()> {
        let key = SigningKey::generate();
        let profile_id = key.verifying_key().to_string();
        let friend_id = SigningKey::generate().verifying_key().to_string();
        let mut domain = JynOperationDomain::new(SqliteStore::temporary().await);

        domain
            .append_operation(
                &key,
                DomainOperation::HeartChanged {
                    profile_id: profile_id.clone(),
                    post_author_profile_id: friend_id.clone(),
                    post_id: "group-post".into(),
                    active: true,
                    recorded_at: 10,
                    group_id: Some("g-1".into()),
                    group_name: Some("reading circle".into()),
                },
            )
            .await?;

        let state = domain
            .read_profile_state(&profile_id)
            .await?
            .expect("state exists");
        assert_eq!(state.hearts.len(), 1);
        assert_eq!(state.hearts[0].group_id.as_deref(), Some("g-1"));
        assert_eq!(
            state.hearts[0].group_name.as_deref(),
            Some("reading circle")
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

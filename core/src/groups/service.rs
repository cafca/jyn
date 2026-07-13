//! The Groups service: local bookkeeping for the groups this node knows,
//! the group-scoped domain operations, and the p2panda-auth layer beneath
//! them.
//!
//! Standalone from `JynSpaces` (ADR-0004): it runs its **own** spaces
//! `Manager` over the same store, with a forge that appends control messages
//! to *group* logs (topic = the group's, ADR-0007) instead of the profile's
//! `Spaces` log. The shared store means one key registry and one auth graph;
//! the shared operations lock keeps the two managers' persists serialized.
//!
//! The reduced domain state (`reduce::read_group_state`) is the source of
//! truth for roster and permissions; the p2panda-auth group is **reconciled**
//! to it by whichever node holds `Manage` — the same
//! derive-then-reconcile pattern the friends/circles spaces use. For
//! members-only groups the reconciled object is a full encrypted space
//! (ticket 03); for public groups it is an auth-only group.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use p2panda_auth::Access;
use p2panda_core::cbor::encode_cbor;
use p2panda_core::{Hash, Operation, SigningKey, VerifyingKey};
use p2panda_spaces::manager::Manager;
use p2panda_spaces::{Event, Forge, SpacesArgs, StrongRemoveResolver};
use p2panda_store::groups::GroupsStore;
use p2panda_store::spaces::SpacesMessage;
use p2panda_store::{SqliteStore, Transaction};
use tracing::{debug, warn};

use super::reduce::{
    read_group_state, GroupComment, GroupHeart, GroupJoinRequest, GroupMemberEntry,
    ReducedGroupState,
};
use super::{
    GroupContentMode, GroupDiscoverability, GroupGovernanceAction, GroupJoinMode, GroupPermission,
    GroupRole,
};
use crate::domain::{
    ensure_spaces_tables, DomainExtensions, DomainOperation, JynOperationDomain, ReducedPost,
    GROUP_GENESIS_CONTEXT_PREFIX,
};
use crate::spaces::{spaces_args_from_operation, JynSpacesStore, GLOBAL_GROUPS_CONTEXT_ID};

type GroupsManager = Manager<JynSpacesStore, GroupsForge, (), StrongRemoveResolver<()>>;
type GroupsMessage = SpacesMessage<SpacesArgs<()>>;
type AuthGroupState =
    p2panda_auth::group::GroupCrdtState<VerifyingKey, Hash, p2panda_spaces::AuthMessage<()>, ()>;

/// Control messages forged for group contexts, waiting to be pushed into the
/// right group's live gossip: `(group_id, operation)`. Also carries domain
/// operations the service appends directly (genesis, governance, requests).
pub type GroupsOutbox = Arc<Mutex<Vec<(String, Operation<DomainExtensions>)>>>;

/// How the local profile relates to a group, for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupViewerStatus {
    /// Holds `Manage` (and posts like any member).
    Owner,
    /// Holds `Write`.
    Member,
    /// Sent a join request that the Owner has not answered yet.
    Pending,
    NonMember,
}

/// A group's state as the *local viewer* may see it: the reduction filtered
/// by the visibility rules (roster follows Content mode; pending requests
/// are Owner-only; a members-only group shows non-members no content).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupView {
    pub group_id: String,
    pub name: String,
    pub content_mode: GroupContentMode,
    pub join_mode: GroupJoinMode,
    pub discoverability: GroupDiscoverability,
    pub created_at: u64,
    pub owner_profile_id: String,
    pub viewer_status: GroupViewerStatus,
    /// `None` when the roster is not visible to this viewer.
    pub member_count: Option<u32>,
    /// Empty when the roster is not visible to this viewer.
    pub members: Vec<GroupMemberEntry>,
    /// Owner-only (minus locally denied requests); empty for everyone else.
    pub pending_requests: Vec<GroupJoinRequest>,
    pub posts: Vec<ReducedPost>,
    pub comments: Vec<GroupComment>,
    pub hearts: Vec<GroupHeart>,
    pub latest_activity_at: u64,
    /// Whether the group has activity newer than the viewer's last visit —
    /// what earns it a river digest door (ADR-0010; membership required).
    pub has_new_activity: bool,
}

/// A group a friend advertises that the viewer has not joined — the Groups
/// hub's friend-based discovery (ADR-0008, ADR-0012).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupSuggestion {
    pub group_id: String,
    pub group_name: String,
    /// The friends advertising membership: provenance for the hub card and
    /// bootstrap peers for reaching the group's topic.
    pub via_friend_profile_ids: Vec<String>,
}

/// What processing a batch of group control messages changed.
#[derive(Debug, Default)]
pub struct GroupsIngestReport {
    /// Groups whose decrypted content or auth state may have changed.
    pub changed_groups: HashSet<String>,
    pub processed_any: bool,
}

struct PendingGroupsMessage {
    group_id: String,
    message: GroupsMessage,
    attempts: u32,
}

/// The forge for group control messages: wraps them into signed
/// `DomainOperation::Spaces` operations on the author's log *for that group*,
/// so they replicate via the group topic and reach members regardless of
/// friendship (ADR-0007).
struct GroupsForge {
    domain: JynOperationDomain,
    private_key: SigningKey,
    profile_id: String,
    store: SqliteStore,
    outbox: GroupsOutbox,
    /// The jyn group awaiting its auth-group creation: `create_group`
    /// generates a random auth id internally, so the forge learns the
    /// binding from this slot (set under the ops lock just before the call).
    pending_create: Arc<Mutex<Option<String>>>,
}

impl std::fmt::Debug for GroupsForge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupsForge")
            .field("profile_id", &self.profile_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct GroupsForgeError(String);

impl std::fmt::Display for GroupsForgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "groups forge error: {}", self.0)
    }
}

impl std::error::Error for GroupsForgeError {}

impl GroupsForge {
    /// The jyn group a control message belongs to, resolved from its args.
    async fn group_for_args(&self, args: &SpacesArgs<()>) -> Result<String, GroupsForgeError> {
        match args {
            SpacesArgs::Auth { group_id, .. } => {
                let auth_group_id = group_id.to_string();
                let known: Option<(String,)> =
                    sqlx::query_as("SELECT group_id FROM jyn_groups WHERE auth_group_id = ?")
                        .bind(&auth_group_id)
                        .fetch_optional(self.store.pool())
                        .await
                        .map_err(|err| GroupsForgeError(format!("registry lookup: {err}")))?;
                if let Some((group_id,)) = known {
                    return Ok(group_id);
                }
                // A create for an auth group we have never seen: it belongs
                // to the group whose creation is in flight.
                let pending = self
                    .pending_create
                    .lock()
                    .expect("groups pending-create lock poisoned")
                    .take();
                let Some(group_id) = pending else {
                    return Err(GroupsForgeError(format!(
                        "auth group {auth_group_id} is not bound to any group"
                    )));
                };
                sqlx::query("UPDATE jyn_groups SET auth_group_id = ? WHERE group_id = ?")
                    .bind(&auth_group_id)
                    .bind(&group_id)
                    .execute(self.store.pool())
                    .await
                    .map_err(|err| GroupsForgeError(format!("registry bind: {err}")))?;
                Ok(group_id)
            }
            SpacesArgs::SpaceMembership { space_id, .. }
            | SpacesArgs::SpaceUpdate { space_id, .. }
            | SpacesArgs::Application { space_id, .. } => {
                // Group spaces are always created with the GroupId as the
                // space id (ADR-0015).
                Ok(space_id.to_string())
            }
            SpacesArgs::KeyBundle { .. } => Err(GroupsForgeError(
                "key bundles belong to the profile spaces service".into(),
            )),
        }
    }
}

impl Forge<()> for GroupsForge {
    type Message = GroupsMessage;
    type Error = GroupsForgeError;

    fn verifying_key(&self) -> VerifyingKey {
        self.private_key.verifying_key()
    }

    async fn forge(&self, args: SpacesArgs<()>) -> Result<Self::Message, Self::Error> {
        let group_id = self.group_for_args(&args).await?;
        let args_bytes =
            encode_cbor(&args).map_err(|err| GroupsForgeError(format!("encode args: {err}")))?;
        let operation = DomainOperation::Spaces {
            profile_id: self.profile_id.clone(),
            args: args_bytes,
        };
        let body_bytes = encode_cbor(&operation)
            .map_err(|err| GroupsForgeError(format!("encode operation: {err}")))?;

        let mut domain = self.domain.clone();
        let header = domain
            .append_group_operation(&self.private_key, &group_id, Some(&group_id), operation)
            .await
            .map_err(|err| GroupsForgeError(format!("append operation: {err:#}")))?;

        let hash = header.hash();
        let author = header.verifying_key;
        self.outbox
            .lock()
            .expect("groups outbox lock poisoned")
            .push((
                group_id,
                Operation {
                    hash,
                    header,
                    body: Some(p2panda_core::Body::from(body_bytes)),
                },
            ));

        Ok(SpacesMessage {
            id: hash,
            author,
            args,
        })
    }
}

/// Creates the groups bookkeeping tables next to the spaces ones.
pub async fn ensure_groups_tables(store: &SqliteStore) -> Result<()> {
    for ddl in [
        "CREATE TABLE IF NOT EXISTS jyn_groups (
            group_id TEXT PRIMARY KEY,
            auth_group_id TEXT,
            kind TEXT
        )",
        "CREATE TABLE IF NOT EXISTS jyn_groups_denied (
            group_id TEXT NOT NULL,
            requester_profile_id TEXT NOT NULL,
            PRIMARY KEY (group_id, requester_profile_id)
        )",
        "CREATE TABLE IF NOT EXISTS jyn_groups_opened (
            group_id TEXT PRIMARY KEY,
            last_opened_at INTEGER NOT NULL
        )",
    ] {
        sqlx::query(ddl)
            .execute(store.pool())
            .await
            .context("failed to create jyn groups table")?;
    }
    Ok(())
}

/// The jyn groups service: one per app.
#[derive(Clone)]
pub struct JynGroups {
    manager: GroupsManager,
    store: JynSpacesStore,
    domain: JynOperationDomain,
    outbox: GroupsOutbox,
    local_profile_id: String,
    private_key: SigningKey,
    pending: Arc<Mutex<Vec<PendingGroupsMessage>>>,
    pending_create: Arc<Mutex<Option<String>>>,
    /// Shared with `JynSpaces`: both managers mutate the same auth graph
    /// row, so all operations across the two must serialize.
    ops_lock: Arc<tokio::sync::Mutex<()>>,
}

impl JynGroups {
    pub async fn new(
        store: SqliteStore,
        private_key: SigningKey,
        local_profile_id: String,
        ops_lock: Arc<tokio::sync::Mutex<()>>,
    ) -> Result<Self> {
        ensure_spaces_tables(&store).await?;
        ensure_groups_tables(&store).await?;

        let credentials = crate::spaces::credentials_for(&private_key)?;
        let domain = JynOperationDomain::new(store.clone());
        let spaces_store = JynSpacesStore::new(store.clone());
        let outbox: GroupsOutbox = Arc::new(Mutex::new(Vec::new()));
        let pending_create = Arc::new(Mutex::new(None));
        let forge = GroupsForge {
            domain: domain.clone(),
            private_key: private_key.clone(),
            profile_id: local_profile_id.clone(),
            store,
            outbox: outbox.clone(),
            pending_create: Arc::clone(&pending_create),
        };
        let manager = Manager::new(
            spaces_store.clone(),
            forge,
            credentials,
            p2panda_encryption::Rng::default(),
        )
        .map_err(|err| anyhow::anyhow!("failed to build groups manager: {err}"))?;

        Ok(Self {
            manager,
            store: spaces_store,
            domain,
            outbox,
            local_profile_id,
            private_key,
            pending: Arc::new(Mutex::new(Vec::new())),
            pending_create,
            ops_lock,
        })
    }

    pub fn local_profile_id(&self) -> &str {
        &self.local_profile_id
    }

    /// Operations forged or appended since the last drain, tagged with the
    /// group they belong to; the sync layer pushes them into that group's
    /// live gossip. They are already persisted and syncable regardless.
    pub fn drain_outbox(&self) -> Vec<(String, Operation<DomainExtensions>)> {
        std::mem::take(&mut self.outbox.lock().expect("groups outbox lock poisoned"))
    }

    /// Creates a Group: mints the GroupId from the genesis op, registers it,
    /// and creates the auth layer beneath it. The creator becomes the Owner
    /// (sole `Manage` holder) and first Member.
    pub async fn create_group(
        &self,
        name: &str,
        content_mode: GroupContentMode,
        join_mode: GroupJoinMode,
        discoverability: GroupDiscoverability,
        created_at: u64,
    ) -> Result<String> {
        anyhow::ensure!(!name.trim().is_empty(), "a group needs a name");
        let _guard = self.ops_lock.lock().await;

        // The genesis op mints the GroupId (its own hash), so it lives on a
        // one-op log under a unique context (see `append_group_operation`).
        let genesis_context = format!(
            "{GROUP_GENESIS_CONTEXT_PREFIX}{}",
            unique_suffix(&self.local_profile_id)
        );
        // Build the genesis op once: the same value is signed onto the log
        // and encoded for the live-gossip body, so the two can never diverge.
        let genesis_op = DomainOperation::GroupCreated {
            creator_profile_id: self.local_profile_id.clone(),
            name: name.trim().to_owned(),
            content_mode,
            join_mode,
            discoverability,
            created_at,
        };
        let mut domain = self.domain.clone();
        let header = domain
            .append_group_operation(
                &self.private_key,
                &genesis_context,
                None,
                genesis_op.clone(),
            )
            .await?;
        let group_id = header.hash().to_string();
        let genesis = Operation {
            hash: header.hash(),
            header: header.clone(),
            body: Some(p2panda_core::Body::from(encode_cbor(&genesis_op)?)),
        };
        self.register_group(&group_id, Some(content_mode)).await?;
        self.push_outbox(&group_id, genesis);

        match content_mode {
            GroupContentMode::Public => {
                // Auth-only group: membership and roles without encryption.
                *self
                    .pending_create
                    .lock()
                    .expect("groups pending-create lock poisoned") = Some(group_id.clone());
                let (groups_y, _auth_group_id, message) = self
                    .manager
                    .create_group(&[(self.private_key.verifying_key(), Access::manage())])
                    .await
                    .map_err(|err| anyhow::anyhow!("failed to create auth group: {err}"))?;
                self.persist_groups_state(&groups_y).await?;
                self.mark_processed(&message.id).await?;
            }
            GroupContentMode::MembersOnly => {
                // A full encrypted space per GroupId, driven by the same
                // Manager flow as the friends/circles spaces (ADR-0015).
                // The space's internal auth group gets a random id, bound
                // via the pending-create slot when the forge sees it.
                *self
                    .pending_create
                    .lock()
                    .expect("groups pending-create lock poisoned") = Some(group_id.clone());
                let space_id: Hash = group_id
                    .parse()
                    .map_err(|err| anyhow::anyhow!("group id is not a hash: {err}"))?;
                let (groups_y, space_y, messages) = self
                    .manager
                    .create_space(space_id, &[])
                    .await
                    .map_err(|err| anyhow::anyhow!("failed to create group space: {err}"))?;
                self.persist_states(Some(&groups_y), Some(space_y)).await?;
                for message in &messages {
                    self.mark_processed(&message.id).await?;
                }
            }
        }

        Ok(group_id)
    }

    /// Sends a join request onto the group's topic. Both join modes go
    /// through this: in Open mode the Owner's node auto-accepts (ADR-0005).
    pub async fn request_join(
        &self,
        group_id: &str,
        display_name: &str,
        greeting: Option<String>,
        recorded_at: u64,
    ) -> Result<()> {
        let _guard = self.ops_lock.lock().await;
        if let Some(state) = read_group_state(&self.domain, group_id).await? {
            anyhow::ensure!(
                !state.is_member(&self.local_profile_id),
                "already a member of this group"
            );
        }
        self.register_group(group_id, None).await?;
        self.append_group_op(
            group_id,
            DomainOperation::GroupJoinRequested {
                group_id: group_id.to_owned(),
                requester_profile_id: self.local_profile_id.clone(),
                requester_display_name: display_name.to_owned(),
                greeting,
                recorded_at,
            },
        )
        .await?;
        Ok(())
    }

    /// Applies a governance action as the local profile. Fails with a clear
    /// message when the local profile does not hold `Manage` — the check
    /// routes through the roles → permitted-actions function.
    pub async fn govern(
        &self,
        group_id: &str,
        action: GroupGovernanceAction,
        recorded_at: u64,
    ) -> Result<()> {
        let _guard = self.ops_lock.lock().await;
        let state = read_group_state(&self.domain, group_id)
            .await?
            .with_context(|| format!("unknown group {group_id}"))?;
        anyhow::ensure!(
            state.permits(&self.local_profile_id, GroupPermission::Manage),
            "only the group's owner can do this"
        );
        self.append_group_op(
            group_id,
            DomainOperation::GroupGoverned {
                group_id: group_id.to_owned(),
                actor_profile_id: self.local_profile_id.clone(),
                action,
                recorded_at,
            },
        )
        .await?;
        // Keep the crypto layer in step immediately where we can.
        if let Some(state) = read_group_state(&self.domain, group_id).await? {
            if let Err(err) = self.reconcile_group_crypto(group_id, &state, false).await {
                debug!("crypto reconcile after governance deferred: {err:#}");
            }
        }
        Ok(())
    }

    /// Transfers ownership: the `Manage` role moves to another Member
    /// (ADR-0003). The old Owner stays a plain Member until they leave.
    pub async fn transfer_ownership(
        &self,
        group_id: &str,
        to_profile_id: &str,
        recorded_at: u64,
    ) -> Result<()> {
        {
            let state = read_group_state(&self.domain, group_id)
                .await?
                .with_context(|| format!("unknown group {group_id}"))?;
            anyhow::ensure!(
                state.is_member(to_profile_id),
                "ownership can only be transferred to a member"
            );
        }
        self.govern(
            group_id,
            GroupGovernanceAction::SetMemberRoles {
                member_profile_id: to_profile_id.to_owned(),
                roles: vec![GroupRole::Owner, GroupRole::Member],
            },
            recorded_at,
        )
        .await?;
        // Demoting ourselves is authored while we still hold `Manage` in the
        // reduced state? No — the promote above already moved it; the demote
        // is validated against the state *at its point in the log*, where
        // both hold `Manage`, so it still counts.
        self.govern(
            group_id,
            GroupGovernanceAction::SetMemberRoles {
                member_profile_id: self.local_profile_id.clone(),
                roles: vec![GroupRole::Member],
            },
            recorded_at,
        )
        .await
    }

    /// Leaves a group. The `Manage` holder must transfer ownership first
    /// (ADR-0003); a sole-owner group goes dormant instead of exiting.
    pub async fn leave(&self, group_id: &str, recorded_at: u64) -> Result<()> {
        let _guard = self.ops_lock.lock().await;
        let state = read_group_state(&self.domain, group_id)
            .await?
            .with_context(|| format!("unknown group {group_id}"))?;
        anyhow::ensure!(
            state.is_member(&self.local_profile_id),
            "not a member of this group"
        );
        anyhow::ensure!(
            !state.permits(&self.local_profile_id, GroupPermission::Manage),
            "transfer ownership before leaving"
        );
        self.append_group_op(
            group_id,
            DomainOperation::GroupLeft {
                group_id: group_id.to_owned(),
                member_profile_id: self.local_profile_id.clone(),
                recorded_at,
            },
        )
        .await?;
        Ok(())
    }

    /// Publishes a member-authored operation (post, edit, delete, lifetime
    /// change, comment, heart) into the group. Posting rights are membership
    /// (`Write`); edits/deletes of own posts stay with the author.
    ///
    /// In a members-only group the operation is sealed to the group's space
    /// — with the spec's lazy re-key right before sealing, so members
    /// removed since the last post lose the new secret (ADR-0003).
    pub async fn publish_to_group(&self, group_id: &str, operation: DomainOperation) -> Result<()> {
        let _guard = self.ops_lock.lock().await;
        let state = read_group_state(&self.domain, group_id)
            .await?
            .with_context(|| format!("unknown group {group_id}"))?;
        let needs_membership = matches!(
            operation,
            DomainOperation::PostPublished { .. }
                | DomainOperation::CommentPublished { .. }
                | DomainOperation::HeartChanged { .. }
        );
        if needs_membership {
            anyhow::ensure!(
                state.permits(&self.local_profile_id, GroupPermission::Write),
                "only members can post into this group"
            );
        }
        match state.content_mode {
            GroupContentMode::Public => self.append_group_op(group_id, operation).await,
            GroupContentMode::MembersOnly => {
                self.encrypt_to_group(group_id, &state, &operation).await
            }
        }
    }

    /// Seals an operation to the group's space. Called with the ops lock
    /// held. The `Manage` holder re-keys stale members out first (lazy
    /// re-key); everyone else publishes with the current secret.
    async fn encrypt_to_group(
        &self,
        group_id: &str,
        state: &ReducedGroupState,
        inner: &DomainOperation,
    ) -> Result<()> {
        // Catch the space up where possible, but never fail the post over
        // it — a repair blocked on someone's key bundle must not gag the
        // group (sealing uses the current space state either way).
        if let Err(err) = self.repair_group_spaces().await {
            debug!(group_id, "pre-publish space repair deferred: {err:#}");
        }
        if state.permits(&self.local_profile_id, GroupPermission::Manage) {
            if let Err(err) = self.reconcile_group_crypto(group_id, state, true).await {
                debug!(group_id, "pre-publish space reconcile deferred: {err:#}");
            }
        }
        let space_id: Hash = group_id
            .parse()
            .map_err(|err| anyhow::anyhow!("group id is not a hash: {err}"))?;
        let space = self
            .manager
            .space(space_id)
            .await
            .map_err(|err| anyhow::anyhow!("failed to load group space: {err}"))?
            .context("not welcomed into this group's space yet")?;
        let plaintext = encode_cbor(inner).context("failed to encode inner operation")?;
        let (space_y, message) = space
            .publish(&plaintext)
            .await
            .map_err(|err| anyhow::anyhow!("failed to encrypt to group space: {err}"))?;
        self.persist_states(None, Some(space_y)).await?;

        // Our own payload is stored decrypted right away (we authored it).
        self.domain
            .store_decrypted_inner_operation(&message.id, inner)
            .await?;
        self.mark_processed(&message.id).await?;
        Ok(())
    }

    /// Emits catch-up membership pointers for group spaces whose local
    /// auth-graph copy trails — but only for groups whose space we govern;
    /// repairing a foreign space would publish welcome-less pointers that
    /// race the owner's originals (see `JynSpaces::repair_spaces`). Called
    /// with the ops lock held.
    async fn repair_group_spaces(&self) -> Result<()> {
        let in_need = self
            .manager
            .spaces_repair_required()
            .await
            .map_err(|err| anyhow::anyhow!("failed to check group spaces repair: {err}"))?;
        if in_need.is_empty() {
            return Ok(());
        }
        let mut ours = Vec::new();
        for space_id in in_need {
            let group_id = space_id.to_string();
            if self.auth_group_id(&group_id).await?.is_none()
                && !self.registered_groups().await?.contains(&group_id)
            {
                continue; // Not a group space (friends/circles are JynSpaces').
            }
            let Some(state) = read_group_state(&self.domain, &group_id).await? else {
                continue;
            };
            if state.permits(&self.local_profile_id, GroupPermission::Manage) {
                ours.push(space_id);
            }
        }
        if ours.is_empty() {
            return Ok(());
        }
        let results = match self.manager.repair_spaces(&ours).await {
            Ok(results) => results,
            Err(err) if err.to_string().contains("not ready yet") => {
                debug!("deferring group spaces repair: {err}");
                return Ok(());
            }
            Err(err) => return Err(anyhow::anyhow!("failed to repair group spaces: {err}")),
        };
        for (space_y, messages) in results {
            if messages.is_empty() {
                continue;
            }
            self.persist_states(None, Some(space_y)).await?;
            for message in &messages {
                self.mark_processed(&message.id).await?;
            }
        }
        Ok(())
    }

    /// Records a denied join request. Local-only by design: a declined
    /// request is never a public record (ADR-0002).
    pub async fn deny_request(&self, group_id: &str, requester_profile_id: &str) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO jyn_groups_denied (group_id, requester_profile_id) VALUES (?, ?)",
        )
        .bind(group_id)
        .bind(requester_profile_id)
        .execute(self.store.store().pool())
        .await
        .context("failed to record denied request")?;
        Ok(())
    }

    /// Marks the group as opened now, clearing its river digest door.
    pub async fn mark_opened(&self, group_id: &str, at: u64) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO jyn_groups_opened (group_id, last_opened_at) VALUES (?, ?)",
        )
        .bind(group_id)
        .bind(at as i64)
        .execute(self.store.store().pool())
        .await
        .context("failed to record group visit")?;
        Ok(())
    }

    /// Every group this node knows (created, joined, requested, or visited).
    pub async fn registered_groups(&self) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT group_id FROM jyn_groups")
            .fetch_all(self.store.store().pool())
            .await
            .context("failed to list groups")?;
        Ok(rows.into_iter().map(|(group_id,)| group_id).collect())
    }

    pub async fn register_group(
        &self,
        group_id: &str,
        kind: Option<GroupContentMode>,
    ) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO jyn_groups (group_id, kind) VALUES (?, ?)")
            .bind(group_id)
            .bind(kind.map(content_mode_str))
            .execute(self.store.store().pool())
            .await
            .context("failed to register group")?;
        Ok(())
    }

    /// Records a group's content mode the first time its genesis reduces
    /// (groups joined or visited register with an unknown `kind`). Lets
    /// [`Self::members_only_groups`] scope the blob-secret scan without
    /// reducing every group. Idempotent: once set, later calls no-op.
    pub async fn record_content_kind(&self, group_id: &str, mode: GroupContentMode) -> Result<()> {
        sqlx::query("UPDATE jyn_groups SET kind = ? WHERE group_id = ? AND kind IS NULL")
            .bind(content_mode_str(mode))
            .bind(group_id)
            .execute(self.store.store().pool())
            .await
            .context("failed to record group content kind")?;
        Ok(())
    }

    /// Registered groups known to be members-only. A public group never seals
    /// an attachment, and a group whose genesis hasn't reduced yet carries no
    /// readable posts, so both are safely excluded from the blob-secret scan.
    pub async fn members_only_groups(&self) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT group_id FROM jyn_groups WHERE kind = ?")
            .bind(content_mode_str(GroupContentMode::MembersOnly))
            .fetch_all(self.store.store().pool())
            .await
            .context("failed to list members-only groups")?;
        Ok(rows.into_iter().map(|(group_id,)| group_id).collect())
    }

    /// The group's state as the local profile may see it, or `None` while
    /// the genesis is unknown.
    pub async fn group_view(&self, group_id: &str) -> Result<Option<GroupView>> {
        let Some(state) = read_group_state(&self.domain, group_id).await? else {
            return Ok(None);
        };
        let denied = self.denied_requests(group_id).await?;
        let last_opened = self.last_opened(group_id).await?;
        Ok(Some(viewer_filtered(
            &state,
            &self.local_profile_id,
            &denied,
            last_opened,
        )))
    }

    /// The Owner-side duties for one group: auto-accept pending join
    /// requests in Open mode (ADR-0005) and reconcile the auth layer to the
    /// reduced roster. No-op on nodes that do not hold `Manage`. Returns
    /// whether anything changed.
    pub async fn process_owner_duties(&self, group_id: &str) -> Result<bool> {
        let _guard = self.ops_lock.lock().await;
        let Some(state) = read_group_state(&self.domain, group_id).await? else {
            return Ok(false);
        };
        if !state.permits(&self.local_profile_id, GroupPermission::Manage) {
            return Ok(false);
        }

        let mut changed = false;
        if state.join_mode == GroupJoinMode::Open {
            let denied = self.denied_requests(group_id).await?;
            for request in &state.pending_requests {
                if denied.contains(&request.requester_profile_id) {
                    continue;
                }
                self.append_group_op(
                    group_id,
                    DomainOperation::GroupGoverned {
                        group_id: group_id.to_owned(),
                        actor_profile_id: self.local_profile_id.clone(),
                        action: GroupGovernanceAction::AddMember {
                            member_profile_id: request.requester_profile_id.clone(),
                            roles: vec![GroupRole::Member],
                        },
                        recorded_at: crate::profile::now_unix_secs(),
                    },
                )
                .await?;
                changed = true;
            }
        }

        let state = if changed {
            read_group_state(&self.domain, group_id)
                .await?
                .unwrap_or(state)
        } else {
            state
        };
        if let Err(err) = self.reconcile_group_crypto(group_id, &state, false).await {
            debug!(group_id, "crypto reconcile deferred: {err:#}");
        }
        Ok(changed)
    }

    /// Feeds a (local or replicated) group-topic operation into the auth
    /// layer. No-op for operations that do not carry control messages.
    pub async fn ingest(
        &self,
        operation: &Operation<DomainExtensions>,
    ) -> Result<GroupsIngestReport> {
        let Some(args) = spaces_args_from_operation::<()>(operation) else {
            return Ok(GroupsIngestReport::default());
        };
        let group_id = operation.header.extensions.log_id.profile_id.clone();
        let _guard = self.ops_lock.lock().await;
        self.register_group(&group_id, None).await?;
        if let SpacesArgs::Auth {
            group_id: auth_group_id,
            ..
        } = &args
        {
            // First writer wins, like the space-owner table: the binding is
            // carried by the message's placement on this group's own log.
            sqlx::query(
                "UPDATE jyn_groups SET auth_group_id = ? WHERE group_id = ? AND auth_group_id IS NULL",
            )
            .bind(auth_group_id.to_string())
            .bind(&group_id)
            .execute(self.store.store().pool())
            .await
            .context("failed to bind auth group")?;
        }
        if self.is_processed(&operation.hash).await? {
            return Ok(GroupsIngestReport::default());
        }
        {
            let mut pending = self.pending.lock().expect("groups pending lock poisoned");
            if pending.iter().any(|p| p.message.id == operation.hash) {
                return Ok(GroupsIngestReport::default());
            }
            pending.push(PendingGroupsMessage {
                group_id,
                message: SpacesMessage {
                    id: operation.hash,
                    author: operation.header.verifying_key,
                    args,
                },
                attempts: 0,
            });
        }
        self.drain_pending_inner().await
    }

    /// Retries queued control messages whose dependencies are met.
    pub async fn drain_pending(&self) -> Result<GroupsIngestReport> {
        let _guard = self.ops_lock.lock().await;
        self.drain_pending_inner().await
    }

    /// Loads unprocessed control messages of the given groups into the queue
    /// and drains it. Called at startup — the pending queue is in-memory.
    pub async fn process_backlog(&self, group_ids: &[String]) -> Result<GroupsIngestReport> {
        let _guard = self.ops_lock.lock().await;
        for group_id in group_ids {
            let operations = self.domain.operations_for_group_raw(group_id).await?;
            for operation in operations {
                let Some(args) = spaces_args_from_operation::<()>(&operation) else {
                    continue;
                };
                if self.is_processed(&operation.hash).await? {
                    continue;
                }
                let mut pending = self.pending.lock().expect("groups pending lock poisoned");
                if pending.iter().all(|p| p.message.id != operation.hash) {
                    pending.push(PendingGroupsMessage {
                        group_id: group_id.clone(),
                        message: SpacesMessage {
                            id: operation.hash,
                            author: operation.header.verifying_key,
                            args,
                        },
                        attempts: 0,
                    });
                }
            }
        }
        self.drain_pending_inner().await
    }

    // ---- internals ----

    async fn append_group_op(&self, group_id: &str, operation: DomainOperation) -> Result<()> {
        let mut domain = self.domain.clone();
        let header = domain
            .append_group_operation(
                &self.private_key,
                group_id,
                Some(group_id),
                operation.clone(),
            )
            .await?;
        let body = p2panda_core::Body::from(encode_cbor(&operation)?);
        self.push_outbox(
            group_id,
            Operation {
                hash: header.hash(),
                header,
                body: Some(body),
            },
        );
        Ok(())
    }

    fn push_outbox(&self, group_id: &str, operation: Operation<DomainExtensions>) {
        self.outbox
            .lock()
            .expect("groups outbox lock poisoned")
            .push((group_id.to_owned(), operation));
    }

    /// Aligns the group's crypto layer with the reduced roster: the auth
    /// group for public groups, the encrypted space for members-only ones.
    /// Space removals re-key, so they only run with `remove_stale` — the
    /// lazy re-key right before the next sealed post (ADR-0003); adds are
    /// always applied (the welcome delivers the current secret).
    async fn reconcile_group_crypto(
        &self,
        group_id: &str,
        state: &ReducedGroupState,
        remove_stale: bool,
    ) -> Result<()> {
        match state.content_mode {
            GroupContentMode::Public => self.reconcile_group_auth(group_id, state).await,
            GroupContentMode::MembersOnly => {
                self.reconcile_group_space(group_id, state, remove_stale)
                    .await
            }
        }
    }

    /// The members-only mirror: space membership follows the reduced roster
    /// (the space carries both auth and encryption; `space.add` emits the
    /// welcome payload that delivers the group secret, ADR-0015).
    async fn reconcile_group_space(
        &self,
        group_id: &str,
        state: &ReducedGroupState,
        remove_stale: bool,
    ) -> Result<()> {
        let space_id: Hash = group_id
            .parse()
            .map_err(|err| anyhow::anyhow!("group id is not a hash: {err}"))?;
        let Some(space) = self
            .manager
            .space(space_id)
            .await
            .map_err(|err| anyhow::anyhow!("failed to load group space: {err}"))?
        else {
            return Ok(());
        };

        let me = self.private_key.verifying_key();
        let members = space
            .members()
            .await
            .map_err(|err| anyhow::anyhow!("failed to list group space members: {err}"))?;
        if !members
            .iter()
            .any(|(member, access)| *member == me && access.is_manage())
        {
            return Ok(());
        }

        let desired: HashMap<VerifyingKey, bool> = state
            .members
            .iter()
            .filter_map(|member| {
                member.profile_id.parse::<VerifyingKey>().ok().map(|actor| {
                    (
                        actor,
                        member.permissions().contains(&GroupPermission::Manage),
                    )
                })
            })
            .collect();
        let actual: HashMap<VerifyingKey, bool> = members
            .iter()
            .map(|(member, access)| (*member, access.is_manage()))
            .collect();

        for (actor, manage) in &desired {
            if *actor == me || actual.contains_key(actor) {
                continue;
            }
            // `space.add` forges (and durably publishes) its auth message
            // *before* building the welcome, so a missing key bundle would
            // leave a dangling half-add behind. Wait for the bundle instead;
            // the next reconcile retries once it arrived.
            if !self.has_key_bundle(actor).await? {
                debug!(group_id, member = %actor, "waiting for the joiner's key bundle");
                continue;
            }
            let access = if *manage {
                Access::manage()
            } else {
                Access::write()
            };
            match space.add(*actor, access).await {
                Ok((groups_y, space_y, auth_msg, space_msgs)) => {
                    self.persist_states(Some(&groups_y), Some(space_y)).await?;
                    self.mark_processed(&auth_msg.id).await?;
                    for space_msg in &space_msgs {
                        self.mark_processed(&space_msg.id).await?;
                    }
                    debug!(group_id, member = %actor, "welcomed member into group space");
                }
                // The auth graph already carries the add (e.g. from a repair
                // of an interrupted admission); the space state catches up
                // through repair, not another add.
                Err(err) if format!("{err:?}").contains("AlreadyAdded") => {
                    debug!(group_id, member = %actor, "member already in the auth graph");
                }
                Err(err) => debug!(group_id, "cannot welcome member yet: {err:?}"),
            }
        }

        if !remove_stale {
            return Ok(());
        }
        for actor in actual.keys() {
            if *actor == me || desired.contains_key(actor) {
                continue;
            }
            match space.remove(*actor).await {
                Ok((groups_y, space_y, auth_msg, space_msg)) => {
                    self.persist_states(Some(&groups_y), Some(space_y)).await?;
                    self.mark_processed(&auth_msg.id).await?;
                    self.mark_processed(&space_msg.id).await?;
                    debug!(group_id, member = %actor, "re-keyed ex-member out of group space");
                }
                Err(err) => warn!(group_id, "failed to remove ex-member from space: {err:?}"),
            }
        }
        Ok(())
    }

    /// The public-group mirror. Only acts when the local actor holds
    /// `Manage` in the *auth* group too (right after a transfer the mirror
    /// trails until the new Owner's node catches up — each side converges
    /// what it is allowed to). Never touches the local actor's own auth
    /// membership; the incoming `Manage` holder fixes it.
    async fn reconcile_group_auth(&self, group_id: &str, state: &ReducedGroupState) -> Result<()> {
        let Some(auth_group_id) = self.auth_group_id(group_id).await? else {
            return Ok(());
        };
        let auth_group_id: VerifyingKey = auth_group_id
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid stored auth group id"))?;
        let Some(group) = self
            .manager
            .group(auth_group_id)
            .await
            .map_err(|err| anyhow::anyhow!("failed to load auth group: {err}"))?
        else {
            return Ok(());
        };

        let me = self.private_key.verifying_key();
        let members = group
            .members()
            .await
            .map_err(|err| anyhow::anyhow!("failed to list auth group members: {err}"))?;
        if !members
            .iter()
            .any(|(member, access)| *member == me && access.is_manage())
        {
            return Ok(());
        }

        let desired: HashMap<VerifyingKey, bool> = state
            .members
            .iter()
            .filter_map(|member| {
                member.profile_id.parse::<VerifyingKey>().ok().map(|actor| {
                    (
                        actor,
                        member.permissions().contains(&GroupPermission::Manage),
                    )
                })
            })
            .collect();
        let actual: HashMap<VerifyingKey, bool> = members
            .iter()
            .map(|(member, access)| (*member, access.is_manage()))
            .collect();

        for (actor, manage) in &desired {
            if *actor == me {
                continue;
            }
            let access = if *manage {
                Access::manage()
            } else {
                Access::write()
            };
            match actual.get(actor) {
                None => match group.add(*actor, access).await {
                    Ok((groups_y, message)) => {
                        self.persist_groups_state(&groups_y).await?;
                        self.mark_processed(&message.id).await?;
                    }
                    Err(err) => debug!("cannot mirror member into auth group yet: {err:?}"),
                },
                Some(has_manage) if *has_manage != *manage => {
                    // Access changes (transfer) go remove-then-re-add: the
                    // pinned crate exposes no promote/demote on the group
                    // API, and the domain log stays the audit record.
                    match group.remove(*actor).await {
                        Ok((groups_y, message)) => {
                            self.persist_groups_state(&groups_y).await?;
                            self.mark_processed(&message.id).await?;
                            match group.add(*actor, access).await {
                                Ok((groups_y, message)) => {
                                    self.persist_groups_state(&groups_y).await?;
                                    self.mark_processed(&message.id).await?;
                                }
                                Err(err) => {
                                    warn!("failed to re-add member with new access: {err:?}")
                                }
                            }
                        }
                        Err(err) => debug!("cannot adjust auth access yet: {err:?}"),
                    }
                }
                Some(_) => {}
            }
        }
        for actor in actual.keys() {
            if *actor == me || desired.contains_key(actor) {
                continue;
            }
            match group.remove(*actor).await {
                Ok((groups_y, message)) => {
                    self.persist_groups_state(&groups_y).await?;
                    self.mark_processed(&message.id).await?;
                }
                Err(err) => debug!("cannot mirror removal into auth group yet: {err:?}"),
            }
        }
        Ok(())
    }

    async fn drain_pending_inner(&self) -> Result<GroupsIngestReport> {
        let mut report = GroupsIngestReport::default();

        loop {
            // Our own group spaces must be caught up with the shared auth
            // graph before membership messages process (see JynSpaces).
            if let Err(err) = self.repair_group_spaces().await {
                warn!("group spaces repair before processing failed: {err:#}");
            }
            let ready: Vec<(String, GroupsMessage)> = {
                let pending = self.pending.lock().expect("groups pending lock poisoned");
                pending
                    .iter()
                    .map(|entry| (entry.group_id.clone(), entry.message.clone()))
                    .collect()
            };
            if ready.is_empty() {
                break;
            }

            let mut progressed = false;
            for (group_id, message) in ready {
                if !self.dependencies_met(&message).await? {
                    continue;
                }
                match self.process_message(&group_id, &message, &mut report).await {
                    Ok(()) => {
                        progressed = true;
                        report.processed_any = true;
                        self.remove_pending(&message.id);
                    }
                    Err(err) if err.to_string().contains("already established") => {
                        debug!(message_id = %message.id, "group control message already applied");
                        self.mark_processed(&message.id).await?;
                        progressed = true;
                        self.remove_pending(&message.id);
                    }
                    Err(err) => {
                        debug!(message_id = %message.id, "group control message not processable yet: {err:#}");
                        let mut pending =
                            self.pending.lock().expect("groups pending lock poisoned");
                        if let Some(entry) = pending.iter_mut().find(|p| p.message.id == message.id)
                        {
                            entry.attempts += 1;
                            if entry.attempts > 32 {
                                warn!(message_id = %message.id, "parking group control message off-queue after repeated failures");
                            }
                        }
                        pending.retain(|p| p.attempts <= 32);
                    }
                }
            }

            if !progressed {
                break;
            }
        }

        Ok(report)
    }

    async fn process_message(
        &self,
        group_id: &str,
        message: &GroupsMessage,
        report: &mut GroupsIngestReport,
    ) -> Result<()> {
        let (groups_y, space_y, events) = self
            .manager
            .process(message)
            .await
            .map_err(|err| anyhow::anyhow!("groups manager process failed: {err}"))?;
        self.persist_states(groups_y.as_ref(), space_y).await?;
        self.mark_processed(&message.id).await?;
        report.changed_groups.insert(group_id.to_owned());

        for event in events {
            match event {
                Event::Application { space_id, data } => {
                    // A sealed group operation decrypted: store the inner op
                    // so `operations_for_group` substitutes it in reduction.
                    match p2panda_core::cbor::decode_cbor::<DomainOperation, _>(&data[..]) {
                        Ok(inner) if !matches!(inner, DomainOperation::Spaces { .. }) => {
                            self.domain
                                .store_decrypted_inner_operation(&message.id, &inner)
                                .await?;
                            report.changed_groups.insert(space_id.to_string());
                        }
                        Ok(_) => {
                            warn!("dropping nested spaces payload in group {group_id}");
                        }
                        Err(err) => {
                            warn!("failed to decode decrypted group payload: {err}");
                        }
                    }
                }
                Event::KeyBundle { author } => {
                    debug!(author = %author, "processed key bundle from a group topic");
                }
                Event::Group(_) | Event::Space(_) => {}
            }
        }
        Ok(())
    }

    async fn dependencies_met(&self, message: &GroupsMessage) -> Result<bool> {
        for dependency in message.args.dependencies() {
            if !self.is_processed(&dependency).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn remove_pending(&self, id: &Hash) {
        self.pending
            .lock()
            .expect("groups pending lock poisoned")
            .retain(|p| &p.message.id != id);
    }

    /// Whether a valid long-term key bundle for this actor is in the shared
    /// registry (fed by `JynSpaces` processing profile-topic bundles).
    async fn has_key_bundle(&self, actor: &VerifyingKey) -> Result<bool> {
        use p2panda_encryption::key_bundle::LongTermKeyBundle;
        use p2panda_encryption::key_registry::KeyRegistry;
        use p2panda_encryption::traits::PreKeyRegistry;
        use p2panda_store::key_registry::KeyRegistryStore;

        let Some(registry) = self
            .store
            .get_key_registry()
            .await
            .map_err(|err| anyhow::anyhow!("failed to read key registry: {err}"))?
        else {
            return Ok(false);
        };
        let (_, bundle): (_, Option<LongTermKeyBundle>) =
            <KeyRegistry<VerifyingKey> as PreKeyRegistry<VerifyingKey, LongTermKeyBundle>>::key_bundle(
                registry, actor,
            )
            .map_err(|err| anyhow::anyhow!("failed to look up key bundle: {err}"))?;
        Ok(bundle.is_some())
    }

    async fn auth_group_id(&self, group_id: &str) -> Result<Option<String>> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT auth_group_id FROM jyn_groups WHERE group_id = ?")
                .bind(group_id)
                .fetch_optional(self.store.store().pool())
                .await
                .context("failed to read auth group binding")?;
        Ok(row.and_then(|(auth_group_id,)| auth_group_id))
    }

    async fn denied_requests(&self, group_id: &str) -> Result<HashSet<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT requester_profile_id FROM jyn_groups_denied WHERE group_id = ?")
                .bind(group_id)
                .fetch_all(self.store.store().pool())
                .await
                .context("failed to read denied requests")?;
        Ok(rows.into_iter().map(|(requester,)| requester).collect())
    }

    async fn last_opened(&self, group_id: &str) -> Result<u64> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT last_opened_at FROM jyn_groups_opened WHERE group_id = ?")
                .bind(group_id)
                .fetch_optional(self.store.store().pool())
                .await
                .context("failed to read group visit")?;
        Ok(row.map(|(at,)| at as u64).unwrap_or(0))
    }

    async fn is_processed(&self, id: &Hash) -> Result<bool> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT op_hash FROM jyn_spaces_processed WHERE op_hash = ?")
                .bind(id.to_string())
                .fetch_optional(self.store.store().pool())
                .await
                .context("failed to read processed set")?;
        Ok(row.is_some())
    }

    async fn mark_processed(&self, id: &Hash) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO jyn_spaces_processed (op_hash) VALUES (?)")
            .bind(id.to_string())
            .execute(self.store.store().pool())
            .await
            .context("failed to mark message processed")?;
        Ok(())
    }

    async fn persist_groups_state(&self, groups_y: &AuthGroupState) -> Result<()> {
        self.persist_states(Some(groups_y), None).await
    }

    /// Persists manager state the way the upstream `_persisted` helpers do
    /// (mirrors `JynSpaces::persist_states`; duplication accepted, ADR-0004).
    async fn persist_states(
        &self,
        groups_y: Option<&AuthGroupState>,
        space_y: Option<p2panda_spaces::space::SpacesState<()>>,
    ) -> Result<()> {
        let permit = self
            .store
            .begin()
            .await
            .map_err(|err| anyhow::anyhow!("failed to begin transaction: {err}"))?;

        if let Some(groups_y) = groups_y {
            GroupsStore::set_groups_state_tx(
                &self.store,
                Hash::digest(GLOBAL_GROUPS_CONTEXT_ID),
                groups_y,
            )
            .await
            .map_err(|err| anyhow::anyhow!("failed to persist groups state: {err}"))?;
        }
        if let Some(space_y) = space_y {
            let space_id = space_y.space_id;
            let state: p2panda_spaces::SpacesStoreState<()> = space_y.into();
            p2panda_store::spaces::SpacesStore::set_space_state_tx(&self.store, &space_id, &state)
                .await
                .map_err(|err| anyhow::anyhow!("failed to persist space state: {err}"))?;
        }

        self.store
            .commit(permit)
            .await
            .map_err(|err| anyhow::anyhow!("failed to commit groups state: {err}"))?;
        Ok(())
    }
}

fn content_mode_str(mode: GroupContentMode) -> &'static str {
    match mode {
        GroupContentMode::Public => "public",
        GroupContentMode::MembersOnly => "members_only",
    }
}

/// Applies the visibility rules to a reduced group state for one viewer:
/// roster follows Content mode (ADR-0002), pending requests are Owner-only
/// (minus local denials), and a members-only group shows non-members
/// identity but no content.
pub fn viewer_filtered(
    state: &ReducedGroupState,
    viewer_id: &str,
    denied: &HashSet<String>,
    last_opened: u64,
) -> GroupView {
    let is_member = state.is_member(viewer_id);
    let is_owner = state.permits(viewer_id, GroupPermission::Manage);
    let viewer_status = if is_owner {
        GroupViewerStatus::Owner
    } else if is_member {
        GroupViewerStatus::Member
    } else if state.has_pending_request_from(viewer_id) {
        GroupViewerStatus::Pending
    } else {
        GroupViewerStatus::NonMember
    };

    let roster_visible = state.content_mode == GroupContentMode::Public || is_member;
    let content_visible = state.content_mode == GroupContentMode::Public || is_member;

    GroupView {
        group_id: state.group_id.clone(),
        name: state.name.clone(),
        content_mode: state.content_mode,
        join_mode: state.join_mode,
        discoverability: state.discoverability,
        created_at: state.created_at,
        owner_profile_id: state
            .owner()
            .map(|owner| owner.profile_id.clone())
            .unwrap_or_default(),
        viewer_status,
        member_count: roster_visible.then_some(state.members.len() as u32),
        members: if roster_visible {
            state.members.clone()
        } else {
            Vec::new()
        },
        pending_requests: if is_owner {
            state
                .pending_requests
                .iter()
                .filter(|request| !denied.contains(&request.requester_profile_id))
                .cloned()
                .collect()
        } else {
            Vec::new()
        },
        posts: if content_visible {
            state.posts.clone()
        } else {
            Vec::new()
        },
        comments: if content_visible {
            state.comments.clone()
        } else {
            Vec::new()
        },
        hearts: if content_visible {
            state.hearts.clone()
        } else {
            Vec::new()
        },
        latest_activity_at: state.latest_activity_at,
        has_new_activity: is_member && state.latest_activity_at > last_opened,
    }
}

/// A unique log-context suffix for genesis ops: hash of author, wall clock,
/// and a process-wide counter.
fn unique_suffix(profile_id: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let seed = format!("{profile_id}/{nanos}/{count}");
    Hash::digest(seed.as_bytes()).to_string()
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use p2panda_core::SigningKey;
    use p2panda_store::SqliteStore;

    use super::*;

    async fn service(store: SqliteStore, key: &SigningKey) -> Result<JynGroups> {
        JynGroups::new(
            store,
            key.clone(),
            key.verifying_key().to_string(),
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .await
    }

    #[tokio::test]
    async fn create_group_mints_id_registers_and_builds_the_auth_group() -> Result<()> {
        let key = SigningKey::generate();
        let owner_id = key.verifying_key().to_string();
        let store = SqliteStore::temporary().await;
        let groups = service(store, &key).await?;

        let group_id = groups
            .create_group(
                "reading circle",
                GroupContentMode::Public,
                GroupJoinMode::Open,
                GroupDiscoverability::Listed,
                10,
            )
            .await?;

        // Registered and viewable, with the creator as Owner.
        assert_eq!(groups.registered_groups().await?, vec![group_id.clone()]);
        let view = groups.group_view(&group_id).await?.expect("view exists");
        assert_eq!(view.name, "reading circle");
        assert_eq!(view.viewer_status, GroupViewerStatus::Owner);
        assert_eq!(view.owner_profile_id, owner_id);
        assert_eq!(view.member_count, Some(1));

        // The auth layer exists: a p2panda-auth group bound to the GroupId,
        // with the creator as sole Manage member.
        let auth_group_id = groups
            .auth_group_id(&group_id)
            .await?
            .expect("auth group bound");
        let auth_group = groups
            .manager
            .group(auth_group_id.parse::<VerifyingKey>().unwrap())
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?
            .expect("auth group exists");
        let members = auth_group
            .members()
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].0, key.verifying_key());
        assert!(members[0].1.is_manage());

        // Genesis + auth create both wait in the outbox for live publish.
        let outbox = groups.drain_outbox();
        assert_eq!(outbox.len(), 2);
        assert!(outbox.iter().all(|(id, _)| id == &group_id));
        Ok(())
    }

    #[tokio::test]
    async fn open_join_is_auto_accepted_and_mirrored_into_the_auth_group() -> Result<()> {
        let owner_key = SigningKey::generate();
        let store = SqliteStore::temporary().await;
        let groups = service(store, &owner_key).await?;
        let group_id = groups
            .create_group(
                "open door",
                GroupContentMode::Public,
                GroupJoinMode::Open,
                GroupDiscoverability::Listed,
                10,
            )
            .await?;

        // A joiner's request lands on the group topic (simulated by writing
        // into the same store with their key).
        let joiner_key = SigningKey::generate();
        let joiner_id = joiner_key.verifying_key().to_string();
        let mut domain = groups.domain.clone();
        domain
            .append_group_operation(
                &joiner_key,
                &group_id,
                Some(&group_id),
                DomainOperation::GroupJoinRequested {
                    group_id: group_id.clone(),
                    requester_profile_id: joiner_id.clone(),
                    requester_display_name: "Wen Li".into(),
                    greeting: None,
                    recorded_at: 20,
                },
            )
            .await?;

        let changed = groups.process_owner_duties(&group_id).await?;
        assert!(changed, "the owner's node admits the open-mode joiner");

        let view = groups.group_view(&group_id).await?.expect("view exists");
        assert!(view.members.iter().any(|m| m.profile_id == joiner_id));
        assert!(view.pending_requests.is_empty());

        // And the auth mirror followed.
        let auth_group_id = groups.auth_group_id(&group_id).await?.unwrap();
        let auth_group = groups
            .manager
            .group(auth_group_id.parse::<VerifyingKey>().unwrap())
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?
            .unwrap();
        let members = auth_group
            .members()
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        assert!(members
            .iter()
            .any(|(member, access)| *member == joiner_key.verifying_key() && access.is_write()));
        Ok(())
    }

    #[tokio::test]
    async fn request_mode_leaves_the_request_pending_and_denial_is_local() -> Result<()> {
        let owner_key = SigningKey::generate();
        let store = SqliteStore::temporary().await;
        let groups = service(store, &owner_key).await?;
        let group_id = groups
            .create_group(
                "ask first",
                GroupContentMode::Public,
                GroupJoinMode::Request,
                GroupDiscoverability::Listed,
                10,
            )
            .await?;

        let joiner_key = SigningKey::generate();
        let joiner_id = joiner_key.verifying_key().to_string();
        let mut domain = groups.domain.clone();
        domain
            .append_group_operation(
                &joiner_key,
                &group_id,
                Some(&group_id),
                DomainOperation::GroupJoinRequested {
                    group_id: group_id.clone(),
                    requester_profile_id: joiner_id.clone(),
                    requester_display_name: "Wen Li".into(),
                    greeting: None,
                    recorded_at: 20,
                },
            )
            .await?;

        groups.process_owner_duties(&group_id).await?;
        let view = groups.group_view(&group_id).await?.unwrap();
        assert!(!view.members.iter().any(|m| m.profile_id == joiner_id));
        assert_eq!(view.pending_requests.len(), 1);

        // A denial hides the request from the owner without any public op.
        groups.deny_request(&group_id, &joiner_id).await?;
        let view = groups.group_view(&group_id).await?.unwrap();
        assert!(view.pending_requests.is_empty());
        let ops = groups.domain.operations_for_group(&group_id).await?;
        assert!(
            !ops.iter()
                .any(|op| matches!(&op.operation, DomainOperation::GroupGoverned { .. })),
            "denial must not put any governance op on the wire"
        );
        Ok(())
    }

    #[tokio::test]
    async fn governance_gates_and_owner_leave_are_enforced_at_the_service() -> Result<()> {
        let owner_key = SigningKey::generate();
        let store = SqliteStore::temporary().await;
        let groups = service(store, &owner_key).await?;
        let group_id = groups
            .create_group(
                "gated",
                GroupContentMode::Public,
                GroupJoinMode::Open,
                GroupDiscoverability::Listed,
                10,
            )
            .await?;

        // The owner cannot leave without transferring first.
        let err = groups.leave(&group_id, 20).await.unwrap_err();
        assert!(err.to_string().contains("transfer ownership"));

        // Ownership can only go to a member.
        let stranger = SigningKey::generate().verifying_key().to_string();
        let err = groups
            .transfer_ownership(&group_id, &stranger, 21)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("member"));
        Ok(())
    }

    #[tokio::test]
    async fn viewer_filter_hides_members_only_content_and_roster_from_non_members() {
        let state = ReducedGroupState {
            group_id: "g".into(),
            creator_profile_id: "owner".into(),
            name: "sealed".into(),
            content_mode: GroupContentMode::MembersOnly,
            join_mode: GroupJoinMode::Request,
            discoverability: GroupDiscoverability::Unlisted,
            created_at: 10,
            members: vec![GroupMemberEntry {
                profile_id: "owner".into(),
                roles: vec![GroupRole::Owner, GroupRole::Member],
                since: 10,
            }],
            pending_requests: vec![GroupJoinRequest {
                requester_profile_id: "asker".into(),
                requester_display_name: "Asker".into(),
                greeting: None,
                recorded_at: 20,
            }],
            membership_history: Vec::new(),
            posts: vec![ReducedPost {
                profile_id: "owner".into(),
                post_id: "p1".into(),
                body: "secret".into(),
                media: Vec::new(),
                visibility: crate::domain::Visibility::Public,
                expires_at: None,
                created_at: 15,
                edited: false,
            }],
            comments: Vec::new(),
            hearts: Vec::new(),
            tombstoned_post_ids: Vec::new(),
            latest_activity_at: 15,
        };
        let denied = HashSet::new();

        // A stranger sees identity but neither roster nor content, and no
        // pending requests.
        let stranger_view = viewer_filtered(&state, "stranger", &denied, 0);
        assert_eq!(stranger_view.viewer_status, GroupViewerStatus::NonMember);
        assert_eq!(stranger_view.name, "sealed");
        assert!(stranger_view.members.is_empty());
        assert_eq!(stranger_view.member_count, None);
        assert!(stranger_view.posts.is_empty());
        assert!(stranger_view.pending_requests.is_empty());
        assert!(!stranger_view.has_new_activity);

        // The requester sees their own pending state, nothing more.
        let asker_view = viewer_filtered(&state, "asker", &denied, 0);
        assert_eq!(asker_view.viewer_status, GroupViewerStatus::Pending);
        assert!(asker_view.pending_requests.is_empty());

        // The owner sees everything, including the pending request.
        let owner_view = viewer_filtered(&state, "owner", &denied, 0);
        assert_eq!(owner_view.viewer_status, GroupViewerStatus::Owner);
        assert_eq!(owner_view.member_count, Some(1));
        assert_eq!(owner_view.posts.len(), 1);
        assert_eq!(owner_view.pending_requests.len(), 1);
        assert!(owner_view.has_new_activity);
    }
}

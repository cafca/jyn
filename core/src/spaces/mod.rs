//! Group encryption for non-public posts, built on `p2panda-spaces`.
//!
//! Every profile owns a single-admin "friends space": the owner is the sole
//! `Manage` member, accepted friends get `Write` access (which also makes
//! them "secret members" of the encryption context, so they can read and
//! comment). Non-public posts, and comments/hearts on them, are CBOR-encoded
//! [`DomainOperation`]s encrypted to a space and carried as
//! [`DomainOperation::Spaces`] wrappers on the author's `Spaces` log.
//!
//! See `docs/2026-07-05-post-encryption-spec.md` for the agreed design.

mod forge;
mod store;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use p2panda_auth::Access;
use p2panda_core::cbor::{decode_cbor, encode_cbor};
use p2panda_core::{Hash, Operation, SigningKey};
use p2panda_encryption::crypto::hkdf::hkdf;
use p2panda_encryption::crypto::x25519::SecretKey;
use p2panda_encryption::Rng;
use p2panda_auth::group::GroupCrdtState;
use p2panda_core::VerifyingKey;
use p2panda_spaces::manager::Manager;
use p2panda_spaces::{
    Credentials, Event, SpaceId, SpacesArgs, SpacesStoreState, StrongRemoveResolver,
};
use p2panda_store::groups::GroupsStore;
use p2panda_store::spaces::{SpacesMessage, SpacesStore};
use p2panda_store::{SqliteStore, Transaction};
use tracing::{debug, warn};

use crate::domain::{
    ensure_spaces_tables, DomainExtensions, DomainOperation, JynOperationDomain,
};
pub use forge::SpacesOutbox;
use forge::JynForge;
pub use store::spaces_args_from_operation;
use store::JynSpacesStore;

/// Matches the private `GLOBAL_GROUPS_CONTEXT_ID` inside `p2panda-spaces`;
/// the auth CRDT state is stored under this key.
const GLOBAL_GROUPS_CONTEXT_ID: &[u8] = b"global-groups-context";

/// Builds an X25519 secret from raw bytes via its serde representation
/// (`SecretKey::from_bytes` is private in the pinned p2panda revision). The
/// bytes are clamped first, exactly like `from_bytes` would, so the stored
/// secret is identical to one built upstream. Determinism and equality are
/// covered by a unit test below.
fn secret_key_from_bytes(mut bytes: [u8; 32]) -> Result<SecretKey> {
    bytes[0] &= 248;
    bytes[31] &= 127;
    bytes[31] |= 64;
    let encoded = encode_cbor(&serde_bytes::Bytes::new(&bytes))
        .context("failed to encode identity secret")?;
    decode_cbor::<SecretKey, _>(&encoded[..]).context("failed to build identity secret")
}

type JynManager = Manager<JynSpacesStore, JynForge, (), StrongRemoveResolver<()>>;
type JynSpacesMessage = SpacesMessage<SpacesArgs<()>>;
type AuthGroupState = GroupCrdtState<VerifyingKey, Hash, p2panda_spaces::AuthMessage<()>, ()>;

/// What processing a batch of spaces messages changed.
#[derive(Debug, Default)]
pub struct IngestReport {
    /// Profiles whose reduced state may have changed (decrypted payloads).
    pub changed_profiles: HashSet<String>,
    /// Authors whose key bundles became known (candidates for member adds).
    pub new_key_bundles: HashSet<String>,
    /// Whether any message was processed at all.
    pub processed_any: bool,
}

struct PendingMessage {
    message: JynSpacesMessage,
    attempts: u32,
}

/// The jyn spaces service: one per app, owner of the friends space.
#[derive(Clone)]
pub struct JynSpaces {
    manager: JynManager,
    store: JynSpacesStore,
    domain: JynOperationDomain,
    outbox: SpacesOutbox,
    local_profile_id: String,
    my_space_id: SpaceId,
    pending: Arc<Mutex<Vec<PendingMessage>>>,
    /// Serializes all spaces operations. The manager's internal RwLock and
    /// the store's single sqlite connection interleave badly when several
    /// tasks (topic ingests, command handlers) drive the manager
    /// concurrently — one waiter can hold the manager lock while another
    /// holds the db connection. One operation at a time sidesteps that.
    ops_lock: Arc<tokio::sync::Mutex<()>>,
}

impl JynSpaces {
    /// Builds the spaces service from the domain store and the node identity.
    ///
    /// The X25519 key-agreement secret is derived deterministically from the
    /// ed25519 identity seed via HKDF, so backing up `node.key` recovers the
    /// full encryption identity. The friends-space id is likewise derived
    /// from the seed: opaque to observers, stable for the owner.
    pub async fn new(
        store: SqliteStore,
        private_key: SigningKey,
        local_profile_id: String,
    ) -> Result<Self> {
        ensure_spaces_tables(&store).await?;

        let seed = private_key.as_bytes();
        let identity_secret_bytes = hkdf::<32>(b"jyn/identity/x25519/v1", seed, None)
            .map_err(|err| anyhow::anyhow!("failed to derive identity secret: {err}"))?;
        let identity_secret = secret_key_from_bytes(identity_secret_bytes)?;
        let credentials = Credentials::from_keys(private_key.clone(), identity_secret);

        let space_seed = hkdf::<32>(b"jyn/friends-space/v1", seed, None)
            .map_err(|err| anyhow::anyhow!("failed to derive friends space id: {err}"))?;
        let my_space_id: SpaceId = Hash::digest(space_seed);

        let domain = JynOperationDomain::new(store.clone());
        let spaces_store = JynSpacesStore::new(store);
        let outbox: SpacesOutbox = Arc::new(Mutex::new(Vec::new()));
        let forge = JynForge::new(
            domain.clone(),
            private_key,
            local_profile_id.clone(),
            outbox.clone(),
        );
        let manager = Manager::new(spaces_store.clone(), forge, credentials, Rng::default())
            .map_err(|err| anyhow::anyhow!("failed to build spaces manager: {err}"))?;

        Ok(Self {
            manager,
            store: spaces_store,
            domain,
            outbox,
            local_profile_id,
            my_space_id,
            pending: Arc::new(Mutex::new(Vec::new())),
            ops_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    pub fn friends_space_id(&self) -> SpaceId {
        self.my_space_id
    }

    /// Operations forged since the last drain; the caller pushes them into
    /// live gossip. They are already persisted and syncable regardless.
    pub fn drain_outbox(&self) -> Vec<Operation<DomainExtensions>> {
        std::mem::take(&mut self.outbox.lock().expect("spaces outbox lock poisoned"))
    }

    /// Ensures our key bundle is published and the friends space exists.
    /// Idempotent; call at startup and after key rotation windows.
    pub async fn ensure_ready(&self) -> Result<()> {
        let _guard = self.ops_lock.lock().await;
        let has_space = SpacesStore::<SpacesStoreState<()>>::has_space(&self.store, &self.my_space_id)
            .await
            .map_err(|err| anyhow::anyhow!("failed to check friends space: {err}"))?;

        let bundle_missing = self.meta_get("key_bundle_published").await?.is_none();
        let bundle_expired = self
            .manager
            .key_bundle_expired()
            .await
            .map_err(|err| anyhow::anyhow!("failed to check key bundle: {err}"))?;
        if bundle_missing || bundle_expired {
            let message = self
                .manager
                .key_bundle_message()
                .await
                .map_err(|err| anyhow::anyhow!("failed to forge key bundle: {err}"))?;
            self.mark_processed(&message.id).await?;
            self.meta_set("key_bundle_published", "1").await?;
            debug!("published spaces key bundle");
        }

        if !has_space {
            let (groups_y, space_y, messages) = self
                .manager
                .create_space(self.my_space_id, &[])
                .await
                .map_err(|err| anyhow::anyhow!("failed to create friends space: {err}"))?;
            self.persist_states(Some(&groups_y), Some(space_y)).await?;
            // Our own control messages must count as processed: the manager
            // already applied them via the direct call, and friends' later
            // messages list them as dependencies.
            for message in &messages {
                self.mark_processed(&message.id).await?;
            }
            self.record_space_owner(&self.my_space_id, &self.local_profile_id)
                .await?;
            debug!("created friends space");
        }

        Ok(())
    }

    /// Aligns friends-space membership with the accepted-friends list.
    /// Friends whose key bundles we have not yet processed are skipped and
    /// picked up on a later reconcile (triggered by their bundle arriving).
    pub async fn reconcile_friends(&self, friend_ids: &[String]) -> Result<()> {
        let _guard = self.ops_lock.lock().await;
        // Our space's local auth-graph copy can trail the global graph when
        // friends' auth operations were processed first; mutating a stale
        // space corrupts (or panics) the resolver, so catch up first.
        self.repair_spaces().await?;
        let Some(space) = self
            .manager
            .space(self.my_space_id)
            .await
            .map_err(|err| anyhow::anyhow!("failed to load friends space: {err}"))?
        else {
            return Ok(());
        };

        let members = space
            .members()
            .await
            .map_err(|err| anyhow::anyhow!("failed to list space members: {err}"))?;
        let member_ids: HashSet<String> = members
            .iter()
            .map(|(member, _)| member.to_string())
            .collect();
        let desired: HashSet<String> = friend_ids.iter().cloned().collect();
        debug!(
            desired = desired.len(),
            members = member_ids.len(),
            "reconciling friends space"
        );

        for friend_id in desired.difference(&member_ids) {
            let Ok(actor) = friend_id.parse::<VerifyingKey>() else {
                warn!(friend_id, "skipping friend with unparsable profile id");
                continue;
            };
            match space.add(actor, Access::write()).await {
                Ok((groups_y, space_y, auth_msg, space_msg)) => {
                    self.persist_states(Some(&groups_y), Some(space_y)).await?;
                    self.mark_processed(&auth_msg.id).await?;
                    self.mark_processed(&space_msg.id).await?;
                    debug!(friend_id, "added friend to friends space");
                }
                Err(err) => {
                    // Most commonly: their key bundle has not arrived yet.
                    debug!(friend_id, "cannot add friend to space yet: {err:?}");
                }
            }
        }

        for member_id in member_ids.difference(&desired) {
            if member_id == &self.local_profile_id {
                continue;
            }
            let Ok(actor) = member_id.parse::<VerifyingKey>() else {
                continue;
            };
            match space.remove(actor).await {
                Ok((groups_y, space_y, auth_msg, space_msg)) => {
                    self.persist_states(Some(&groups_y), Some(space_y)).await?;
                    self.mark_processed(&auth_msg.id).await?;
                    self.mark_processed(&space_msg.id).await?;
                    debug!(member_id, "removed ex-friend from friends space");
                }
                Err(err) => {
                    warn!(member_id, "failed to remove member from space: {err}");
                }
            }
        }

        Ok(())
    }

    /// Encrypts a domain operation to our own friends space.
    pub async fn encrypt_local(&self, inner: &DomainOperation) -> Result<()> {
        let _guard = self.ops_lock.lock().await;
        self.repair_spaces().await?;
        self.encrypt_to_space(self.my_space_id, inner).await
    }

    /// Encrypts a domain operation to the friends space of another profile
    /// (used for comments/hearts on their encrypted posts). Fails if we have
    /// not been welcomed into their space.
    pub async fn encrypt_to_owner(
        &self,
        owner_profile_id: &str,
        inner: &DomainOperation,
    ) -> Result<()> {
        let _guard = self.ops_lock.lock().await;
        self.repair_spaces().await?;
        let space_id = self
            .space_of_owner(owner_profile_id)
            .await?
            .with_context(|| {
                format!("no known encryption space for profile {owner_profile_id}")
            })?;
        self.encrypt_to_space(space_id, inner).await
    }

    async fn encrypt_to_space(&self, space_id: SpaceId, inner: &DomainOperation) -> Result<()> {
        let space = self
            .manager
            .space(space_id)
            .await
            .map_err(|err| anyhow::anyhow!("failed to load space: {err}"))?
            .context("encryption space does not exist locally")?;
        let plaintext = encode_cbor(inner).context("failed to encode inner operation")?;
        let (space_y, message) = space
            .publish(&plaintext)
            .await
            .map_err(|err| anyhow::anyhow!("failed to encrypt to space: {err}"))?;
        self.persist_states(None, Some(space_y)).await?;

        // Our own payload is stored decrypted right away (we authored it) and
        // never needs to pass through the manager again.
        self.domain
            .store_decrypted_inner_operation(&message.id, inner)
            .await?;
        self.mark_processed(&message.id).await?;
        Ok(())
    }

    /// Feeds a (local or replicated) operation into spaces processing. No-op
    /// for operations that do not carry spaces messages.
    pub async fn ingest(&self, operation: &Operation<DomainExtensions>) -> Result<IngestReport> {
        let Some(args) = spaces_args_from_operation::<()>(operation) else {
            return Ok(IngestReport::default());
        };
        let _guard = self.ops_lock.lock().await;
        if self.is_processed(&operation.hash).await? {
            return Ok(IngestReport::default());
        }
        {
            let mut pending = self.pending.lock().expect("spaces pending lock poisoned");
            if pending.iter().any(|p| p.message.id == operation.hash) {
                return Ok(IngestReport::default());
            }
            pending.push(PendingMessage {
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

    /// Repeatedly processes queued messages whose dependencies are met until
    /// no further progress is possible.
    pub async fn drain_pending(&self) -> Result<IngestReport> {
        let _guard = self.ops_lock.lock().await;
        self.drain_pending_inner().await
    }

    /// Emits catch-up membership pointers for spaces whose local auth-graph
    /// copy trails the shared graph. Called with `ops_lock` held.
    async fn repair_spaces(&self) -> Result<()> {
        let in_need = self
            .manager
            .spaces_repair_required()
            .await
            .map_err(|err| anyhow::anyhow!("failed to check spaces repair: {err}"))?;
        if in_need.is_empty() {
            return Ok(());
        }
        let results = match self.manager.repair_spaces(&in_need).await {
            Ok(results) => results,
            // A space can show up as trailing before we have processed the
            // group create that establishes it; repair simply runs again
            // after the next processing round.
            Err(err) if err.to_string().contains("not ready yet") => {
                debug!("deferring spaces repair: {err}");
                return Ok(());
            }
            Err(err) => return Err(anyhow::anyhow!("failed to repair spaces: {err}")),
        };
        for (space_y, messages) in results {
            self.persist_states(None, Some(space_y)).await?;
            for message in &messages {
                self.mark_processed(&message.id).await?;
            }
        }
        debug!(spaces = in_need.len(), "repaired trailing spaces");
        Ok(())
    }

    async fn drain_pending_inner(&self) -> Result<IngestReport> {
        let mut report = IngestReport::default();

        loop {
            let ready: Vec<JynSpacesMessage> = {
                let mut candidates = Vec::new();
                let pending = self.pending.lock().expect("spaces pending lock poisoned");
                for entry in pending.iter() {
                    candidates.push(entry.message.clone());
                }
                candidates
            };
            if ready.is_empty() {
                break;
            }

            let mut progressed = false;
            for message in ready {
                if !self.dependencies_met(&message).await? {
                    continue;
                }
                match self.process_message(&message, &mut report).await {
                    Ok(()) => {
                        progressed = true;
                        report.processed_any = true;
                        self.remove_pending(&message.id);
                    }
                    // The state this message would create already exists
                    // (e.g. a repair pointer overlapping the original
                    // membership message). Applied is applied: mark it
                    // processed so dependants unblock, and move on.
                    Err(err) if err.to_string().contains("already established") => {
                        debug!(message_id = %message.id, "spaces message already applied");
                        self.mark_processed(&message.id).await?;
                        progressed = true;
                        self.remove_pending(&message.id);
                    }
                    Err(err) => {
                        debug!(message_id = %message.id, "spaces message not processable yet: {err:#}");
                        let mut pending =
                            self.pending.lock().expect("spaces pending lock poisoned");
                        if let Some(entry) =
                            pending.iter_mut().find(|p| p.message.id == message.id)
                        {
                            entry.attempts += 1;
                            // Keep parked messages bounded: an undecryptable
                            // payload (e.g. a space we never get welcomed to)
                            // is dropped from memory; a later sync or restart
                            // re-parks it since it is never marked processed.
                            if entry.attempts > 32 {
                                warn!(message_id = %message.id, "parking spaces message off-queue after repeated failures");
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

        if report.processed_any {
            // Friends' auth operations may have moved the shared graph past
            // our spaces' local copies; catch up while we hold the lock.
            if let Err(err) = self.repair_spaces().await {
                warn!("spaces repair after processing failed: {err:#}");
            }
        }

        Ok(report)
    }

    async fn process_message(
        &self,
        message: &JynSpacesMessage,
        report: &mut IngestReport,
    ) -> Result<()> {
        let (groups_y, space_y, events) = self
            .manager
            .process(message)
            .await
            .map_err(|err| anyhow::anyhow!("manager process failed: {err}"))?;
        self.persist_states(groups_y.as_ref(), space_y).await?;
        self.mark_processed(&message.id).await?;

        let author_profile_id = message.author.to_string();
        if let SpacesArgs::SpaceMembership { space_id, .. } = &message.args {
            // Single-admin spaces: membership messages are always authored by
            // the space owner, which is how members learn whose space it is.
            self.record_space_owner(space_id, &author_profile_id).await?;
        }

        for event in events {
            match event {
                Event::Application { data, .. } => {
                    match decode_cbor::<DomainOperation, _>(&data[..]) {
                        Ok(inner) if !matches!(inner, DomainOperation::Spaces { .. }) => {
                            self.domain
                                .store_decrypted_inner_operation(&message.id, &inner)
                                .await?;
                            report.changed_profiles.insert(author_profile_id.clone());
                        }
                        Ok(_) => {
                            warn!("dropping nested spaces payload from {author_profile_id}");
                        }
                        Err(err) => {
                            warn!("failed to decode decrypted payload: {err}");
                        }
                    }
                }
                Event::KeyBundle { author } => {
                    debug!(author = %author, "processed key bundle");
                    report.new_key_bundles.insert(author.to_string());
                }
                Event::Group(_) | Event::Space(_) => {}
            }
        }
        Ok(())
    }

    async fn dependencies_met(&self, message: &JynSpacesMessage) -> Result<bool> {
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
            .expect("spaces pending lock poisoned")
            .retain(|p| &p.message.id != id);
    }

    /// Loads every unprocessed spaces operation of the given profiles into
    /// the queue and drains it. Called at startup for the local profile and
    /// all synced contacts.
    pub async fn process_backlog(&self, profile_ids: &[String]) -> Result<IngestReport> {
        let _guard = self.ops_lock.lock().await;
        for profile_id in profile_ids {
            let operations = self.domain.operations_for_profile_raw(profile_id).await?;
            for operation in operations {
                let Some(args) = spaces_args_from_operation::<()>(&operation) else {
                    continue;
                };
                if self.is_processed(&operation.hash).await? {
                    continue;
                }
                let mut pending = self.pending.lock().expect("spaces pending lock poisoned");
                if pending.iter().all(|p| p.message.id != operation.hash) {
                    pending.push(PendingMessage {
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

    /// The encryption space owned by a profile, learned from processed
    /// membership messages (or our own, which is registered at creation).
    pub async fn space_of_owner(&self, owner_profile_id: &str) -> Result<Option<SpaceId>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT space_id FROM jyn_spaces_owner WHERE owner_profile_id = ?")
                .bind(owner_profile_id)
                .fetch_optional(self.store.store().pool())
                .await
                .context("failed to read space owner")?;
        row.map(|(space_id,)| {
            space_id
                .parse::<Hash>()
                .map_err(|err| anyhow::anyhow!("invalid stored space id: {err}"))
        })
        .transpose()
    }

    async fn record_space_owner(&self, space_id: &SpaceId, owner_profile_id: &str) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO jyn_spaces_owner (space_id, owner_profile_id) VALUES (?, ?)",
        )
        .bind(space_id.to_string())
        .bind(owner_profile_id)
        .execute(self.store.store().pool())
        .await
        .context("failed to record space owner")?;
        Ok(())
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

    async fn meta_get(&self, key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM jyn_spaces_meta WHERE key = ?")
                .bind(key)
                .fetch_optional(self.store.store().pool())
                .await
                .context("failed to read spaces meta")?;
        Ok(row.map(|(value,)| value))
    }

    async fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query("INSERT OR REPLACE INTO jyn_spaces_meta (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(self.store.store().pool())
            .await
            .context("failed to write spaces meta")?;
        Ok(())
    }

    /// Persists manager state the way the upstream `_persisted` helpers do
    /// (they are `test_utils`-gated, so we mirror them here).
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
            let state: SpacesStoreState<()> = space_y.into();
            SpacesStore::set_space_state_tx(&self.store, &space_id, &state)
                .await
                .map_err(|err| anyhow::anyhow!("failed to persist space state: {err}"))?;
        }

        self.store
            .commit(permit)
            .await
            .map_err(|err| anyhow::anyhow!("failed to commit spaces state: {err}"))?;
        Ok(())
    }
}

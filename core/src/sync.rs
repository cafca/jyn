//! Topic sync for jyn profiles over p2panda LogSync.
//!
//! Every profile has one topic; the local node joins its own topic (live
//! mode) and one topic per synced contact. All domain operations live in a
//! single persistent store (`domain.sqlite3`), so restarts recover the full
//! operation history without separate caches and LogSync catch-up can serve
//! everything we know.
//!
//! Replaces the file-sharing `profile_sync` module (in-memory store, profile
//! record migration and JSON cache files are gone).

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use flume::Sender;
use futures_util::StreamExt;
use p2panda_core::cbor::{decode_cbor, encode_cbor};
use p2panda_core::Topic;
use p2panda_core::{Body, Operation, SigningKey, VerifyingKey};
use p2panda_net::addrs::NodeInfo;
use p2panda_net::iroh_endpoint::{EndpointAddr, RelayUrl};
use p2panda_net::sync::{SyncHandle, SyncSubscription};
use p2panda_net::utils::from_verifying_key;
use p2panda_net::LogSync;
use p2panda_store::{SqliteStore, SqliteStoreBuilder};
use p2panda_sync::protocols::TopicLogSyncEvent;

use crate::bridge::NetworkEvent;
use crate::domain::{
    group_sync_topic, DomainExtensions, DomainLogId, DomainOperation, JynOperationDomain,
    JynTopicMap, ReducedProfileState, Visibility,
};
use crate::groups::{GroupDiscoverability, GroupSuggestion, JynGroups};
use crate::node::AppNode;
use crate::profile::load_private_key_from_data_dir;
use crate::spaces::{derive_circle_members, JynSpaces, SpaceKind};

const DOMAIN_DB_FILE: &str = "domain.sqlite3";
const CONTACT_BOOTSTRAP_SETTLE_MILLIS: u64 = 750;
const CONTACT_SYNC_RETRY_ATTEMPTS: usize = 6;
const CONTACT_SYNC_RETRY_INTERVAL_MILLIS: u64 = 1_000;

type DomainStore = SqliteStore;
type DomainSync = LogSync<DomainStore, DomainLogId, DomainExtensions>;
type DomainSyncHandle =
    SyncHandle<Operation<DomainExtensions>, TopicLogSyncEvent<DomainExtensions>>;
type DomainSyncSubscription = SyncSubscription<TopicLogSyncEvent<DomainExtensions>>;

struct ContactSync {
    handle: DomainSyncHandle,
    // Aborted by stop_contact_sync (unfriending, from the friendship milestone).
    #[allow(dead_code)]
    task: tokio::task::JoinHandle<()>,
}

struct GroupSync {
    // Lives for the process lifetime; the handle sits in the shared map so
    // topic tasks and command handlers can publish into the group's gossip.
    #[allow(dead_code)]
    task: tokio::task::JoinHandle<()>,
}

/// Live-publish handles per group topic, shared with the group topic tasks.
type GroupHandles =
    std::sync::Arc<std::sync::RwLock<HashMap<String, std::sync::Arc<DomainSyncHandle>>>>;

pub(crate) struct JynSyncService {
    store: DomainStore,
    topic_map: JynTopicMap,
    domain: JynOperationDomain,
    log_sync: DomainSync,
    local_handle: std::sync::Arc<DomainSyncHandle>,
    local_topic: Topic,
    address_book: p2panda_net::AddressBook,
    relay_url: Option<RelayUrl>,
    local_profile_id: String,
    local_private_key: SigningKey,
    contact_streams: HashMap<String, ContactSync>,
    event_tx: Sender<NetworkEvent>,
    spaces: JynSpaces,
    groups: JynGroups,
    group_streams: HashMap<String, GroupSync>,
    group_handles: GroupHandles,
    _local_task: tokio::task::JoinHandle<()>,
}

impl JynSyncService {
    pub(crate) async fn new(
        node: &AppNode,
        local_profile_id: impl Into<String>,
        event_tx: Sender<NetworkEvent>,
    ) -> Result<Self> {
        let local_profile_id = local_profile_id.into();
        let local_private_key = load_private_key_from_data_dir(&node.data_dir)?;
        let store = open_domain_store(&node.data_dir).await?;
        let topic_map = JynTopicMap::new(store.clone());
        let domain = JynOperationDomain::new(store.clone());

        let topic = topic_map
            .register_profile_author(&local_profile_id, local_private_key.verifying_key())
            .await;
        node.address_book
            .add_topic(node.node_id(), topic)
            .await
            .context("failed to register local profile sync topic")?;

        let spaces = JynSpaces::new(
            store.clone(),
            local_private_key.clone(),
            local_profile_id.clone(),
        )
        .await
        .context("failed to initialize group encryption")?;
        spaces
            .ensure_ready()
            .await
            .context("failed to prepare group encryption")?;

        let log_sync = LogSync::builder(store.clone(), node.endpoint.clone(), node.gossip.clone())
            .spawn()
            .await
            .context("failed to spawn LogSync for jyn sync")?;
        let local_handle = std::sync::Arc::new(
            log_sync
                .stream(topic, true)
                .await
                .context("failed to join local profile sync topic")?,
        );
        let local_subscription = local_handle
            .subscribe()
            .await
            .context("failed to subscribe to local profile sync topic")?;
        let local_task = spawn_topic_task(
            local_subscription,
            store.clone(),
            local_profile_id.clone(),
            event_tx.clone(),
            true,
            spaces.clone(),
            local_profile_id.clone(),
            local_handle.clone(),
        );

        let groups = JynGroups::new(
            store.clone(),
            local_private_key.clone(),
            local_profile_id.clone(),
            spaces.ops_lock(),
        )
        .await
        .context("failed to initialize groups service")?;

        let service = Self {
            store,
            topic_map,
            domain,
            log_sync,
            local_handle,
            local_topic: topic,
            address_book: node.address_book.clone(),
            relay_url: node.relay_url.clone(),
            local_profile_id,
            local_private_key,
            contact_streams: HashMap::new(),
            event_tx,
            spaces,
            groups,
            group_streams: HashMap::new(),
            group_handles: std::sync::Arc::new(std::sync::RwLock::new(HashMap::new())),
            _local_task: local_task,
        };
        // Flush any startup-forged spaces messages (key bundle, space
        // creation) into live sync and pick up unprocessed backlog.
        let _ = service
            .spaces
            .process_backlog(std::slice::from_ref(&service.local_profile_id))
            .await;
        service.flush_spaces_outbox();
        let _ = service.sync_local_profile_peers().await;
        Ok(service)
    }

    #[allow(dead_code)] // Used from the friendship milestone on.
    pub(crate) fn local_profile_id(&self) -> &str {
        &self.local_profile_id
    }

    /// Appends an operation to the local store and publishes it into live
    /// sync on its topic (the local topic, or a synced contact's topic for
    /// operations targeting foreign profiles, e.g. friendship requests).
    ///
    /// This is the plaintext path: non-public posts must go through
    /// [`Self::publish_encrypted`] instead, which this guards structurally.
    pub(crate) async fn publish(&mut self, operation: DomainOperation) -> Result<()> {
        if let DomainOperation::PostPublished { visibility, .. } = &operation {
            anyhow::ensure!(
                *visibility == Visibility::Public,
                "non-public posts must be encrypted; use publish_encrypted"
            );
        }
        let body =
            Body::from(encode_cbor(&operation).context("failed to encode domain operation body")?);
        let header = self
            .domain
            .append_operation(&self.local_private_key, operation)
            .await?;
        let target_profile_id = header.extensions.log_id.profile_id.clone();
        let operation = Operation {
            hash: header.hash(),
            header,
            body: Some(body),
        };

        if target_profile_id == self.local_profile_id {
            self.local_handle
                .publish(operation)
                .context("failed to publish operation to local topic live mode")?;
            // Reflect the new local state back to the UI immediately.
            self.emit_local_state().await;
        } else {
            let contact = self
                .contact_streams
                .get(&target_profile_id)
                .with_context(|| {
                    format!("no active sync for {target_profile_id}; cannot publish to their topic")
                })?;
            contact
                .handle
                .publish(operation)
                .context("failed to publish operation to contact topic live mode")?;
        }
        Ok(())
    }

    pub(crate) async fn read_profile_state(
        &self,
        profile_id: &str,
    ) -> Result<Option<ReducedProfileState>> {
        self.domain.read_profile_state(profile_id).await
    }

    /// Encrypts a domain operation to one of our own spaces and pushes the
    /// resulting wrapper operations into live sync.
    ///
    /// A Circles publish reconciles circles membership including removals
    /// right before sealing, inside the spaces lock — the spec's lazy
    /// re-key: dropped friends-of-friends only cost a re-key when there is
    /// actually something new to protect.
    pub(crate) async fn publish_encrypted(
        &mut self,
        operation: DomainOperation,
        kind: SpaceKind,
    ) -> Result<()> {
        self.spaces.encrypt_local(kind, &operation).await?;
        self.flush_spaces_outbox();
        self.emit_local_state().await;
        Ok(())
    }

    /// Encrypts a domain operation (comment/heart) to the friends or circles
    /// space of the given profile. Fails if we have not been welcomed there.
    pub(crate) async fn publish_encrypted_to_owner(
        &mut self,
        owner_profile_id: &str,
        operation: DomainOperation,
        kind: SpaceKind,
    ) -> Result<()> {
        self.spaces
            .encrypt_to_owner(owner_profile_id, kind, &operation)
            .await?;
        self.flush_spaces_outbox();
        self.emit_local_state().await;
        Ok(())
    }

    /// Aligns encryption-space membership with the current social graph:
    /// friends space with the accepted-friends list (removals re-key
    /// eagerly), circles space with friends ∪ friends-of-friends (additions
    /// only — removals wait for the next Circles post). Also joins the
    /// topics of new circle members, so their key bundles and posts flow.
    /// Idempotent and cheap when nothing changed.
    pub(crate) async fn reconcile_spaces(&mut self) -> Result<()> {
        // Membership itself is derived inside the spaces lock (a set
        // computed here could go stale while another task re-keys); the
        // lists below only decide which topics to join.
        self.spaces.reconcile_friends().await?;
        self.spaces.reconcile_circles(false).await?;
        self.flush_spaces_outbox();

        let friends = self
            .read_profile_state(&self.local_profile_id)
            .await?
            .map(|state| state.followed_profile_ids)
            .unwrap_or_default();
        let members = derive_circle_members(&self.domain, &self.local_profile_id).await?;
        for member in members {
            if friends.contains(&member) || self.contact_streams.contains_key(&member) {
                continue;
            }
            if let Err(err) = self.sync_contact_profile(&member).await {
                tracing::debug!("failed to start sync with circle member {member}: {err:#}");
            }
        }
        Ok(())
    }

    /// Loads unprocessed spaces operations of every known profile (own,
    /// friends, friends-of-friends) into the manager. Needed after a restart:
    /// the pending queue is in-memory, and already-stored operations are
    /// never redelivered by sync.
    pub(crate) async fn process_spaces_backlog(&mut self) -> Result<()> {
        let mut profiles = vec![self.local_profile_id.clone()];
        if let Some(own) = self.read_profile_state(&self.local_profile_id).await? {
            let members = derive_circle_members(&self.domain, &self.local_profile_id).await?;
            profiles.extend(own.followed_profile_ids);
            profiles.extend(members);
            profiles.sort();
            profiles.dedup();
        }
        let report = self.spaces.process_backlog(&profiles).await?;
        self.flush_spaces_outbox();
        for changed in &report.changed_profiles {
            let is_local = changed == &self.local_profile_id;
            emit_topic_state(&self.domain, changed, &self.event_tx, is_local).await;
        }
        if !report.new_key_bundles.is_empty() {
            self.reconcile_spaces().await?;
        }
        Ok(())
    }

    /// Snapshots the domain store (operations, group-encryption state, key
    /// secrets) into `dest` via `VACUUM INTO` — consistent while live.
    pub(crate) async fn snapshot_store_into(&self, dest: &Path) -> Result<()> {
        let dest = dest.to_string_lossy().replace('\'', "''");
        sqlx::query(&format!("VACUUM INTO '{dest}'"))
            .execute(self.store.pool())
            .await
            .context("failed to snapshot domain store")?;
        Ok(())
    }

    /// Pushes operations forged by the spaces manager (they are already
    /// persisted and syncable) into live gossip on the local topic.
    fn flush_spaces_outbox(&self) {
        for operation in self.spaces.drain_outbox() {
            if let Err(err) = self.local_handle.publish(operation) {
                tracing::warn!("failed to publish spaces operation to live sync: {err}");
            }
        }
    }

    async fn emit_local_state(&self) {
        match self.domain.read_profile_state(&self.local_profile_id).await {
            Ok(Some(state)) => {
                let _ = self
                    .event_tx
                    .send(NetworkEvent::LocalStateUpdated { state });
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!("failed to reduce local profile state: {err:#}");
            }
        }
    }

    /// Nudges any known peers of the local topic into a sync session (other
    /// devices, or friends catching up on our stream).
    pub(crate) async fn sync_local_profile_peers(&self) -> Result<usize> {
        let node_infos = self
            .address_book
            .node_infos_by_topics([self.local_topic])
            .await
            .context("failed to query local profile topic peers")?;
        let peer_ids = node_infos
            .into_iter()
            .map(|node_info| node_info.node_id)
            .filter(|node_id| *node_id != self.local_private_key.verifying_key())
            .collect::<Vec<_>>();
        if peer_ids.is_empty() {
            return Ok(0);
        }

        if self.relay_url.is_some() {
            tokio::time::sleep(Duration::from_millis(CONTACT_BOOTSTRAP_SETTLE_MILLIS)).await;
        }
        for peer_id in &peer_ids {
            self.topic_map
                .register_profile_author(&self.local_profile_id, *peer_id)
                .await;
            self.local_handle.initiate_session(*peer_id);
        }

        Ok(peer_ids.len())
    }

    /// Joins a contact's topic and starts (or re-nudges) sync with them.
    pub(crate) async fn sync_contact_profile(&mut self, profile_id: &str) -> Result<bool> {
        if profile_id == self.local_profile_id {
            return Ok(false);
        }

        let public_key = normalize_profile_id(profile_id)?;
        self.seed_contact_bootstrap(public_key).await?;

        let topic = self
            .topic_map
            .register_profile_author(profile_id, public_key)
            .await;
        self.address_book
            .add_topic(self.local_private_key.verifying_key(), topic)
            .await
            .with_context(|| {
                format!("failed to register local interest in profile topic for {profile_id}")
            })?;
        self.address_book
            .set_topics(public_key, [topic])
            .await
            .with_context(|| format!("failed to register profile sync topic for {profile_id}"))?;
        wait_for_topic_registration(&self.address_book, topic, public_key, profile_id).await?;

        let mut started = false;
        if !self.contact_streams.contains_key(profile_id) {
            let handle = self
                .log_sync
                .stream(topic, true)
                .await
                .with_context(|| format!("failed to join LogSync topic for {profile_id}"))?;
            let subscription = handle.subscribe().await.with_context(|| {
                format!("failed to subscribe to LogSync topic for {profile_id}")
            })?;
            let task = spawn_topic_task(
                subscription,
                self.store.clone(),
                profile_id.to_owned(),
                self.event_tx.clone(),
                false,
                self.spaces.clone(),
                self.local_profile_id.clone(),
                self.local_handle.clone(),
            );
            self.contact_streams
                .insert(profile_id.to_owned(), ContactSync { handle, task });
            started = true;
        }

        let sync = self
            .contact_streams
            .get(profile_id)
            .expect("contact sync task inserted");
        if started && self.relay_url.is_some() {
            tokio::time::sleep(Duration::from_millis(CONTACT_BOOTSTRAP_SETTLE_MILLIS)).await;
        }
        sync.handle.initiate_session(public_key);

        // Retry the first session until something authored by the contact
        // themselves arrives (our own operations on their topic — e.g. a
        // friendship request — don't count as having reached them).
        if !self.has_operations_from(profile_id).await? {
            for _ in 0..CONTACT_SYNC_RETRY_ATTEMPTS {
                if self.has_operations_from(profile_id).await? {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(CONTACT_SYNC_RETRY_INTERVAL_MILLIS)).await;
                sync.handle.initiate_session(public_key);
            }
        }

        Ok(started)
    }

    /// Whether any operation *authored by* this profile's owner is stored —
    /// the signal that we have actually heard from them at least once.
    pub(crate) async fn has_operations_from(&self, profile_id: &str) -> Result<bool> {
        Ok(self
            .domain
            .operations_for_profile(profile_id)
            .await?
            .iter()
            .any(|entry| entry.author.to_string() == profile_id))
    }

    /// Stops syncing a contact's topic (unfriend).
    pub(crate) fn stop_contact_sync(&mut self, profile_id: &str) -> bool {
        if let Some(sync) = self.contact_streams.remove(profile_id) {
            sync.task.abort();
            true
        } else {
            false
        }
    }

    /// The groups service, for command handlers.
    pub(crate) fn groups(&self) -> &JynGroups {
        &self.groups
    }

    /// Joins a group's replication topic (ADR-0007) and starts its topic
    /// task. Idempotent; re-nudges known peers when already joined.
    pub(crate) async fn join_group_topic(&mut self, group_id: &str) -> Result<()> {
        if !self.group_streams.contains_key(group_id) {
            let topic = group_sync_topic(group_id);
            self.address_book
                .add_topic(self.local_private_key.verifying_key(), topic)
                .await
                .with_context(|| format!("failed to register group topic for {group_id}"))?;
            let handle =
                self.log_sync.stream(topic, true).await.with_context(|| {
                    format!("failed to join LogSync topic for group {group_id}")
                })?;
            let subscription = handle.subscribe().await.with_context(|| {
                format!("failed to subscribe to LogSync topic for group {group_id}")
            })?;
            let handle = std::sync::Arc::new(handle);
            self.group_handles
                .write()
                .expect("group handles lock poisoned")
                .insert(group_id.to_owned(), std::sync::Arc::clone(&handle));
            let task = spawn_group_topic_task(
                subscription,
                self.store.clone(),
                group_id.to_owned(),
                self.event_tx.clone(),
                self.groups.clone(),
                std::sync::Arc::clone(&self.group_handles),
            );
            self.group_streams
                .insert(group_id.to_owned(), GroupSync { task });
        }
        self.initiate_group_sessions(group_id).await
    }

    /// Starts sync sessions with every peer known for the group's topic.
    async fn initiate_group_sessions(&self, group_id: &str) -> Result<()> {
        let topic = group_sync_topic(group_id);
        let handle = self
            .group_handles
            .read()
            .expect("group handles lock poisoned")
            .get(group_id)
            .cloned();
        let Some(handle) = handle else {
            return Ok(());
        };
        let node_infos = self
            .address_book
            .node_infos_by_topics([topic])
            .await
            .with_context(|| format!("failed to query peers of group {group_id}"))?;
        for node_info in node_infos {
            if node_info.node_id != self.local_private_key.verifying_key() {
                handle.initiate_session(node_info.node_id);
            }
        }
        Ok(())
    }

    /// Registers a group locally, seeds reach through the given profiles
    /// (e.g. the friend whose advertisement or heart pointed here, or the
    /// Owner), and joins its topic — membership not required: for public
    /// groups this is the visit-only read path (ADR-0010).
    pub(crate) async fn sync_group(
        &mut self,
        group_id: &str,
        via_profile_ids: &[String],
    ) -> Result<()> {
        self.groups.register_group(group_id, None).await?;
        let topic = group_sync_topic(group_id);
        let mut seeded = false;
        for via in via_profile_ids {
            let Ok(public_key) = via.parse::<VerifyingKey>() else {
                warn_invalid_profile_id(via);
                continue;
            };
            if public_key == self.local_private_key.verifying_key() {
                continue;
            }
            self.seed_bootstrap_with_relay(public_key, None).await?;
            self.address_book
                .add_topic(public_key, topic)
                .await
                .with_context(|| format!("failed to seed group topic peer {via}"))?;
            seeded = true;
        }
        self.join_group_topic(group_id).await?;
        if seeded && self.relay_url.is_some() {
            tokio::time::sleep(Duration::from_millis(CONTACT_BOOTSTRAP_SETTLE_MILLIS)).await;
            self.initiate_group_sessions(group_id).await?;
        }
        self.emit_group_state(group_id).await;
        Ok(())
    }

    /// Re-nudges sync sessions for every joined group topic; the periodic
    /// maintenance uses this so an offline Owner receives pending join
    /// requests once they return (ADR-0005).
    pub(crate) async fn nudge_group_sessions(&self) {
        let group_ids: Vec<String> = self.group_streams.keys().cloned().collect();
        for group_id in group_ids {
            if let Err(err) = self.initiate_group_sessions(&group_id).await {
                tracing::debug!("failed to nudge group {group_id}: {err:#}");
            }
        }
    }

    /// Joins the profile topics that members-only groups need for key
    /// exchange, friendship or not: the Owner needs each joiner's key bundle
    /// to welcome them (their bundle lives on their profile's Spaces log),
    /// and members need the Owner's bundle to open the welcome (ADR-0015).
    pub(crate) async fn sync_group_peer_profiles(&mut self) {
        let group_ids = match self.groups.registered_groups().await {
            Ok(group_ids) => group_ids,
            Err(err) => {
                tracing::debug!("failed to list groups for key exchange: {err:#}");
                return;
            }
        };
        let mut peers: Vec<String> = Vec::new();
        for group_id in group_ids {
            let Ok(Some(state)) = crate::groups::read_group_state(&self.domain, &group_id).await
            else {
                continue;
            };
            if state.content_mode != crate::groups::GroupContentMode::MembersOnly {
                continue;
            }
            if state.permits(
                &self.local_profile_id,
                crate::groups::GroupPermission::Manage,
            ) {
                peers.extend(
                    state
                        .members
                        .iter()
                        .map(|member| member.profile_id.clone())
                        .chain(
                            state
                                .pending_requests
                                .iter()
                                .map(|request| request.requester_profile_id.clone()),
                        ),
                );
            } else if state.is_member(&self.local_profile_id)
                || state.has_pending_request_from(&self.local_profile_id)
            {
                if let Some(owner) = state.owner() {
                    peers.push(owner.profile_id.clone());
                }
            }
        }
        peers.sort();
        peers.dedup();
        for peer in peers {
            if peer == self.local_profile_id || self.contact_streams.contains_key(&peer) {
                continue;
            }
            if let Err(err) = self.sync_contact_profile(&peer).await {
                tracing::debug!("failed to sync group peer {peer}: {err:#}");
            }
        }
    }

    /// Pushes operations the groups service appended (genesis, governance,
    /// control messages, posts) into their groups' live gossip.
    pub(crate) fn flush_groups_outbox(&self) {
        flush_groups_outbox_shared(&self.groups, &self.group_handles);
    }

    /// Emits the group's viewer-filtered state to the UI, if known.
    pub(crate) async fn emit_group_state(&self, group_id: &str) {
        emit_group_state_shared(&self.groups, group_id, &self.event_tx).await;
    }

    /// Rejoins all known groups after startup, processes their control
    /// backlog, and runs the Owner-side duties.
    pub(crate) async fn resume_groups(&mut self) -> Result<()> {
        let group_ids = self.groups.registered_groups().await?;
        if group_ids.is_empty() {
            return Ok(());
        }
        if let Err(err) = self.groups.process_backlog(&group_ids).await {
            tracing::warn!("failed to process groups backlog: {err:#}");
        }
        for group_id in &group_ids {
            if let Err(err) = self.join_group_topic(group_id).await {
                tracing::warn!("failed to resume group {group_id}: {err:#}");
            }
            if let Err(err) = self.groups.process_owner_duties(group_id).await {
                tracing::debug!("owner duties for {group_id} deferred: {err:#}");
            }
            self.emit_group_state(group_id).await;
        }
        if let Err(err) = self.reconcile_group_advertisements().await {
            tracing::debug!("advertisement reconcile at startup deferred: {err:#}");
        }
        self.sync_group_peer_profiles().await;
        self.emit_group_suggestions().await;
        self.flush_groups_outbox();
        Ok(())
    }

    /// Runs owner duties (auto-accepts, auth reconcile) for every known
    /// group, keeps membership advertisements in step, and reflects any
    /// changes. Idempotent background work.
    pub(crate) async fn reconcile_groups(&mut self) -> Result<()> {
        for group_id in self.groups.registered_groups().await? {
            let changed = self
                .groups
                .process_owner_duties(&group_id)
                .await
                .unwrap_or(false);
            self.flush_groups_outbox();
            if changed {
                self.emit_group_state(&group_id).await;
            }
        }
        self.reconcile_group_advertisements().await
    }

    /// Aligns the friend-visible membership advertisements with reality:
    /// one active advertisement per `listed` group the local profile is a
    /// member of, carrying the current name; everything else retracted
    /// (ADR-0008). Idempotent; a rename re-advertises under the new name.
    pub(crate) async fn reconcile_group_advertisements(&mut self) -> Result<()> {
        let Some(own) = self.read_profile_state(&self.local_profile_id).await? else {
            return Ok(());
        };
        let current: HashMap<String, String> = own
            .advertised_groups
            .iter()
            .map(|ad| (ad.group_id.clone(), ad.group_name.clone()))
            .collect();
        let mut desired: HashMap<String, String> = HashMap::new();
        for group_id in self.groups.registered_groups().await? {
            let Some(state) = crate::groups::read_group_state(&self.domain, &group_id).await?
            else {
                continue;
            };
            if state.is_member(&self.local_profile_id)
                && state.discoverability == GroupDiscoverability::Listed
            {
                desired.insert(group_id, state.name.clone());
            }
        }

        let now = crate::profile::now_unix_secs();
        for (group_id, group_name) in &desired {
            if current.get(group_id) != Some(group_name) {
                self.publish(DomainOperation::GroupMembershipAdvertised {
                    profile_id: self.local_profile_id.clone(),
                    group_id: group_id.clone(),
                    group_name: group_name.clone(),
                    active: true,
                    recorded_at: now,
                })
                .await?;
            }
        }
        for (group_id, group_name) in &current {
            if !desired.contains_key(group_id) {
                self.publish(DomainOperation::GroupMembershipAdvertised {
                    profile_id: self.local_profile_id.clone(),
                    group_id: group_id.clone(),
                    group_name: group_name.clone(),
                    active: false,
                    recorded_at: now,
                })
                .await?;
            }
        }
        Ok(())
    }

    /// Recomputes and emits friend-based group suggestions.
    pub(crate) async fn emit_group_suggestions(&self) {
        emit_group_suggestions_shared(&self.domain, &self.local_profile_id, &self.event_tx).await;
    }

    async fn seed_contact_bootstrap(&self, public_key: VerifyingKey) -> Result<()> {
        self.seed_bootstrap_with_relay(public_key, None).await
    }

    /// Seeds address-book bootstrap info for a peer, preferring an explicitly
    /// known relay (e.g. from a friend code) over our own.
    pub(crate) async fn seed_bootstrap_with_relay(
        &self,
        public_key: VerifyingKey,
        relay_url: Option<RelayUrl>,
    ) -> Result<()> {
        let Some(relay_url) = relay_url.or_else(|| self.relay_url.clone()) else {
            return Ok(());
        };

        let endpoint_addr =
            EndpointAddr::new(from_verifying_key(public_key)).with_relay_url(relay_url);
        self.address_book
            .insert_node_info(NodeInfo::from(endpoint_addr).bootstrap())
            .await
            .with_context(|| format!("failed to seed bootstrap info for {}", public_key))?;
        Ok(())
    }
}

async fn open_domain_store(data_dir: &Path) -> Result<DomainStore> {
    std::fs::create_dir_all(data_dir).with_context(|| {
        format!(
            "failed to create data directory {} for domain store",
            data_dir.display()
        )
    })?;
    let db_path = data_dir.join(DOMAIN_DB_FILE);
    SqliteStoreBuilder::new()
        .database_url(&format!("sqlite://{}?mode=rwc", db_path.display()))
        // A single connection keeps writes serialized (and matches how the
        // profile data store is opened).
        .max_connections(1)
        .build()
        .await
        .with_context(|| format!("failed to open domain store {}", db_path.display()))
}

/// Ingests operations arriving on a topic and reflects the topic profile's
/// new reduced state to the UI as events.
#[allow(clippy::too_many_arguments)]
fn spawn_topic_task(
    mut subscription: DomainSyncSubscription,
    store: DomainStore,
    profile_id: String,
    event_tx: Sender<NetworkEvent>,
    is_local: bool,
    spaces: JynSpaces,
    local_profile_id: String,
    local_handle: std::sync::Arc<DomainSyncHandle>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut domain = JynOperationDomain::new(store);

        // Applies a spaces processing report: re-emits profiles whose reduced
        // state gained decrypted payloads and, when new key bundles arrived,
        // reconciles friends-space membership (a friend can only be added
        // once their bundle is known) and pushes forged control messages
        // into live sync.
        async fn apply_spaces_report(
            report: crate::spaces::IngestReport,
            domain: &JynOperationDomain,
            spaces: &JynSpaces,
            profile_id: &str,
            local_profile_id: &str,
            event_tx: &Sender<NetworkEvent>,
            local_handle: &DomainSyncHandle,
        ) {
            for changed in &report.changed_profiles {
                if changed != profile_id {
                    let is_local = changed == local_profile_id;
                    emit_topic_state(domain, changed, event_tx, is_local).await;
                }
            }
            if !report.new_key_bundles.is_empty() {
                if let Err(err) = spaces.reconcile_friends().await {
                    tracing::warn!("spaces reconcile failed: {err:#}");
                }
                // Additions only; removals re-key and wait for the next
                // Circles post.
                if let Err(err) = spaces.reconcile_circles(false).await {
                    tracing::warn!("circles reconcile failed: {err:#}");
                }
                for operation in spaces.drain_outbox() {
                    if let Err(err) = local_handle.publish(operation) {
                        tracing::warn!("failed to publish spaces operation: {err}");
                    }
                }
            }
        }

        while let Some(message) = subscription.next().await {
            let Ok(message) = message else {
                continue;
            };

            match message.event {
                TopicLogSyncEvent::OperationReceived { operation, .. } => {
                    let operation = *operation;
                    if let Err(err) = domain.ingest_remote_operation(operation.clone()).await {
                        tracing::warn!(
                            topic_profile_id = %profile_id,
                            "failed to ingest synced operation: {err:#}"
                        );
                        continue;
                    }
                    // Spaces messages additionally run through the group
                    // encryption manager; decrypted payloads become visible
                    // to reduction, so re-emit affected profiles.
                    match spaces.ingest(&operation).await {
                        Ok(report) => {
                            apply_spaces_report(
                                report,
                                &domain,
                                &spaces,
                                &profile_id,
                                &local_profile_id,
                                &event_tx,
                                &local_handle,
                            )
                            .await;
                        }
                        Err(err) => {
                            tracing::warn!(
                                topic_profile_id = %profile_id,
                                "spaces processing failed: {err:#}"
                            );
                        }
                    }
                    emit_topic_state(&domain, &profile_id, &event_tx, is_local).await;
                    // Only a friend's advertisement can change the hub
                    // suggestions; skip the (own + every-friend + every-group)
                    // fan-out for ordinary posts, hearts, and profile edits.
                    let advertises_group = !is_local
                        && operation.body.as_ref().is_some_and(|body| {
                            matches!(
                                decode_cbor::<DomainOperation, _>(&body.to_bytes()[..]),
                                Ok(DomainOperation::GroupMembershipAdvertised { .. })
                            )
                        });
                    if advertises_group {
                        emit_group_suggestions_shared(&domain, &local_profile_id, &event_tx).await;
                    }
                }
                TopicLogSyncEvent::SyncFinished { .. }
                | TopicLogSyncEvent::LiveModeStarted
                | TopicLogSyncEvent::SessionFinished { .. } => {
                    // Sync sessions can deliver dependencies out of order
                    // across authors; retry parked spaces messages now.
                    match spaces.drain_pending().await {
                        Ok(report) => {
                            apply_spaces_report(
                                report,
                                &domain,
                                &spaces,
                                &profile_id,
                                &local_profile_id,
                                &event_tx,
                                &local_handle,
                            )
                            .await;
                        }
                        Err(err) => {
                            tracing::warn!(
                                topic_profile_id = %profile_id,
                                "spaces drain failed: {err:#}"
                            );
                        }
                    }
                    emit_topic_state(&domain, &profile_id, &event_tx, is_local).await;
                }
                TopicLogSyncEvent::Failed { error } => {
                    tracing::warn!(
                        topic_profile_id = %profile_id,
                        "LogSync replication failed: {error}"
                    );
                }
                TopicLogSyncEvent::SessionStarted | TopicLogSyncEvent::SyncStarted { .. } => {}
            }
        }
    })
}

/// Ingests operations arriving on a group topic, feeds control messages to
/// the groups service, runs the Owner-side duties, and reflects the group's
/// new state to the UI.
fn spawn_group_topic_task(
    mut subscription: DomainSyncSubscription,
    store: DomainStore,
    group_id: String,
    event_tx: Sender<NetworkEvent>,
    groups: JynGroups,
    group_handles: GroupHandles,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut domain = JynOperationDomain::new(store);

        async fn after_change(
            domain: &JynOperationDomain,
            groups: &JynGroups,
            group_id: &str,
            group_handles: &GroupHandles,
            event_tx: &Sender<NetworkEvent>,
        ) {
            // Owner-side duties: auto-accept open joins, reconcile the auth
            // layer. A no-op on every other node.
            if let Err(err) = groups.process_owner_duties(group_id).await {
                tracing::debug!(group_id, "owner duties deferred: {err:#}");
            }
            flush_groups_outbox_shared(groups, group_handles);
            emit_group_state_shared(groups, group_id, event_tx).await;
            // The viewer's own membership shapes the suggestions (a joined
            // group stops being one).
            emit_group_suggestions_shared(domain, groups.local_profile_id(), event_tx).await;
        }

        // Backlog catch-up delivers each stored op as its own
        // `OperationReceived` followed by one batch-completion event. Running
        // the (expensive) duties+reduce+emit per backlog op is O(ops²); once
        // live we pay it per op, but during catch-up we defer to the single
        // batch-completion pass below.
        let mut live = false;
        while let Some(message) = subscription.next().await {
            let Ok(message) = message else {
                continue;
            };

            match message.event {
                TopicLogSyncEvent::OperationReceived { operation, .. } => {
                    let operation = *operation;
                    if let Err(err) = domain.ingest_remote_operation(operation.clone()).await {
                        tracing::warn!(
                            group_id,
                            "failed to ingest synced group operation: {err:#}"
                        );
                        continue;
                    }
                    if let Err(err) = groups.ingest(&operation).await {
                        tracing::warn!(group_id, "group control processing failed: {err:#}");
                    }
                    if live {
                        after_change(&domain, &groups, &group_id, &group_handles, &event_tx).await;
                    }
                }
                TopicLogSyncEvent::SyncFinished { .. }
                | TopicLogSyncEvent::LiveModeStarted
                | TopicLogSyncEvent::SessionFinished { .. } => {
                    if let Err(err) = groups.drain_pending().await {
                        tracing::warn!(group_id, "group control drain failed: {err:#}");
                    }
                    // Caught up: fold the whole backlog once, and pay per-op
                    // from here on so live gossip still updates promptly.
                    live = true;
                    after_change(&domain, &groups, &group_id, &group_handles, &event_tx).await;
                }
                TopicLogSyncEvent::Failed { error } => {
                    tracing::warn!(group_id, "group LogSync replication failed: {error}");
                }
                TopicLogSyncEvent::SessionStarted | TopicLogSyncEvent::SyncStarted { .. } => {}
            }
        }
    })
}

fn flush_groups_outbox_shared(groups: &JynGroups, group_handles: &GroupHandles) {
    for (group_id, operation) in groups.drain_outbox() {
        let handle = group_handles
            .read()
            .expect("group handles lock poisoned")
            .get(&group_id)
            .cloned();
        match handle {
            Some(handle) => {
                if let Err(err) = handle.publish(operation) {
                    tracing::warn!(group_id, "failed to publish group operation live: {err}");
                }
            }
            // Persisted and syncable regardless; the next session serves it.
            None => tracing::debug!(group_id, "no live handle for group operation yet"),
        }
    }
}

async fn emit_group_state_shared(
    groups: &JynGroups,
    group_id: &str,
    event_tx: &Sender<NetworkEvent>,
) {
    match groups.group_view(group_id).await {
        Ok(Some(view)) => {
            // We just reduced the genesis; record its content mode so the
            // blob-secret scan can skip this group when it's public.
            if let Err(err) = groups
                .record_content_kind(group_id, view.content_mode)
                .await
            {
                tracing::debug!(group_id, "failed to record group content kind: {err:#}");
            }
            let _ = event_tx.send(NetworkEvent::GroupUpdated { view });
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(group_id, "failed to reduce group state: {err:#}");
        }
    }
}

fn warn_invalid_profile_id(profile_id: &str) {
    tracing::warn!("skipping invalid profile id {profile_id}");
}

/// Aggregates friends' membership advertisements into hub suggestions:
/// groups the viewer's *own friends* are in (never strangers', ADR-0008)
/// that the viewer has not joined. Emitted as a full snapshot.
async fn emit_group_suggestions_shared(
    domain: &JynOperationDomain,
    local_profile_id: &str,
    event_tx: &Sender<NetworkEvent>,
) {
    let own = match domain.read_profile_state(local_profile_id).await {
        Ok(Some(own)) => own,
        _ => return,
    };
    let mut by_group: std::collections::BTreeMap<String, (String, Vec<String>)> =
        std::collections::BTreeMap::new();
    for friend_id in &own.followed_profile_ids {
        let Ok(Some(friend)) = domain.read_profile_state(friend_id).await else {
            continue;
        };
        for ad in &friend.advertised_groups {
            let entry = by_group
                .entry(ad.group_id.clone())
                .or_insert_with(|| (ad.group_name.clone(), Vec::new()));
            entry.1.push(friend_id.clone());
        }
    }

    let mut suggestions = Vec::new();
    for (group_id, (group_name, via)) in by_group {
        // Groups the viewer already belongs to are not suggestions.
        if let Ok(Some(state)) = crate::groups::read_group_state(domain, &group_id).await {
            if state.is_member(local_profile_id) {
                continue;
            }
        }
        suggestions.push(GroupSuggestion {
            group_id,
            group_name,
            via_friend_profile_ids: via,
        });
    }
    let _ = event_tx.send(NetworkEvent::GroupSuggestionsUpdated { suggestions });
}

async fn emit_topic_state(
    domain: &JynOperationDomain,
    profile_id: &str,
    event_tx: &Sender<NetworkEvent>,
    is_local: bool,
) {
    match domain.read_profile_state(profile_id).await {
        Ok(Some(state)) => {
            let event = if is_local {
                NetworkEvent::LocalStateUpdated { state }
            } else {
                NetworkEvent::ContactStateUpdated {
                    profile_id: profile_id.to_owned(),
                    state,
                }
            };
            let _ = event_tx.send(event);
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(
                topic_profile_id = %profile_id,
                "failed to reduce topic profile state: {err:#}"
            );
        }
    }
}

async fn wait_for_topic_registration(
    address_book: &p2panda_net::AddressBook,
    topic: Topic,
    public_key: VerifyingKey,
    profile_id: &str,
) -> Result<()> {
    let timeout = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => {
                anyhow::bail!("timed out waiting for topic bootstrap registration for {profile_id}");
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                let node_infos = address_book
                    .node_infos_by_topics([topic])
                    .await
                    .with_context(|| format!("failed to read topic bootstrap registration for {profile_id}"))?;
                if node_infos.into_iter().any(|info| info.node_id == public_key) {
                    return Ok(());
                }
            }
        }
    }
}

fn normalize_profile_id(profile_id: &str) -> Result<VerifyingKey> {
    profile_id
        .parse()
        .with_context(|| format!("invalid profile ID {profile_id}"))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use tempfile::tempdir;

    use super::*;
    use crate::domain::Visibility;
    use crate::node::NodeOptions;
    use crate::profile::ProfileStore;

    #[tokio::test(flavor = "multi_thread")]
    async fn published_operations_survive_service_restart() -> Result<()> {
        let dir = tempdir()?;
        let node = AppNode::with_data_dir(
            dir.path(),
            NodeOptions {
                mdns_enabled: false,
                relay_url: None,
                ..Default::default()
            },
        )
        .await?;
        let profile = ProfileStore::load_or_create(dir.path())?;
        let profile_id = profile.profile().profile_id.clone();
        drop(profile);

        let (event_tx, event_rx) = flume::unbounded();
        {
            let mut sync = JynSyncService::new(&node, profile_id.clone(), event_tx.clone()).await?;
            // A friends post goes through the encrypted path; the reduced
            // state below only sees it if encrypt → decrypt-substitution →
            // reduction all work.
            sync.publish_encrypted(
                DomainOperation::PostPublished {
                    profile_id: profile_id.clone(),
                    post_id: "post-1".into(),
                    body: "hello river".into(),
                    media: Vec::new(),
                    visibility: Visibility::Friends,
                    expires_at: None,
                    created_at: 10,
                },
                SpaceKind::Friends,
            )
            .await?;

            // Publishing reflects the new state back as an event.
            let event = event_rx.recv_timeout(Duration::from_secs(5))?;
            match event {
                NetworkEvent::LocalStateUpdated { state } => {
                    assert_eq!(state.posts.len(), 1);
                    assert_eq!(state.posts[0].body, "hello river");
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }

        // A fresh service over the same data dir sees the post without any
        // record migration or cache files.
        let sync = JynSyncService::new(&node, profile_id.clone(), event_tx).await?;
        let state = sync
            .read_profile_state(&profile_id)
            .await?
            .expect("state exists after restart");
        assert_eq!(state.posts.len(), 1);
        assert_eq!(state.posts[0].post_id, "post-1");

        Ok(())
    }

    /// The spec's recovery promise: a backup archive plus the seed phrase
    /// deterministically restores identity, opaque group-encryption state
    /// and content — including the ability to read own encrypted posts.
    #[tokio::test(flavor = "multi_thread")]
    async fn backup_restores_identity_and_encrypted_history() -> Result<()> {
        let dir = tempdir()?;
        let options = NodeOptions {
            mdns_enabled: false,
            relay_url: None,
            ..Default::default()
        };
        let node = AppNode::with_data_dir(dir.path(), options.clone()).await?;
        let profile = ProfileStore::load_or_create(dir.path())?;
        let profile_id = profile.profile().profile_id.clone();
        drop(profile);

        let private_key = crate::profile::load_private_key_from_data_dir(dir.path())?;
        let phrase = crate::backup::seed_phrase(&private_key)?;

        let (event_tx, _event_rx) = flume::unbounded();
        let archive = dir.path().join("jyn.backup");
        {
            let mut sync = JynSyncService::new(&node, profile_id.clone(), event_tx.clone()).await?;
            sync.publish_encrypted(
                DomainOperation::PostPublished {
                    profile_id: profile_id.clone(),
                    post_id: "post-1".into(),
                    body: "sealed memories".into(),
                    media: Vec::new(),
                    visibility: Visibility::Friends,
                    expires_at: None,
                    created_at: 10,
                },
                SpaceKind::Friends,
            )
            .await?;

            let snapshot = dir.path().join("domain-snapshot.sqlite3");
            sync.snapshot_store_into(&snapshot).await?;
            crate::backup::write_archive(
                &private_key,
                vec![("domain.sqlite3".to_owned(), std::fs::read(&snapshot)?)],
                &archive,
            )?;
        }

        // Restore into a brand-new data dir using only archive + phrase.
        let restored_dir = tempdir()?;
        crate::backup::restore_backup(restored_dir.path(), &archive, &phrase)?;

        let restored_node = AppNode::with_data_dir(restored_dir.path(), options).await?;
        let restored_profile = ProfileStore::load_or_create(restored_dir.path())?;
        assert_eq!(
            restored_profile.profile().profile_id,
            profile_id,
            "identity must survive restore"
        );
        drop(restored_profile);

        let sync = JynSyncService::new(&restored_node, profile_id.clone(), event_tx).await?;
        let state = sync
            .read_profile_state(&profile_id)
            .await?
            .expect("state exists after restore");
        assert_eq!(state.posts.len(), 1);
        assert_eq!(state.posts[0].body, "sealed memories");

        Ok(())
    }
}

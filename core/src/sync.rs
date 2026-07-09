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
use p2panda_core::cbor::encode_cbor;
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
    DomainExtensions, DomainLogId, DomainOperation, JynOperationDomain, JynTopicMap,
    ReducedProfileState, Visibility,
};
use crate::node::AppNode;
use crate::profile::load_private_key_from_data_dir;
use crate::spaces::JynSpaces;

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

    /// Encrypts a domain operation to our own friends space and pushes the
    /// resulting wrapper operations into live sync.
    pub(crate) async fn publish_encrypted(&mut self, operation: DomainOperation) -> Result<()> {
        self.spaces.encrypt_local(&operation).await?;
        self.flush_spaces_outbox();
        self.emit_local_state().await;
        Ok(())
    }

    /// Encrypts a domain operation (comment/heart) to the friends space of
    /// the given profile. Fails if we have not been welcomed there yet.
    pub(crate) async fn publish_encrypted_to_owner(
        &mut self,
        owner_profile_id: &str,
        operation: DomainOperation,
    ) -> Result<()> {
        self.spaces
            .encrypt_to_owner(owner_profile_id, &operation)
            .await?;
        self.flush_spaces_outbox();
        self.emit_local_state().await;
        Ok(())
    }

    /// Aligns friends-space membership with the current accepted-friends
    /// list, then pushes any forged control messages into live sync.
    /// Idempotent and cheap when nothing changed.
    pub(crate) async fn reconcile_spaces(&mut self) -> Result<()> {
        let friends = self
            .read_profile_state(&self.local_profile_id)
            .await?
            .map(|state| state.followed_profile_ids)
            .unwrap_or_default();
        self.spaces.reconcile_friends(&friends).await?;
        self.flush_spaces_outbox();
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
                let friends = match domain.read_profile_state(local_profile_id).await {
                    Ok(Some(state)) => state.followed_profile_ids,
                    _ => Vec::new(),
                };
                if let Err(err) = spaces.reconcile_friends(&friends).await {
                    tracing::warn!("spaces reconcile failed: {err:#}");
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
            sync.publish_encrypted(DomainOperation::PostPublished {
                profile_id: profile_id.clone(),
                post_id: "post-1".into(),
                body: "hello river".into(),
                media: Vec::new(),
                visibility: Visibility::Friends,
                expires_at: None,
                created_at: 10,
            })
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
            sync.publish_encrypted(DomainOperation::PostPublished {
                profile_id: profile_id.clone(),
                post_id: "post-1".into(),
                body: "sealed memories".into(),
                media: Vec::new(),
                visibility: Visibility::Friends,
                expires_at: None,
                created_at: 10,
            })
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

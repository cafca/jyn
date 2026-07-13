//! The Bevy ↔ tokio bridge: the UI sends [`NetworkCommand`]s, the network
//! runtime answers with [`NetworkEvent`]s over flume channels. The p2panda
//! node and all async work live on a dedicated runtime thread.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::SystemTime;

use anyhow::{Context, Result};
use flume::{Receiver, Sender, TryRecvError};
use std::sync::Mutex;

use crate::diagnostics::{
    now_unix_ms, DiagnosticsSnapshot, NodeIdentitySnapshot, PeerConnectionState,
    PeerDiscoveryMethod, PeerSnapshot,
};
use crate::domain::{DomainOperation, ReducedPost, ReducedProfileState, Visibility};
use crate::local_stores::{KeepsStore, OutgoingRequestsStore, PrivatePostsStore};
use crate::node::{AppNode, NodeOptions};
use crate::profile::{now_unix_secs, ProfileStore, UserProfile};
use crate::settings::load_settings;
use crate::spaces::SpaceKind;
use crate::sync::JynSyncService;

type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'static>>;

/// A file staged on the composer, imported into the blob store on cast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaDraft {
    pub path: PathBuf,
    pub kind: crate::domain::MediaKind,
    /// Duration for recorded voice notes.
    pub duration_ms: Option<u64>,
    /// Waveform peaks for recorded voice notes.
    pub waveform: Option<Vec<u8>>,
}

/// A post being cast from the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostDraft {
    pub body: String,
    pub visibility: Visibility,
    /// Lifetime in seconds from now; `None` = permanent.
    pub lifetime_secs: Option<u64>,
    pub media: Vec<MediaDraft>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkCommand {
    /// Replay persisted state (profile, own posts, private posts, friends)
    /// into events after startup.
    RecoverStartup,
    RequestDiagnostics,
    /// Join a contact's topic and sync their stream.
    SyncContactProfile {
        profile_id: String,
    },
    PublishPost {
        draft: PostDraft,
    },
    EditPost {
        post_id: String,
        body: String,
        /// Attachments the author kept, verbatim; removed ones are simply
        /// absent. The edit op replaces the post's full media list.
        kept_media: Vec<crate::domain::MediaAttachment>,
        /// Freshly staged files to append.
        new_media: Vec<MediaDraft>,
    },
    DeletePost {
        post_id: String,
    },
    /// Promote (`None`) or let it go again (`Some(unix_secs)`).
    SetPostLifetime {
        post_id: String,
        expires_at: Option<u64>,
    },
    UpdateProfile {
        display_name: String,
        bio: String,
        default_visibility: Visibility,
        default_lifetime_secs: Option<u64>,
        mark_onboarded: bool,
    },
    /// The share-code ritual: decode a friend code, reach out, and place a
    /// friendship request on the target's topic.
    RequestFriendship {
        friend_code: String,
        greeting: Option<String>,
    },
    /// In-app request to a profile discovery already put in front of us
    /// (e.g. a ghost card carried in by a friend's heart).
    RequestFriendshipById {
        profile_id: String,
        greeting: Option<String>,
    },
    /// Answer a pending request. Accepting follows the requester back and
    /// starts syncing their stream.
    RespondFriendship {
        requester_profile_id: String,
        accept: bool,
    },
    RemoveFriend {
        profile_id: String,
    },
    /// Aligns friends-space membership (group encryption) with the current
    /// friends list. Idempotent; dispatched after state updates.
    ReconcileSpaces,
    /// Writes an encrypted snapshot of identity-critical state (domain +
    /// profile-data stores) to the given path. Decryptable only with the
    /// recovery phrase.
    ExportBackup {
        dest_path: String,
    },
    /// Automatic reaction when someone we requested started following us:
    /// follow back, making the friendship mutual.
    FollowBack {
        profile_id: String,
    },
    /// Toggle a named heart on someone's post.
    SetHeart {
        post_author_profile_id: String,
        post_id: String,
        active: bool,
    },
    PublishComment {
        post_author_profile_id: String,
        post_id: String,
        body: String,
    },
    /// Keep a private copy of a post — a lease that dies with the post's
    /// lifetime or the author's delete.
    KeepPost {
        post_author_profile_id: String,
        post_id: String,
    },
    ReleaseKeep {
        post_author_profile_id: String,
        post_id: String,
    },
    /// Fetch a post attachment's blob into the media cache.
    FetchMedia {
        blob_hash: String,
    },
    /// Drop expired private posts from disk (replicated posts expire by
    /// read-time filtering; kept copies are pruned from the keeps store).
    DrainExpired,
}

impl NetworkCommand {
    /// User-initiated commands surface failures in the UI; background
    /// commands only log them.
    fn is_user_action(&self) -> bool {
        match self {
            Self::PublishPost { .. }
            | Self::EditPost { .. }
            | Self::DeletePost { .. }
            | Self::SetPostLifetime { .. }
            | Self::UpdateProfile { .. }
            | Self::RequestFriendship { .. }
            | Self::RequestFriendshipById { .. }
            | Self::RespondFriendship { .. }
            | Self::RemoveFriend { .. }
            | Self::SetHeart { .. }
            | Self::PublishComment { .. }
            | Self::KeepPost { .. }
            | Self::ReleaseKeep { .. }
            | Self::ExportBackup { .. } => true,
            Self::RecoverStartup
            | Self::RequestDiagnostics
            | Self::SyncContactProfile { .. }
            | Self::ReconcileSpaces
            | Self::FollowBack { .. }
            | Self::FetchMedia { .. }
            | Self::DrainExpired => false,
        }
    }

    fn context_label(&self) -> &'static str {
        match self {
            Self::RecoverStartup => "startup recovery",
            Self::RequestDiagnostics => "diagnostics",
            Self::SyncContactProfile { .. } => "contact sync",
            Self::PublishPost { .. } => "casting the post",
            Self::EditPost { .. } => "editing the post",
            Self::DeletePost { .. } => "deleting the post",
            Self::SetPostLifetime { .. } => "changing the post lifetime",
            Self::UpdateProfile { .. } => "updating the profile",
            Self::RequestFriendship { .. } | Self::RequestFriendshipById { .. } => {
                "sending the friendship request"
            }
            Self::RespondFriendship { .. } => "answering the friendship request",
            Self::RemoveFriend { .. } => "unfriending",
            Self::ReconcileSpaces => "updating encryption membership",
            Self::ExportBackup { .. } => "exporting the backup",
            Self::FollowBack { .. } => "completing the friendship",
            Self::SetHeart { .. } => "casting the heart",
            Self::PublishComment { .. } => "publishing the comment",
            Self::KeepPost { .. } => "keeping the post",
            Self::ReleaseKeep { .. } => "releasing the keep",
            Self::FetchMedia { .. } => "fetching media",
            Self::DrainExpired => "draining expired posts",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NetworkEvent {
    DiagnosticsSnapshot {
        snapshot: DiagnosticsSnapshot,
    },
    /// The local profile as loaded at startup (feeds onboarding state and
    /// composer defaults before any operation exists).
    ProfileLoaded {
        profile: UserProfile,
    },
    /// The local profile's reduced replicated state changed (own posts,
    /// follows, pending friend requests).
    LocalStateUpdated {
        state: ReducedProfileState,
    },
    /// The full private (local-only) post list after any change.
    PrivatePostsUpdated {
        posts: Vec<ReducedPost>,
    },
    /// A synced contact's reduced state changed.
    ContactStateUpdated {
        profile_id: String,
        state: ReducedProfileState,
    },
    /// The full keeps list after any change or lease pruning.
    KeepsUpdated {
        keeps: Vec<crate::local_stores::KeepRecord>,
    },
    /// A fetched (or freshly cast) attachment blob is available locally.
    MediaReady {
        blob_hash: String,
        path: PathBuf,
    },
    MediaFailed {
        blob_hash: String,
        error_message: String,
    },
    Error {
        context: String,
        error_message: String,
    },
}

/// A command paired with an optional responder: fire-and-forget callers pass
/// `None` and failures surface as [`NetworkEvent::Error`]s; awaitable callers
/// receive the command's outcome directly instead.
type CommandEnvelope = (
    NetworkCommand,
    Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
);

pub struct AsyncBridge {
    command_tx: Sender<CommandEnvelope>,
    event_rx: Receiver<NetworkEvent>,
    runtime_thread: Mutex<Option<JoinHandle<()>>>,
}

impl AsyncBridge {
    pub fn spawn(node_options: NodeOptions) -> Result<Self> {
        let data_dir = crate::app_config::resolve_data_dir()?;
        Self::spawn_with_data_dir(node_options, data_dir)
    }

    pub fn spawn_with_data_dir(node_options: NodeOptions, data_dir: PathBuf) -> Result<Self> {
        let bridge = Self::spawn_with_worker(
            move |event_tx| {
                Box::pin(async move { RuntimeState::new(data_dir, node_options, event_tx).await })
            },
            |state, command, events| {
                Box::pin(async move { default_handle_command(state, command, events).await })
            },
        )?;
        bridge.send(NetworkCommand::RecoverStartup)?;
        Ok(bridge)
    }

    pub(crate) fn spawn_with_worker<State, Init, Worker>(init: Init, worker: Worker) -> Result<Self>
    where
        State: Send + Sync + 'static,
        Init: FnOnce(Sender<NetworkEvent>) -> BoxFuture<Result<State>> + Send + 'static,
        Worker: Fn(Arc<State>, NetworkCommand, Sender<NetworkEvent>) -> BoxFuture<Result<()>>
            + Send
            + Sync
            + 'static,
    {
        let (command_tx, command_rx) = flume::unbounded();
        let (event_tx, event_rx) = flume::unbounded();
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel(1);
        let worker = Arc::new(worker);

        let runtime_thread = thread::Builder::new()
            .name("jyn-network".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        let _ = init_tx.send(Err(anyhow::Error::new(err)));
                        return;
                    }
                };

                runtime.block_on(async move {
                    let state = match init(event_tx.clone()).await {
                        Ok(state) => {
                            let state = Arc::new(state);
                            let _ = init_tx.send(Ok(()));
                            state
                        }
                        Err(err) => {
                            let _ = init_tx.send(Err(err));
                            return;
                        }
                    };

                    run_network_loop(state, command_rx, event_tx, worker).await;
                });
            })
            .context("failed to spawn network bridge thread")?;

        init_rx
            .recv()
            .context("network bridge startup channel closed before initialization completed")??;

        Ok(Self {
            command_tx,
            event_rx,
            runtime_thread: Mutex::new(Some(runtime_thread)),
        })
    }

    pub fn send(&self, command: NetworkCommand) -> Result<()> {
        self.command_tx
            .send((command, None))
            .context("failed to send command to network thread")
    }

    /// Sends a command and returns a receiver that resolves with the
    /// command's outcome once the worker finished it. Unlike [`Self::send`],
    /// failures do NOT additionally surface as [`NetworkEvent::Error`]s —
    /// the caller owns the error.
    pub fn send_awaited(
        &self,
        command: NetworkCommand,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>> {
        let (responder, receiver) = tokio::sync::oneshot::channel();
        self.command_tx
            .send((command, Some(responder)))
            .context("failed to send command to network thread")?;
        Ok(receiver)
    }

    pub fn try_recv(&self) -> Result<Option<NetworkEvent>, TryRecvError> {
        match self.event_rx.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// A receiver handle for a dedicated consumer thread. Events are
    /// delivered to whichever receiver claims them first — use either this
    /// or `try_recv`/`drain_events`, not both.
    pub(crate) fn event_receiver(&self) -> Receiver<NetworkEvent> {
        self.event_rx.clone()
    }

    /// A fire-and-forget command sender handle for a consumer thread.
    pub(crate) fn command_sender(&self) -> Sender<CommandEnvelope> {
        self.command_tx.clone()
    }

    pub fn drain_events(&self) -> Vec<NetworkEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }
}

impl Drop for AsyncBridge {
    fn drop(&mut self) {
        let (replacement_tx, replacement_rx) = flume::unbounded();
        let old_tx = std::mem::replace(&mut self.command_tx, replacement_tx);
        drop(replacement_rx);
        drop(old_tx);

        let mut runtime_thread_guard = match self.runtime_thread.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(runtime_thread) = runtime_thread_guard.take() {
            let _ = runtime_thread.join();
        }
    }
}

async fn run_network_loop<State, Worker>(
    state: Arc<State>,
    command_rx: Receiver<CommandEnvelope>,
    event_tx: Sender<NetworkEvent>,
    worker: Arc<Worker>,
) where
    State: Send + Sync + 'static,
    Worker: Fn(Arc<State>, NetworkCommand, Sender<NetworkEvent>) -> BoxFuture<Result<()>>
        + Send
        + Sync
        + 'static,
{
    while let Ok((command, responder)) = command_rx.recv_async().await {
        let state = Arc::clone(&state);
        let event_tx = event_tx.clone();
        let worker = Arc::clone(&worker);
        let is_user_action = command.is_user_action();
        let context_label = command.context_label();

        tokio::spawn(async move {
            let result = worker(state, command, event_tx.clone()).await;
            match responder {
                Some(responder) => {
                    if let Err(err) = &result {
                        tracing::debug!("awaited command failed ({context_label}): {err:#}");
                    }
                    let _ = responder.send(result.map_err(|err| err.to_string()));
                }
                None => {
                    if let Err(err) = result {
                        if is_user_action {
                            let _ = event_tx.send(NetworkEvent::Error {
                                context: context_label.to_owned(),
                                error_message: err.to_string(),
                            });
                        } else {
                            tracing::warn!(
                                "background network command failed ({context_label}): {err:#}"
                            );
                        }
                    }
                }
            }
        });
    }
}

struct RuntimeState {
    node: AppNode,
    sync: Arc<tokio::sync::Mutex<JynSyncService>>,
    profile: tokio::sync::Mutex<ProfileStore>,
    private_posts: PrivatePostsStore,
    keeps: KeepsStore,
    outgoing_requests: OutgoingRequestsStore,
    local_profile_id: String,
    media_cache_dir: PathBuf,
    _maintenance_task: tokio::task::JoinHandle<()>,
}

impl RuntimeState {
    async fn new(
        data_dir: PathBuf,
        node_options: NodeOptions,
        event_tx: Sender<NetworkEvent>,
    ) -> Result<Self> {
        let _settings = load_settings(&data_dir)?;
        let node = AppNode::with_data_dir(data_dir, node_options).await?;
        let profile_store = ProfileStore::load_or_create(&node.data_dir)?;
        let local_profile_id = profile_store.profile().profile_id.clone();
        let private_posts = PrivatePostsStore::open(&node.data_dir)?;
        let keeps = KeepsStore::open(&node.data_dir)?;
        let outgoing_requests = OutgoingRequestsStore::open(&node.data_dir)?;
        let sync = Arc::new(tokio::sync::Mutex::new(
            JynSyncService::new(&node, local_profile_id.clone(), event_tx).await?,
        ));
        let maintenance_task = spawn_sync_maintenance(
            Arc::clone(&sync),
            outgoing_requests.clone(),
            local_profile_id.clone(),
        );
        let media_cache_dir = node.data_dir.join("media-cache");
        Ok(Self {
            node,
            sync,
            profile: tokio::sync::Mutex::new(profile_store),
            private_posts,
            keeps,
            outgoing_requests,
            local_profile_id,
            media_cache_dir,
            _maintenance_task: maintenance_task,
        })
    }
}

/// Periodically re-initiates sync with peers we are still waiting on: the
/// targets of outstanding friendship requests (they may have been offline
/// when the request or their answer was published) and friends we have never
/// heard from directly.
fn spawn_sync_maintenance(
    sync: Arc<tokio::sync::Mutex<JynSyncService>>,
    outgoing_requests: OutgoingRequestsStore,
    local_profile_id: String,
) -> tokio::task::JoinHandle<()> {
    let interval_secs = std::env::var("JYN_MAINTENANCE_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30);

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;

            let mut targets = outgoing_requests.list().unwrap_or_default();
            {
                let sync_guard = sync.lock().await;
                if let Ok(Some(own)) = sync_guard.read_profile_state(&local_profile_id).await {
                    // Friends we never heard from, plus circle members
                    // (friends-of-friends) whose bundles and posts we still
                    // need — their friend named them, but their own topic has
                    // not answered yet.
                    let mut watched = own.followed_profile_ids.clone();
                    for friend_id in &own.followed_profile_ids {
                        if let Ok(Some(friend_state)) =
                            sync_guard.read_profile_state(friend_id).await
                        {
                            watched.extend(friend_state.followed_profile_ids);
                        }
                    }
                    for profile_id in watched {
                        if profile_id == local_profile_id {
                            continue;
                        }
                        let heard_from = sync_guard
                            .has_operations_from(&profile_id)
                            .await
                            .unwrap_or(true);
                        if !heard_from && !targets.contains(&profile_id) {
                            targets.push(profile_id);
                        }
                    }
                }
            }

            for profile_id in targets {
                let mut sync_guard = sync.lock().await;
                if let Err(err) = sync_guard.sync_contact_profile(&profile_id).await {
                    tracing::debug!("periodic re-sync with {profile_id} failed: {err:#}");
                }
            }
        }
    })
}

async fn default_handle_command(
    state: Arc<RuntimeState>,
    command: NetworkCommand,
    event_tx: Sender<NetworkEvent>,
) -> Result<()> {
    match command {
        NetworkCommand::RecoverStartup => recover_startup(&state, &event_tx).await,
        NetworkCommand::RequestDiagnostics => {
            let snapshot = collect_diagnostics_snapshot(&state).await?;
            event_tx
                .send(NetworkEvent::DiagnosticsSnapshot { snapshot })
                .context("failed to send DiagnosticsSnapshot event")?;
            Ok(())
        }
        NetworkCommand::SyncContactProfile { profile_id } => {
            let mut sync = state.sync.lock().await;
            sync.sync_contact_profile(&profile_id).await?;
            Ok(())
        }
        NetworkCommand::PublishPost { draft } => publish_post(&state, &event_tx, draft).await,
        NetworkCommand::EditPost {
            post_id,
            body,
            kept_media,
            new_media,
        } => edit_post(&state, &event_tx, post_id, body, kept_media, new_media).await,
        NetworkCommand::DeletePost { post_id } => delete_post(&state, &event_tx, post_id).await,
        NetworkCommand::SetPostLifetime {
            post_id,
            expires_at,
        } => set_post_lifetime(&state, &event_tx, post_id, expires_at).await,
        NetworkCommand::UpdateProfile {
            display_name,
            bio,
            default_visibility,
            default_lifetime_secs,
            mark_onboarded,
        } => {
            update_profile(
                &state,
                &event_tx,
                display_name,
                bio,
                default_visibility,
                default_lifetime_secs,
                mark_onboarded,
            )
            .await
        }
        NetworkCommand::RequestFriendship {
            friend_code,
            greeting,
        } => request_friendship(&state, friend_code, greeting).await,
        NetworkCommand::RequestFriendshipById {
            profile_id,
            greeting,
        } => request_friendship_by_id(&state, profile_id, greeting).await,
        NetworkCommand::RespondFriendship {
            requester_profile_id,
            accept,
        } => respond_friendship(&state, requester_profile_id, accept).await,
        NetworkCommand::RemoveFriend { profile_id } => remove_friend(&state, profile_id).await,
        NetworkCommand::ReconcileSpaces => {
            let mut sync = state.sync.lock().await;
            sync.reconcile_spaces().await
        }
        NetworkCommand::ExportBackup { dest_path } => export_backup(&state, dest_path).await,
        NetworkCommand::FollowBack { profile_id } => follow_back(&state, profile_id).await,
        NetworkCommand::SetHeart {
            post_author_profile_id,
            post_id,
            active,
        } => {
            let operation = DomainOperation::HeartChanged {
                profile_id: state.local_profile_id.clone(),
                post_author_profile_id: post_author_profile_id.clone(),
                post_id: post_id.clone(),
                active,
                recorded_at: now_unix_secs(),
            };
            publish_interaction(&state, &post_author_profile_id, &post_id, operation).await
        }
        NetworkCommand::PublishComment {
            post_author_profile_id,
            post_id,
            body,
        } => {
            anyhow::ensure!(!body.trim().is_empty(), "a comment needs some words");
            let now = now_unix_secs();
            let comment_id = new_post_id(&state.local_profile_id, now);
            let operation = DomainOperation::CommentPublished {
                profile_id: state.local_profile_id.clone(),
                comment_id,
                post_author_profile_id: post_author_profile_id.clone(),
                post_id: post_id.clone(),
                body: body.trim().to_owned(),
                created_at: now,
            };
            publish_interaction(&state, &post_author_profile_id, &post_id, operation).await
        }
        NetworkCommand::KeepPost {
            post_author_profile_id,
            post_id,
        } => keep_post(&state, &event_tx, post_author_profile_id, post_id).await,
        NetworkCommand::ReleaseKeep {
            post_author_profile_id,
            post_id,
        } => {
            if state.keeps.release(&post_author_profile_id, &post_id)? {
                unpin_and_prune_prefix(
                    &state,
                    &format!("keep/{post_author_profile_id}/{post_id}/"),
                )
                .await;
            }
            emit_keeps(&state, &event_tx)
        }
        NetworkCommand::FetchMedia { blob_hash } => fetch_media(&state, &event_tx, blob_hash).await,
        NetworkCommand::DrainExpired => drain_expired(&state, &event_tx).await,
    }
}

/// The visibility of a post as its author currently replicates it, from the
/// author's reduced state (local or contact). `None` = post unknown here.
async fn post_visibility(
    state: &RuntimeState,
    author_profile_id: &str,
    post_id: &str,
) -> Result<Option<Visibility>> {
    let sync = state.sync.lock().await;
    let author_state = sync.read_profile_state(author_profile_id).await?;
    Ok(author_state.and_then(|reduced| {
        reduced
            .posts
            .iter()
            .find(|post| post.post_id == post_id)
            .map(|post| post.visibility)
    }))
}

/// The encryption space a post of this visibility lives in. Callers handle
/// `Public` (plaintext) and `Private` (local-only) before asking.
fn space_kind_for(visibility: Visibility) -> SpaceKind {
    match visibility {
        Visibility::Circles => SpaceKind::Circles,
        _ => SpaceKind::Friends,
    }
}

/// Routes a heart/comment either to plaintext sync (public target post) or
/// into the post author's encryption space (non-public target post), so the
/// interaction is visible to exactly the audience that can see the post.
async fn publish_interaction(
    state: &RuntimeState,
    post_author_profile_id: &str,
    post_id: &str,
    operation: DomainOperation,
) -> Result<()> {
    // No visibility means the post is no longer in the author's reduced state:
    // it expired and was torn down, or was deleted. Either way there's nothing
    // left to react to — say so plainly rather than "unknown post".
    let visibility = post_visibility(state, post_author_profile_id, post_id)
        .await?
        .with_context(|| {
            format!(
                "that post has expired or was removed, so it can't be reacted to (post {post_id})"
            )
        })?;
    let mut sync = state.sync.lock().await;
    if visibility == Visibility::Public {
        sync.publish(operation).await
    } else {
        sync.publish_encrypted_to_owner(
            post_author_profile_id,
            operation,
            space_kind_for(visibility),
        )
        .await
    }
}

/// Downloads an attachment blob from known peers into the media cache.
/// Failures surface as `MediaFailed` (so the card can stop spinning), not
/// as UI error lines — media arrives when peers do.
/// Releases a post's or keep's hold on its attachments: prunes the
/// materialized cache files and removes the pins under `prefix`, so any blob
/// no other post or keep still pins becomes eligible for GC. Best-effort —
/// blob teardown never fails a user action.
async fn unpin_and_prune_prefix(state: &RuntimeState, prefix: &str) {
    match state.node.blobs.pins().list_prefix(prefix).await {
        Ok(pins) => {
            for pin in pins {
                crate::media::prune_cached(&state.media_cache_dir, &pin.hash.to_string());
            }
        }
        Err(err) => tracing::warn!("failed to list pins under {prefix}: {err}"),
    }
    if let Err(err) = state.node.blobs.pins().delete_prefix(prefix).await {
        tracing::warn!("failed to unpin under {prefix}: {err}");
    }
}

/// Tears down an expired-or-deleted non-public post on this device: removes its
/// `feed/` attachment pins (so unreferenced blobs lose their GC root and their
/// materialized plaintext cache files are pruned) and erases the post's content
/// — both the locally-cached decrypted body and the encrypted payload of the
/// stored operation — so neither the readable text nor its ciphertext can be
/// recovered from disk; only header-only metadata remains. Built once and reused by
/// both explicit delete and expiry drain. Kept copies survive untouched — their
/// pins live under a separate `keep/…` namespace, so a blob shared with a keep
/// is reclaimed only when the last referencing pin is gone. Idempotent and
/// best-effort: re-running it on an already-torn-down post is a no-op, and blob
/// teardown never fails the user action that triggered it.
async fn teardown_feed_presence(state: &RuntimeState, post_id: &str) {
    unpin_and_prune_prefix(state, &format!("feed/{post_id}/")).await;
    let sync = state.sync.lock().await;
    if let Err(err) = sync
        .erase_post_content(&state.local_profile_id, post_id)
        .await
    {
        tracing::warn!("failed to erase post content for {post_id}: {err}");
    }
}

/// Recipient-side teardown of an expired non-public post received from a friend
/// or friend-of-friend, so expired content leaves this device (and the network)
/// too, not just the author's. For each attachment it prunes the materialized
/// plaintext cache file and stops serving the ciphertext blob to other peers; a
/// recipient never pins fetched blobs, so the ciphertext is already GC-eligible
/// on this device — this just makes us stop being a source before the sweep. It
/// also erases the post's content — the decrypted body and the encrypted payload
/// of the stored operation — so neither the readable text nor its ciphertext can
/// be recovered from disk. A copy the recipient explicitly KEPT is untouched: its blobs stay held
/// under the recipient's own `keep/…` pins and its text lives in the keep
/// snapshot, both independent of this teardown (a kept blob re-exports from the
/// store on next view, and its per-blob secret comes from the keep snapshot).
/// Idempotent and best-effort.
async fn teardown_received_post(
    state: &RuntimeState,
    author_profile_id: &str,
    post: &crate::domain::ReducedPost,
    kept_hashes: &std::collections::HashSet<String>,
) {
    for attachment in &post.media {
        // A blob we still hold via one of our own keeps is left entirely alone:
        // its lifecycle (including whether we serve it) is governed by the keep,
        // not by the original post's expiry. Blocking it here would also be
        // irreversible — nothing ever unblocks a hash.
        if kept_hashes.contains(&attachment.blob_hash) {
            continue;
        }
        crate::media::prune_cached(&state.media_cache_dir, &attachment.blob_hash);
        if let Ok(hash) = attachment.blob_hash.parse::<p2panda_blobs::Hash>() {
            state.node.blobs.block_serving_hashes([hash]);
        }
    }
    let sync = state.sync.lock().await;
    if let Err(err) = sync
        .erase_post_content(author_profile_id, &post.post_id)
        .await
    {
        tracing::warn!(
            "failed to erase content for received post {}: {err}",
            post.post_id
        );
    }
}

/// Writes an encrypted snapshot of the identity-critical stores to
/// `dest_path`, plus blob bytes per the media-backup setting. Restore itself
/// happens before the node starts, via `crate::backup::restore_backup`; the
/// staged blobs re-import on the next start (`import_restored_blobs`).
async fn export_backup(state: &RuntimeState, dest_path: String) -> Result<()> {
    let private_key = crate::profile::load_private_key_from_data_dir(&state.node.data_dir)?;
    let staging = tempfile::tempdir().context("failed to create backup staging dir")?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();

    // Collect blob candidates before the sqlite snapshots: reading the
    // profile-data store from this task right after its `VACUUM INTO` starves
    // the single-connection pool until the acquire times out (the release
    // races the helper-thread acquire across runtimes).
    let mode = load_settings(&state.node.data_dir)?.media_backup_mode;
    let blob_hashes = backup_blob_hashes(state, mode).await?;

    let domain_snapshot = staging.path().join("domain.sqlite3");
    {
        let sync = state.sync.lock().await;
        sync.snapshot_store_into(&domain_snapshot).await?;
    }
    files.push((
        "domain.sqlite3".to_owned(),
        std::fs::read(&domain_snapshot).context("failed to read domain snapshot")?,
    ));

    let profile_snapshot = staging.path().join("profile-store.sqlite3");
    state.private_posts.snapshot_into(&profile_snapshot).await?;
    files.push((
        "profile-store.sqlite3".to_owned(),
        std::fs::read(&profile_snapshot).context("failed to read profile-data snapshot")?,
    ));

    files.extend(crate::backup::collect_plain_files(&state.node.data_dir));

    for blob_hash in blob_hashes {
        let Ok(hash) = blob_hash.parse::<p2panda_blobs::Hash>() else {
            continue;
        };
        // Skip blobs we don't hold locally (e.g. a friend's photo we never
        // viewed) — they re-fetch from peers after a restore.
        if !state.node.blobs.has(hash).await.unwrap_or(false) {
            continue;
        }
        match state.node.blobs.get_bytes(hash).await {
            Ok(bytes) => files.push((
                format!("{}{blob_hash}", crate::backup::BLOB_ENTRY_PREFIX),
                bytes.to_vec(),
            )),
            Err(err) => tracing::warn!("skipping blob {blob_hash} in backup: {err}"),
        }
    }

    crate::backup::write_archive(&private_key, files, std::path::Path::new(&dest_path))
}

/// The blob hashes a backup should carry under the given mode. Expired and
/// tombstoned posts never contribute — their bytes are meant to leave the
/// world, and a backup must not smuggle them back in.
async fn backup_blob_hashes(
    state: &RuntimeState,
    mode: crate::settings::MediaBackupMode,
) -> Result<Vec<String>> {
    use crate::settings::MediaBackupMode;

    if mode == MediaBackupMode::MetadataOnly {
        return Ok(Vec::new());
    }
    let now = now_unix_secs();
    let mut hashes = std::collections::BTreeSet::new();
    let mut collect = |posts: &[ReducedPost]| {
        for post in posts {
            if post.is_expired(now) {
                continue;
            }
            for attachment in &post.media {
                hashes.insert(attachment.blob_hash.clone());
            }
        }
    };

    // Keeps and private posts in every blob-carrying mode: nothing on the
    // network can re-serve them, so losing them is permanent.
    let kept: Vec<ReducedPost> = state
        .keeps
        .list()
        .context("keeps for backup")?
        .into_iter()
        .map(|keep| keep.snapshot)
        .collect();
    collect(&kept);
    collect(&state.private_posts.list().context("private for backup")?);

    if mode == MediaBackupMode::Full {
        let sync = state.sync.lock().await;
        if let Some(own) = sync.read_profile_state(&state.local_profile_id).await? {
            collect(&own.posts);
            for contact_id in &own.followed_profile_ids {
                if let Some(contact) = sync.read_profile_state(contact_id).await? {
                    collect(&contact.posts);
                }
            }
        }
    }

    Ok(hashes.into_iter().collect())
}

/// Imports blobs staged by a restore into the blob store, re-pinning them
/// from the restored records: own and private posts pin under `feed/…`,
/// keeps under `keep/…`. Blobs justified by neither (a friend's media held
/// at export time) import bare — resident like any downloaded blob, GC-able
/// once nothing references them. Runs once; the staging dir is removed.
async fn import_restored_blobs(state: &RuntimeState) -> Result<()> {
    let staging = state.node.data_dir.join(crate::backup::RESTORED_BLOBS_DIR);
    if !staging.is_dir() {
        return Ok(());
    }

    // Pin names by blob hash, reconstructed the way the original import and
    // keep paths would have pinned them.
    let mut pins_by_hash: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    {
        let sync = state.sync.lock().await;
        if let Some(own) = sync.read_profile_state(&state.local_profile_id).await? {
            for post in &own.posts {
                for attachment in &post.media {
                    pins_by_hash
                        .entry(attachment.blob_hash.clone())
                        .or_default()
                        .push(format!("feed/{}/{}", post.post_id, attachment.blob_hash));
                }
            }
        }
    }
    for post in state.private_posts.list()? {
        for attachment in &post.media {
            pins_by_hash
                .entry(attachment.blob_hash.clone())
                .or_default()
                .push(format!("feed/{}/{}", post.post_id, attachment.blob_hash));
        }
    }
    for keep in state.keeps.list()? {
        for attachment in &keep.snapshot.media {
            pins_by_hash
                .entry(attachment.blob_hash.clone())
                .or_default()
                .push(format!(
                    "keep/{}/{}/{}",
                    keep.author_profile_id, keep.post_id, attachment.blob_hash
                ));
        }
    }

    let entries =
        std::fs::read_dir(&staging).context("failed to read restored-blobs staging dir")?;
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let temp_tag = match state.node.blobs.add_path(&path).temp_tag().await {
            Ok(tag) => tag,
            Err(err) => {
                tracing::warn!("failed to import restored blob {name}: {err}");
                continue;
            }
        };
        let hash = temp_tag.hash();
        if hash.to_string() != name {
            tracing::warn!("restored blob {name} hashes to {hash}; dropping the mismatched bytes");
            drop(temp_tag);
            continue;
        }
        for pin in pins_by_hash.get(&name).into_iter().flatten() {
            if let Err(err) = state.node.blobs.pins().set(pin.clone(), hash).await {
                tracing::warn!("failed to pin restored blob {name} as {pin}: {err}");
            }
        }
        drop(temp_tag);
    }

    std::fs::remove_dir_all(&staging).context("failed to clear restored-blobs staging dir")?;
    tracing::info!("imported restored blobs into the blob store");
    Ok(())
}

/// Finds the per-blob decryption secret for a blob hash by scanning every
/// post source that can reference it: our own stream, friends' streams,
/// local private posts, and kept snapshots. `None` = plaintext (public) blob.
async fn find_blob_secret(state: &RuntimeState, blob_hash: &str) -> Result<Option<Vec<u8>>> {
    let secret_in = |posts: &[crate::domain::ReducedPost]| {
        posts.iter().find_map(|post| {
            post.media
                .iter()
                .find(|attachment| attachment.blob_hash == blob_hash)
                .and_then(|attachment| attachment.blob_secret.clone())
        })
    };

    let sync = state.sync.lock().await;
    if let Some(local) = sync.read_profile_state(&state.local_profile_id).await? {
        if let Some(secret) = secret_in(&local.posts) {
            return Ok(Some(secret));
        }
    }
    // Search every profile we receive posts from — direct friends *and*
    // friends-of-friends. A Circles post reaches a friend-of-friend, whose
    // author is not in our own follow list, so restricting to followed
    // profiles would fail to find the per-blob secret and leave the fetched
    // ciphertext undecrypted in the cache.
    for contact_id in sync.contact_profile_ids().await? {
        if let Some(contact) = sync.read_profile_state(&contact_id).await? {
            if let Some(secret) = secret_in(&contact.posts) {
                return Ok(Some(secret));
            }
        }
    }
    drop(sync);

    if let Some(secret) = secret_in(&state.private_posts.list()?) {
        return Ok(Some(secret));
    }
    let kept: Vec<crate::domain::ReducedPost> = state
        .keeps
        .list()?
        .into_iter()
        .map(|keep| keep.snapshot)
        .collect();
    Ok(secret_in(&kept))
}

async fn fetch_media(
    state: &RuntimeState,
    event_tx: &Sender<NetworkEvent>,
    blob_hash: String,
) -> Result<()> {
    let result: Result<PathBuf> = async {
        let hash: p2panda_blobs::Hash = blob_hash
            .parse()
            .map_err(|err| anyhow::anyhow!("invalid blob hash {blob_hash}: {err}"))?;
        if !state.node.blobs.has(hash).await? {
            state.node.blobs.download(hash).await?;
        }
        std::fs::create_dir_all(&state.media_cache_dir).with_context(|| {
            format!(
                "failed to create media cache {}",
                state.media_cache_dir.display()
            )
        })?;
        // Materialize a standalone copy out of the content-addressed store
        // (whose on-disk layout the app can't hand to the OS directly). This
        // copy is disposable — prune/eviction can delete it and it re-exports
        // on the next view.
        let path = state.media_cache_dir.join(&blob_hash);
        state
            .node
            .blobs
            .export(hash, &path)
            .finish()
            .await
            .map_err(|err| anyhow::anyhow!("failed to export blob {blob_hash}: {err}"))?;
        // Encrypted blob (non-public post): the store holds ciphertext, the
        // cache holds plaintext. The per-blob secret comes from whichever
        // post payload references this hash.
        if let Some(secret) = find_blob_secret(state, &blob_hash).await? {
            let ciphertext = std::fs::read(&path)
                .with_context(|| format!("failed to read exported blob {blob_hash}"))?;
            let plaintext = crate::media::blob_crypto::decrypt_blob(&ciphertext, &secret)?;
            std::fs::write(&path, plaintext)
                .with_context(|| format!("failed to write decrypted blob {blob_hash}"))?;
        }
        crate::media::evict_to_budget(
            &state.media_cache_dir,
            crate::media::MEDIA_CACHE_BUDGET_BYTES,
            &blob_hash,
        );
        Ok(path)
    }
    .await;

    let event = match result {
        Ok(path) => NetworkEvent::MediaReady { blob_hash, path },
        Err(err) => NetworkEvent::MediaFailed {
            blob_hash,
            error_message: err.to_string(),
        },
    };
    event_tx.send(event).context("failed to send media event")
}

async fn keep_post(
    state: &RuntimeState,
    event_tx: &Sender<NetworkEvent>,
    post_author_profile_id: String,
    post_id: String,
) -> Result<()> {
    // Snapshot the post from the author's reduced state (own or a friend's).
    let sync = state.sync.lock().await;
    let author_state = sync
        .read_profile_state(&post_author_profile_id)
        .await?
        .with_context(|| format!("no state known for {post_author_profile_id}"))?;
    let snapshot = author_state
        .posts
        .iter()
        .find(|post| post.post_id == post_id)
        .with_context(|| format!("post {post_id} not found in their stream"))?
        .clone();
    drop(sync);

    // A keep is a lease on the bytes, not just the text: pin the attachments
    // under our own keep/ name so they survive the original post's expiry or
    // deletion (and its feed/ pin going away). Released in `release` and when
    // the lease lapses in `drain_expired`.
    for attachment in &snapshot.media {
        match attachment.blob_hash.parse::<p2panda_blobs::Hash>() {
            Ok(hash) => {
                if let Err(err) = state
                    .node
                    .blobs
                    .pins()
                    .set(
                        format!(
                            "keep/{post_author_profile_id}/{post_id}/{}",
                            attachment.blob_hash
                        ),
                        hash,
                    )
                    .await
                {
                    tracing::warn!("failed to pin kept blob {}: {err}", attachment.blob_hash);
                }
            }
            Err(err) => tracing::warn!(
                "kept post {post_id} carries an unparseable blob hash {}: {err}",
                attachment.blob_hash
            ),
        }
    }

    state.keeps.keep(crate::local_stores::KeepRecord {
        post_id,
        author_profile_id: post_author_profile_id,
        snapshot,
        kept_at: now_unix_secs(),
    })?;
    emit_keeps(state, event_tx)
}

fn emit_keeps(state: &RuntimeState, event_tx: &Sender<NetworkEvent>) -> Result<()> {
    event_tx
        .send(NetworkEvent::KeepsUpdated {
            keeps: state.keeps.list()?,
        })
        .context("failed to send KeepsUpdated event")
}

async fn request_friendship(
    state: &RuntimeState,
    friend_code: String,
    greeting: Option<String>,
) -> Result<()> {
    let code = crate::friend_code::FriendCode::decode(&friend_code)?;
    let target_key = code.verifying_key()?;
    let target_profile_id = code.profile_id_string()?;
    anyhow::ensure!(
        target_profile_id != state.local_profile_id,
        "that's your own friend code"
    );

    let display_name = state.profile.lock().await.profile().display_name.clone();
    let mut sync = state.sync.lock().await;
    let relay_url = code
        .relay_url
        .as_deref()
        .map(|url| url.parse())
        .transpose()
        .context("friend code carries an invalid relay URL")?;
    sync.seed_bootstrap_with_relay(target_key, relay_url)
        .await?;
    // Join their topic (read-only until they accept) so the request can
    // travel and their answer can reach us.
    sync.sync_contact_profile(&target_profile_id).await?;
    sync.publish(DomainOperation::FriendshipRequested {
        requester_profile_id: state.local_profile_id.clone(),
        target_profile_id: target_profile_id.clone(),
        requester_display_name: display_name,
        greeting,
        recorded_at: now_unix_secs(),
    })
    .await?;
    state.outgoing_requests.add(&target_profile_id)?;
    Ok(())
}

/// Like `request_friendship`, but for a profile known only by id — reach is
/// bootstrapped over our own relay.
async fn request_friendship_by_id(
    state: &RuntimeState,
    target_profile_id: String,
    greeting: Option<String>,
) -> Result<()> {
    anyhow::ensure!(target_profile_id != state.local_profile_id, "that's you");
    let target_key: p2panda_core::VerifyingKey = target_profile_id
        .parse()
        .with_context(|| format!("invalid profile id {target_profile_id}"))?;

    let display_name = state.profile.lock().await.profile().display_name.clone();
    let mut sync = state.sync.lock().await;
    sync.seed_bootstrap_with_relay(target_key, None).await?;
    sync.sync_contact_profile(&target_profile_id).await?;
    sync.publish(DomainOperation::FriendshipRequested {
        requester_profile_id: state.local_profile_id.clone(),
        target_profile_id: target_profile_id.clone(),
        requester_display_name: display_name,
        greeting,
        recorded_at: now_unix_secs(),
    })
    .await?;
    state.outgoing_requests.add(&target_profile_id)?;
    Ok(())
}

async fn respond_friendship(
    state: &RuntimeState,
    requester_profile_id: String,
    accept: bool,
) -> Result<()> {
    let mut sync = state.sync.lock().await;
    sync.publish(DomainOperation::FriendshipResponded {
        target_profile_id: state.local_profile_id.clone(),
        requester_profile_id: requester_profile_id.clone(),
        accepted: accept,
        recorded_at: now_unix_secs(),
    })
    .await?;

    if accept {
        sync.publish(DomainOperation::ContactFollowChanged {
            profile_id: state.local_profile_id.clone(),
            followed_profile_id: requester_profile_id.clone(),
            recorded_at: now_unix_secs(),
            active: true,
        })
        .await?;
        sync.sync_contact_profile(&requester_profile_id).await?;
        // Add the new friend to the encryption space as soon as their key
        // bundle allows; retried on later state updates if it hasn't arrived.
        let _ = sync.reconcile_spaces().await;
    }
    Ok(())
}

async fn remove_friend(state: &RuntimeState, profile_id: String) -> Result<()> {
    let mut sync = state.sync.lock().await;
    sync.publish(DomainOperation::ContactFollowChanged {
        profile_id: state.local_profile_id.clone(),
        followed_profile_id: profile_id.clone(),
        recorded_at: now_unix_secs(),
        active: false,
    })
    .await?;
    sync.stop_contact_sync(&profile_id);
    state.outgoing_requests.remove(&profile_id)?;
    // Removing them from the friends space re-keys it, so they cannot read
    // anything published from here on.
    sync.reconcile_spaces().await?;
    Ok(())
}

async fn follow_back(state: &RuntimeState, profile_id: String) -> Result<()> {
    // Follow-back completes a friendship *we* initiated. Without this guard,
    // any synced non-friend (circle members' topics sync too) could follow
    // us and be auto-befriended — gaining friends-space access unconsented.
    if !state.outgoing_requests.list()?.contains(&profile_id) {
        tracing::debug!("ignoring follow-back for {profile_id}: no outstanding request");
        return Ok(());
    }
    let mut sync = state.sync.lock().await;
    sync.publish(DomainOperation::ContactFollowChanged {
        profile_id: state.local_profile_id.clone(),
        followed_profile_id: profile_id.clone(),
        recorded_at: now_unix_secs(),
        active: true,
    })
    .await?;
    sync.sync_contact_profile(&profile_id).await?;
    state.outgoing_requests.remove(&profile_id)?;
    let _ = sync.reconcile_spaces().await;
    Ok(())
}

async fn recover_startup(state: &RuntimeState, event_tx: &Sender<NetworkEvent>) -> Result<()> {
    // Blobs staged by a restore enter the blob store first, so restored
    // posts' media is servable without any peer round-trip.
    if let Err(err) = import_restored_blobs(state).await {
        tracing::warn!("failed to import restored blobs: {err:#}");
    }

    // Profile first: onboarding state and composer defaults.
    let profile = state.profile.lock().await.profile().clone();
    event_tx
        .send(NetworkEvent::ProfileLoaded { profile })
        .context("failed to send ProfileLoaded event")?;

    // Local replicated state (own posts, follows, pending requests).
    let mut sync = state.sync.lock().await;
    let mut topics_to_sync = state.outgoing_requests.list()?;
    if let Some(local_state) = sync.read_profile_state(&state.local_profile_id).await? {
        // Surface friends' current reduced states from the persistent store
        // right away, then re-join their topics for live updates.
        for profile_id in &local_state.followed_profile_ids {
            if let Some(contact_state) = sync.read_profile_state(profile_id).await? {
                let _ = event_tx.send(NetworkEvent::ContactStateUpdated {
                    profile_id: profile_id.clone(),
                    state: contact_state,
                });
            }
            if !topics_to_sync.contains(profile_id) {
                topics_to_sync.push(profile_id.clone());
            }
        }
        event_tx
            .send(NetworkEvent::LocalStateUpdated { state: local_state })
            .context("failed to send LocalStateUpdated event")?;
    }
    // Friends' topics and topics of profiles we asked for friendship (to
    // observe their answer). Failures are per-contact and non-fatal.
    for profile_id in topics_to_sync {
        if let Err(err) = sync.sync_contact_profile(&profile_id).await {
            tracing::warn!("failed to resume sync with {profile_id}: {err:#}");
        }
    }
    // Spaces messages that arrived but were not yet processed when the app
    // last closed (the pending queue is in-memory), then membership catch-up
    // — which also joins circle members' (friends-of-friends) topics.
    if let Err(err) = sync.process_spaces_backlog().await {
        tracing::warn!("failed to process spaces backlog: {err:#}");
    }
    if let Err(err) = sync.reconcile_spaces().await {
        tracing::warn!("failed to reconcile spaces at startup: {err:#}");
    }
    drop(sync);

    // Private posts (drain anything that expired while the app was closed).
    let now = now_unix_secs();
    for post in &state.private_posts.drain_expired(now)? {
        unpin_and_prune_prefix(state, &format!("feed/{}/", post.post_id)).await;
    }
    event_tx
        .send(NetworkEvent::PrivatePostsUpdated {
            posts: state.private_posts.list()?,
        })
        .context("failed to send PrivatePostsUpdated event")?;

    // Keeps, after enforcing leases that lapsed while the app was closed.
    drain_expired(state, event_tx).await?;
    emit_keeps(state, event_tx)?;

    Ok(())
}

async fn publish_post(
    state: &RuntimeState,
    event_tx: &Sender<NetworkEvent>,
    draft: PostDraft,
) -> Result<()> {
    anyhow::ensure!(
        !draft.body.trim().is_empty() || !draft.media.is_empty(),
        "a post needs some words or something attached"
    );
    let now = now_unix_secs();
    let post_id = new_post_id(&state.local_profile_id, now);
    let expires_at = draft.lifetime_secs.map(|secs| now + secs);
    // Everything non-public is sealed — including local-only Private posts,
    // so their blobs are unreadable even if the content hash ever leaks.
    let media = import_attachments(
        state,
        event_tx,
        &post_id,
        &draft.media,
        draft.visibility != Visibility::Public,
    )
    .await?;

    if draft.visibility == Visibility::Private {
        state.private_posts.upsert(crate::domain::ReducedPost {
            profile_id: state.local_profile_id.clone(),
            post_id,
            body: draft.body,
            media,
            visibility: Visibility::Private,
            expires_at,
            created_at: now,
            edited: false,
        })?;
        event_tx
            .send(NetworkEvent::PrivatePostsUpdated {
                posts: state.private_posts.list()?,
            })
            .context("failed to send PrivatePostsUpdated event")?;
        return Ok(());
    }

    let operation = DomainOperation::PostPublished {
        profile_id: state.local_profile_id.clone(),
        post_id,
        body: draft.body,
        media,
        visibility: draft.visibility,
        expires_at,
        created_at: now,
        edited: false,
    };
    let mut sync = state.sync.lock().await;
    if draft.visibility == Visibility::Public {
        sync.publish(operation).await
    } else {
        // Friends posts encrypt to the friends space, Circles posts to the
        // circles space (friends ∪ friends-of-friends); only members read
        // them. A Circles publish lazily re-keys the space first.
        sync.publish_encrypted(operation, space_kind_for(draft.visibility))
            .await
    }
}

/// Imports staged files into the blob store, pins them under the post, and
/// copies them into the media cache so the author's own UI renders them
/// without a fetch.
///
/// With `encrypt` set (non-public posts), each file is sealed under a fresh
/// per-blob key before it enters the store: the blob replicates as
/// ciphertext, its content address is the ciphertext hash, and the key rides
/// in the attachment metadata inside the group-encrypted post payload.
async fn import_attachments(
    state: &RuntimeState,
    event_tx: &Sender<NetworkEvent>,
    post_id: &str,
    drafts: &[MediaDraft],
    encrypt: bool,
) -> Result<Vec<crate::domain::MediaAttachment>> {
    let mut attachments = Vec::with_capacity(drafts.len());
    for media in drafts {
        let byte_len = std::fs::metadata(&media.path)
            .with_context(|| format!("failed to read attachment {}", media.path.display()))?
            .len();
        // Post-time guard, authoritative over every caller (covers edits and,
        // once wired, transcoded output). The composer rejects oversized files
        // earlier for immediate feedback; this backstops the storage boundary.
        if let Some(max) = crate::media::max_bytes_for_kind(media.kind) {
            anyhow::ensure!(
                byte_len <= max,
                "attachment {} is too large ({byte_len} bytes; limit {max} for this kind)",
                media.path.display()
            );
        }
        let (import_path, blob_secret, _ciphertext_guard) = if encrypt {
            let plaintext = std::fs::read(&media.path)
                .with_context(|| format!("failed to read attachment {}", media.path.display()))?;
            let (ciphertext, secret) = crate::media::blob_crypto::encrypt_blob(&plaintext)?;
            let sealed = tempfile::NamedTempFile::new()
                .context("failed to create sealed attachment file")?;
            std::fs::write(sealed.path(), &ciphertext)
                .context("failed to write sealed attachment")?;
            (sealed.path().to_path_buf(), Some(secret), Some(sealed))
        } else {
            (media.path.clone(), None, None)
        };
        // Import under a single named pin. Awaiting add_path() directly would
        // also create an auto-tag that nothing ever cleans up, so the blob
        // would survive unpin forever; holding a temp tag while we set only
        // our feed/ pin leaves the post as the blob's sole GC root, so
        // deleting or draining the post can actually reclaim it.
        let temp_tag = state
            .node
            .blobs
            .add_path(&import_path)
            .temp_tag()
            .await
            .map_err(|err| anyhow::anyhow!("failed to import {}: {err}", media.path.display()))?;
        let hash = temp_tag.hash();
        let blob_hash = hash.to_string();
        state
            .node
            .blobs
            .pins()
            .set(format!("feed/{post_id}/{blob_hash}"), hash)
            .await
            .map_err(|err| anyhow::anyhow!("failed to pin attachment: {err}"))?;
        drop(temp_tag);

        // The cache always holds plaintext (keyed by blob hash), so the
        // author's UI renders without decrypting.
        std::fs::create_dir_all(&state.media_cache_dir).ok();
        let cached = state.media_cache_dir.join(&blob_hash);
        if !cached.exists() && std::fs::copy(&media.path, &cached).is_ok() {
            crate::media::evict_to_budget(
                &state.media_cache_dir,
                crate::media::MEDIA_CACHE_BUDGET_BYTES,
                &blob_hash,
            );
            let _ = event_tx.send(NetworkEvent::MediaReady {
                blob_hash: blob_hash.clone(),
                path: cached,
            });
        }

        attachments.push(crate::domain::MediaAttachment {
            kind: media.kind,
            blob_hash,
            byte_len,
            mime: crate::media::mime_for(&media.path),
            duration_ms: media.duration_ms,
            waveform: media.waveform.clone(),
            width: None,
            height: None,
            file_name: media
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            blob_secret,
        });
    }
    Ok(attachments)
}

async fn edit_post(
    state: &RuntimeState,
    event_tx: &Sender<NetworkEvent>,
    post_id: String,
    body: String,
    kept_media: Vec<crate::domain::MediaAttachment>,
    new_media: Vec<MediaDraft>,
) -> Result<()> {
    // Visibility decides whether new attachments are sealed, so resolve it
    // before importing: local-only private posts and replicated non-public
    // posts both get encrypted blobs.
    let is_private = state
        .private_posts
        .list()?
        .iter()
        .any(|post| post.post_id == post_id);
    let visibility = if is_private {
        Visibility::Private
    } else {
        match post_visibility(state, &state.local_profile_id, &post_id).await? {
            Some(visibility) => visibility,
            // Post already gone from our reduced state (deleted, or expired and
            // torn down). Editing a post that no longer exists is a no-op — and
            // must not fall back to importing plaintext blobs / publishing a
            // plaintext edit for what was an encrypted post.
            None => return Ok(()),
        }
    };

    let mut media = kept_media;
    media.extend(
        import_attachments(
            state,
            event_tx,
            &post_id,
            &new_media,
            visibility != Visibility::Public,
        )
        .await?,
    );
    anyhow::ensure!(
        !body.trim().is_empty() || !media.is_empty(),
        "a post needs some words or something attached"
    );

    // Private posts are edited in place; replicated posts get an edit op.
    if state.private_posts.edit(&post_id, &body, media.clone())? {
        event_tx
            .send(NetworkEvent::PrivatePostsUpdated {
                posts: state.private_posts.list()?,
            })
            .context("failed to send PrivatePostsUpdated event")?;
        return Ok(());
    }

    let operation = DomainOperation::PostEdited {
        profile_id: state.local_profile_id.clone(),
        post_id,
        body,
        media: Some(media),
        edited_at: now_unix_secs(),
    };
    let mut sync = state.sync.lock().await;
    if visibility == Visibility::Public {
        sync.publish(operation).await
    } else {
        sync.publish_encrypted(operation, space_kind_for(visibility))
            .await
    }
}

async fn delete_post(
    state: &RuntimeState,
    event_tx: &Sender<NetworkEvent>,
    post_id: String,
) -> Result<()> {
    if state.private_posts.remove(&post_id)? {
        unpin_and_prune_prefix(state, &format!("feed/{post_id}/")).await;
        event_tx
            .send(NetworkEvent::PrivatePostsUpdated {
                posts: state.private_posts.list()?,
            })
            .context("failed to send PrivatePostsUpdated event")?;
        return Ok(());
    }

    let Some(visibility) = post_visibility(state, &state.local_profile_id, &post_id).await? else {
        // The post is already gone from our own reduced state — deleted, or
        // expired and torn down (teardown purges its decrypted row). There is
        // nothing left to tombstone, and we must NOT fall back to a plaintext
        // publish: doing so would broadcast a `PostDeleted` for what was an
        // encrypted post. Its pins were already reclaimed when it went.
        return Ok(());
    };
    let operation = DomainOperation::PostDeleted {
        profile_id: state.local_profile_id.clone(),
        post_id: post_id.clone(),
        deleted_at: now_unix_secs(),
    };
    let mut sync = state.sync.lock().await;
    if visibility == Visibility::Public {
        sync.publish(operation).await?;
    } else {
        sync.publish_encrypted(operation, space_kind_for(visibility))
            .await?;
    }
    drop(sync);
    // Public posts are plaintext by design: only reclaim their pins. Non-public
    // posts additionally purge their decrypted body so delete converges on the
    // same "nothing readable left on disk" end state as expiry.
    if visibility == Visibility::Public {
        unpin_and_prune_prefix(state, &format!("feed/{post_id}/")).await;
    } else {
        teardown_feed_presence(state, &post_id).await;
    }
    Ok(())
}

async fn set_post_lifetime(
    state: &RuntimeState,
    event_tx: &Sender<NetworkEvent>,
    post_id: String,
    expires_at: Option<u64>,
) -> Result<()> {
    if state.private_posts.set_lifetime(&post_id, expires_at)? {
        event_tx
            .send(NetworkEvent::PrivatePostsUpdated {
                posts: state.private_posts.list()?,
            })
            .context("failed to send PrivatePostsUpdated event")?;
        return Ok(());
    }

    // Changing a lifetime changes a post's bucket, so we re-home it (ADR-0016):
    // publish a self-contained snapshot of its current state into the new
    // bucket and disown the old copy with a re-home marker. Reduction dedupes
    // by post id (newest ordering wins), so the snapshot supersedes every
    // earlier copy while GC can later drop the old bucket cleanly.
    let Some(post) = current_local_post(state, &post_id).await? else {
        // Post already gone from our reduced state (deleted, or expired and
        // torn down). Nothing to re-home, and a plaintext fallback publish
        // would leak an op for what was an encrypted post.
        return Ok(());
    };
    if post.expires_at == expires_at {
        return Ok(());
    }
    let now = now_unix_secs();
    let visibility = post.visibility;
    let rehomed = DomainOperation::PostRehomed {
        profile_id: state.local_profile_id.clone(),
        post_id: post_id.clone(),
        moved_at: now,
    };
    let snapshot = DomainOperation::PostPublished {
        profile_id: state.local_profile_id.clone(),
        post_id,
        body: post.body,
        media: post.media,
        visibility,
        expires_at,
        created_at: post.created_at,
        edited: post.edited,
    };

    let mut sync = state.sync.lock().await;
    if visibility == Visibility::Public {
        // Marker first, while the old copy is still the post's current one, so
        // it is placed into the old bucket; then the snapshot into the new one.
        sync.publish(rehomed).await?;
        sync.publish(snapshot).await
    } else {
        sync.publish_encrypted(rehomed, space_kind_for(visibility))
            .await?;
        sync.publish_encrypted(snapshot, space_kind_for(visibility))
            .await
    }
}

/// The author's own current copy of a post from reduced state, or `None` if
/// it is no longer present (deleted, or expired and torn down).
async fn current_local_post(
    state: &RuntimeState,
    post_id: &str,
) -> Result<Option<crate::domain::ReducedPost>> {
    let sync = state.sync.lock().await;
    let own_state = sync.read_profile_state(&state.local_profile_id).await?;
    Ok(own_state.and_then(|reduced| {
        reduced
            .posts
            .into_iter()
            .find(|post| post.post_id == post_id)
    }))
}

#[allow(clippy::too_many_arguments)]
async fn update_profile(
    state: &RuntimeState,
    event_tx: &Sender<NetworkEvent>,
    display_name: String,
    bio: String,
    default_visibility: Visibility,
    default_lifetime_secs: Option<u64>,
    mark_onboarded: bool,
) -> Result<()> {
    anyhow::ensure!(
        default_visibility != Visibility::Private,
        "the profile default cannot be private-only"
    );
    let profile = {
        let mut profile_store = state.profile.lock().await;
        profile_store.update(display_name, bio, default_visibility, default_lifetime_secs)?;
        if mark_onboarded {
            profile_store.mark_onboarded()?;
        }
        profile_store.profile().clone()
    };

    event_tx
        .send(NetworkEvent::ProfileLoaded {
            profile: profile.clone(),
        })
        .context("failed to send ProfileLoaded event")?;

    let mut sync = state.sync.lock().await;
    sync.publish(DomainOperation::ProfileUpdated {
        profile_id: profile.profile_id,
        display_name: profile.display_name,
        bio: profile.bio,
        default_visibility: profile.default_visibility,
        default_lifetime_secs: profile.default_lifetime_secs,
        created_at: profile.created_at,
        updated_at: profile.updated_at,
    })
    .await
}

async fn drain_expired(state: &RuntimeState, event_tx: &Sender<NetworkEvent>) -> Result<()> {
    let now = now_unix_secs();

    // GC's dead-post set, captured *before* teardown erases expired posts from
    // reduced state (ADR-0016): the pairs whose posts are tombstoned or expired
    // across everyone we know. Reaction reaping and bucket dropping both key off
    // this, so it must reflect posts that are about to be torn down this pass.
    let gc_profiles: Vec<String> = {
        let sync = state.sync.lock().await;
        let mut profiles = vec![state.local_profile_id.clone()];
        profiles.extend(sync.contact_profile_ids().await.unwrap_or_default());
        profiles.sort();
        profiles.dedup();
        profiles
    };
    let dead_targets = {
        let sync = state.sync.lock().await;
        sync.dead_post_targets(&gc_profiles, now)
            .await
            .unwrap_or_else(|err| {
                tracing::warn!("failed to compute GC dead-post set: {err:#}");
                Default::default()
            })
    };

    let drained = state.private_posts.drain_expired(now)?;
    if !drained.is_empty() {
        for post in &drained {
            unpin_and_prune_prefix(state, &format!("feed/{}/", post.post_id)).await;
        }
        event_tx
            .send(NetworkEvent::PrivatePostsUpdated {
                posts: state.private_posts.list()?,
            })
            .context("failed to send PrivatePostsUpdated event")?;
    }

    // Tear down the author's own *replicated* non-public posts that have
    // expired. Reduction and read-time filtering already hide them; this is the
    // side-effect that actually reclaims their media and purges their decrypted
    // text. Public posts (plaintext by design) and permanent posts are left
    // untouched. Running on startup too (via `recover_startup`) gives offline
    // convergence: a device that missed the expiry catches up on next launch.
    // Best-effort like the keep-lease block below: a read failure here must not
    // abort the whole drain and skip keep enforcement.
    let expired_own: Vec<String> = {
        let sync = state.sync.lock().await;
        match sync.read_profile_state(&state.local_profile_id).await {
            Ok(Some(own)) => own
                .posts
                .iter()
                .filter(|post| post.is_expired(now) && post.visibility != Visibility::Public)
                .map(|post| post.post_id.clone())
                .collect(),
            Ok(None) => Vec::new(),
            Err(err) => {
                tracing::warn!("failed to read local state for expiry teardown: {err:#}");
                Vec::new()
            }
        }
    };
    for post_id in &expired_own {
        teardown_feed_presence(state, post_id).await;
    }

    // Recipient side: tear down expired non-public posts we received from
    // friends and friends-of-friends, so expired content leaves this device
    // and stops being served — the piece that makes expiry reach the whole
    // network, not just the author. Best-effort per contact.
    let contacts = {
        let sync = state.sync.lock().await;
        sync.contact_profile_ids().await.unwrap_or_else(|err| {
            tracing::warn!("failed to list contacts for recipient teardown: {err:#}");
            Vec::new()
        })
    };
    // Blobs we hold under our own keeps must survive a received-post teardown;
    // collect their hashes once so the teardown can skip them.
    let kept_hashes: std::collections::HashSet<String> = state
        .keeps
        .list()
        .unwrap_or_default()
        .into_iter()
        .flat_map(|keep| {
            keep.snapshot
                .media
                .into_iter()
                .map(|attachment| attachment.blob_hash)
        })
        .collect();
    for contact_id in &contacts {
        let expired_received: Vec<crate::domain::ReducedPost> = {
            let sync = state.sync.lock().await;
            match sync.read_profile_state(contact_id).await {
                Ok(Some(contact)) => contact
                    .posts
                    .into_iter()
                    .filter(|post| post.is_expired(now) && post.visibility != Visibility::Public)
                    .collect(),
                Ok(None) => Vec::new(),
                Err(err) => {
                    tracing::warn!("failed to read {contact_id} for recipient teardown: {err:#}");
                    Vec::new()
                }
            }
        };
        for post in &expired_received {
            teardown_received_post(state, contact_id, post, &kept_hashes).await;
        }
    }

    // Enforce keep leases: a keep dies with its snapshot's lifetime and with
    // the author's tombstone.
    let keeps = state.keeps.list()?;
    if !keeps.is_empty() {
        let sync = state.sync.lock().await;
        let mut tombstoned: Vec<(String, String)> = Vec::new();
        for keep in &keeps {
            if let Ok(Some(author_state)) = sync.read_profile_state(&keep.author_profile_id).await {
                if author_state.is_tombstoned(&keep.post_id) {
                    tombstoned.push((keep.author_profile_id.clone(), keep.post_id.clone()));
                }
            }
        }
        drop(sync);
        let dead = state.keeps.prune_dead(now, |author, post_id| {
            tombstoned.iter().any(|(a, p)| a == author && p == post_id)
        })?;
        if !dead.is_empty() {
            for keep in &dead {
                unpin_and_prune_prefix(
                    state,
                    &format!("keep/{}/{}/", keep.author_profile_id, keep.post_id),
                )
                .await;
            }
            emit_keeps(state, event_tx)?;
        }
    }

    // GC of the co-deletion structure (ADR-0016), after per-post teardown:
    // (1) reap reactions whose target post is dead — on our own topic and each
    //     contact's, so both our reactions and received ones leave with the
    //     post they were on, even though they live in their own month buckets;
    // (2) drop fully-drained bucket logs whole — prune their operations and
    //     un-announce them so expired buckets stop syncing — then reclaim the
    //     pins and cache of anything they freed. Runs on the same drain path as
    //     everything else, so an offline device converges on next start.
    for holder in &gc_profiles {
        let reaped = {
            let sync = state.sync.lock().await;
            sync.reap_reactions_for_dead_targets(holder, &dead_targets)
                .await
        };
        if let Err(err) = reaped {
            tracing::warn!("failed to reap reactions on {holder}: {err:#}");
        }
    }

    for topic_profile_id in &gc_profiles {
        let freed = {
            let sync = state.sync.lock().await;
            sync.drop_drained_buckets(topic_profile_id, now, &dead_targets)
                .await
        };
        match freed {
            Ok(freed) => reclaim_dropped_content(state, freed).await,
            Err(err) => {
                tracing::warn!("failed to drop drained buckets on {topic_profile_id}: {err:#}")
            }
        }
    }

    Ok(())
}

/// Reclaims the blob pins and materialized cache of content whose bucket log GC
/// just dropped: unpins each freed post's `feed/…` namespace, and prunes the
/// cache of every freed blob and stops serving it (so a recipient's dropped
/// ciphertext leaves this device too). A blob still held by a keep survives —
/// its pin lives under a separate `keep/…` namespace. Best-effort.
async fn reclaim_dropped_content(state: &RuntimeState, freed: crate::domain::DrainedContent) {
    for post_id in &freed.post_ids {
        unpin_and_prune_prefix(state, &format!("feed/{post_id}/")).await;
    }
    for blob_hash in &freed.blob_hashes {
        crate::media::prune_cached(&state.media_cache_dir, blob_hash);
        if let Ok(hash) = blob_hash.parse::<p2panda_blobs::Hash>() {
            state.node.blobs.block_serving_hashes([hash]);
        }
    }
}

async fn collect_diagnostics_snapshot(state: &RuntimeState) -> Result<DiagnosticsSnapshot> {
    let node_ids = state
        .node
        .address_book
        .node_ids()
        .await
        .context("failed to list known peers")?;
    let local_id = state.node.node_id();
    let peers = node_ids
        .into_iter()
        .filter(|node_id| *node_id != local_id)
        .map(|node_id| PeerSnapshot {
            node_id: node_id.to_string(),
            state: PeerConnectionState::Known,
            discovered_via: PeerDiscoveryMethod::Unknown,
            last_seen_unix_ms: None,
            rtt_ms: None,
        })
        .collect();

    Ok(DiagnosticsSnapshot {
        captured_at_unix_ms: now_unix_ms(),
        node_identity: NodeIdentitySnapshot {
            node_id: local_id.to_string(),
            relay_url: state.node.relay_url.as_ref().map(|url| url.to_string()),
            local_listen_addrs: Vec::new(),
        },
        peers,
        connection_history: Vec::new(),
        error_log: Vec::new(),
        gossip_topics: Vec::new(),
    })
}

/// Generates a unique post id from the author, wall clock and a process-wide
/// counter (hash-truncated for readability in logs and debug output).
fn new_post_id(profile_id: &str, now_unix: u64) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    let seed = format!("{profile_id}/{now_unix}/{nanos}/{count}");
    let digest = p2panda_core::Hash::digest(seed.as_bytes()).to_string();
    digest.chars().take(32).collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anyhow::Result;

    use super::*;

    fn spawn_test_bridge<Worker>(worker: Worker) -> AsyncBridge
    where
        Worker: Fn(Arc<()>, NetworkCommand, Sender<NetworkEvent>) -> BoxFuture<Result<()>>
            + Send
            + Sync
            + 'static,
    {
        AsyncBridge::spawn_with_worker(|_events| Box::pin(async { Ok(()) }), worker).unwrap()
    }

    #[test]
    fn commands_sent_from_ui_thread_are_received_by_network_thread() {
        let bridge = spawn_test_bridge(|_, command, events| {
            Box::pin(async move {
                match command {
                    NetworkCommand::PublishPost { draft } => {
                        events
                            .send_async(NetworkEvent::PrivatePostsUpdated {
                                posts: vec![crate::domain::ReducedPost {
                                    profile_id: "test".into(),
                                    post_id: "echo".into(),
                                    body: draft.body,
                                    media: Vec::new(),
                                    visibility: draft.visibility,
                                    expires_at: None,
                                    created_at: 1,
                                    edited: false,
                                }],
                            })
                            .await?;
                    }
                    other => panic!("unexpected command: {other:?}"),
                }
                Ok(())
            })
        });

        bridge
            .send(NetworkCommand::PublishPost {
                draft: PostDraft {
                    body: "hello".into(),
                    visibility: Visibility::Private,
                    lifetime_secs: None,
                    media: Vec::new(),
                },
            })
            .unwrap();

        std::thread::sleep(Duration::from_millis(100));
        let events = bridge.drain_events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            NetworkEvent::PrivatePostsUpdated { posts } => {
                assert_eq!(posts[0].body, "hello");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn user_action_failures_surface_as_error_events() {
        let bridge = spawn_test_bridge(|_, command, _| {
            Box::pin(async move {
                match command {
                    NetworkCommand::DeletePost { .. } => anyhow::bail!("boom"),
                    NetworkCommand::RequestDiagnostics => anyhow::bail!("quiet failure"),
                    other => panic!("unexpected command: {other:?}"),
                }
            })
        });

        // A failing user action produces an Error event...
        bridge
            .send(NetworkCommand::DeletePost {
                post_id: "p".into(),
            })
            .unwrap();
        // ...a failing background command does not.
        bridge.send(NetworkCommand::RequestDiagnostics).unwrap();

        std::thread::sleep(Duration::from_millis(100));
        let events = bridge.drain_events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            NetworkEvent::Error {
                context,
                error_message,
            } => {
                assert_eq!(context, "deleting the post");
                assert_eq!(error_message, "boom");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn post_ids_are_unique() {
        let first = new_post_id("profile", 1);
        let second = new_post_id("profile", 1);
        assert_ne!(first, second);
        assert_eq!(first.len(), 32);
    }
}

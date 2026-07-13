//! The headless app runtime behind the Flutter API: owns the network bridge
//! and derives UI-ready state from its events, replacing the former Bevy
//! plugin's per-frame systems with a pump thread.
//!
//! Flow: [`NetworkEvent`]s are applied to [`RiverState`] and friend/profile
//! bookkeeping, then pushed to Dart as [`JynEvent`] snapshots. Commands go
//! the other way through [`AsyncBridge::send_awaited`], so every user action
//! resolves (or throws) at its call site in Dart.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::app_config::{resolve_data_dir, resolve_node_options};
use crate::bridge::{AsyncBridge, NetworkCommand, NetworkEvent};
use crate::diagnostics::DiagnosticsSnapshot;
use crate::domain::PendingFriendRequest;
use crate::groups::{GroupSuggestion, GroupView, GroupViewerStatus};
use crate::media::MediaCache;
use crate::notifications::NotificationState;
use crate::profile::{now_unix_secs, UserProfile};
use crate::settings::{AppSettings, SettingsStore};
use crate::state::{GhostCard, GroupDiscoveryCard, RiverPost, RiverState};

const DIAGNOSTIC_POLL_INTERVAL: Duration = Duration::from_secs(1);
const EXPIRY_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const PUMP_RECV_TIMEOUT: Duration = Duration::from_millis(200);

/// Everything pushed up to the Flutter side. Each variant is a full snapshot
/// of its slice of state, so Dart providers can replace rather than patch.
#[derive(Debug, Clone)]
pub enum JynEvent {
    /// The materialized feed: alive posts newest-first, discovery ghosts
    /// (doors to authors we don't follow yet), one digest door per
    /// member-group with new activity (ADR-0010), and friends' heart-driven
    /// group discovery cards (ADR-0009).
    River {
        posts: Vec<RiverPost>,
        ghosts: Vec<GhostCard>,
        doors: Vec<GroupDigestDoor>,
        group_cards: Vec<GroupDiscoveryCard>,
    },
    /// One group's state changed, as this viewer may see it. Dart folds
    /// these into the Groups hub and the group place screens.
    Group {
        view: GroupView,
    },
    /// The hub's friend-based suggestions (full snapshot, ADR-0012).
    GroupSuggestions {
        suggestions: Vec<GroupSuggestion>,
    },
    /// The local profile (onboarding state, composer defaults, name).
    Profile {
        profile: UserProfile,
    },
    /// Friends and pending incoming requests.
    Friends {
        friends: Vec<FriendEntry>,
        pending: Vec<PendingFriendRequest>,
    },
    Diagnostics {
        snapshot: DiagnosticsSnapshot,
    },
    /// A fetched (or freshly cast) attachment blob is available locally.
    MediaReady {
        blob_hash: String,
        path: String,
    },
    MediaFailed {
        blob_hash: String,
        error_message: String,
    },
    /// A background failure worth showing (user-action failures throw at
    /// their Dart call sites instead).
    Error {
        context: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendEntry {
    pub profile_id: String,
    pub display_name: String,
    /// Whether the friendship is mutual yet.
    pub follows_me_back: bool,
}

/// A single river entry per member-group with new activity, opening the
/// group place — group posts never interleave individually (ADR-0010).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDigestDoor {
    pub group_id: String,
    pub name: String,
    /// Sorts the door into the reverse-chron river.
    pub latest_activity_at: u64,
}

/// Where runtime events are delivered. The Flutter API installs a stream
/// sink behind this; tests install a channel.
pub trait EventSink: Send + Sync {
    /// Returns false when the far side is gone.
    fn push(&self, event: JynEvent) -> bool;
}

impl EventSink for flume::Sender<JynEvent> {
    fn push(&self, event: JynEvent) -> bool {
        self.send(event).is_ok()
    }
}

struct Shared {
    river: RiverState,
    profile: Option<UserProfile>,
    follow_back_sent: HashSet<String>,
    media: MediaCache,
    notifications: NotificationState,
    /// Latest viewer-filtered state per known group.
    groups: HashMap<String, GroupView>,
    /// Group activity changed since the last river snapshot, so the digest
    /// doors need re-deriving even though no river source is dirty.
    doors_dirty: bool,
    sink: Option<Box<dyn EventSink>>,
    /// Events that arrived before Dart attached its sink.
    pending: Vec<JynEvent>,
}

impl Shared {
    fn emit(&mut self, event: JynEvent) {
        match &self.sink {
            Some(sink) => {
                if !sink.push(event) {
                    self.sink = None;
                }
            }
            None => self.pending.push(event),
        }
    }
}

pub struct AppRuntime {
    bridge: AsyncBridge,
    shared: Arc<Mutex<Shared>>,
    settings: Mutex<SettingsStore>,
    data_dir: PathBuf,
    app_focused: Arc<AtomicBool>,
}

static RUNTIME: OnceLock<AppRuntime> = OnceLock::new();

impl AppRuntime {
    /// The process-wide runtime, starting the node on first use.
    /// `data_dir_override` only affects the first call.
    pub fn get_or_start(data_dir_override: Option<PathBuf>) -> Result<&'static AppRuntime> {
        if let Some(runtime) = RUNTIME.get() {
            return Ok(runtime);
        }
        let runtime = Self::start(data_dir_override)?;
        Ok(RUNTIME.get_or_init(|| runtime))
    }

    pub fn get() -> Result<&'static AppRuntime> {
        RUNTIME
            .get()
            .context("the jyn runtime has not been started yet")
    }

    fn start(data_dir_override: Option<PathBuf>) -> Result<Self> {
        let data_dir = match data_dir_override {
            Some(dir) => dir,
            None => resolve_data_dir().context("failed to resolve jyn data directory")?,
        };
        crate::data_schema::ensure_data_schema(&data_dir)
            .context("failed to verify on-disk data schema version")?;
        let settings_store =
            SettingsStore::load(&data_dir).context("failed to load app settings from disk")?;
        let node_options = resolve_node_options(settings_store.settings())
            .context("failed to resolve node options from settings")?;
        let bridge = AsyncBridge::spawn_with_data_dir(node_options, data_dir.clone())
            .context("failed to initialize async bridge")?;

        let shared = Arc::new(Mutex::new(Shared {
            river: RiverState::default(),
            profile: None,
            follow_back_sent: HashSet::new(),
            media: MediaCache::new(&data_dir),
            notifications: NotificationState::default(),
            groups: HashMap::new(),
            doors_dirty: false,
            sink: None,
            pending: Vec::new(),
        }));

        let runtime = Self {
            bridge,
            shared,
            settings: Mutex::new(settings_store),
            data_dir,
            app_focused: Arc::new(AtomicBool::new(true)),
        };
        runtime.spawn_pump();
        Ok(runtime)
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    pub fn set_app_focused(&self, focused: bool) {
        self.app_focused.store(focused, Ordering::Relaxed);
    }

    /// Installs the sink events flow into, flushing anything buffered while
    /// Dart was still attaching.
    pub fn set_event_sink(&self, sink: Box<dyn EventSink>) {
        let mut shared = self.lock_shared();
        for event in std::mem::take(&mut shared.pending) {
            if !sink.push(event) {
                return;
            }
        }
        shared.sink = Some(sink);
    }

    /// Fire-and-forget send; failures of user actions come back as error
    /// events. Prefer [`Self::run_command`] for anything user-initiated.
    pub fn send_command(&self, command: NetworkCommand) -> Result<()> {
        self.bridge.send(command)
    }

    /// Sends a command and resolves with its outcome.
    pub async fn run_command(&self, command: NetworkCommand) -> Result<()> {
        let receiver = self.bridge.send_awaited(command)?;
        let outcome = receiver
            .await
            .context("the network runtime dropped the command")?;
        outcome.map_err(|message| anyhow::anyhow!(message))
    }

    pub fn settings(&self) -> AppSettings {
        self.lock_settings().settings().clone()
    }

    pub fn with_settings_store<T>(&self, apply: impl FnOnce(&mut SettingsStore) -> T) -> T {
        apply(&mut self.lock_settings())
    }

    /// The encoded `jyn-` friend code for the local profile, once loaded.
    pub fn my_friend_code(&self) -> Result<String> {
        let profile = self
            .lock_shared()
            .profile
            .clone()
            .context("profile not loaded yet")?;
        let key = profile
            .profile_id
            .parse()
            .map_err(|_| anyhow::anyhow!("own profile id is not a valid public key"))?;
        let relay_url = self
            .lock_settings()
            .settings()
            .relay_url_for_node()?
            .map(|url| url.to_string());
        crate::friend_code::FriendCode::new(key, relay_url, profile.display_name).encode()
    }

    /// The local file for a blob if present in the media cache.
    pub fn local_media_path(&self, blob_hash: &str) -> Option<PathBuf> {
        self.lock_shared().media.local_path_for(blob_hash)
    }

    /// Asks the runtime to fetch a blob unless it's local or a fetch is
    /// already in flight; completion arrives as [`JynEvent::MediaReady`] /
    /// [`JynEvent::MediaFailed`].
    pub fn request_media(&self, blob_hash: String) -> Result<()> {
        {
            let mut shared = self.lock_shared();
            // Already local (fetched earlier, or cached when we cast it):
            // Dart's mediaPaths map is empty on a fresh launch, so re-emit
            // MediaReady to hand it the path. Returning silently would leave
            // the attachment stuck on its loading placeholder forever.
            if let Some(path) = shared.media.local_path_for(&blob_hash) {
                shared.emit(JynEvent::MediaReady {
                    blob_hash,
                    path: path.to_string_lossy().into_owned(),
                });
                return Ok(());
            }
            if !shared.media.fetch_requested.insert(blob_hash.clone()) {
                return Ok(());
            }
        }
        self.bridge.send(NetworkCommand::FetchMedia { blob_hash })
    }

    fn lock_shared(&self) -> MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_settings(&self) -> MutexGuard<'_, SettingsStore> {
        self.settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The pump replaces the Bevy `Update` systems: it applies network
    /// events, re-materializes the river on changes and expiries, keeps
    /// diagnostics fresh, and forwards everything to Dart.
    fn spawn_pump(&self) {
        let shared = Arc::clone(&self.shared);
        let events = self.bridge.event_receiver();
        let commands = self.bridge.command_sender();
        let app_focused = Arc::clone(&self.app_focused);

        std::thread::Builder::new()
            .name("jyn-pump".into())
            .spawn(move || {
                let send_command = |command: NetworkCommand| {
                    let _ = commands.send((command, None));
                };
                let mut last_diagnostics_request = Instant::now() - DIAGNOSTIC_POLL_INTERVAL;
                let mut last_expiry_check = Instant::now() - EXPIRY_CHECK_INTERVAL;

                loop {
                    let event = match events.recv_timeout(PUMP_RECV_TIMEOUT) {
                        Ok(event) => Some(event),
                        Err(flume::RecvTimeoutError::Timeout) => None,
                        Err(flume::RecvTimeoutError::Disconnected) => break,
                    };

                    let now = Instant::now();
                    {
                        let mut guard = shared
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if let Some(event) = event {
                            let focused = app_focused.load(Ordering::Relaxed);
                            apply_event(&mut guard, event, focused, now, &send_command);
                            // Apply whatever arrived in the same burst before
                            // re-materializing once.
                            while let Ok(event) = events.try_recv() {
                                apply_event(&mut guard, event, focused, now, &send_command);
                            }
                        }

                        let expiry_check_due =
                            now.duration_since(last_expiry_check) >= EXPIRY_CHECK_INTERVAL;
                        if expiry_check_due {
                            last_expiry_check = now;
                        }
                        refresh_river(&mut guard, expiry_check_due, &send_command);
                    }

                    if now.duration_since(last_diagnostics_request) >= DIAGNOSTIC_POLL_INTERVAL {
                        last_diagnostics_request = now;
                        send_command(NetworkCommand::RequestDiagnostics);
                    }
                }
                tracing::info!("jyn pump thread exits: network runtime is gone");
            })
            .expect("failed to spawn jyn pump thread");
    }
}

/// Applies one network event to the derived state and forwards snapshots to
/// Dart. Ported from the Bevy plugin's `poll_network_events` +
/// `apply_network_event`.
fn apply_event(
    shared: &mut Shared,
    event: NetworkEvent,
    app_focused: bool,
    now: Instant,
    send_command: &impl Fn(NetworkCommand),
) {
    shared.notifications.on_event(&event, app_focused, now);

    match event {
        NetworkEvent::MediaReady { blob_hash, path } => {
            shared
                .media
                .record_local_path(blob_hash.clone(), path.clone());
            shared.emit(JynEvent::MediaReady {
                blob_hash,
                path: path.to_string_lossy().into_owned(),
            });
        }
        NetworkEvent::MediaFailed {
            blob_hash,
            error_message,
        } => {
            tracing::debug!("media fetch failed for {blob_hash}: {error_message}");
            shared.media.fetch_requested.remove(&blob_hash);
            shared.emit(JynEvent::MediaFailed {
                blob_hash,
                error_message,
            });
        }
        NetworkEvent::DiagnosticsSnapshot { snapshot } => {
            shared.emit(JynEvent::Diagnostics { snapshot });
        }
        NetworkEvent::ProfileLoaded { profile } => {
            shared
                .river
                .set_own_display_name(profile.display_name.clone());
            shared.profile = Some(profile.clone());
            shared.emit(JynEvent::Profile { profile });
        }
        NetworkEvent::LocalStateUpdated { state } => {
            shared.river.apply_local_state(state);
            emit_friends(shared);
            // Friends list may have changed; keep encryption membership in
            // step. Idempotent, so over-triggering is harmless.
            send_command(NetworkCommand::ReconcileSpaces);
        }
        NetworkEvent::PrivatePostsUpdated { posts } => {
            shared.river.apply_private_posts(posts);
        }
        NetworkEvent::KeepsUpdated { keeps } => {
            shared.river.apply_keeps(keeps);
        }
        NetworkEvent::ContactStateUpdated { profile_id, state } => {
            // A contact we requested friendship from now follows us: their
            // acceptance. Follow back once to make the friendship mutual.
            let accepted_us = shared
                .profile
                .as_ref()
                .map(|profile| state.followed_profile_ids.contains(&profile.profile_id))
                .unwrap_or(false);
            if accepted_us
                && !shared.river.follows(&profile_id)
                && shared.follow_back_sent.insert(profile_id.clone())
            {
                send_command(NetworkCommand::FollowBack {
                    profile_id: profile_id.clone(),
                });
            }
            shared.river.apply_contact_state(profile_id, state);
            emit_friends(shared);
            // A contact update can carry their key bundle, unblocking their
            // pending addition to the friends space.
            send_command(NetworkCommand::ReconcileSpaces);
        }
        NetworkEvent::GroupUpdated { view } => {
            shared.groups.insert(view.group_id.clone(), view.clone());
            shared.doors_dirty = true;
            shared.emit(JynEvent::Group { view });
        }
        NetworkEvent::GroupSuggestionsUpdated { suggestions } => {
            shared.emit(JynEvent::GroupSuggestions { suggestions });
        }
        NetworkEvent::Error {
            context,
            error_message,
        } => {
            shared.emit(JynEvent::Error {
                context,
                message: error_message,
            });
        }
    }
}

/// Re-materializes and pushes the river when sources changed or (at most
/// once per [`EXPIRY_CHECK_INTERVAL`]) when a lifetime ran out. Ported from
/// the Bevy plugin's `tick_river`.
fn refresh_river(
    shared: &mut Shared,
    expiry_check_due: bool,
    send_command: &impl Fn(NetworkCommand),
) {
    let now = now_unix_secs();
    let expired = expiry_check_due && shared.river.expiry_due(now);
    if !shared.river.is_dirty() && !expired && !shared.doors_dirty {
        return;
    }
    if shared.river.is_dirty() || expired {
        shared.river.materialize(now);
    }
    shared.doors_dirty = false;
    if expired {
        send_command(NetworkCommand::DrainExpired);
    }
    let posts = shared.river.river.clone();
    let ghosts = shared.river.ghosts.clone();
    let doors = digest_doors(&shared.groups);
    // A discovery card for a group the viewer already belongs to is just
    // noise — their river has the digest door instead.
    let group_cards: Vec<GroupDiscoveryCard> = shared
        .river
        .group_cards
        .iter()
        .filter(|card| {
            !shared.groups.get(&card.group_id).is_some_and(|view| {
                matches!(
                    view.viewer_status,
                    GroupViewerStatus::Owner | GroupViewerStatus::Member
                )
            })
        })
        .cloned()
        .collect();
    shared.emit(JynEvent::River {
        posts,
        ghosts,
        doors,
        group_cards,
    });
}

/// One digest door per member-group with activity newer than the viewer's
/// last visit, sorted into the reverse-chron river by recency (ADR-0010).
/// A river door requires membership — visited public groups get none.
fn digest_doors(groups: &HashMap<String, GroupView>) -> Vec<GroupDigestDoor> {
    let mut doors: Vec<GroupDigestDoor> = groups
        .values()
        .filter(|view| {
            matches!(
                view.viewer_status,
                GroupViewerStatus::Owner | GroupViewerStatus::Member
            ) && view.has_new_activity
        })
        .map(|view| GroupDigestDoor {
            group_id: view.group_id.clone(),
            name: view.name.clone(),
            latest_activity_at: view.latest_activity_at,
        })
        .collect();
    doors.sort_by(|left, right| {
        right
            .latest_activity_at
            .cmp(&left.latest_activity_at)
            .then_with(|| left.group_id.cmp(&right.group_id))
    });
    doors
}

/// Derives the friends-screen data from the local reduced state: everyone we
/// follow, with mutuality, plus pending incoming requests.
fn emit_friends(shared: &mut Shared) {
    let Some(own) = shared.river.own_state() else {
        return;
    };
    let my_id = own.profile_id.clone();
    let pending = own.pending_requests.clone();
    let friends: Vec<FriendEntry> = own
        .followed_profile_ids
        .clone()
        .into_iter()
        .map(|profile_id| {
            let display_name = shared.river.contact_display_name(&profile_id);
            let follows_me_back = shared
                .river
                .contact_state(&profile_id)
                .map(|state| state.followed_profile_ids.contains(&my_id))
                .unwrap_or(false);
            FriendEntry {
                profile_id,
                display_name,
                follows_me_back,
            }
        })
        .collect();
    shared.emit(JynEvent::Friends { friends, pending });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ReducedProfileState, Visibility};

    fn test_shared(sink: flume::Sender<JynEvent>) -> Shared {
        let dir = std::env::temp_dir().join("jyn-runtime-test");
        Shared {
            river: RiverState::default(),
            profile: None,
            follow_back_sent: HashSet::new(),
            media: MediaCache::new(&dir),
            notifications: NotificationState::default(),
            groups: HashMap::new(),
            doors_dirty: false,
            sink: Some(Box::new(sink)),
            pending: Vec::new(),
        }
    }

    fn reduced_state(profile_id: &str, followed: Vec<String>) -> ReducedProfileState {
        ReducedProfileState {
            profile_id: profile_id.into(),
            display_name: Some(format!("{profile_id}-name")),
            bio: String::new(),
            default_visibility: Visibility::Friends,
            default_lifetime_secs: None,
            posts: Vec::new(),
            followed_profile_ids: followed,
            hearts: Vec::new(),
            comments: Vec::new(),
            pending_requests: Vec::new(),
            tombstoned_post_ids: Vec::new(),
            advertised_groups: Vec::new(),
        }
    }

    fn own_profile(profile_id: &str) -> UserProfile {
        UserProfile {
            version: 1,
            profile_id: profile_id.into(),
            display_name: "Me".into(),
            bio: String::new(),
            default_visibility: Visibility::Friends,
            default_lifetime_secs: None,
            onboarded: true,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn accepted_request_triggers_exactly_one_follow_back() {
        let (event_tx, _event_rx) = flume::unbounded();
        let mut shared = test_shared(event_tx);
        let sent = std::sync::Mutex::new(Vec::new());
        let send_command = |command: NetworkCommand| sent.lock().unwrap().push(command);

        apply_event(
            &mut shared,
            NetworkEvent::ProfileLoaded {
                profile: own_profile("me"),
            },
            true,
            Instant::now(),
            &send_command,
        );
        apply_event(
            &mut shared,
            NetworkEvent::LocalStateUpdated {
                state: reduced_state("me", vec![]),
            },
            true,
            Instant::now(),
            &send_command,
        );

        // anna (whom we requested) now follows us: exactly one follow-back,
        // even across a burst of her contact updates.
        for _ in 0..3 {
            apply_event(
                &mut shared,
                NetworkEvent::ContactStateUpdated {
                    profile_id: "anna".into(),
                    state: reduced_state("anna", vec!["me".into()]),
                },
                true,
                Instant::now(),
                &send_command,
            );
        }
        let commands = sent.lock().unwrap();
        let follow_backs: Vec<_> = commands
            .iter()
            .filter(|command| matches!(command, NetworkCommand::FollowBack { .. }))
            .collect();
        assert_eq!(
            follow_backs.as_slice(),
            &[&NetworkCommand::FollowBack {
                profile_id: "anna".into()
            }]
        );
    }

    #[test]
    fn friends_snapshot_carries_names_and_mutuality() {
        let (event_tx, event_rx) = flume::unbounded();
        let mut shared = test_shared(event_tx);
        let send_command = |_: NetworkCommand| {};

        apply_event(
            &mut shared,
            NetworkEvent::ContactStateUpdated {
                profile_id: "anna".into(),
                state: reduced_state("anna", vec!["me".into()]),
            },
            true,
            Instant::now(),
            &send_command,
        );
        apply_event(
            &mut shared,
            NetworkEvent::LocalStateUpdated {
                state: reduced_state("me", vec!["anna".into(), "bob".into()]),
            },
            true,
            Instant::now(),
            &send_command,
        );

        let friends = event_rx
            .drain()
            .filter_map(|event| match event {
                JynEvent::Friends { friends, .. } => Some(friends),
                _ => None,
            })
            .last()
            .expect("a friends snapshot");
        assert_eq!(
            friends,
            vec![
                FriendEntry {
                    profile_id: "anna".into(),
                    display_name: "anna-name".into(),
                    follows_me_back: true,
                },
                FriendEntry {
                    profile_id: "bob".into(),
                    // No synced state for bob yet: short id fallback.
                    display_name: "bob".into(),
                    follows_me_back: false,
                },
            ]
        );
    }

    #[test]
    fn river_refresh_emits_snapshot_and_drains_on_expiry() {
        let (event_tx, event_rx) = flume::unbounded();
        let mut shared = test_shared(event_tx);
        let sent = std::sync::Mutex::new(Vec::new());
        let send_command = |command: NetworkCommand| sent.lock().unwrap().push(command);

        let now = now_unix_secs();
        let mut state = reduced_state("me", vec![]);
        state.posts.push(crate::domain::ReducedPost {
            profile_id: "me".into(),
            post_id: "p1".into(),
            body: "hi".into(),
            media: Vec::new(),
            visibility: Visibility::Friends,
            expires_at: Some(now.saturating_sub(1)),
            created_at: now.saturating_sub(10),
            edited: false,
        });
        apply_event(
            &mut shared,
            NetworkEvent::LocalStateUpdated { state },
            true,
            Instant::now(),
            &send_command,
        );

        refresh_river(&mut shared, true, &send_command);
        let river = event_rx
            .drain()
            .filter_map(|event| match event {
                JynEvent::River { posts, .. } => Some(posts),
                _ => None,
            })
            .last()
            .expect("a river snapshot");
        // The already-expired post never surfaces.
        assert!(river.is_empty());
    }
}

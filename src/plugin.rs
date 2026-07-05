use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bevy::app::Plugin;
use bevy::prelude::{
    App, IntoScheduleConfigs, Local, NonSendMut, Query, Res, ResMut, Update, With,
};
use bevy::window::{PrimaryWindow, Window};
use bevy_egui::{egui, EguiContexts};
use directories::ProjectDirs;
use flume::TryRecvError;

use crate::bridge::{AsyncBridge, NetworkCommand, NetworkEvent};
use crate::media::MediaState;
use crate::node::NodeOptions;
use crate::notifications::NotificationState;
use crate::profile::now_unix_secs;
use crate::render::{sync_card_effects, WaterRenderPlugin};
use crate::settings::{AppSettings, SettingsStore};
use crate::state::RiverState;
use crate::ui::{ui_system, UiState};

pub struct JynPlugin;
const INSECURE_SKIP_RELAY_CERT_VERIFY_ENV: &str = "JYN_INSECURE_SKIP_RELAY_CERT_VERIFY";
const DIAGNOSTIC_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(bevy::prelude::Resource)]
pub(crate) struct DiagnosticsPollState {
    last_requested_at: Instant,
}

#[derive(bevy::prelude::Resource)]
struct PluginStartupError {
    message: String,
}

struct PluginResources {
    bridge: AsyncBridge,
    settings_store: SettingsStore,
    data_dir: std::path::PathBuf,
}

impl Default for DiagnosticsPollState {
    fn default() -> Self {
        Self {
            last_requested_at: Instant::now() - DIAGNOSTIC_POLL_INTERVAL,
        }
    }
}

impl Plugin for JynPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(WaterRenderPlugin);
        match initialize_plugin_resources() {
            Ok(resources) => {
                app.insert_non_send_resource(MediaState::new(&resources.data_dir));
                app.insert_resource(resources.bridge);
                app.insert_resource(resources.settings_store);
                app.insert_resource(UiState::default());
                app.insert_resource(RiverState::default());
                app.insert_resource(NotificationState::default());
                app.insert_resource(DiagnosticsPollState::default());
                app.add_systems(
                    Update,
                    (
                        poll_network_events,
                        tick_river,
                        ui_system,
                        sync_card_effects,
                    )
                        .chain(),
                );
            }
            Err(err) => {
                tracing::error!("failed to initialize jyn plugin: {err:#}");
                app.insert_resource(PluginStartupError {
                    message: err.to_string(),
                });
                app.add_systems(Update, (render_startup_error_ui, sync_card_effects).chain());
            }
        }
    }
}

fn initialize_plugin_resources() -> Result<PluginResources> {
    let data_dir = resolve_data_dir().context("failed to resolve jyn data directory")?;
    crate::data_schema::ensure_data_schema(&data_dir)
        .context("failed to verify on-disk data schema version")?;
    let settings_store =
        SettingsStore::load(&data_dir).context("failed to load app settings from disk")?;
    let settings = settings_store.settings().clone();
    let node_options =
        resolve_node_options(&settings).context("failed to resolve node options from settings")?;
    let bridge = AsyncBridge::spawn_with_data_dir(node_options, data_dir.clone())
        .context("failed to initialize async bridge")?;

    Ok(PluginResources {
        bridge,
        settings_store,
        data_dir,
    })
}

/// Data directory for the app. `JYN_DATA_DIR` overrides the platform
/// default so multiple instances can run side by side during development.
pub(crate) fn resolve_data_dir() -> Result<std::path::PathBuf> {
    const APP_NAME: &str = "jyn";
    if let Ok(dir) = std::env::var("JYN_DATA_DIR") {
        return Ok(std::path::PathBuf::from(dir));
    }
    ProjectDirs::from("", "", APP_NAME)
        .map(|dirs| dirs.data_dir().to_path_buf())
        .with_context(|| format!("failed to resolve app data directory for {APP_NAME}"))
}

fn resolve_node_options(settings: &AppSettings) -> Result<NodeOptions> {
    let relay_url = settings.relay_url_for_node()?;

    let insecure_skip_relay_cert_verify = std::env::var(INSECURE_SKIP_RELAY_CERT_VERIFY_ENV)
        .ok()
        .map(|value| parse_bool_env_var(&value, INSECURE_SKIP_RELAY_CERT_VERIFY_ENV))
        .transpose()?
        .unwrap_or(false);

    Ok(NodeOptions {
        relay_url,
        mdns_enabled: settings.mdns_enabled,
        insecure_skip_relay_cert_verify,
    })
}

fn parse_bool_env_var(value: &str, name: &str) -> Result<bool> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!(
            "invalid boolean value for {name}: {value} (expected true/false, 1/0, yes/no, on/off)"
        ),
    }
}

fn render_startup_error_ui(
    mut egui_contexts: EguiContexts,
    startup_error: Res<PluginStartupError>,
) {
    let ctx = egui_contexts.ctx_mut();
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("Startup Error");
        ui.separator();
        ui.label("The jyn runtime failed to initialize.");
        ui.label("Check logs for details, then restart the app after fixing the issue.");
        ui.add_space(8.0);
        ui.monospace(&startup_error.message);
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn poll_network_events(
    bridge: Res<AsyncBridge>,
    mut river: ResMut<RiverState>,
    mut ui_state: ResMut<UiState>,
    mut notifications: ResMut<NotificationState>,
    mut media: NonSendMut<MediaState>,
    diagnostics_poll_state: Option<ResMut<DiagnosticsPollState>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
) {
    let app_focused = primary_window
        .iter()
        .next()
        .map(|window| window.focused)
        .unwrap_or(true);
    let now = Instant::now();
    if let Some(mut diagnostics_poll_state) = diagnostics_poll_state {
        if now.duration_since(diagnostics_poll_state.last_requested_at) >= DIAGNOSTIC_POLL_INTERVAL
            && bridge.send(NetworkCommand::RequestDiagnostics).is_ok()
        {
            diagnostics_poll_state.last_requested_at = now;
        }
    }

    loop {
        match bridge.try_recv() {
            Ok(Some(event)) => {
                notifications.on_event(&event, app_focused, now);
                match event {
                    NetworkEvent::MediaReady { blob_hash, path } => {
                        media.record_local_path(blob_hash, path);
                    }
                    NetworkEvent::MediaFailed {
                        blob_hash,
                        error_message,
                    } => {
                        tracing::debug!("media fetch failed for {blob_hash}: {error_message}");
                        media.fetch_requested.remove(&blob_hash);
                    }
                    event => {
                        if let Some(reaction) =
                            apply_network_event(&mut river, &mut ui_state, event)
                        {
                            let _ = bridge.send(reaction);
                        }
                    }
                }
            }
            Ok(None) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => break,
        }
    }
}

fn apply_network_event(
    river: &mut RiverState,
    ui_state: &mut UiState,
    event: NetworkEvent,
) -> Option<NetworkCommand> {
    match event {
        NetworkEvent::DiagnosticsSnapshot { snapshot } => {
            ui_state.peers_known = snapshot.peers.len();
        }
        NetworkEvent::ProfileLoaded { profile } => {
            river.set_own_display_name(profile.display_name.clone());
            ui_state.apply_profile(profile);
        }
        NetworkEvent::LocalStateUpdated { state } => {
            river.apply_local_state(state);
        }
        NetworkEvent::PrivatePostsUpdated { posts } => {
            river.apply_private_posts(posts);
        }
        NetworkEvent::KeepsUpdated { keeps } => {
            river.apply_keeps(keeps);
        }
        NetworkEvent::ContactStateUpdated { profile_id, state } => {
            // A contact we requested friendship from now follows us: their
            // acceptance. Follow back once to make the friendship mutual.
            let accepted_us = ui_state
                .profile
                .as_ref()
                .map(|profile| state.followed_profile_ids.contains(&profile.profile_id))
                .unwrap_or(false);
            let reaction = if accepted_us
                && !river.follows(&profile_id)
                && ui_state.mark_follow_back_sent(&profile_id)
            {
                Some(NetworkCommand::FollowBack {
                    profile_id: profile_id.clone(),
                })
            } else {
                None
            };
            river.apply_contact_state(profile_id, state);
            return reaction;
        }
        NetworkEvent::Error {
            context,
            error_message,
        } => {
            ui_state.push_error(format!("{context}: {error_message}"));
        }
        // Handled in poll_network_events, where MediaState is available.
        NetworkEvent::MediaReady { .. } | NetworkEvent::MediaFailed { .. } => {}
    }
    None
}

/// Rebuilds the river when its sources changed or a lifetime ran out, and
/// asks the runtime to drain expired private posts from disk. Runs its
/// expiry check at most once per second; countdown pills tick per-frame in
/// the renderer from `expires_at` directly.
pub(crate) fn tick_river(
    bridge: Res<AsyncBridge>,
    mut river: ResMut<RiverState>,
    mut last_expiry_check: Local<Option<Instant>>,
) {
    let now_wall = now_unix_secs();

    if river.is_dirty() {
        river.materialize(now_wall);
        return;
    }

    let now = Instant::now();
    let due = last_expiry_check
        .map(|last| now.duration_since(last) >= Duration::from_secs(1))
        .unwrap_or(true);
    if !due {
        return;
    }
    *last_expiry_check = Some(now);

    if river.expiry_due(now_wall) {
        river.materialize(now_wall);
        let _ = bridge.send(NetworkCommand::DrainExpired);
    }
}

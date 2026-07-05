//! The egui layer: home river, inline composer and onboarding, rendered
//! transparently over the Bevy water underlay. Functional layout in this
//! milestone; the full "Current & Still" treatment lands with the visual
//! layer milestone.

use std::collections::{HashSet, VecDeque};

use bevy::prelude::{NonSendMut, Res, ResMut, Resource};
use bevy_egui::{egui, EguiContexts};

use crate::bridge::{AsyncBridge, MediaDraft, NetworkCommand, PostDraft};
use crate::domain::{MediaAttachment, MediaKind, Visibility};
use crate::friend_code::FriendCode;
use crate::media::{self, MediaState};
use crate::profile::{now_unix_secs, UserProfile};
use crate::render::{CardEffect, CardEffects};
use crate::settings::SettingsStore;
use crate::state::{RiverPost, RiverState};
use crate::time_format::{format_remaining, UrgencyTier};

pub mod theme;

/// Left inset of the content column, clearing the current spine (design: 58px).
const COLUMN_LEFT_PX: f32 = 58.0;
const COLUMN_RIGHT_PX: f32 = 16.0;
/// The river is a single column (the design is 508px wide); cap it so a wide
/// window shows a readable column rather than sparse full-width rows.
const MAX_APP_WIDTH: f32 = 640.0;
const ERROR_LOG_LIMIT: usize = 3;

/// Bounds a scroll area's content to the actually-visible viewport width,
/// capped at [`MAX_APP_WIDTH`]. Without this, a stale content-width estimate
/// (egui reports a placeholder 5000px screen on the very first frame, before
/// the window size is known) leaves `available_width()` over-reported for the
/// life of the scroll area, and rows overflow the right edge with no
/// horizontal scrollbar to reach them. `clip_rect` is the real viewport and
/// is immune to that cache.
fn constrain_column(ui: &mut egui::Ui) {
    ui.set_width(ui.clip_rect().width().min(MAX_APP_WIDTH));
}

/// Lays out an inset text/content block bounded to the column width, so long
/// labels wrap instead of stretching the whole column past the window. A bare
/// `ui.horizontal(|ui| { ui.add_space(58); <label> })` doesn't bound its
/// children's natural width — nested containers and non-wrapping labels grow
/// `min_rect` past the column and clip off the right edge.
fn inset_block(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let width = row_content_width(ui);
    ui.horizontal(|ui| {
        ui.add_space(COLUMN_LEFT_PX);
        ui.allocate_ui_with_layout(
            egui::vec2(width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(width);
                add(ui);
            },
        );
    });
}

/// The width available to a card/composer frame, measured in the *vertical*
/// context before entering a row's `ui.horizontal`. A non-wrapping horizontal
/// layout hands its children an effectively-infinite width, so
/// `available_width()` read inside one is meaningless — always compute the
/// column width out here and pass it in.
fn row_content_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() - COLUMN_LEFT_PX - COLUMN_RIGHT_PX - 3.0).max(120.0)
}

/// Composer lifetime presets (label, seconds; `None` = permanent).
const LIFETIME_OPTIONS: &[(&str, Option<u64>)] = &[
    ("1h", Some(3600)),
    ("12h", Some(12 * 3600)),
    ("36h", Some(36 * 3600)),
    ("3d", Some(3 * 24 * 3600)),
    ("1w", Some(7 * 24 * 3600)),
    ("settled", None),
];

const VISIBILITY_OPTIONS: &[(Visibility, &str)] = &[
    (Visibility::Friends, "◑ friends"),
    (Visibility::Circles, "◑ circles"),
    (Visibility::Public, "◉ public"),
    (Visibility::Private, "◐ only you"),
];

/// Which surface fills the window. All of them float over the same water.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    River,
    Profile {
        profile_id: String,
    },
    Post {
        author_profile_id: String,
        post_id: String,
    },
}

#[derive(Debug, Resource, Default)]
pub struct UiState {
    pub screen: Screen,
    theme_installed: bool,
    /// The local profile (onboarding state, composer defaults, name).
    pub profile: Option<UserProfile>,
    pub composer_body: String,
    pub composer_visibility: Visibility,
    pub composer_lifetime_secs: Option<u64>,
    composer_initialized: bool,
    pub onboarding_name_input: String,
    pub peers_known: usize,
    pub errors: VecDeque<String>,
    pub friends_open: bool,
    pub friend_code_input: String,
    /// Friend targets we already fired the automatic follow-back for, so a
    /// burst of contact updates doesn't publish it repeatedly.
    follow_back_sent: HashSet<String>,
    /// Post ids with their comment thread expanded.
    pub open_threads: HashSet<String>,
    /// Per-post comment drafts.
    pub comment_drafts: std::collections::HashMap<String, String>,
    /// An own post currently being edited: (post id, draft body).
    pub editing_post: Option<(String, String)>,
}

impl UiState {
    /// Applies a freshly loaded profile: onboarding state and, once,
    /// the composer defaults.
    pub fn apply_profile(&mut self, profile: UserProfile) {
        if !self.composer_initialized {
            self.composer_visibility = profile.default_visibility;
            self.composer_lifetime_secs = profile.default_lifetime_secs;
            self.composer_initialized = true;
        }
        if self.onboarding_name_input.is_empty() {
            self.onboarding_name_input = profile.display_name.clone();
        }
        self.profile = Some(profile);
    }

    pub fn push_error(&mut self, message: String) {
        self.errors.push_back(message);
        while self.errors.len() > ERROR_LOG_LIMIT {
            self.errors.pop_front();
        }
    }

    /// Returns true the first time a follow-back is recorded for a profile.
    pub fn mark_follow_back_sent(&mut self, profile_id: &str) -> bool {
        self.follow_back_sent.insert(profile_id.to_owned())
    }
}

pub fn ui_system(
    mut egui_contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    river: Res<RiverState>,
    bridge: Res<AsyncBridge>,
    settings: Res<SettingsStore>,
    mut effects: ResMut<CardEffects>,
    mut media: NonSendMut<MediaState>,
) {
    let ctx = egui_contexts.ctx_mut();

    if !ui_state.theme_installed {
        theme::install(ctx);
        ui_state.theme_installed = true;
    }

    // The underlay animates continuously; keep egui repainting every frame.
    ctx.request_repaint();

    effects.cards.clear();
    effects.scroll_clip = None;

    let onboarded = ui_state
        .profile
        .as_ref()
        .map(|profile| profile.onboarded)
        .unwrap_or(false);

    if !onboarded {
        render_onboarding(ctx, &mut ui_state, &bridge);
        return;
    }

    let drift_time = ctx.input(|input| input.time);
    let now = now_unix_secs();

    match ui_state.screen.clone() {
        Screen::River => render_river_screen(
            ctx,
            &mut ui_state,
            &river,
            &bridge,
            &settings,
            &mut effects,
            &mut media,
            drift_time,
            now,
        ),
        Screen::Profile { profile_id } => render_profile_screen(
            ctx,
            &profile_id,
            &mut ui_state,
            &river,
            &bridge,
            &settings,
            &mut effects,
            &mut media,
            drift_time,
            now,
        ),
        Screen::Post {
            author_profile_id,
            post_id,
        } => render_post_screen(
            ctx,
            &author_profile_id,
            &post_id,
            &mut ui_state,
            &river,
            &bridge,
            &mut effects,
            &mut media,
            drift_time,
            now,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_river_screen(
    ctx: &egui::Context,
    ui_state: &mut UiState,
    river: &RiverState,
    bridge: &AsyncBridge,
    settings: &SettingsStore,
    effects: &mut CardEffects,
    media: &mut MediaState,
    drift_time: f64,
    now: u64,
) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    effects.scroll_clip = Some(ui.clip_rect());
                    constrain_column(ui);
                    ui.add_space(14.0);
                    render_header(ui, ui_state);
                    ui.add_space(10.0);
                    if ui_state.friends_open {
                        render_friends_panel(ui, ui_state, river, bridge, settings);
                        ui.add_space(12.0);
                    }
                    render_composer(ui, ui_state, bridge, media);
                    ui.add_space(12.0);

                    for (index, river_post) in river.river.iter().enumerate() {
                        if let Some(effect) = render_post_card(
                            ui, river_post, index, drift_time, now, ui_state, bridge, media,
                        ) {
                            effects.cards.push(effect);
                        }
                        ui.add_space(12.0);
                    }

                    for ghost in &river.ghosts {
                        render_ghost_card(ui, ghost, bridge);
                        ui.add_space(12.0);
                    }

                    if river.river.is_empty() {
                        render_empty_river(ui);
                    }
                    ui.add_space(40.0);
                    tracing::info!(
                        content_w = ui.min_rect().width(),
                        clip_w = ui.clip_rect().width(),
                    );
                });
        });
}

/// A greyed-out door: someone a friend's heart pointed at, not yet a friend.
fn render_ghost_card(ui: &mut egui::Ui, ghost: &crate::state::GhostCard, bridge: &AsyncBridge) {
    let width = row_content_width(ui);
    ui.horizontal(|ui| {
        ui.add_space(COLUMN_LEFT_PX);
        egui::Frame::new()
            .fill(egui::Color32::from_rgba_unmultiplied(30, 45, 52, 110))
            .stroke(egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(243, 154, 200, 60),
            ))
            .corner_radius(12.0)
            .inner_margin(13.0)
            .show(ui, |ui| {
                ui.set_width(width - 26.0);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "♥ {} let something drift to you",
                            ghost.carrier_display_name
                        ))
                        .small()
                        .color(theme::PROVENANCE_PINK),
                    );
                    ui.horizontal(|ui| {
                        let short: String = ghost.author_profile_id.chars().take(12).collect();
                        ui.label(
                            egui::RichText::new(format!("{short}… · not yet a friend"))
                                .color(theme::TEXT_MUTED),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(
                                    egui::RichText::new("＋ request friendship")
                                        .small()
                                        .color(theme::ACCENT),
                                )
                                .clicked()
                            {
                                let _ = bridge.send(NetworkCommand::RequestFriendshipById {
                                    profile_id: ghost.author_profile_id.clone(),
                                    greeting: None,
                                });
                            }
                        });
                    });
                });
            });
    });
}

fn render_header(ui: &mut egui::Ui, ui_state: &mut UiState) {
    let row_width = ui.available_width();
    ui.horizontal(|ui| {
        ui.set_max_width(row_width);
        ui.add_space(COLUMN_LEFT_PX);
        ui.monospace(
            egui::RichText::new(format!(
                "▚ RIVER · you@pond · {} peers",
                ui_state.peers_known
            ))
            .color(theme::MONO_HUD),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(COLUMN_RIGHT_PX);
            if ui
                .button(egui::RichText::new("◍ you").color(theme::ACCENT))
                .clicked()
            {
                if let Some(profile) = &ui_state.profile {
                    ui_state.screen = Screen::Profile {
                        profile_id: profile.profile_id.clone(),
                    };
                }
            }
            let label = if ui_state.friends_open {
                "⚓ friends ▴"
            } else {
                "⚓ friends ▾"
            };
            if ui
                .button(egui::RichText::new(label).color(theme::ACCENT))
                .clicked()
            {
                ui_state.friends_open = !ui_state.friends_open;
            }
        });
    });
    for error in &ui_state.errors {
        ui.horizontal(|ui| {
            ui.add_space(COLUMN_LEFT_PX);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(error)
                        .small()
                        .color(egui::Color32::from_rgb(0xf0, 0xa5, 0x66)),
                )
                .wrap(),
            );
        });
    }
}

/// The friends panel: your code, the code-entry ritual, pending requests
/// and the friends list. Lives here until the Profile screen exists.
fn render_friends_panel(
    ui: &mut egui::Ui,
    ui_state: &mut UiState,
    river: &RiverState,
    bridge: &AsyncBridge,
    settings: &SettingsStore,
) {
    let width = row_content_width(ui);
    ui.horizontal(|ui| {
        ui.add_space(COLUMN_LEFT_PX);
        egui::Frame::new()
            .fill(egui::Color32::from_rgba_unmultiplied(20, 60, 70, 210))
            .stroke(egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(90, 220, 215, 90),
            ))
            .corner_radius(12.0)
            .inner_margin(13.0)
            .show(ui, |ui| {
                ui.set_width(width - 26.0);
                ui.vertical(|ui| {
                    // My code, ready to hand over any trusted channel.
                    if let Some(profile) = &ui_state.profile {
                        let code = profile
                            .profile_id
                            .parse()
                            .ok()
                            .map(|key| {
                                let relay = settings
                                    .settings()
                                    .relay_url_for_node()
                                    .ok()
                                    .flatten()
                                    .map(|url| url.to_string());
                                FriendCode::new(key, relay, profile.display_name.clone())
                            })
                            .and_then(|code| code.encode().ok());
                        if let Some(code) = code {
                            ui.horizontal(|ui| {
                                ui.monospace(
                                    egui::RichText::new("your code")
                                        .small()
                                        .color(egui::Color32::from_rgb(0x7f, 0xb8, 0xb6)),
                                );
                                if ui.button("⧉ copy").clicked() {
                                    ui.ctx().copy_text(code.clone());
                                }
                            });
                            let preview: String = code.chars().take(40).collect();
                            ui.monospace(
                                egui::RichText::new(format!("{preview}…"))
                                    .small()
                                    .color(egui::Color32::from_rgb(0x6f, 0xd8, 0xd0)),
                            );
                        }
                    }
                    ui.add_space(8.0);

                    // Enter a friend's code.
                    ui.horizontal(|ui| {
                        let hint = egui::TextEdit::singleline(&mut ui_state.friend_code_input)
                            .hint_text("paste a jyn- code…")
                            .desired_width(ui.available_width() - 100.0);
                        ui.add(hint);
                        let can_request = ui_state.friend_code_input.trim().starts_with("jyn-");
                        if ui
                            .add_enabled(can_request, egui::Button::new("＋ request"))
                            .clicked()
                        {
                            let _ = bridge.send(NetworkCommand::RequestFriendship {
                                friend_code: ui_state.friend_code_input.trim().to_owned(),
                                greeting: None,
                            });
                            ui_state.friend_code_input.clear();
                        }
                    });

                    // Pending incoming requests.
                    let pending = river
                        .own_state()
                        .map(|own| own.pending_requests.clone())
                        .unwrap_or_default();
                    if !pending.is_empty() {
                        ui.add_space(8.0);
                        ui.monospace(
                            egui::RichText::new("asking to be friends")
                                .small()
                                .color(egui::Color32::from_rgb(0xf7, 0xc3, 0xde)),
                        );
                        for request in pending {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&request.requester_display_name)
                                        .color(egui::Color32::from_rgb(0xe6, 0xfb, 0xf8)),
                                );
                                if let Some(greeting) = &request.greeting {
                                    ui.label(
                                        egui::RichText::new(format!("“{greeting}”"))
                                            .small()
                                            .italics()
                                            .color(egui::Color32::from_rgb(0x7f, 0xb8, 0xb6)),
                                    );
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("decline").clicked() {
                                            let _ =
                                                bridge.send(NetworkCommand::RespondFriendship {
                                                    requester_profile_id: request
                                                        .requester_profile_id
                                                        .clone(),
                                                    accept: false,
                                                });
                                        }
                                        if ui.button("accept").clicked() {
                                            let _ =
                                                bridge.send(NetworkCommand::RespondFriendship {
                                                    requester_profile_id: request
                                                        .requester_profile_id
                                                        .clone(),
                                                    accept: true,
                                                });
                                        }
                                    },
                                );
                            });
                        }
                    }

                    // Friends (profiles we follow), with mutuality status.
                    let followed = river
                        .own_state()
                        .map(|own| own.followed_profile_ids.clone())
                        .unwrap_or_default();
                    if !followed.is_empty() {
                        ui.add_space(8.0);
                        ui.monospace(
                            egui::RichText::new("friends")
                                .small()
                                .color(egui::Color32::from_rgb(0x7f, 0xb8, 0xb6)),
                        );
                        let my_id = ui_state
                            .profile
                            .as_ref()
                            .map(|profile| profile.profile_id.clone())
                            .unwrap_or_default();
                        for profile_id in followed {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(river.contact_display_name(&profile_id))
                                        .color(egui::Color32::from_rgb(0xe6, 0xfb, 0xf8)),
                                );
                                let mutual = river
                                    .contact_state(&profile_id)
                                    .map(|state| state.followed_profile_ids.contains(&my_id))
                                    .unwrap_or(false);
                                let status = if mutual {
                                    "· in the river"
                                } else {
                                    "· waiting for them"
                                };
                                ui.label(
                                    egui::RichText::new(status)
                                        .small()
                                        .color(egui::Color32::from_rgb(0x59, 0x91, 0x8d)),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("unfriend").clicked() {
                                            let _ = bridge.send(NetworkCommand::RemoveFriend {
                                                profile_id: profile_id.clone(),
                                            });
                                        }
                                    },
                                );
                            });
                        }
                    }
                });
            });
    });
}

fn render_composer(
    ui: &mut egui::Ui,
    ui_state: &mut UiState,
    bridge: &AsyncBridge,
    media: &mut MediaState,
) {
    let width = row_content_width(ui);
    ui.horizontal(|ui| {
        ui.add_space(COLUMN_LEFT_PX);
        egui::Frame::new()
            .fill(egui::Color32::from_rgba_unmultiplied(30, 80, 90, 190))
            .stroke(egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(90, 220, 215, 102),
            ))
            .corner_radius(12.0)
            .inner_margin(13.0)
            .show(ui, |ui| {
                ui.set_width(width - 26.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.monospace(
                            egui::RichText::new("◈ → your river")
                                .color(egui::Color32::from_rgb(0x7f, 0xe8, 0xe0)),
                        );
                    });
                    ui.add_space(6.0);

                    let response = ui.add(
                        egui::TextEdit::multiline(&mut ui_state.composer_body)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY)
                            .hint_text("Say something into the current…")
                            .frame(false),
                    );
                    let _ = response;
                    ui.add_space(6.0);

                    render_composer_media_row(ui, media);
                    ui.add_space(6.0);

                    ui.horizontal_wrapped(|ui| {
                        let visibility_label = VISIBILITY_OPTIONS
                            .iter()
                            .find(|(visibility, _)| *visibility == ui_state.composer_visibility)
                            .map(|(_, label)| *label)
                            .unwrap_or("◑ friends");
                        egui::ComboBox::from_id_salt("composer-visibility")
                            .width(96.0)
                            .selected_text(visibility_label)
                            .show_ui(ui, |ui| {
                                for (visibility, label) in VISIBILITY_OPTIONS {
                                    ui.selectable_value(
                                        &mut ui_state.composer_visibility,
                                        *visibility,
                                        *label,
                                    );
                                }
                            });

                        let lifetime_label = LIFETIME_OPTIONS
                            .iter()
                            .find(|(_, secs)| *secs == ui_state.composer_lifetime_secs)
                            .map(|(label, _)| format!("◔ ebbs {label}"))
                            .unwrap_or_else(|| "◔ ebbs".to_owned());
                        egui::ComboBox::from_id_salt("composer-lifetime")
                            .width(96.0)
                            .selected_text(lifetime_label)
                            .show_ui(ui, |ui| {
                                for (label, secs) in LIFETIME_OPTIONS {
                                    let text = match secs {
                                        Some(_) => format!("◔ ebbs {label}"),
                                        None => "◆ settled".to_owned(),
                                    };
                                    ui.selectable_value(
                                        &mut ui_state.composer_lifetime_secs,
                                        *secs,
                                        text,
                                    );
                                }
                            });

                        {
                            let can_cast = !ui_state.composer_body.trim().is_empty()
                                || media.pending_audio.is_some()
                                || !media.pending_attachments.is_empty();
                            if ui
                                .add_enabled(can_cast, egui::Button::new("Cast"))
                                .clicked()
                            {
                                let mut drafts = Vec::new();
                                if let Some(audio) = media.pending_audio.take() {
                                    drafts.push(MediaDraft {
                                        path: audio.path,
                                        kind: MediaKind::Audio,
                                        duration_ms: Some(audio.duration_ms),
                                        waveform: Some(audio.waveform),
                                    });
                                }
                                for path in media.pending_attachments.drain(..) {
                                    drafts.push(MediaDraft {
                                        kind: media::classify(&path),
                                        path,
                                        duration_ms: None,
                                        waveform: None,
                                    });
                                }
                                let draft = PostDraft {
                                    body: ui_state.composer_body.trim().to_owned(),
                                    visibility: ui_state.composer_visibility,
                                    lifetime_secs: ui_state.composer_lifetime_secs,
                                    media: drafts,
                                };
                                if bridge.send(NetworkCommand::PublishPost { draft }).is_ok() {
                                    ui_state.composer_body.clear();
                                }
                            }
                        }
                    });
                });
            });
    });
}

#[allow(clippy::too_many_arguments)]
fn render_post_card(
    ui: &mut egui::Ui,
    river_post: &RiverPost,
    index: usize,
    drift_time: f64,
    now: u64,
    ui_state: &mut UiState,
    bridge: &AsyncBridge,
    media: &mut MediaState,
) -> Option<CardEffect> {
    let post = &river_post.post;
    let is_permanent = post.expires_at.is_none();
    let (remaining_label, tier) = match post.expires_at {
        Some(expires_at) => format_remaining(now, expires_at),
        None => ("settled".to_owned(), UrgencyTier::Settled),
    };

    // Ephemeral cards drift gently in the current; periods 8-11s, desynced.
    let drift = if is_permanent {
        0.0
    } else {
        let period = 8.0 + (index % 4) as f64;
        let phase = index as f64 * 1.7;
        (3.0 * (drift_time / period * std::f64::consts::TAU + phase).sin()) as f32
    };

    let (fill, stroke, corner_radius) = match tier {
        UrgencyTier::Settled => (
            egui::Color32::from_rgba_unmultiplied(40, 66, 75, 235),
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(120, 140, 150, 128),
            ),
            5.0,
        ),
        UrgencyTier::Normal => (
            egui::Color32::from_rgba_unmultiplied(28, 84, 94, 160),
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(90, 220, 215, 115),
            ),
            12.0,
        ),
        UrgencyTier::Warm | UrgencyTier::Critical => (
            egui::Color32::from_rgba_unmultiplied(90, 58, 36, 180),
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(240, 170, 120, 128),
            ),
            12.0,
        ),
    };

    let mut effect = None;
    let width = row_content_width(ui);
    ui.horizontal(|ui| {
        ui.add_space(COLUMN_LEFT_PX + drift);
        let response = egui::Frame::new()
            .fill(fill)
            .stroke(stroke)
            .corner_radius(corner_radius)
            .inner_margin(13.0)
            .show(ui, |ui| {
                ui.set_width(width - 26.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let author_label = egui::RichText::new(&river_post.author_display_name)
                            .strong()
                            .color(egui::Color32::from_rgb(0xe6, 0xfb, 0xf8));
                        if ui
                            .add(egui::Label::new(author_label).sense(egui::Sense::click()))
                            .clicked()
                        {
                            ui_state.screen = Screen::Profile {
                                profile_id: river_post.author_profile_id.clone(),
                            };
                        }
                        let context = if post.visibility == Visibility::Private {
                            "· your pond · only you"
                        } else if river_post.is_self {
                            "· your river"
                        } else {
                            "· their river"
                        };
                        ui.label(
                            egui::RichText::new(context)
                                .small()
                                .color(egui::Color32::from_rgb(0x7f, 0xb8, 0xb6)),
                        );
                        if post.edited {
                            ui.label(
                                egui::RichText::new("· edited")
                                    .small()
                                    .color(egui::Color32::from_rgb(0x59, 0x91, 0x8d)),
                            );
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (text, color) = match tier {
                                UrgencyTier::Settled => (
                                    "◆ settled".to_owned(),
                                    egui::Color32::from_rgb(0xc3, 0xd0, 0xd8),
                                ),
                                UrgencyTier::Normal => (
                                    format!("◔ {remaining_label}"),
                                    egui::Color32::from_rgb(0x9d, 0xf6, 0xee),
                                ),
                                UrgencyTier::Warm | UrgencyTier::Critical => (
                                    format!("◕ {remaining_label}"),
                                    egui::Color32::from_rgb(0xff, 0xd8, 0xb3),
                                ),
                            };
                            ui.monospace(egui::RichText::new(text).color(color));
                        });
                    });
                    ui.add_space(4.0);
                    let editing_this = ui_state
                        .editing_post
                        .as_ref()
                        .map(|(editing_id, _)| *editing_id == post.post_id)
                        .unwrap_or(false);
                    if editing_this {
                        let mut save = false;
                        let mut cancel = false;
                        if let Some((_, draft)) = ui_state.editing_post.as_mut() {
                            ui.add(
                                egui::TextEdit::multiline(draft)
                                    .desired_rows(2)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.horizontal(|ui| {
                                save = ui.button("save").clicked();
                                cancel = ui.button("cancel").clicked();
                            });
                        }
                        if save {
                            if let Some((_, draft)) = ui_state.editing_post.take() {
                                let _ = bridge.send(NetworkCommand::EditPost {
                                    post_id: post.post_id.clone(),
                                    body: draft.trim().to_owned(),
                                });
                            }
                        } else if cancel {
                            ui_state.editing_post = None;
                        }
                    }
                    if !editing_this && !post.body.is_empty() {
                        ui.label(
                            egui::RichText::new(&post.body)
                                .color(egui::Color32::from_rgb(0xd7, 0xf2, 0xef)),
                        );
                    }
                    for attachment in &post.media {
                        ui.add_space(6.0);
                        render_attachment(ui, river_post, attachment, media, bridge);
                    }
                    ui.add_space(6.0);

                    render_card_footer(ui, river_post, ui_state, bridge);
                });
            });

        if !is_permanent {
            let band_frac = match tier {
                UrgencyTier::Warm | UrgencyTier::Critical => 0.22,
                _ => 0.36,
            };
            effect = Some(CardEffect {
                id: post.post_id.clone(),
                rect: response.response.rect,
                band_frac,
                warm: matches!(tier, UrgencyTier::Warm | UrgencyTier::Critical),
            });
        }
    });
    effect
}

/// Hearts, keep, comments and (on own posts) the author controls.
fn render_card_footer(
    ui: &mut egui::Ui,
    river_post: &RiverPost,
    ui_state: &mut UiState,
    bridge: &AsyncBridge,
) {
    let post = &river_post.post;
    let is_private = post.visibility == Visibility::Private;

    // Named hearts — never a bare count.
    if !river_post.hearts.is_empty() {
        let names = river_post
            .hearts
            .iter()
            .map(|heart| heart.hearter_display_name.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        ui.label(
            egui::RichText::new(format!("♥ {names}"))
                .small()
                .color(egui::Color32::from_rgb(0xf3, 0x9a, 0xc8)),
        );
        ui.add_space(2.0);
    }

    ui.horizontal(|ui| {
        if ui
            .button(egui::RichText::new("⤢").small().color(theme::TEXT_MUTED))
            .on_hover_text("Open the post")
            .clicked()
        {
            ui_state.screen = Screen::Post {
                author_profile_id: river_post.author_profile_id.clone(),
                post_id: post.post_id.clone(),
            };
        }
        if !is_private {
            let heart_label = if river_post.hearted_by_me {
                "♥ hearted"
            } else {
                "♡ heart"
            };
            if ui
                .button(
                    egui::RichText::new(heart_label)
                        .small()
                        .color(egui::Color32::from_rgb(0xf3, 0x9a, 0xc8)),
                )
                .clicked()
            {
                let _ = bridge.send(NetworkCommand::SetHeart {
                    post_author_profile_id: river_post.author_profile_id.clone(),
                    post_id: post.post_id.clone(),
                    active: !river_post.hearted_by_me,
                });
            }

            if !river_post.is_self {
                let keep_label = if river_post.kept_by_me {
                    "◆ kept"
                } else {
                    "◇ keep"
                };
                let keep_button = ui.button(
                    egui::RichText::new(keep_label)
                        .small()
                        .color(egui::Color32::from_rgb(0xbf, 0xee, 0xea)),
                );
                if keep_button
                    .on_hover_text(
                        "A kept copy is a lease: it ebbs with the post's lifetime \
                         and follows the author's delete.",
                    )
                    .clicked()
                {
                    let command = if river_post.kept_by_me {
                        NetworkCommand::ReleaseKeep {
                            post_author_profile_id: river_post.author_profile_id.clone(),
                            post_id: post.post_id.clone(),
                        }
                    } else {
                        NetworkCommand::KeepPost {
                            post_author_profile_id: river_post.author_profile_id.clone(),
                            post_id: post.post_id.clone(),
                        }
                    };
                    let _ = bridge.send(command);
                }
            }

            let replies_label = match river_post.comments.len() {
                0 => "reply".to_owned(),
                1 => "1 reply".to_owned(),
                n => format!("{n} replies"),
            };
            let replies_clicked = ui
                .button(
                    egui::RichText::new(replies_label)
                        .small()
                        .color(egui::Color32::from_rgb(0x7f, 0xb8, 0xb6)),
                )
                .clicked();
            if replies_clicked && !ui_state.open_threads.remove(&post.post_id) {
                ui_state.open_threads.insert(post.post_id.clone());
            }
        }

        // Author sovereignty, exercised in place.
        if river_post.is_self {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(egui::RichText::new("delete").small())
                    .on_hover_text("Reaches every copy, kept ones included.")
                    .clicked()
                {
                    let _ = bridge.send(NetworkCommand::DeletePost {
                        post_id: post.post_id.clone(),
                    });
                }
                if ui.button(egui::RichText::new("edit").small()).clicked() {
                    ui_state.editing_post = Some((post.post_id.clone(), post.body.clone()));
                }
                if post.expires_at.is_some() {
                    if ui
                        .button(egui::RichText::new("↑ promote").small())
                        .on_hover_text("Make it permanent — it settles into your pond.")
                        .clicked()
                    {
                        let _ = bridge.send(NetworkCommand::SetPostLifetime {
                            post_id: post.post_id.clone(),
                            expires_at: None,
                        });
                    }
                } else if ui
                    .button(egui::RichText::new("↩ let it go…").small())
                    .on_hover_text("Give it 36 hours to ebb away again.")
                    .clicked()
                {
                    let _ = bridge.send(NetworkCommand::SetPostLifetime {
                        post_id: post.post_id.clone(),
                        expires_at: Some(now_unix_secs() + 36 * 3600),
                    });
                }
            });
        }
    });

    // The flat thread, expanded in place.
    if ui_state.open_threads.contains(&post.post_id) && !is_private {
        ui.add_space(6.0);
        for comment in &river_post.comments {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(&comment.commenter_display_name)
                        .small()
                        .strong()
                        .color(egui::Color32::from_rgb(0xbf, 0xee, 0xea)),
                );
                ui.label(
                    egui::RichText::new(&comment.body)
                        .small()
                        .color(egui::Color32::from_rgb(0xd7, 0xf2, 0xef)),
                );
            });
        }
        let draft = ui_state
            .comment_drafts
            .entry(post.post_id.clone())
            .or_default();
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(draft)
                    .hint_text("say something back…")
                    .desired_width(ui.available_width() - 60.0),
            );
            let can_send = !draft.trim().is_empty();
            if ui
                .add_enabled(can_send, egui::Button::new("send"))
                .clicked()
            {
                let _ = bridge.send(NetworkCommand::PublishComment {
                    post_author_profile_id: river_post.author_profile_id.clone(),
                    post_id: post.post_id.clone(),
                    body: draft.trim().to_owned(),
                });
                draft.clear();
            }
        });
    }
}

/// The composer's media row: record a voice note, attach files, staged chips.
fn render_composer_media_row(ui: &mut egui::Ui, media: &mut MediaState) {
    // Collect finished file dialogs.
    if let Some(receiver) = &media.file_dialog {
        if let Ok(result) = receiver.try_recv() {
            if let Some(paths) = result {
                media.pending_attachments.extend(paths);
            }
            media.file_dialog = None;
        }
    }

    ui.horizontal_wrapped(|ui| {
        // Voice note: one per post, recorded in place.
        if let Some(recording) = &media.recording {
            let label = format!("◼ stop · {}s", recording.elapsed_secs());
            let level = recording.level();
            if ui
                .button(egui::RichText::new(label).color(egui::Color32::from_rgb(0xff, 0xd8, 0xb3)))
                .clicked()
            {
                let recording = media.recording.take().expect("recording present");
                let recording_dir = media.recording_dir.clone();
                match recording.stop(&recording_dir) {
                    Ok(audio) => media.pending_audio = Some(audio),
                    Err(err) => media.mic_error = Some(err.to_string()),
                }
            }
            // A tiny live level meter.
            let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    rect.min,
                    egui::vec2(rect.width() * level.sqrt(), rect.height()),
                ),
                2.0,
                egui::Color32::from_rgb(0x6f, 0xe6, 0xdd),
            );
        } else if media.pending_audio.is_none()
            && ui
                .button(
                    egui::RichText::new("● record")
                        .color(egui::Color32::from_rgb(0xf3, 0x9a, 0xc8)),
                )
                .clicked()
        {
            match crate::media::ActiveRecording::start() {
                Ok(recording) => {
                    media.recording = Some(recording);
                    media.mic_error = None;
                }
                Err(err) => media.mic_error = Some(err.to_string()),
            }
        }

        if ui.button("📎 attach").clicked() && media.file_dialog.is_none() {
            let (tx, rx) = flume::bounded(1);
            std::thread::spawn(move || {
                let picked = rfd::FileDialog::new()
                    .pick_files()
                    .map(|paths| paths.into_iter().collect::<Vec<_>>());
                let _ = tx.send(picked);
            });
            media.file_dialog = Some(rx);
        }
    });

    if let Some(error) = &media.mic_error {
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("🎙 {error}"))
                    .small()
                    .color(egui::Color32::from_rgb(0xf0, 0xa5, 0x66)),
            )
            .wrap(),
        );
    }

    // Staged chips.
    let mut remove_audio = false;
    let mut remove_attachment: Option<usize> = None;
    ui.horizontal_wrapped(|ui| {
        if let Some(audio) = &media.pending_audio {
            let seconds = audio.duration_ms / 1000;
            if ui.button(format!("🎙 voice note · {seconds}s ✕")).clicked() {
                remove_audio = true;
            }
        }
        for (index, path) in media.pending_attachments.iter().enumerate() {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".to_owned());
            if ui.button(format!("📎 {name} ✕")).clicked() {
                remove_attachment = Some(index);
            }
        }
    });
    if remove_audio {
        media.pending_audio = None;
    }
    if let Some(index) = remove_attachment {
        media.pending_attachments.remove(index);
    }
}

/// One attachment inside a card: photo inline, audio player, or a file card
/// (the video decode path arrives with the media-video feature; until then
/// videos open externally like any file).
fn render_attachment(
    ui: &mut egui::Ui,
    river_post: &RiverPost,
    attachment: &MediaAttachment,
    media: &mut MediaState,
    bridge: &AsyncBridge,
) {
    let local_path = media.local_path_for(&attachment.blob_hash);

    // Photos and audio fetch eagerly; videos and files on demand.
    let eager = matches!(attachment.kind, MediaKind::Photo | MediaKind::Audio);
    if local_path.is_none() && eager && media.fetch_requested.insert(attachment.blob_hash.clone()) {
        let _ = bridge.send(NetworkCommand::FetchMedia {
            blob_hash: attachment.blob_hash.clone(),
        });
    }

    match attachment.kind {
        MediaKind::Photo => match local_path {
            Some(path) => {
                if let Some(texture) = media.texture_for(ui.ctx(), &attachment.blob_hash, &path) {
                    let max_width = ui.available_width();
                    let size = texture.size_vec2();
                    let scale = (max_width / size.x).min(1.0);
                    ui.image((texture.id(), size * scale));
                } else {
                    render_file_chip(ui, attachment, Some(path));
                }
            }
            None => {
                ui.label(
                    egui::RichText::new("〰 the photo is still drifting in…")
                        .small()
                        .color(egui::Color32::from_rgb(0x59, 0x91, 0x8d)),
                );
            }
        },
        MediaKind::Audio => {
            ui.horizontal(|ui| {
                let playing = media.playback.is_playing(&river_post.post.post_id);
                let label = if playing { "⏸" } else { "▶" };
                match &local_path {
                    Some(path) => {
                        if ui.button(label).clicked() {
                            if let Err(err) = media.playback.toggle(&river_post.post.post_id, path)
                            {
                                tracing::warn!("playback failed: {err:#}");
                            }
                        }
                    }
                    None => {
                        ui.add_enabled(false, egui::Button::new("▶"));
                    }
                }

                // The waveform travels in the operation and renders before
                // the audio itself has arrived.
                if let Some(waveform) = &attachment.waveform {
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2((waveform.len() as f32) * 4.0, 24.0),
                        egui::Sense::hover(),
                    );
                    for (index, peak) in waveform.iter().enumerate() {
                        let height = (f32::from(*peak) / 255.0) * rect.height();
                        let x = rect.min.x + index as f32 * 4.0;
                        let bar = egui::Rect::from_min_max(
                            egui::pos2(x, rect.center().y - height / 2.0),
                            egui::pos2(x + 2.0, rect.center().y + height / 2.0),
                        );
                        let t = index as f32 / waveform.len() as f32;
                        let color = egui::Color32::from_rgb(
                            (0x5f as f32 * (1.0 - t) + 0x2a as f32 * t) as u8,
                            (0xe0 as f32 * (1.0 - t) + 0x7a as f32 * t) as u8,
                            (0xd6 as f32 * (1.0 - t) + 0x75 as f32 * t) as u8,
                        );
                        ui.painter().rect_filled(bar, 1.0, color);
                    }
                }

                if let Some(duration_ms) = attachment.duration_ms {
                    let secs = duration_ms / 1000;
                    ui.monospace(
                        egui::RichText::new(format!("{}:{:02}", secs / 60, secs % 60))
                            .small()
                            .color(egui::Color32::from_rgb(0x7f, 0xb8, 0xb6)),
                    );
                }
                if local_path.is_none() {
                    ui.label(
                        egui::RichText::new("drifting in…")
                            .small()
                            .color(egui::Color32::from_rgb(0x59, 0x91, 0x8d)),
                    );
                }
            });
        }
        MediaKind::Video => {
            let video_key = format!("{}/{}", river_post.post.post_id, attachment.blob_hash);
            let playing = media
                .active_video
                .as_ref()
                .map(|video| video.key == video_key)
                .unwrap_or(false);

            if playing {
                render_active_video(ui, media);
                if ui.small_button("◼ stop").clicked() {
                    media.active_video = None;
                }
            } else {
                render_file_chip(ui, attachment, local_path.clone());
                ui.horizontal(|ui| match &local_path {
                    Some(path) => {
                        if ui.small_button("▶ play").clicked() {
                            let display_width =
                                (ui.available_width() * ui.ctx().pixels_per_point()) as u32;
                            start_video(
                                media,
                                &river_post.post.post_id,
                                video_key,
                                path,
                                display_width,
                            );
                        }
                    }
                    None => {
                        if ui.small_button("fetch").clicked()
                            && media.fetch_requested.insert(attachment.blob_hash.clone())
                        {
                            let _ = bridge.send(NetworkCommand::FetchMedia {
                                blob_hash: attachment.blob_hash.clone(),
                            });
                        }
                    }
                });
            }
        }
        MediaKind::File => {
            render_file_chip(ui, attachment, local_path);
            if local_path_missing_and_unrequested(media, attachment) {
                ui.horizontal(|ui| {
                    if ui.small_button("fetch").clicked()
                        && media.fetch_requested.insert(attachment.blob_hash.clone())
                    {
                        let _ = bridge.send(NetworkCommand::FetchMedia {
                            blob_hash: attachment.blob_hash.clone(),
                        });
                    }
                });
            }
        }
    }
}

/// Starts inline playback: ffmpeg decode downscaled to the card's display
/// width; the audio track follows once its off-thread extraction finishes.
/// Any failure degrades to the file chip (the player simply never appears).
fn start_video(
    media: &mut MediaState,
    post_id: &str,
    video_key: String,
    path: &std::path::Path,
    display_width: u32,
) {
    let scratch = media.recording_dir.clone();
    match crate::media::video::VideoPlayer::open(path, &scratch, display_width) {
        Ok(player) => {
            media.active_video = Some(crate::media::ActiveVideo {
                key: video_key,
                audio_post_id: post_id.to_owned(),
                player,
                texture: None,
            });
        }
        Err(err) => {
            tracing::warn!("inline video unavailable, use 'open' instead: {err:#}");
        }
    }
}

/// Uploads the newest decoded frame and draws the video surface.
fn render_active_video(ui: &mut egui::Ui, media: &mut MediaState) {
    let ctx = ui.ctx().clone();

    // Start the audio track when its extraction lands.
    let ready_audio = media.active_video.as_ref().and_then(|video| {
        video
            .player
            .poll_audio()
            .map(|path| (video.audio_post_id.clone(), path))
    });
    if let Some((post_id, path)) = ready_audio {
        if let Err(err) = media.playback.toggle(&post_id, &path) {
            tracing::debug!("video audio playback failed: {err:#}");
        }
    }

    let Some(video) = media.active_video.as_mut() else {
        return;
    };
    if let Some(frame) = video.player.take_frame() {
        let image = egui::ColorImage::from_rgba_unmultiplied(frame.size, &frame.rgba);
        match &mut video.texture {
            Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
            None => {
                video.texture = Some(ctx.load_texture(
                    format!("video-{}", video.key),
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
    }
    if let Some(texture) = &video.texture {
        let max_width = ui.available_width();
        let size = texture.size_vec2();
        let scale = (max_width / size.x).min(1.0);
        ui.image((texture.id(), size * scale));
    } else {
        ui.label(
            egui::RichText::new("〰 surfacing…")
                .small()
                .color(theme::TEXT_MUTED),
        );
    }
    if video.player.is_finished() && video.player.take_frame().is_none() && video.texture.is_some()
    {
        // Leave the last frame on screen; playback ends naturally.
    }
}

fn local_path_missing_and_unrequested(
    media: &mut MediaState,
    attachment: &MediaAttachment,
) -> bool {
    media.local_path_for(&attachment.blob_hash).is_none()
        && !media.fetch_requested.contains(&attachment.blob_hash)
}

fn render_file_chip(
    ui: &mut egui::Ui,
    attachment: &MediaAttachment,
    local_path: Option<std::path::PathBuf>,
) {
    ui.horizontal(|ui| {
        let icon = match attachment.kind {
            MediaKind::Video => "🎞",
            _ => "📄",
        };
        let name = attachment
            .file_name
            .clone()
            .unwrap_or_else(|| attachment.blob_hash.chars().take(12).collect());
        let size_mb = attachment.byte_len as f64 / (1024.0 * 1024.0);
        ui.label(
            egui::RichText::new(format!("{icon} {name} · {size_mb:.1} MB"))
                .small()
                .color(egui::Color32::from_rgb(0xbf, 0xee, 0xea)),
        );
        if let Some(path) = local_path {
            if ui.small_button("open").clicked() {
                // Open a copy named like the original so the OS routes it to
                // the right application (the cache file is a bare hash).
                let result = media::named_copy_for_opening(
                    &path,
                    &attachment.blob_hash,
                    attachment.file_name.as_deref(),
                    &attachment.mime,
                )
                .and_then(|named| open::that(&named).map_err(anyhow::Error::from));
                if let Err(err) = result {
                    tracing::warn!("failed to open {}: {err:#}", path.display());
                }
            }
        }
    });
}

fn render_empty_river(ui: &mut egui::Ui) {
    ui.add_space(30.0);
    inset_block(ui, |ui| {
        ui.label(
            egui::RichText::new("The river is quiet.")
                .color(egui::Color32::from_rgb(0x7f, 0xb8, 0xb6)),
        );
        ui.label(
            egui::RichText::new(
                "Cast something into the current — even just for yourself (◐ only you).",
            )
            .small()
            .color(egui::Color32::from_rgb(0x59, 0x91, 0x8d)),
        );
    });
}

/// A back row shared by the inner screens.
fn render_back_row(ui: &mut egui::Ui, ui_state: &mut UiState, label: &str) {
    ui.horizontal(|ui| {
        ui.add_space(COLUMN_LEFT_PX);
        if ui
            .button(egui::RichText::new("← river").color(theme::ACCENT))
            .clicked()
        {
            ui_state.screen = Screen::River;
        }
        ui.monospace(egui::RichText::new(label).color(theme::MONO_HUD));
    });
}

#[allow(clippy::too_many_arguments)]
fn render_profile_screen(
    ctx: &egui::Context,
    profile_id: &str,
    ui_state: &mut UiState,
    river: &RiverState,
    bridge: &AsyncBridge,
    settings: &SettingsStore,
    effects: &mut CardEffects,
    media: &mut MediaState,
    drift_time: f64,
    now: u64,
) {
    let is_own = ui_state
        .profile
        .as_ref()
        .map(|profile| profile.profile_id == profile_id)
        .unwrap_or(false);

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    effects.scroll_clip = Some(ui.clip_rect());
                    constrain_column(ui);
                    ui.add_space(14.0);
                    render_back_row(ui, ui_state, "▚ PROFILE");
                    ui.add_space(10.0);

                    // Identity card.
                    let width = row_content_width(ui);
                    ui.horizontal(|ui| {
                        ui.add_space(COLUMN_LEFT_PX);
                        egui::Frame::new()
                            .fill(egui::Color32::from_rgba_unmultiplied(20, 60, 70, 210))
                            .stroke(egui::Stroke::new(
                                1.0,
                                egui::Color32::from_rgba_unmultiplied(90, 220, 215, 90),
                            ))
                            .corner_radius(12.0)
                            .inner_margin(14.0)
                            .show(ui, |ui| {
                                ui.set_width(width - 28.0);
                                ui.vertical(|ui| {
                                    let display_name = if is_own {
                                        ui_state
                                            .profile
                                            .as_ref()
                                            .map(|profile| profile.display_name.clone())
                                            .unwrap_or_default()
                                    } else {
                                        river.contact_display_name(profile_id)
                                    };
                                    ui.heading(
                                        egui::RichText::new(&display_name)
                                            .color(theme::TEXT_PRIMARY),
                                    );
                                    let fingerprint: String = profile_id.chars().take(16).collect();
                                    ui.monospace(
                                        egui::RichText::new(format!(
                                            "⚿ {fingerprint}… · born on their machine"
                                        ))
                                        .small()
                                        .color(theme::MONO_HUD),
                                    );
                                    let bio = if is_own {
                                        ui_state
                                            .profile
                                            .as_ref()
                                            .map(|profile| profile.bio.clone())
                                            .unwrap_or_default()
                                    } else {
                                        river
                                            .contact_state(profile_id)
                                            .map(|state| state.bio.clone())
                                            .unwrap_or_default()
                                    };
                                    if !bio.is_empty() {
                                        ui.label(egui::RichText::new(&bio).color(theme::TEXT_BODY));
                                    }

                                    if is_own {
                                        ui.add_space(8.0);
                                        render_own_profile_settings(ui, ui_state, bridge);
                                    } else {
                                        ui.add_space(8.0);
                                        render_relationship_row(ui, profile_id, river, bridge);
                                    }
                                });
                            });
                    });
                    ui.add_space(10.0);

                    if is_own && ui_state.friends_open {
                        render_friends_panel(ui, ui_state, river, bridge, settings);
                        ui.add_space(10.0);
                    }
                    if is_own {
                        ui.horizontal(|ui| {
                            ui.add_space(COLUMN_LEFT_PX);
                            let label = if ui_state.friends_open {
                                "⚓ friends ▴"
                            } else {
                                "⚓ friends ▾"
                            };
                            if ui
                                .button(egui::RichText::new(label).color(theme::ACCENT))
                                .clicked()
                            {
                                ui_state.friends_open = !ui_state.friends_open;
                            }
                        });
                        ui.add_space(10.0);
                    }

                    // One stream, all lifetimes mixed: what survives becomes
                    // the pond.
                    let mut shown = 0usize;
                    for (index, river_post) in river
                        .river
                        .iter()
                        .filter(|post| post.author_profile_id == profile_id)
                        .enumerate()
                    {
                        shown += 1;
                        if let Some(effect) = render_post_card(
                            ui, river_post, index, drift_time, now, ui_state, bridge, media,
                        ) {
                            effects.cards.push(effect);
                        }
                        ui.add_space(12.0);
                    }
                    if shown == 0 {
                        inset_block(ui, |ui| {
                            ui.label(
                                egui::RichText::new("Their stretch of the river is quiet.")
                                    .color(theme::TEXT_SECONDARY),
                            );
                        });
                    }
                    ui.add_space(40.0);
                });
        });
}

/// Defaults and identity edits, embedded where they govern — there is no
/// settings screen.
fn render_own_profile_settings(ui: &mut egui::Ui, ui_state: &mut UiState, bridge: &AsyncBridge) {
    ui.monospace(
        egui::RichText::new("defaults")
            .small()
            .color(theme::TEXT_SECONDARY),
    );
    let Some(profile) = ui_state.profile.clone() else {
        return;
    };
    let mut visibility = profile.default_visibility;
    let mut lifetime = profile.default_lifetime_secs;
    let mut changed = false;

    ui.horizontal(|ui| {
        let visibility_label = VISIBILITY_OPTIONS
            .iter()
            .find(|(candidate, _)| *candidate == visibility)
            .map(|(_, label)| *label)
            .unwrap_or("◑ friends");
        egui::ComboBox::from_id_salt("profile-default-visibility")
            .width(96.0)
            .selected_text(visibility_label)
            .show_ui(ui, |ui| {
                for (candidate, label) in VISIBILITY_OPTIONS {
                    if *candidate == Visibility::Private {
                        continue; // The default reach cannot be private-only.
                    }
                    changed |= ui
                        .selectable_value(&mut visibility, *candidate, *label)
                        .changed();
                }
            });

        let lifetime_label = LIFETIME_OPTIONS
            .iter()
            .find(|(_, secs)| *secs == lifetime)
            .map(|(label, _)| format!("◔ ebbs {label}"))
            .unwrap_or_else(|| "◔ ebbs".to_owned());
        egui::ComboBox::from_id_salt("profile-default-lifetime")
            .width(96.0)
            .selected_text(lifetime_label)
            .show_ui(ui, |ui| {
                for (label, secs) in LIFETIME_OPTIONS {
                    let text = match secs {
                        Some(_) => format!("◔ ebbs {label}"),
                        None => "◆ settled".to_owned(),
                    };
                    changed |= ui.selectable_value(&mut lifetime, *secs, text).changed();
                }
            });
    });

    if changed {
        let _ = bridge.send(NetworkCommand::UpdateProfile {
            display_name: profile.display_name.clone(),
            bio: profile.bio.clone(),
            default_visibility: visibility,
            default_lifetime_secs: lifetime,
            mark_onboarded: false,
        });
    }
}

fn render_relationship_row(
    ui: &mut egui::Ui,
    profile_id: &str,
    river: &RiverState,
    bridge: &AsyncBridge,
) {
    let followed = river.follows(profile_id);
    let follows_me = ui_state_follows_me(profile_id, river);
    ui.horizontal(|ui| {
        let status = match (followed, follows_me) {
            (true, true) => "friends · in the river",
            (true, false) => "waiting for them",
            (false, _) => "not yet a friend",
        };
        ui.label(
            egui::RichText::new(status)
                .small()
                .color(theme::TEXT_SECONDARY),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if followed {
                if ui.button("unfriend").clicked() {
                    let _ = bridge.send(NetworkCommand::RemoveFriend {
                        profile_id: profile_id.to_owned(),
                    });
                }
            } else if ui
                .button(egui::RichText::new("＋ request friendship").color(theme::ACCENT))
                .clicked()
            {
                let _ = bridge.send(NetworkCommand::RequestFriendshipById {
                    profile_id: profile_id.to_owned(),
                    greeting: None,
                });
            }
        });
    });
}

fn ui_state_follows_me(profile_id: &str, river: &RiverState) -> bool {
    let Some(own) = river.own_state() else {
        return false;
    };
    river
        .contact_state(profile_id)
        .map(|state| state.followed_profile_ids.contains(&own.profile_id))
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
fn render_post_screen(
    ctx: &egui::Context,
    author_profile_id: &str,
    post_id: &str,
    ui_state: &mut UiState,
    river: &RiverState,
    bridge: &AsyncBridge,
    effects: &mut CardEffects,
    media: &mut MediaState,
    drift_time: f64,
    now: u64,
) {
    // The post view is the card at full attention with its thread open.
    ui_state.open_threads.insert(post_id.to_owned());
    let river_post = river
        .river
        .iter()
        .find(|post| post.author_profile_id == author_profile_id && post.post.post_id == post_id)
        .cloned();

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    effects.scroll_clip = Some(ui.clip_rect());
                    constrain_column(ui);
                    ui.add_space(14.0);
                    render_back_row(ui, ui_state, "▚ POST");
                    ui.add_space(10.0);

                    match &river_post {
                        Some(river_post) => {
                            if let Some(effect) = render_post_card(
                                ui, river_post, 0, drift_time, now, ui_state, bridge, media,
                            ) {
                                effects.cards.push(effect);
                            }
                        }
                        None => {
                            ui.horizontal(|ui| {
                                ui.add_space(COLUMN_LEFT_PX);
                                ui.label(
                                    egui::RichText::new(
                                        "This post has ebbed away — its lifetime ended                                          or its author let it go.",
                                    )
                                    .color(theme::TEXT_SECONDARY),
                                );
                            });
                        }
                    }
                    ui.add_space(40.0);
                });
        });
}

fn render_onboarding(ctx: &egui::Context, ui_state: &mut UiState, bridge: &AsyncBridge) {
    let profile_id = ui_state
        .profile
        .as_ref()
        .map(|profile| profile.profile_id.clone())
        .unwrap_or_default();
    let fingerprint: String = profile_id.chars().take(16).collect();

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                ui.set_max_width(320.0);
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(20, 60, 70, 220))
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgba_unmultiplied(90, 220, 215, 102),
                    ))
                    .corner_radius(12.0)
                    .inner_margin(18.0)
                    .show(ui, |ui| {
                        ui.heading(
                            egui::RichText::new("jyn")
                                .color(egui::Color32::from_rgb(0x9d, 0xf6, 0xee)),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("A keypair was born on this machine.")
                                .color(egui::Color32::from_rgb(0xd7, 0xf2, 0xef)),
                        );
                        ui.monospace(
                            egui::RichText::new(format!("⚿ {fingerprint}…"))
                                .small()
                                .color(egui::Color32::from_rgb(0x6f, 0xd8, 0xd0)),
                        );
                        ui.label(
                            egui::RichText::new("no account · no email · no server")
                                .small()
                                .color(egui::Color32::from_rgb(0x7f, 0xb8, 0xb6)),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(
                                "⚠ No key backup yet — this identity lives on this disk alone.",
                            )
                            .small()
                            .color(egui::Color32::from_rgb(0xf0, 0xa5, 0x66)),
                        );
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new("What should your friends call you?")
                                .color(egui::Color32::from_rgb(0xe6, 0xfb, 0xf8)),
                        );
                        ui.add_space(4.0);
                        ui.text_edit_singleline(&mut ui_state.onboarding_name_input);
                        ui.add_space(12.0);

                        let can_start = !ui_state.onboarding_name_input.trim().is_empty();
                        if ui
                            .add_enabled(can_start, egui::Button::new("Step into the river"))
                            .clicked()
                        {
                            let profile = ui_state.profile.clone();
                            let (visibility, lifetime) = profile
                                .map(|profile| {
                                    (profile.default_visibility, profile.default_lifetime_secs)
                                })
                                .unwrap_or((Visibility::Friends, None));
                            let _ = bridge.send(NetworkCommand::UpdateProfile {
                                display_name: ui_state.onboarding_name_input.trim().to_owned(),
                                bio: String::new(),
                                default_visibility: visibility,
                                default_lifetime_secs: lifetime,
                                mark_onboarded: true,
                            });
                        }
                    });
            });
        });
}

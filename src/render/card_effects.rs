use std::collections::HashMap;

use bevy::prelude::*;
use bevy::render::camera::ClearColorConfig;
use bevy::window::PrimaryWindow;
use bevy_egui::egui;

use super::water_material::{WaterMaterial, WaterMode};

/// Horizontal position of the current spine, in egui points from the window's
/// left edge (matches the design handoff: left 30px, width 11px).
const SPINE_LEFT_PX: f32 = 30.0;
const SPINE_WIDTH_PX: f32 = 11.0;
const SPINE_GLOW_WIDTH_PX: f32 = 46.0;

const Z_BACKGROUND: f32 = -100.0;
const Z_SPINE_GLOW: f32 = -60.0;
const Z_SPINE: f32 = -50.0;
const Z_WATERLINE: f32 = -10.0;

/// One card's water effect, reported by the egui layer every frame.
#[derive(Debug, Clone)]
pub struct CardEffect {
    /// Stable key (post id) for entity reuse.
    pub id: String,
    /// Final (drift-translated) card rect in egui points.
    pub rect: egui::Rect,
    /// Height of the waterline band as a fraction of the card (0 = none).
    pub band_frac: f32,
    /// Warm/amber variant for low-lifetime cards.
    pub warm: bool,
}

/// Per-frame contract between the egui layer and the Bevy water underlay.
///
/// `ui_system` rebuilds `cards` (and the scroll viewport clip) every frame;
/// `sync_card_effects` — chained directly after it, before rendering — maps
/// them onto quad entities, so effects track the UI with zero frame lag.
#[derive(Resource, Default)]
pub struct CardEffects {
    pub cards: Vec<CardEffect>,
    pub scroll_clip: Option<egui::Rect>,
}

/// The three static underlay quads that track the window rather than a card.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StaticQuad {
    Background,
    Spine,
    SpineGlow,
}

#[derive(Component)]
pub(crate) struct WaterlineQuad;

/// Shared unit-rectangle mesh scaled per entity.
#[derive(Resource)]
pub(crate) struct UnitQuad(Handle<Mesh>);

pub(crate) fn setup_render(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
) {
    commands.spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb_u8(7, 24, 32)),
            ..default()
        },
    ));

    let unit_quad = meshes.add(Rectangle::new(1.0, 1.0));

    commands.spawn((
        StaticQuad::Background,
        Mesh2d(unit_quad.clone()),
        MeshMaterial2d(materials.add(WaterMaterial::new(WaterMode::Background))),
        Transform::from_xyz(0.0, 0.0, Z_BACKGROUND),
    ));

    let mut glow = WaterMaterial::new(WaterMode::Glow);
    glow.settings.tint = Vec4::new(0.235, 0.882, 0.882, 0.4);
    commands.spawn((
        StaticQuad::SpineGlow,
        Mesh2d(unit_quad.clone()),
        MeshMaterial2d(materials.add(glow)),
        Transform::from_xyz(0.0, 0.0, Z_SPINE_GLOW),
    ));

    commands.spawn((
        StaticQuad::Spine,
        Mesh2d(unit_quad.clone()),
        MeshMaterial2d(materials.add(WaterMaterial::new(WaterMode::Spine))),
        Transform::from_xyz(0.0, 0.0, Z_SPINE),
    ));

    commands.insert_resource(UnitQuad(unit_quad));
}

/// Convert an egui rect (points, origin top-left, y-down) to a world-space
/// transform for a unit quad (1 world unit = 1 logical pixel, origin center).
fn rect_to_transform(rect: egui::Rect, window_size: Vec2, z: f32) -> Transform {
    let center = rect.center();
    Transform::from_xyz(
        center.x - window_size.x / 2.0,
        window_size.y / 2.0 - center.y,
        z,
    )
    .with_scale(Vec3::new(
        rect.width().max(0.0),
        rect.height().max(0.0),
        1.0,
    ))
}

/// Query for the per-card waterline quads managed by [`sync_card_effects`].
type WaterlineQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Transform,
        &'static mut Visibility,
        &'static MeshMaterial2d<WaterMaterial>,
    ),
    (With<WaterlineQuad>, Without<StaticQuad>),
>;

#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_card_effects(
    mut commands: Commands,
    effects: Res<CardEffects>,
    windows: Query<&Window, With<PrimaryWindow>>,
    unit_quad: Option<Res<UnitQuad>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    mut static_quads: Query<
        (&StaticQuad, &mut Transform, &MeshMaterial2d<WaterMaterial>),
        Without<WaterlineQuad>,
    >,
    mut waterlines: WaterlineQuery,
    mut entity_by_id: Local<HashMap<String, Entity>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(unit_quad) = unit_quad else {
        return;
    };
    let window_size = Vec2::new(window.width(), window.height());
    let full_window = egui::Rect::from_min_size(
        egui::pos2(0.0, 0.0),
        egui::vec2(window_size.x, window_size.y),
    );

    let spine_rect = egui::Rect::from_min_size(
        egui::pos2(SPINE_LEFT_PX, 0.0),
        egui::vec2(SPINE_WIDTH_PX, window_size.y),
    );
    let glow_rect = egui::Rect::from_center_size(
        spine_rect.center(),
        egui::vec2(SPINE_GLOW_WIDTH_PX, window_size.y),
    );

    // The static quads track the window: background fills it, the current
    // spine (and its glow skirt) runs the full height at a fixed x.
    for (kind, mut transform, material) in &mut static_quads {
        match kind {
            StaticQuad::Background => {
                *transform = rect_to_transform(full_window, window_size, Z_BACKGROUND);
            }
            StaticQuad::Spine => {
                *transform = rect_to_transform(spine_rect, window_size, Z_SPINE);
                if let Some(material) = materials.get_mut(&material.0) {
                    material.settings.element_height_px = window_size.y;
                }
            }
            StaticQuad::SpineGlow => {
                *transform = rect_to_transform(glow_rect, window_size, Z_SPINE_GLOW);
            }
        }
    }

    // Per-card waterline quads, keyed by post id. Quads are shrunk to their
    // visible intersection with the scroll viewport; the uv `rect` uniform
    // keeps the band anchored to the full card while clipped.
    let clip = effects.scroll_clip.unwrap_or(full_window);
    let mut seen: HashMap<&str, &CardEffect> = HashMap::new();
    for card in &effects.cards {
        if card.band_frac > 0.0 {
            seen.insert(card.id.as_str(), card);
        }
    }

    // Update or hide existing quads; despawn ones whose card disappeared.
    let mut stale: Vec<String> = Vec::new();
    for (entity, mut transform, mut visibility, material) in &mut waterlines {
        let id = entity_by_id
            .iter()
            .find_map(|(id, e)| (*e == entity).then(|| id.clone()));
        let Some(id) = id else {
            commands.entity(entity).despawn();
            continue;
        };
        match seen.remove(id.as_str()) {
            Some(card) => {
                let visible = card.rect.intersect(clip);
                if visible.width() <= 0.0 || visible.height() <= 0.0 {
                    *visibility = Visibility::Hidden;
                    continue;
                }
                *visibility = Visibility::Visible;
                *transform = rect_to_transform(visible, window_size, Z_WATERLINE);
                if let Some(material) = materials.get_mut(&material.0) {
                    apply_card_settings(material, card, visible);
                }
            }
            None => {
                commands.entity(entity).despawn();
                stale.push(id);
            }
        }
    }
    for id in stale {
        entity_by_id.remove(&id);
    }

    // Spawn quads for new cards.
    for (id, card) in seen {
        let visible = card.rect.intersect(clip);
        if visible.width() <= 0.0 || visible.height() <= 0.0 {
            continue;
        }
        let mut material = WaterMaterial::new(WaterMode::Waterline);
        apply_card_settings(&mut material, card, visible);
        let entity = commands
            .spawn((
                WaterlineQuad,
                Mesh2d(unit_quad.0.clone()),
                MeshMaterial2d(materials.add(material)),
                rect_to_transform(visible, window_size, Z_WATERLINE),
            ))
            .id();
        entity_by_id.insert(id.to_owned(), entity);
    }
}

fn apply_card_settings(material: &mut WaterMaterial, card: &CardEffect, visible: egui::Rect) {
    material.settings.band_frac = card.band_frac;
    material.settings.element_height_px = card.rect.height();
    material.settings.tint = if card.warm {
        // Warm/amber low-lifetime variant.
        Vec4::new(1.9, 0.85, 0.42, 1.0)
    } else {
        Vec4::ONE
    };
    // Map the visible sub-rect back into full-card uv space.
    let size = card.rect.size();
    material.settings.rect = Vec4::new(
        (visible.min.x - card.rect.min.x) / size.x,
        (visible.min.y - card.rect.min.y) / size.y,
        (visible.max.x - card.rect.min.x) / size.x,
        (visible.max.y - card.rect.min.y) / size.y,
    );
}

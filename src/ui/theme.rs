//! The "Current & Still" egui theme: deep-water visuals over the Bevy
//! underlay, mono HUD accents, and design-token colors from the handoff.
//!
//! Space Mono is loaded at runtime when `assets/fonts/SpaceMono-*.ttf`
//! exists (OFL-licensed, not vendored in the repository); egui's built-in
//! monospace stands in otherwise.

use bevy_egui::egui::{self, Color32};

// Design tokens (option 1b — "Current & Still").
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xe6, 0xfb, 0xf8);
pub const TEXT_BODY: Color32 = Color32::from_rgb(0xd7, 0xf2, 0xef);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0x7f, 0xb8, 0xb6);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x59, 0x91, 0x8d);
pub const MONO_HUD: Color32 = Color32::from_rgb(0x6f, 0xd8, 0xd0);
pub const ACCENT: Color32 = Color32::from_rgb(0x6f, 0xe6, 0xdd);
pub const ACCENT_BRIGHT: Color32 = Color32::from_rgb(0x9d, 0xf6, 0xee);
pub const HEART_PINK: Color32 = Color32::from_rgb(0xf3, 0x9a, 0xc8);
pub const PROVENANCE_PINK: Color32 = Color32::from_rgb(0xf7, 0xc3, 0xde);
pub const WARM_TEXT: Color32 = Color32::from_rgb(0xff, 0xd8, 0xb3);
pub const WARM_STRONG: Color32 = Color32::from_rgb(0xf0, 0xa5, 0x66);
pub const STONE_TEXT: Color32 = Color32::from_rgb(0xc3, 0xd0, 0xd8);

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);

    let mut style = (*ctx.style()).clone();

    style.visuals.dark_mode = true;
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.window_fill = Color32::from_rgba_unmultiplied(16, 50, 60, 240);
    style.visuals.override_text_color = None;
    style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(60, 200, 195, 90);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT_BRIGHT);
    style.visuals.hyperlink_color = ACCENT;

    let widgets = &mut style.visuals.widgets;
    let idle_fill = Color32::from_rgba_unmultiplied(20, 70, 78, 178);
    let hover_fill = Color32::from_rgba_unmultiplied(30, 95, 104, 200);
    let stroke = egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(110, 220, 215, 90));
    widgets.inactive.bg_fill = idle_fill;
    widgets.inactive.weak_bg_fill = idle_fill;
    widgets.inactive.bg_stroke = stroke;
    widgets.inactive.fg_stroke = egui::Stroke::new(1.0, Color32::from_rgb(0xbf, 0xee, 0xea));
    widgets.hovered.bg_fill = hover_fill;
    widgets.hovered.weak_bg_fill = hover_fill;
    widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    widgets.hovered.fg_stroke = egui::Stroke::new(1.2, ACCENT_BRIGHT);
    widgets.active.bg_fill = hover_fill;
    widgets.active.weak_bg_fill = hover_fill;
    widgets.active.fg_stroke = egui::Stroke::new(1.2, ACCENT_BRIGHT);
    widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_BODY);
    widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(60, 110, 118, 100));
    widgets.open.bg_fill = hover_fill;
    widgets.open.weak_bg_fill = hover_fill;

    use egui::{FontFamily, FontId, TextStyle};
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(20.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (
            TextStyle::Monospace,
            FontId::new(11.0, FontFamily::Monospace),
        ),
        (
            TextStyle::Button,
            FontId::new(12.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
    ]
    .into();

    ctx.set_style(style);
}

/// Installs Space Mono for the monospace family when the OFL files are
/// present next to the crate or the executable (dev run), or in a macOS
/// `.app` bundle's `Contents/Resources/fonts` (packaged build).
fn install_fonts(ctx: &egui::Context) {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
    let candidates = [
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/fonts"),
        exe_dir
            .clone()
            .map(|dir| dir.join("assets/fonts"))
            .unwrap_or_default(),
        // macOS bundle: Contents/MacOS/jyn -> Contents/Resources/fonts.
        exe_dir
            .map(|dir| dir.join("../Resources/fonts"))
            .unwrap_or_default(),
    ];
    let regular = candidates
        .iter()
        .map(|dir| dir.join("SpaceMono-Regular.ttf"))
        .find(|path| path.exists());
    let Some(regular) = regular else {
        return;
    };
    let Ok(bytes) = std::fs::read(&regular) else {
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "space-mono".to_owned(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "space-mono".to_owned());
    ctx.set_fonts(fonts);
}

//! Photo decoding for inline display: decode, downscale, upload once as an
//! egui texture cached by blob hash.

use std::path::Path;

use anyhow::{Context, Result};
use bevy_egui::egui;

/// Longest edge for inline display; the original blob stays untouched.
const MAX_INLINE_EDGE: u32 = 1024;

pub fn load_texture(
    ctx: &egui::Context,
    blob_hash: &str,
    path: &Path,
) -> Result<egui::TextureHandle> {
    let image =
        image::open(path).with_context(|| format!("failed to decode image {}", path.display()))?;
    let image = if image.width() > MAX_INLINE_EDGE || image.height() > MAX_INLINE_EDGE {
        image.thumbnail(MAX_INLINE_EDGE, MAX_INLINE_EDGE)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color_image =
        egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_flat_samples().as_slice());
    Ok(ctx.load_texture(
        format!("photo-{blob_hash}"),
        color_image,
        egui::TextureOptions::LINEAR,
    ))
}

//! Media handling for posts: in-app voice recording, audio playback, photo
//! textures, and the UI-side bookkeeping for blob-backed attachments.
//!
//! `MediaState` is a *non-send* Bevy resource: it owns the cpal input stream
//! and the rodio output stream, both of which must stay on the main thread.

pub mod photo;
pub mod playback;
pub mod record;
pub mod video;
pub mod waveform;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use bevy_egui::egui;

use crate::domain::MediaKind;

pub use record::{ActiveRecording, RecordedAudio};

const PHOTO_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];
const AUDIO_EXTENSIONS: &[&str] = &["wav", "mp3", "flac", "ogg", "m4a"];
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "webm", "mkv", "avi"];

/// Classifies an attachment by file extension.
pub fn classify(path: &Path) -> MediaKind {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();
    if PHOTO_EXTENSIONS.contains(&extension.as_str()) {
        MediaKind::Photo
    } else if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        MediaKind::Audio
    } else if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        MediaKind::Video
    } else {
        MediaKind::File
    }
}

pub fn mime_for(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "m4a" => "audio/mp4",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        _ => "application/octet-stream",
    }
    .to_owned()
}

/// Materializes a copy of a cached blob under its original (sanitized) file
/// name, so "open" hands the OS something it can route to the right app —
/// the cache itself is content-addressed (`media-cache/<hash>`, no
/// extension), which would otherwise open PDFs in a text editor.
pub fn named_copy_for_opening(
    cache_path: &Path,
    blob_hash: &str,
    file_name: Option<&str>,
    mime: &str,
) -> anyhow::Result<PathBuf> {
    let short_hash: String = blob_hash.chars().take(8).collect();
    let name = file_name
        .map(sanitize_file_name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("{short_hash}{}", extension_for_mime(mime)));

    let named_dir = cache_path
        .parent()
        .map(|dir| dir.join("named"))
        .unwrap_or_else(|| PathBuf::from("named"));
    std::fs::create_dir_all(&named_dir)?;
    let target = named_dir.join(format!("{short_hash}-{name}"));
    if !target.exists() {
        std::fs::copy(cache_path, &target)?;
    }
    Ok(target)
}

/// Keeps only the final path component and drops control characters, so a
/// hostile attachment name can't escape the media cache.
fn sanitize_file_name(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim_matches('.');
    base.chars()
        .filter(|c| !c.is_control() && *c != ':')
        .collect()
}

/// A best-effort extension for attachments that arrived without a name.
fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "application/pdf" => ".pdf",
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        "audio/wav" => ".wav",
        "audio/mpeg" => ".mp3",
        "audio/flac" => ".flac",
        "audio/ogg" => ".ogg",
        "audio/mp4" => ".m4a",
        "video/mp4" => ".mp4",
        "video/quicktime" => ".mov",
        "video/webm" => ".webm",
        "video/x-matroska" => ".mkv",
        _ => "",
    }
}

/// Non-send UI-side media state.
pub struct MediaState {
    /// Where fetched blobs land (`<data_dir>/media-cache/<hash>`).
    pub media_cache_dir: PathBuf,
    /// Scratch space for fresh recordings before they are cast.
    pub recording_dir: PathBuf,
    pub recording: Option<ActiveRecording>,
    pub mic_error: Option<String>,
    /// A finished voice note staged on the composer.
    pub pending_audio: Option<RecordedAudio>,
    /// Files staged on the composer (photos, videos, anything).
    pub pending_attachments: Vec<PathBuf>,
    pub playback: playback::Playback,
    textures: HashMap<String, egui::TextureHandle>,
    texture_failures: HashSet<String>,
    /// Blob hashes we already asked the runtime to fetch.
    pub fetch_requested: HashSet<String>,
    /// Blob hash → local file, from fetch completions and own casts.
    local_paths: HashMap<String, PathBuf>,
    /// Pending file-picker dialog (runs on its own thread).
    pub file_dialog: Option<flume::Receiver<Option<Vec<PathBuf>>>>,
    /// The one video playing inline right now: (post id + blob hash key,
    /// player, texture updated per frame).
    pub active_video: Option<ActiveVideo>,
}

pub struct ActiveVideo {
    pub key: String,
    /// The post to route the extracted audio track to once it's ready.
    pub audio_post_id: String,
    pub player: video::VideoPlayer,
    pub texture: Option<egui::TextureHandle>,
}

impl MediaState {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            media_cache_dir: data_dir.join("media-cache"),
            recording_dir: data_dir.join("recordings"),
            recording: None,
            mic_error: None,
            pending_audio: None,
            pending_attachments: Vec::new(),
            playback: playback::Playback::default(),
            textures: HashMap::new(),
            texture_failures: HashSet::new(),
            fetch_requested: HashSet::new(),
            local_paths: HashMap::new(),
            file_dialog: None,
            active_video: None,
        }
    }

    /// The local file for a blob, if it exists (fetched, or cached on cast).
    pub fn local_path_for(&mut self, blob_hash: &str) -> Option<PathBuf> {
        if let Some(path) = self.local_paths.get(blob_hash) {
            if path.exists() {
                return Some(path.clone());
            }
        }
        let cached = self.media_cache_dir.join(blob_hash);
        if cached.exists() {
            self.local_paths
                .insert(blob_hash.to_owned(), cached.clone());
            return Some(cached);
        }
        None
    }

    pub fn record_local_path(&mut self, blob_hash: String, path: PathBuf) {
        self.fetch_requested.remove(&blob_hash);
        self.local_paths.insert(blob_hash, path);
    }

    /// The cached texture for a photo blob, loading it on first use.
    pub fn texture_for(
        &mut self,
        ctx: &egui::Context,
        blob_hash: &str,
        path: &Path,
    ) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.textures.get(blob_hash) {
            return Some(texture.clone());
        }
        if self.texture_failures.contains(blob_hash) {
            return None;
        }
        match photo::load_texture(ctx, blob_hash, path) {
            Ok(texture) => {
                self.textures.insert(blob_hash.to_owned(), texture.clone());
                Some(texture)
            }
            Err(err) => {
                tracing::warn!("failed to load photo texture: {err:#}");
                self.texture_failures.insert(blob_hash.to_owned());
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_copies_carry_the_original_name_and_extension() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let cache = dir.path().join("media-cache");
        std::fs::create_dir_all(&cache)?;
        let blob = cache.join("abcdef1234567890");
        std::fs::write(&blob, b"%PDF-1.4 fake")?;

        let named = named_copy_for_opening(
            &blob,
            "abcdef1234567890",
            Some("Lighthouse Essay.pdf"),
            "application/pdf",
        )?;
        assert!(named.ends_with("named/abcdef12-Lighthouse Essay.pdf"));
        assert_eq!(std::fs::read(&named)?, b"%PDF-1.4 fake");

        // No name: fall back to hash + mime extension.
        let unnamed = named_copy_for_opening(&blob, "abcdef1234567890", None, "application/pdf")?;
        assert!(unnamed.ends_with("named/abcdef12-abcdef12.pdf"));

        // Hostile names cannot escape the cache directory.
        let hostile = named_copy_for_opening(
            &blob,
            "abcdef1234567890",
            Some("../../../etc/passwd"),
            "application/pdf",
        )?;
        assert!(hostile.ends_with("named/abcdef12-passwd"));
        assert!(hostile.starts_with(&cache));

        Ok(())
    }
}

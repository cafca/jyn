//! Media plumbing: blob cache lookups and voice-note analysis. Capture and
//! playback live entirely in Flutter; the core only deals in files.

use std::path::Path;

use anyhow::Result;

use crate::media::WavSummary;
use crate::runtime::AppRuntime;

/// Reduces a recorded WAV to the duration + waveform peaks that travel
/// inside the post operation. Call after the recorder finished, pass the
/// result into the matching `MediaDraftInput`.
pub fn voice_note_summary(wav_path: String) -> Result<WavSummary> {
    crate::media::wav_summary(Path::new(&wav_path))
}

/// The local file for a blob if it's already in the media cache.
pub fn local_media_path(blob_hash: String) -> Result<Option<String>> {
    Ok(AppRuntime::get()?
        .local_media_path(&blob_hash)
        .map(|path| path.to_string_lossy().into_owned()))
}

/// Fetches a blob into the media cache unless it's local or already in
/// flight. Completion arrives as a MediaReady / MediaFailed event.
pub fn request_media(blob_hash: String) -> Result<()> {
    AppRuntime::get()?.request_media(blob_hash)
}

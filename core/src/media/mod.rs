//! Media handling for posts: classification, blob-cache bookkeeping, and
//! waveform peak extraction.
//!
//! Capture and playback live in the Flutter app; the core only deals in
//! files. Voice notes arrive as WAV paths recorded by the app, and
//! [`wav_summary`] reduces them to the peaks + duration that travel inside
//! the post operation.

pub mod blob_crypto;
pub mod waveform;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::domain::MediaKind;

use waveform::{peaks, WAVEFORM_BUCKETS};

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

/// Duration and waveform peaks for a recorded voice note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavSummary {
    pub duration_ms: u64,
    pub waveform: Vec<u8>,
}

/// Reads a WAV file (any sample format, any channel count) and reduces it to
/// the duration + [`WAVEFORM_BUCKETS`] peaks a post carries.
pub fn wav_summary(path: &Path) -> anyhow::Result<WavSummary> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("failed to open WAV at {}", path.display()))?;
    let spec = reader.spec();
    anyhow::ensure!(spec.channels > 0, "WAV has zero channels");

    // Downmix to mono by averaging channels.
    let channels = spec.channels as usize;
    let mono: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            let samples: Result<Vec<f32>, _> = reader.samples::<f32>().collect();
            downmix(&samples.context("failed to decode WAV samples")?, channels)
        }
        hound::SampleFormat::Int => {
            let max = (1_i64 << (spec.bits_per_sample - 1)) as f32;
            let samples: Result<Vec<i32>, _> = reader.samples::<i32>().collect();
            let floats: Vec<f32> = samples
                .context("failed to decode WAV samples")?
                .iter()
                .map(|s| *s as f32 / max)
                .collect();
            downmix(&floats, channels)
        }
    };

    let duration_ms = (mono.len() as u64)
        .saturating_mul(1000)
        .checked_div(spec.sample_rate as u64)
        .unwrap_or(0);

    Ok(WavSummary {
        duration_ms,
        waveform: peaks(&mono, WAVEFORM_BUCKETS),
    })
}

fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
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

/// Bytes over which an attachment of a given kind is refused at post time and
/// hidden (never fetched) at render time. This keeps any single file from
/// dominating — or thrashing — the media cache. Audio and generic files are
/// uncapped. Mirrored in `app/lib/src/media_limits.dart`; keep the two in sync.
pub const MAX_PHOTO_BYTES: u64 = 15 * 1024 * 1024;
pub const MAX_VIDEO_BYTES: u64 = 200 * 1024 * 1024;

/// The size ceiling for a media kind, or `None` if the kind is uncapped.
pub fn max_bytes_for_kind(kind: MediaKind) -> Option<u64> {
    match kind {
        MediaKind::Photo => Some(MAX_PHOTO_BYTES),
        MediaKind::Video => Some(MAX_VIDEO_BYTES),
        MediaKind::Audio | MediaKind::File => None,
    }
}

/// Soft ceiling for the on-demand media cache. The cache is a disposable,
/// re-materializable view of the pinned blob store (see [`crate::bridge`]);
/// once it grows past this, the least-recently-touched files are evicted and
/// re-exported from the store on next view.
pub const MEDIA_CACHE_BUDGET_BYTES: u64 = 512 * 1024 * 1024;

/// Removes a blob's materialized cache file, if present. The pinned blob
/// store remains the source of truth; the file re-materializes on next fetch.
/// Best-effort: a missing file or an unlink error is not fatal.
pub fn prune_cached(media_cache_dir: &Path, blob_hash: &str) {
    let path = media_cache_dir.join(blob_hash);
    if path.exists() {
        if let Err(err) = std::fs::remove_file(&path) {
            tracing::debug!("failed to prune media cache file {}: {err}", path.display());
        }
    }
}

/// Keeps the media cache under `budget_bytes` by evicting oldest-first (by
/// modification time). The cache is derived from the blob store, so eviction
/// never loses data — evicted blobs re-export on next view. Best-effort.
///
/// `keep` is the blob we just materialized: it is never evicted, so a single
/// attachment larger than the whole budget still survives the pass that would
/// otherwise delete the very file we are about to hand the UI (the cache just
/// sits over budget until something smaller can be reclaimed instead).
pub fn evict_to_budget(media_cache_dir: &Path, budget_bytes: u64, keep: &str) {
    let mut entries: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    let mut total: u64 = 0;
    let read_dir = match std::fs::read_dir(media_cache_dir) {
        Ok(read_dir) => read_dir,
        Err(_) => return,
    };
    for entry in read_dir.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        // Count every file toward disk usage, but never evict the one we just
        // wrote — it is in flight to the UI.
        total = total.saturating_add(metadata.len());
        if entry.file_name().to_string_lossy() == keep {
            continue;
        }
        let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
        entries.push((entry.path(), metadata.len(), modified));
    }
    if total <= budget_bytes {
        return;
    }
    // Oldest first, so the just-written file is the last candidate to go.
    entries.sort_by_key(|(_, _, modified)| *modified);
    for (path, len, _) in entries {
        if total <= budget_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// Blob-cache bookkeeping: which blobs have local files, which fetches are
/// already in flight.
pub struct MediaCache {
    /// Where fetched blobs land (`<data_dir>/media-cache/<hash>`).
    pub media_cache_dir: PathBuf,
    /// Blob hashes we already asked the runtime to fetch.
    pub fetch_requested: HashSet<String>,
    /// Blob hash → local file, from fetch completions and own casts.
    local_paths: HashMap<String, PathBuf>,
}

impl MediaCache {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            media_cache_dir: data_dir.join("media-cache"),
            fetch_requested: HashSet::new(),
            local_paths: HashMap::new(),
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

    #[test]
    fn size_caps_apply_to_photos_and_videos_only() {
        assert_eq!(max_bytes_for_kind(MediaKind::Photo), Some(15 * 1024 * 1024));
        assert_eq!(
            max_bytes_for_kind(MediaKind::Video),
            Some(200 * 1024 * 1024)
        );
        assert_eq!(max_bytes_for_kind(MediaKind::Audio), None);
        assert_eq!(max_bytes_for_kind(MediaKind::File), None);
    }

    #[test]
    fn prune_cached_removes_the_file_and_tolerates_absence() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let blob = "abc123def456";
        std::fs::write(dir.path().join(blob), b"payload")?;

        prune_cached(dir.path(), blob);
        assert!(!dir.path().join(blob).exists());

        // Idempotent: pruning an already-absent blob is a no-op.
        prune_cached(dir.path(), blob);
        Ok(())
    }

    #[test]
    fn evict_to_budget_drops_oldest_first_until_under_budget() -> anyhow::Result<()> {
        use std::time::{Duration, SystemTime};

        let dir = tempfile::tempdir()?;
        // Three 100-byte files, stamped oldest → newest.
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        for (i, name) in ["old", "mid", "new"].iter().enumerate() {
            std::fs::write(dir.path().join(name), vec![0u8; 100])?;
            std::fs::File::open(dir.path().join(name))?
                .set_modified(base + Duration::from_secs(i as u64 * 10))?;
        }

        // Budget holds ~1.5 files: the two oldest go, the newest survives.
        evict_to_budget(dir.path(), 150, "new");
        assert!(!dir.path().join("old").exists());
        assert!(!dir.path().join("mid").exists());
        assert!(dir.path().join("new").exists());
        Ok(())
    }

    #[test]
    fn evict_to_budget_under_budget_keeps_everything() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("a"), vec![0u8; 100])?;
        std::fs::write(dir.path().join("b"), vec![0u8; 100])?;

        evict_to_budget(dir.path(), 1024, "b");
        assert!(dir.path().join("a").exists());
        assert!(dir.path().join("b").exists());
        Ok(())
    }

    #[test]
    fn evict_to_budget_never_evicts_the_just_written_blob() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        // One file, larger than the whole budget: the pass must keep it rather
        // than delete the very blob we are about to hand the UI.
        std::fs::write(dir.path().join("huge"), vec![0u8; 1000])?;

        evict_to_budget(dir.path(), 512, "huge");
        assert!(dir.path().join("huge").exists());
        Ok(())
    }

    #[test]
    fn wav_summary_reads_duration_and_peaks() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("note.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 1000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec)?;
        // 500 samples at 1 kHz = 500 ms: quiet half then loud half.
        for _ in 0..250 {
            writer.write_sample(i16::MAX / 4)?;
        }
        for _ in 0..250 {
            writer.write_sample(i16::MAX / 2)?;
        }
        writer.finalize()?;

        let summary = wav_summary(&path)?;
        assert_eq!(summary.duration_ms, 500);
        assert_eq!(summary.waveform.len(), WAVEFORM_BUCKETS);
        // Loud half normalizes to 255, quiet half to ~128.
        assert_eq!(*summary.waveform.last().unwrap(), 255);
        assert!((125..=131).contains(&summary.waveform[0]));

        Ok(())
    }
}

//! Media handling for posts: classification, blob-cache bookkeeping, and
//! waveform peak extraction.
//!
//! Capture and playback live in the Flutter app; the core only deals in
//! files. Voice notes arrive as WAV paths recorded by the app, and
//! [`wav_summary`] reduces them to the peaks + duration that travel inside
//! the post operation.

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

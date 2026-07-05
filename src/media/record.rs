//! In-app voice recording via cpal.
//!
//! The `cpal::Stream` is `!Send`, so the active recording lives in the
//! non-send `MediaState` resource on the main thread; samples accumulate in
//! a shared buffer written from cpal's audio callback. Every failure maps to
//! an error the composer shows next to a disabled mic button — a missing or
//! denied microphone must never crash the app.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::waveform::{peaks, WAVEFORM_BUCKETS};

/// A finished voice note, ready to attach to a post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedAudio {
    pub path: PathBuf,
    pub duration_ms: u64,
    pub waveform: Vec<u8>,
}

pub struct ActiveRecording {
    // Dropping the stream stops capture.
    _stream: cpal::Stream,
    buffer: Arc<Mutex<Vec<f32>>>,
    level: Arc<AtomicU32>,
    sample_rate: u32,
    channels: u16,
    started: Instant,
}

impl ActiveRecording {
    /// Opens the default input device and starts capturing.
    pub fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no microphone available")?;
        let config = device
            .default_input_config()
            .context("microphone has no usable input configuration")?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();

        let buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
        let level = Arc::new(AtomicU32::new(0));
        let callback_buffer = Arc::clone(&buffer);
        let callback_level = Arc::clone(&level);

        let stream = device
            .build_input_stream(
                &config.into(),
                move |data: &[f32], _| {
                    let peak = data.iter().fold(0.0_f32, |max, s| max.max(s.abs()));
                    callback_level.store(peak.to_bits(), Ordering::Relaxed);
                    if let Ok(mut buffer) = callback_buffer.lock() {
                        buffer.extend_from_slice(data);
                    }
                },
                |err| tracing::warn!("microphone stream error: {err}"),
                None,
            )
            .context("failed to open microphone stream")?;
        stream.play().context("failed to start microphone stream")?;

        Ok(Self {
            _stream: stream,
            buffer,
            level,
            sample_rate,
            channels,
            started: Instant::now(),
        })
    }

    /// The most recent input peak (0..=1), for the composer's level meter.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed)).clamp(0.0, 1.0)
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// Stops capturing and writes a mono 16-bit WAV into `scratch_dir`.
    pub fn stop(self, scratch_dir: &Path) -> Result<RecordedAudio> {
        let samples = self
            .buffer
            .lock()
            .map_err(|_| anyhow::anyhow!("recording buffer poisoned"))?
            .clone();
        drop(self._stream);
        anyhow::ensure!(!samples.is_empty(), "nothing was recorded");

        // Downmix interleaved channels to mono.
        let channels = self.channels.max(1) as usize;
        let mono: Vec<f32> = samples
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect();

        std::fs::create_dir_all(scratch_dir).with_context(|| {
            format!(
                "failed to create recording directory {}",
                scratch_dir.display()
            )
        })?;
        let path = scratch_dir.join(format!(
            "voice-note-{}.wav",
            crate::profile::now_unix_secs()
        ));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec)
            .with_context(|| format!("failed to create WAV file {}", path.display()))?;
        for sample in &mono {
            writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
        }
        writer.finalize().context("failed to finalize WAV file")?;

        let duration_ms = (mono.len() as u64 * 1000) / self.sample_rate.max(1) as u64;
        Ok(RecordedAudio {
            path,
            duration_ms,
            waveform: peaks(&mono, WAVEFORM_BUCKETS),
        })
    }
}

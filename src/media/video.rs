//! Inline video playback by driving the system `ffmpeg` binary.
//!
//! Instead of linking libav (version-fragile, heavy build deps), a spawned
//! `ffmpeg` process decodes to raw RGBA on stdout; a pacing thread keeps the
//! latest due frame in a shared slot the UI uploads as a texture. Audio is
//! extracted to a temporary WAV and played through rodio.
//!
//! The degradation boundary is runtime: no `ffmpeg` on the PATH (or any
//! decode failure) means [`VideoPlayer::open`] fails and the card falls back
//! to the open-externally file chip.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// The newest decoded frame, ready for texture upload.
pub struct VideoFrame {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

pub struct VideoPlayer {
    pub width: u32,
    pub height: u32,
    latest_frame: Arc<Mutex<Option<VideoFrame>>>,
    finished: Arc<AtomicBool>,
    process: Child,
    audio_rx: flume::Receiver<PathBuf>,
    _decode_thread: std::thread::JoinHandle<()>,
}

impl VideoPlayer {
    /// Probes and starts decoding a local video file, downscaled at decode
    /// time to at most `max_display_width` physical pixels — everything
    /// downstream (pipe, copies, pixel conversion, texture upload) scales
    /// with that. Fails cleanly when ffmpeg/ffprobe are unavailable or the
    /// file cannot be decoded.
    pub fn open(path: &Path, scratch_dir: &Path, max_display_width: u32) -> Result<Self> {
        let (source_width, source_height, fps) = probe(path)?;

        // Explicit even output dimensions, so the raw-frame byte math below
        // is exact (a ≤1px aspect rounding is invisible).
        let max_width = max_display_width.clamp(64, 1920) & !1;
        let (width, height) = if source_width > max_width {
            let scaled_height =
                ((source_height as u64 * max_width as u64) / source_width as u64) as u32;
            (max_width, scaled_height.max(2) & !1)
        } else {
            (source_width & !1, source_height & !1)
        };

        let mut command = Command::new("ffmpeg");
        command
            // Hardware decode where the codec supports it; ffmpeg falls back
            // to software on its own otherwise.
            .args(["-v", "error", "-hwaccel", "videotoolbox", "-i"])
            .arg(path);
        if (width, height) != (source_width, source_height) {
            command.args(["-vf", &format!("scale={width}:{height}")]);
        }
        let mut process = command
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()
            .context("failed to start ffmpeg (is it installed?)")?;
        let stdout = process.stdout.take().context("ffmpeg has no stdout")?;

        // Audio extraction can take seconds on long videos; run it off-thread
        // and let the player poll for the result. Silent videos and failures
        // just play without sound.
        let (audio_tx, audio_rx) = flume::bounded(1);
        {
            let path = path.to_owned();
            let scratch_dir = scratch_dir.to_owned();
            std::thread::spawn(move || {
                if let Some(audio) = extract_audio(&path, &scratch_dir) {
                    let _ = audio_tx.send(audio);
                }
            });
        }

        let latest_frame = Arc::new(Mutex::new(None));
        let finished = Arc::new(AtomicBool::new(false));
        let frame_slot = Arc::clone(&latest_frame);
        let finished_flag = Arc::clone(&finished);
        let frame_bytes = width as usize * height as usize * 4;
        let frame_interval = Duration::from_secs_f64(1.0 / fps.max(1.0));

        let decode_thread = std::thread::Builder::new()
            .name("jyn-video-decode".into())
            .spawn(move || {
                let mut reader = std::io::BufReader::new(stdout);
                let started = Instant::now();
                let mut frame_index: u64 = 0;
                let mut buffer = vec![0u8; frame_bytes];
                loop {
                    if reader.read_exact(&mut buffer).is_err() {
                        break;
                    }
                    // Pace frames against the wall clock; drop nothing (the
                    // pipe applies backpressure), just wait until due.
                    let due = started + frame_interval * frame_index as u32;
                    let now = Instant::now();
                    if due > now {
                        std::thread::sleep(due - now);
                    }
                    if let Ok(mut slot) = frame_slot.lock() {
                        *slot = Some(VideoFrame {
                            size: [width as usize, height as usize],
                            rgba: buffer.clone(),
                        });
                    }
                    frame_index += 1;
                }
                finished_flag.store(true, Ordering::Relaxed);
            })
            .context("failed to spawn video decode thread")?;

        Ok(Self {
            width,
            height,
            latest_frame,
            finished,
            process,
            audio_rx,
            _decode_thread: decode_thread,
        })
    }

    /// Takes the newest decoded frame, if a new one arrived since last call.
    pub fn take_frame(&self) -> Option<VideoFrame> {
        self.latest_frame.lock().ok()?.take()
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }

    /// The extracted audio track, once ready (yields at most once).
    pub fn poll_audio(&self) -> Option<PathBuf> {
        self.audio_rx.try_recv().ok()
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// Returns (width, height, fps) via ffprobe.
fn probe(path: &Path) -> Result<(u32, u32, f64)> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .context("failed to run ffprobe (is it installed?)")?;
    anyhow::ensure!(output.status.success(), "ffprobe could not read the video");

    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next().context("ffprobe returned no stream")?;
    let mut parts = line.trim().split(',');
    let width: u32 = parts
        .next()
        .and_then(|value| value.parse().ok())
        .context("missing video width")?;
    let height: u32 = parts
        .next()
        .and_then(|value| value.parse().ok())
        .context("missing video height")?;
    let fps = parts
        .next()
        .map(parse_frame_rate)
        .unwrap_or(30.0)
        .clamp(1.0, 120.0);
    Ok((width, height, fps))
}

/// Parses ffprobe's rational frame rate ("30000/1001", "25/1").
fn parse_frame_rate(value: &str) -> f64 {
    let mut parts = value.trim().split('/');
    let numerator: f64 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .unwrap_or(30.0);
    let denominator: f64 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .unwrap_or(1.0);
    if denominator.abs() <= f64::EPSILON {
        return 30.0;
    }
    numerator / denominator
}

/// Extracts the audio track to a temporary WAV, if there is one.
fn extract_audio(path: &Path, scratch_dir: &Path) -> Option<PathBuf> {
    std::fs::create_dir_all(scratch_dir).ok()?;
    let target = scratch_dir.join(format!(
        "video-audio-{}.wav",
        crate::profile::now_unix_secs()
    ));
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(path)
        .args(["-vn", "-acodec", "pcm_s16le"])
        .arg(&target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    (status.success() && target.exists()).then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rates_parse_from_rationals() {
        assert!((parse_frame_rate("30000/1001") - 29.97).abs() < 0.01);
        assert!((parse_frame_rate("25/1") - 25.0).abs() < f64::EPSILON);
        assert!((parse_frame_rate("garbage") - 30.0).abs() < f64::EPSILON);
    }

    /// End-to-end decode against the real ffmpeg binary; skipped silently
    /// where ffmpeg isn't installed (e.g. CI).
    #[test]
    fn decodes_frames_from_a_generated_test_video() {
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            eprintln!("skipping: ffmpeg not installed");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let video_path = dir.path().join("test.mp4");
        let status = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1:size=64x48:rate=10",
            ])
            .arg(&video_path)
            .status()
            .unwrap();
        assert!(status.success(), "failed to generate test video");

        // No upscaling: small videos keep their dimensions.
        let player = VideoPlayer::open(&video_path, dir.path(), 1024).unwrap();
        assert_eq!((player.width, player.height), (64, 48));

        let deadline = Instant::now() + Duration::from_secs(5);
        let frame = loop {
            if let Some(frame) = player.take_frame() {
                break frame;
            }
            assert!(Instant::now() < deadline, "no frame arrived within 5s");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(frame.size, [64, 48]);
        assert_eq!(frame.rgba.len(), 64 * 48 * 4);
    }

    /// Large videos are downscaled at decode time to the display width, and
    /// the predicted even dimensions match what ffmpeg actually emits (the
    /// raw-frame reader depends on that byte math being exact).
    #[test]
    fn decode_downscales_to_the_display_width() {
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            eprintln!("skipping: ffmpeg not installed");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let video_path = dir.path().join("large.mp4");
        let status = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1:size=1280x720:rate=10",
            ])
            .arg(&video_path)
            .status()
            .unwrap();
        assert!(status.success(), "failed to generate test video");

        let player = VideoPlayer::open(&video_path, dir.path(), 464).unwrap();
        assert_eq!(player.width, 464);
        assert_eq!(player.height, 260); // 720 * 464/1280 = 261 → forced even.

        let deadline = Instant::now() + Duration::from_secs(5);
        let frame = loop {
            if let Some(frame) = player.take_frame() {
                break frame;
            }
            assert!(Instant::now() < deadline, "no frame arrived within 5s");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(frame.size, [464, 260]);
        assert_eq!(frame.rgba.len(), 464 * 260 * 4);
    }
}

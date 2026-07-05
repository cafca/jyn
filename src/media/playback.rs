//! Audio playback via rodio. One post plays at a time; the output stream is
//! `!Send` and lives in the non-send `MediaState`. Bevy is built without
//! `bevy_audio`, so there is no device contention.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};

#[derive(Default)]
pub struct Playback {
    stream: Option<(OutputStream, OutputStreamHandle)>,
    current: Option<(String, Sink)>,
}

impl Playback {
    /// Starts, pauses or resumes playback of `path` for the given post.
    /// Returns whether the post is playing after the toggle.
    pub fn toggle(&mut self, post_id: &str, path: &Path) -> Result<bool> {
        if let Some((current_id, sink)) = &self.current {
            if current_id == post_id && !sink.empty() {
                if sink.is_paused() {
                    sink.play();
                    return Ok(true);
                }
                sink.pause();
                return Ok(false);
            }
        }

        let handle = self.output_handle()?;
        let file = File::open(path)
            .with_context(|| format!("failed to open audio file {}", path.display()))?;
        let source = Decoder::new(BufReader::new(file))
            .with_context(|| format!("failed to decode audio file {}", path.display()))?;
        let sink = Sink::try_new(&handle).context("failed to open audio sink")?;
        sink.append(source);
        sink.play();
        self.current = Some((post_id.to_owned(), sink));
        Ok(true)
    }

    pub fn is_playing(&self, post_id: &str) -> bool {
        self.current
            .as_ref()
            .map(|(current_id, sink)| current_id == post_id && !sink.is_paused() && !sink.empty())
            .unwrap_or(false)
    }

    fn output_handle(&mut self) -> Result<OutputStreamHandle> {
        if self.stream.is_none() {
            let (stream, handle) =
                OutputStream::try_default().context("no audio output device available")?;
            self.stream = Some((stream, handle));
        }
        Ok(self
            .stream
            .as_ref()
            .expect("output stream just initialized")
            .1
            .clone())
    }
}

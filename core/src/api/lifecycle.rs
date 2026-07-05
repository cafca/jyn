//! Node lifecycle and the event stream.

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::Result;

use crate::frb_generated::StreamSink;
use crate::runtime::{AppRuntime, EventSink, JynEvent};

static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
    if let Some(guard) = crate::logging::init_logging() {
        let _ = LOG_GUARD.set(guard);
    }
}

/// Starts the p2panda node (idempotent). `data_dir_override` beats the
/// `JYN_DATA_DIR` environment variable and the platform default; it only
/// takes effect on the first call of the process.
pub fn start_node(data_dir_override: Option<String>) -> Result<()> {
    AppRuntime::get_or_start(data_dir_override.map(PathBuf::from))?;
    Ok(())
}

#[flutter_rust_bridge::frb(ignore)]
struct FrbEventSink(StreamSink<JynEvent>);

impl EventSink for FrbEventSink {
    fn push(&self, event: JynEvent) -> bool {
        self.0.add(event).is_ok()
    }
}

/// The runtime's event stream. Events that fired between `start_node` and
/// this call are buffered and replayed, so no startup state is lost.
pub fn events(sink: StreamSink<JynEvent>) -> Result<()> {
    AppRuntime::get()?.set_event_sink(Box::new(FrbEventSink(sink)));
    Ok(())
}

/// Desktop notifications only fire while the app is unfocused; Flutter
/// reports focus changes here.
pub fn set_app_focused(focused: bool) -> Result<()> {
    AppRuntime::get()?.set_app_focused(focused);
    Ok(())
}

/// The resolved data directory of the running node.
pub fn node_data_dir() -> Result<String> {
    Ok(AppRuntime::get()?.data_dir().to_string_lossy().into_owned())
}

/// The encoded `jyn-` friend code for the local profile. Fails until the
/// profile has loaded (wait for the first Profile event).
pub fn my_friend_code() -> Result<String> {
    AppRuntime::get()?.my_friend_code()
}

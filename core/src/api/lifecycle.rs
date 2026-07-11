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

/// The 24-word recovery phrase for the local identity. It recovers the
/// signing key and, with a backup archive, all encrypted content — treat it
/// like the key it is.
pub fn recovery_phrase() -> Result<String> {
    let data_dir = AppRuntime::get()?.data_dir().to_path_buf();
    let key = crate::profile::load_private_key_from_data_dir(&data_dir)?;
    crate::backup::seed_phrase(&key)
}

/// Whether this machine has no jyn identity yet (no `node.key` in the data
/// directory). Callable before `start_node`; gates the restore-from-backup
/// offer at first launch.
pub fn is_fresh_install() -> Result<bool> {
    let data_dir = crate::app_config::resolve_data_dir()?;
    Ok(!data_dir.join("node.key").exists())
}

/// Restores a backup archive into the app's data directory using the
/// recovery phrase. Must be called *before* `start_node` (fails once the
/// node runs); the restored identity and content are live after the next
/// `start_node`.
pub fn restore_backup(archive_path: String, recovery_phrase: String) -> Result<()> {
    anyhow::ensure!(
        AppRuntime::get().is_err(),
        "restore is only possible before the node starts"
    );
    let data_dir = crate::app_config::resolve_data_dir()?;
    crate::backup::restore_backup(
        &data_dir,
        std::path::Path::new(&archive_path),
        &recovery_phrase,
    )
}

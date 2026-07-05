//! Tracing setup for the embedded core: a platform-appropriate log file plus
//! stderr. Formerly lived in the Bevy binary's `main.rs`.

use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

/// Initialise a tracing subscriber that writes to a platform-appropriate log
/// file and to stderr.
///
/// The returned guard must be kept alive for the duration of the process;
/// dropping it flushes and closes the background log writer thread. Returns
/// `None` (and stays silent) when no log directory can be resolved or a
/// subscriber is already installed.
///
/// Log levels are controlled via the `RUST_LOG` environment variable and
/// default to `info`.
pub fn init_logging() -> Option<WorkerGuard> {
    let log_dir = resolve_log_dir()?;
    std::fs::create_dir_all(&log_dir).ok()?;

    let file_appender = tracing_appender::rolling::never(&log_dir, "app.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_ansi(false).with_writer(non_blocking))
        .with(fmt::layer().with_writer(std::io::stderr))
        .try_init()
        .ok()?;

    tracing::info!(
        log_file = %log_dir.join("app.log").display(),
        "logging initialised"
    );

    Some(guard)
}

/// Returns the platform-appropriate directory for the application log file.
///
/// | Platform | Path |
/// |----------|------|
/// | macOS    | `~/Library/Logs/jyn/` |
/// | Linux    | `~/.local/share/jyn/logs/` |
/// | Windows  | `%APPDATA%\jyn\logs\` |
fn resolve_log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library").join("Logs").join("jyn"))
    }

    #[cfg(not(target_os = "macos"))]
    {
        directories::ProjectDirs::from("", "", "jyn").map(|dirs| dirs.data_dir().join("logs"))
    }
}

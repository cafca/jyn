//! App-level startup configuration: data directory resolution and node
//! options derived from settings + environment. Salvaged from the former
//! Bevy plugin so the embedded runtime can reuse it.

use anyhow::{Context, Result};
use directories::ProjectDirs;

use crate::node::NodeOptions;
use crate::settings::AppSettings;

const INSECURE_SKIP_RELAY_CERT_VERIFY_ENV: &str = "JYN_INSECURE_SKIP_RELAY_CERT_VERIFY";

/// Data directory for the app. `JYN_DATA_DIR` overrides the platform
/// default so multiple instances can run side by side during development.
pub fn resolve_data_dir() -> Result<std::path::PathBuf> {
    const APP_NAME: &str = "jyn";
    if let Ok(dir) = std::env::var("JYN_DATA_DIR") {
        return Ok(std::path::PathBuf::from(dir));
    }
    ProjectDirs::from("", "", APP_NAME)
        .map(|dirs| dirs.data_dir().to_path_buf())
        .with_context(|| format!("failed to resolve app data directory for {APP_NAME}"))
}

pub fn resolve_node_options(settings: &AppSettings) -> Result<NodeOptions> {
    let relay_url = settings.relay_url_for_node()?;

    let insecure_skip_relay_cert_verify = std::env::var(INSECURE_SKIP_RELAY_CERT_VERIFY_ENV)
        .ok()
        .map(|value| parse_bool_env_var(&value, INSECURE_SKIP_RELAY_CERT_VERIFY_ENV))
        .transpose()?
        .unwrap_or(false);

    Ok(NodeOptions {
        relay_url,
        mdns_enabled: settings.mdns_enabled,
        insecure_skip_relay_cert_verify,
        gc_enabled: true,
    })
}

fn parse_bool_env_var(value: &str, name: &str) -> Result<bool> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!(
            "invalid boolean value for {name}: {value} (expected true/false, 1/0, yes/no, on/off)"
        ),
    }
}

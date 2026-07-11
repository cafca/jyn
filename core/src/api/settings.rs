//! App settings. Relay and mDNS changes persist immediately but only take
//! effect on the next node start (same contract as the Bevy app).

use anyhow::Result;

use crate::runtime::AppRuntime;
use crate::settings::{MediaBackupMode, RelayMode};

#[derive(Debug, Clone)]
pub struct SettingsView {
    pub mdns_enabled: bool,
    pub relay_mode: RelayMode,
    pub custom_relay_url: Option<String>,
    pub default_download_dir: Option<String>,
    pub media_backup_mode: MediaBackupMode,
}

pub fn get_settings() -> Result<SettingsView> {
    let settings = AppRuntime::get()?.settings();
    Ok(SettingsView {
        mdns_enabled: settings.mdns_enabled,
        relay_mode: settings.relay_mode,
        custom_relay_url: settings.custom_relay_url.clone(),
        default_download_dir: settings
            .default_download_dir
            .map(|dir| dir.to_string_lossy().into_owned()),
        media_backup_mode: settings.media_backup_mode,
    })
}

/// Returns whether the value changed. Applies to the next backup export.
pub fn set_media_backup_mode(mode: MediaBackupMode) -> Result<bool> {
    AppRuntime::get()?.with_settings_store(|store| store.set_media_backup_mode(mode))
}

/// Returns whether the value changed. Takes effect on next node start.
pub fn set_mdns_enabled(enabled: bool) -> Result<bool> {
    AppRuntime::get()?.with_settings_store(|store| store.set_mdns_enabled(enabled))
}

/// Returns whether the value changed. Takes effect on next node start.
pub fn set_relay_config(relay_mode: RelayMode, custom_relay_url: Option<String>) -> Result<bool> {
    AppRuntime::get()?
        .with_settings_store(|store| store.set_relay_config(relay_mode, custom_relay_url))
}

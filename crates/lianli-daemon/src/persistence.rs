//! Persistence helpers for config files, RGB presets, and template stores.
//!
//! Extracted from `ipc_server` so the service layer can write config without
//! reaching into the IPC module.

use anyhow::{Context, Result};
use lianli_shared::config::AppConfig;
use lianli_shared::rgb::RgbPreset;
use std::fs;
use std::path::Path;

/// Atomically write a JSON-serialized value to `path`.
///
/// Writes to a sibling `<filename>.tmp` file first, then renames — so a crash
/// mid-write can never leave a half-written config on disk.
pub fn write_json<T: serde::Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dir for {}", path.display()))?;
    }
    let json = serde_json::to_string_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).with_context(|| format!("writing tmp file {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Serialize and save the daemon's config to disk.
pub fn write_config(path: &Path, config: &AppConfig) -> Result<()> {
    write_json(path, config)
}

/// Load RGB presets from disk (returns an empty vec if the file is missing or
/// unparseable — presets are non-critical state).
pub fn read_rgb_presets(path: &Path) -> Vec<RgbPreset> {
    match fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Persist RGB presets to disk.
pub fn write_rgb_presets(path: &Path, presets: &[RgbPreset]) -> Result<()> {
    write_json(path, presets)
}

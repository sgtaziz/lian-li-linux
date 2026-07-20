//! Fetch and install templates from the upstream GitHub repo.
//!
//! The catalog lives at a hardcoded URL on `main`. Browsing is session-only
//! — [`fetch_manifest`] does a single HTTP GET each time the UI opens the
//! gallery, nothing is cached to disk. Only [`install_template`] writes to
//! `~/.config/lianli/templates/<id>/`, after verifying every file's sha256.

use crate::sensors::SensorInfo;
use crate::template::{resolve_sensor_categories, LcdTemplate};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, info, warn};

const CATALOG_BASE_URL: &str =
    "https://raw.githubusercontent.com/sgtaziz/lian-li-linux/main/templates";
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    if let Some(rest) = url.strip_prefix("file://") {
        return std::fs::read(rest).with_context(|| format!("reading {rest}"));
    }
    let resp = client()?
        .get(url)
        .send()
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("status for {url}"))?;
    Ok(resp
        .bytes()
        .with_context(|| format!("body of {url}"))?
        .to_vec())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogManifest {
    pub schema_version: u32,
    pub templates: Vec<CatalogTemplate>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogTemplate {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    pub min_daemon_version: String,
    pub folder: String,
    pub template_file: String,
    pub template_sha256: String,
    pub preview: String,
    pub preview_sha256: String,
    #[serde(default)]
    pub base_width: u32,
    #[serde(default)]
    pub base_height: u32,
    #[serde(default)]
    pub rotated: bool,
    #[serde(default)]
    pub files: Vec<CatalogFile>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogFile {
    pub path: String,
    pub sha256: String,
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("lianli-gui/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("building reqwest client")
}

fn asset_url(folder: &str, path: &str) -> String {
    format!("{CATALOG_BASE_URL}/assets/{folder}/{path}")
}

pub fn fetch_manifest() -> Result<CatalogManifest> {
    let url = format!("{CATALOG_BASE_URL}/default_templates.json");
    debug!("fetching template catalog manifest from {url}");
    let bytes = fetch_bytes(&url)?;
    let manifest: CatalogManifest =
        serde_json::from_slice(&bytes).context("parsing manifest JSON")?;
    if manifest.schema_version != 1 {
        bail!(
            "unsupported catalog schema version {}; update lianli",
            manifest.schema_version
        );
    }
    info!(
        "fetched catalog with {} template(s)",
        manifest.templates.len()
    );
    Ok(manifest)
}

pub fn fetch_preview(template: &CatalogTemplate) -> Result<Vec<u8>> {
    let url = asset_url(&template.folder, &template.preview);
    let bytes = fetch_bytes(&url)?;
    verify_sha256(&bytes, &template.preview_sha256).context("preview sha256 mismatch")?;
    Ok(bytes)
}

pub fn is_supported(template: &CatalogTemplate, daemon_version: &str) -> bool {
    version_ge(daemon_version, &template.min_daemon_version)
}

/// Downloads a template + all referenced files into
/// `~/.config/lianli/templates/<id>/`, verifies sha256 of every payload, then
/// parses `template.json`, resolves `sensor_category` hints against the local
/// machine's sensors, and returns the final in-memory [`LcdTemplate`] ready
/// for the caller to insert into the user's template list.
pub fn install_template(template: &CatalogTemplate, sensors: &[SensorInfo]) -> Result<LcdTemplate> {
    let install_root = templates_install_dir()?;
    let target_dir = install_root.join(&template.id);
    let staging_dir = install_root.join(format!(".{}.staging", template.id));
    if staging_dir.exists() {
        std::fs::remove_dir_all(&staging_dir).ok();
    }
    std::fs::create_dir_all(&staging_dir)
        .with_context(|| format!("creating {}", staging_dir.display()))?;

    let tpl_url = asset_url(&template.folder, &template.template_file);
    let tpl_bytes = fetch_bytes(&tpl_url)?;
    verify_sha256(&tpl_bytes, &template.template_sha256)
        .context("template.json sha256 mismatch")?;
    std::fs::write(staging_dir.join(&template.template_file), &tpl_bytes)
        .context("writing staged template.json")?;

    for file in &template.files {
        let url = asset_url(&template.folder, &file.path);
        let bytes = fetch_bytes(&url)?;
        verify_sha256(&bytes, &file.sha256)
            .with_context(|| format!("{} sha256 mismatch", file.path))?;
        let dest = staging_dir.join(&file.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&dest, &bytes).with_context(|| format!("writing {}", dest.display()))?;
    }

    let mut lcd_template: LcdTemplate = serde_json::from_slice(&tpl_bytes)
        .context("parsing downloaded template.json after sha256 verify")?;
    rewrite_asset_paths(&mut lcd_template, &target_dir);
    resolve_sensor_categories(&mut lcd_template, sensors);
    lcd_template.validate().map_err(|e| anyhow!("{e}"))?;

    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir)
            .with_context(|| format!("removing old {}", target_dir.display()))?;
    }
    std::fs::rename(&staging_dir, &target_dir).with_context(|| {
        format!(
            "renaming {} -> {}",
            staging_dir.display(),
            target_dir.display()
        )
    })?;

    info!(
        "installed template '{}' to {}",
        template.id,
        target_dir.display()
    );
    Ok(lcd_template)
}

fn rewrite_asset_paths(template: &mut LcdTemplate, base: &std::path::Path) {
    use crate::template::{FontRef, TemplateBackground, WidgetKind};

    fn rewrite_font(font: &mut FontRef, base: &std::path::Path) {
        if let Some(p) = font.path.as_mut() {
            if p.is_relative() {
                *p = base.join(&*p);
            }
        }
    }

    if let TemplateBackground::Image { path } = &mut template.background {
        if path.is_relative() {
            *path = base.join(&*path);
        }
    }
    for widget in template.widgets.iter_mut() {
        match &mut widget.kind {
            WidgetKind::Image { path, .. } | WidgetKind::Video { path, .. } => {
                if path.is_relative() {
                    *path = base.join(&*path);
                }
            }
            WidgetKind::Label { font, .. } | WidgetKind::ValueText { font, .. } => {
                rewrite_font(font, base);
            }
            WidgetKind::ClockDigital { font, .. } => rewrite_font(font, base),
            WidgetKind::ClockAnalog { numbers_font, .. } => rewrite_font(numbers_font, base),
            WidgetKind::Sparkline {
                axis_label_font, ..
            } => rewrite_font(axis_label_font, base),
            _ => {}
        }
    }
}

fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<()> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected_hex) {
        bail!("sha256 mismatch: expected {expected_hex}, got {actual}");
    }
    Ok(())
}

fn templates_install_dir() -> Result<PathBuf> {
    let base = config_base_dir()?;
    let dir = base.join("lianli").join("templates");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

fn config_base_dir() -> Result<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or_else(|| anyhow!("neither $XDG_CONFIG_HOME nor $HOME is set"))
}

pub fn template_previews_dir() -> Option<PathBuf> {
    let base = config_base_dir().ok()?;
    let dir = base.join("lianli").join("template_previews");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

pub fn template_preview_path(template_id: &str) -> Option<PathBuf> {
    template_previews_dir().map(|d| d.join(format!("{template_id}.png")))
}

fn version_ge(have: &str, need: &str) -> bool {
    let parse = |s: &str| -> (u32, u32, u32) {
        let mut it = s.trim_start_matches('v').split('.').map(|p| {
            p.split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|d| d.parse::<u32>().ok())
                .unwrap_or(0)
        });
        (
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
        )
    };
    let h = parse(have);
    let n = parse(need);
    match h.0.cmp(&n.0) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match h.1.cmp(&n.1) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => h.2 >= n.2,
        },
    }
}

pub fn filter_supported(
    templates: Vec<CatalogTemplate>,
    daemon_version: &str,
) -> Vec<CatalogTemplate> {
    let (supported, unsupported): (Vec<_>, Vec<_>) = templates
        .into_iter()
        .partition(|t| is_supported(t, daemon_version));
    if !unsupported.is_empty() {
        warn!(
            "skipping {} template(s) that require a newer daemon version",
            unsupported.len()
        );
    }
    supported
}

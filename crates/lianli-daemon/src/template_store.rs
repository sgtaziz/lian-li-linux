//! Persistence for LCD templates.

use anyhow::{Context, Result};
use lianli_media::CustomAsset;
use lianli_shared::screen::ScreenInfo;
use lianli_shared::sensors::SensorInfo;
use lianli_shared::template::LcdTemplate;
use lianli_shared::template_catalog::template_preview_path;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

const PREVIEW_WIDTH: u32 = 240;
const PREVIEW_HEIGHT: u32 = 240;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct TemplateFile {
    #[serde(default)]
    templates: Vec<LcdTemplate>,
}

pub fn templates_path_for(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lcd_templates.json")
}

pub fn load_user_templates(path: &Path) -> Vec<LcdTemplate> {
    if !path.exists() {
        return Vec::new();
    }
    match fs::read_to_string(path) {
        Ok(json) => match serde_json::from_str::<TemplateFile>(&json) {
            Ok(file) => file.templates,
            Err(e) => {
                warn!("Failed to parse {}: {e}", path.display());
                Vec::new()
            }
        },
        Err(e) => {
            warn!("Failed to read {}: {e}", path.display());
            Vec::new()
        }
    }
}

pub fn save_user_templates(path: &Path, templates: &[LcdTemplate]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dir for {}", path.display()))?;
    }
    let file = TemplateFile {
        templates: templates.to_vec(),
    };
    let json = serde_json::to_string_pretty(&file)?;
    fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

pub fn all_templates(user: &[LcdTemplate], _sensors: &[SensorInfo]) -> Vec<LcdTemplate> {
    user.to_vec()
}

#[allow(dead_code)]
pub fn resolve_template(
    id: &str,
    user: &[LcdTemplate],
    _sensors: &[SensorInfo],
) -> Option<LcdTemplate> {
    user.iter().find(|t| t.id == id).cloned()
}

pub fn regenerate_template_previews(templates: &[LcdTemplate], sensors: &[SensorInfo]) {
    for template in templates {
        if let Err(e) = render_template_preview(template, sensors) {
            warn!("preview render failed for template '{}': {e}", template.id);
        }
    }
}

fn render_template_preview(template: &LcdTemplate, sensors: &[SensorInfo]) -> Result<()> {
    let out_path = template_preview_path(&template.id)
        .context("computing preview path (no XDG_CONFIG_HOME / HOME)")?;
    let screen = ScreenInfo {
        width: PREVIEW_WIDTH,
        height: PREVIEW_HEIGHT,
        max_fps: 30,
        jpeg_quality: 85,
        max_payload: 4 * 1024 * 1024,
        device_rotation: 0,
        h264: false,
    };
    let asset = CustomAsset::new(template, 0.0, &screen, sensors).context("CustomAsset::new")?;
    asset.seed_preview_history();
    let frame = match asset.render_frame(true).context("render_frame")? {
        Some(f) => f,
        None => asset.blank_frame(),
    };
    let img = image::load_from_memory(&frame.data).context("decoding rendered JPEG")?;
    img.save(&out_path)
        .with_context(|| format!("writing {}", out_path.display()))?;
    Ok(())
}

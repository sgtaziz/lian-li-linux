//! Persistence + resolution for LCD templates. Built-ins live in
//! `lianli_shared::template_defaults` and are merged in at read time.

use anyhow::{Context, Result};
use lianli_shared::sensors::SensorInfo;
use lianli_shared::template::LcdTemplate;
use lianli_shared::template_defaults::{
    builtin_template_resolved, builtin_templates, is_builtin_id, BUILTIN_COOLER_ID,
    BUILTIN_DOUBLEGAUGE_ID,
};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

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
            Ok(file) => file
                .templates
                .into_iter()
                .filter(|t| {
                    if is_builtin_id(&t.id) {
                        warn!(
                            "Ignoring user template with reserved built-in id '{}'",
                            t.id
                        );
                        false
                    } else {
                        true
                    }
                })
                .collect(),
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
        templates: templates
            .iter()
            .filter(|t| !is_builtin_id(&t.id))
            .cloned()
            .collect(),
    };
    let json = serde_json::to_string_pretty(&file)?;
    fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

pub fn all_templates(user: &[LcdTemplate], sensors: &[SensorInfo]) -> Vec<LcdTemplate> {
    let mut out: Vec<LcdTemplate> = [BUILTIN_COOLER_ID, BUILTIN_DOUBLEGAUGE_ID]
        .iter()
        .filter_map(|id| {
            builtin_template_resolved(id, sensors)
                .or_else(|| builtin_templates().into_iter().find(|t| &t.id == id))
        })
        .collect();
    out.extend(user.iter().cloned());
    out
}

#[allow(dead_code)]
pub fn resolve_template(
    id: &str,
    user: &[LcdTemplate],
    sensors: &[SensorInfo],
) -> Option<LcdTemplate> {
    if let Some(t) = builtin_template_resolved(id, sensors) {
        return Some(t);
    }
    user.iter().find(|t| t.id == id).cloned()
}

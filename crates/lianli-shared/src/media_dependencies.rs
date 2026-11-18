use crate::config::LcdConfig;
use crate::media::MediaType;
use crate::template::{LcdTemplate, TemplateBackground, WidgetKind};
use crate::ui_message::{UiError, UiMessage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    File,
    Image,
    Video,
    Gif,
    Font,
}

fn lcd_owner(lcd: &str, key: &str, english: String) -> (String, Option<UiMessage>) {
    (english, Some(UiMessage::new(key).param("lcd", lcd)))
}

pub fn stored_lcd_dependencies(config: &LcdConfig) -> Vec<AssetDependency> {
    let owner = format!("LCD[{}]", config.device_id());
    let mut dependencies = Vec::new();
    if let Some(path) = &config.path {
        let (owner, owner_ui) = lcd_owner(
            &owner,
            "status.mediaAccess.savedMedia",
            format!("{owner} saved media"),
        );
        dependencies.push(AssetDependency {
            path: path.clone(),
            owner,
            owner_ui,
            kind: match config.media_type {
                MediaType::Image | MediaType::Sensor => AssetKind::Image,
                MediaType::Video => AssetKind::Video,
                MediaType::Gif => AssetKind::Gif,
                _ => AssetKind::File,
            },
        });
    }
    if let Some(path) = config
        .sensor
        .as_ref()
        .and_then(|sensor| sensor.font_path.as_ref())
    {
        let (owner, owner_ui) = lcd_owner(
            &owner,
            "status.mediaAccess.savedSensorFont",
            format!("{owner} saved sensor font"),
        );
        dependencies.push(AssetDependency {
            path: path.clone(),
            owner,
            owner_ui,
            kind: AssetKind::Font,
        });
    }
    dependencies
}

pub fn map_lcd_paths(config: &mut LcdConfig, mut map: impl FnMut(&mut PathBuf)) {
    if let Some(path) = &mut config.path {
        map(path);
    }
    if let Some(path) = config
        .sensor
        .as_mut()
        .and_then(|sensor| sensor.font_path.as_mut())
    {
        map(path);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetDependency {
    pub path: PathBuf,
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_ui: Option<UiMessage>,
    pub kind: AssetKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetAccessIssue {
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_ui: Option<UiMessage>,
    pub path: Option<PathBuf>,
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_ui: Option<UiMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetAccessReport {
    pub uid: u32,
    pub checked: usize,
    pub failed: usize,
    pub issues: Vec<AssetAccessIssue>,
}

pub fn validate_dependency_input(
    lcds: &[LcdConfig],
    templates: &[LcdTemplate],
) -> Result<(), String> {
    if lcds.len() > 256 || templates.len() > 1024 {
        return Err(
            "Media preflight supports at most 256 LCD entries and 1024 templates per state file"
                .into(),
        );
    }
    if lcds.iter().any(|lcd| {
        lcd.serial.as_ref().is_some_and(|id| id.len() > 256)
            || lcd.template_id.as_ref().is_some_and(|id| id.len() > 256)
    }) || templates.iter().any(|template| {
        template.id.len() > 256
            || template.widgets.len() > 1024
            || template.widgets.iter().any(|widget| widget.id.len() > 256)
    }) {
        return Err("Media preflight requires identifiers of at most 256 bytes and at most 1024 widgets per template".into());
    }
    Ok(())
}

pub fn validate_dependency_paths(dependencies: &[AssetDependency]) -> Result<(), String> {
    if dependencies
        .iter()
        .any(|dependency| dependency.path.as_os_str().len() > 4096)
    {
        return Err("Media paths must not exceed 4096 bytes".into());
    }
    Ok(())
}

pub fn template_dependencies(template: &LcdTemplate) -> Vec<AssetDependency> {
    template_dependencies_for(template, None)
}

fn template_dependencies_for(template: &LcdTemplate, lcd: Option<&str>) -> Vec<AssetDependency> {
    let owned = |english: String, key: &str, lcd_key: &str| match lcd {
        Some(lcd) => (
            format!("{lcd}: {english}"),
            UiMessage::new(lcd_key).param("lcd", lcd),
        ),
        None => (english, UiMessage::new(key)),
    };
    let mut dependencies = Vec::new();
    let background = match &template.background {
        TemplateBackground::Image { path } => Some((path, AssetKind::Image)),
        TemplateBackground::Video { path, .. } => Some((path, AssetKind::Video)),
        TemplateBackground::Color { .. } => None,
    };
    if let Some((path, kind)) = background {
        let (owner, owner_ui) = owned(
            format!("Template '{}' background", template.id),
            "status.mediaAccess.templateBackground",
            "status.mediaAccess.lcdTemplateBackground",
        );
        dependencies.push(AssetDependency {
            path: path.clone(),
            owner,
            owner_ui: Some(owner_ui.param("template", &template.id)),
            kind,
        });
    }
    for widget in &template.widgets {
        let (owner, owner_ui) = owned(
            format!("Template '{}' widget '{}'", template.id, widget.id),
            "status.mediaAccess.templateWidget",
            "status.mediaAccess.lcdTemplateWidget",
        );
        let owner_ui = owner_ui
            .param("template", &template.id)
            .param("widget", &widget.id);
        let dependency = match &widget.kind {
            WidgetKind::Image { path, .. } => Some((path, AssetKind::Image)),
            WidgetKind::Video { path, .. } => Some((path, AssetKind::Video)),
            kind => kind
                .font_ref()
                .and_then(|font| font.path.as_ref())
                .map(|path| (path, AssetKind::Font)),
        };
        if let Some((path, kind)) = dependency {
            dependencies.push(AssetDependency {
                path: path.clone(),
                owner,
                owner_ui: Some(owner_ui),
                kind,
            });
        }
    }
    dependencies
}

pub fn lcd_dependencies(
    config: &LcdConfig,
    templates: &[LcdTemplate],
) -> Result<Vec<AssetDependency>, UiError> {
    let owner = format!("LCD[{}]", config.device_id());
    let lcd_error = |key: &str, english: String| {
        UiError::new(UiMessage::new(key).param("lcd", &owner), english)
    };
    let mut dependencies = Vec::new();
    match config.media_type {
        MediaType::Image | MediaType::Video | MediaType::Gif => {
            let path = config.path.clone().ok_or_else(|| {
                lcd_error(
                    "status.mediaAccess.noMediaPath",
                    format!("{owner} has no media path"),
                )
            })?;
            dependencies.push(AssetDependency {
                path,
                owner: owner.clone(),
                owner_ui: None,
                kind: if config.media_type == MediaType::Image {
                    AssetKind::Image
                } else if config.media_type == MediaType::Gif {
                    AssetKind::Gif
                } else {
                    AssetKind::Video
                },
            });
        }
        MediaType::Sensor => {
            if let Some(path) = &config.path {
                let (owner, owner_ui) = lcd_owner(
                    &owner,
                    "status.mediaAccess.sensorBackground",
                    format!("{owner} sensor background"),
                );
                dependencies.push(AssetDependency {
                    path: path.clone(),
                    owner,
                    owner_ui,
                    kind: AssetKind::Image,
                });
            }
            if let Some(path) = config
                .sensor
                .as_ref()
                .and_then(|sensor| sensor.font_path.as_ref())
            {
                let (owner, owner_ui) = lcd_owner(
                    &owner,
                    "status.mediaAccess.sensorFont",
                    format!("{owner} sensor font"),
                );
                dependencies.push(AssetDependency {
                    path: path.clone(),
                    owner,
                    owner_ui,
                    kind: AssetKind::Font,
                });
            }
        }
        MediaType::Custom => {
            let id = config.template_id.as_deref().ok_or_else(|| {
                lcd_error(
                    "status.mediaAccess.noTemplateId",
                    format!("{owner} has no template ID"),
                )
            })?;
            let template = templates
                .iter()
                .find(|template| template.id == id)
                .ok_or_else(|| {
                    UiError::new(
                        UiMessage::new("status.mediaAccess.missingTemplate")
                            .param("lcd", &owner)
                            .param("template", id),
                        format!("{owner} references missing template '{id}'"),
                    )
                })?;
            dependencies = template_dependencies_for(template, Some(&owner));
        }
        MediaType::Color | MediaType::Doublegauge | MediaType::Cooler => {}
    }
    Ok(dependencies)
}

pub fn map_template_paths(template: &mut LcdTemplate, mut map: impl FnMut(&mut PathBuf)) {
    match &mut template.background {
        TemplateBackground::Image { path } | TemplateBackground::Video { path, .. } => map(path),
        TemplateBackground::Color { .. } => {}
    }
    for widget in &mut template.widgets {
        if let Some(path) = widget.kind.asset_path_mut() {
            map(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_children_and_fonts_are_discovered_and_rewritten_together() {
        let mut template: LcdTemplate = serde_json::from_value(serde_json::json!({
            "id": "custom", "name": "Custom", "base_width": 400, "base_height": 400,
            "background": {"type": "image", "path": "background.png"},
            "widgets": [
                {"id": "image", "x": 0, "y": 0, "width": 100, "height": 100,
                 "kind": {"type": "image", "path": "image.png"}},
                {"id": "video", "x": 0, "y": 0, "width": 100, "height": 100,
                 "kind": {"type": "video", "path": "video.mp4"}},
                {"id": "label", "x": 0, "y": 0, "width": 100, "height": 100,
                 "kind": {"type": "label", "text": "Text", "font_size": 20, "color": [255,255,255], "font": {"path": "font.ttf"}}}
            ]
        })).unwrap();
        let dependencies = template_dependencies(&template);
        assert_eq!(dependencies.len(), 4);
        assert_eq!(dependencies[3].kind, AssetKind::Font);
        assert!(dependencies[3].owner.contains("widget 'label'"));
        map_template_paths(&mut template, |path| {
            *path = PathBuf::from("/managed").join(&*path)
        });
        let rewritten = template_dependencies(&template);
        for (old, new) in dependencies.iter().zip(rewritten) {
            assert_eq!(new.path, PathBuf::from("/managed").join(&old.path));
            assert_eq!(new.owner, old.owner);
        }
    }

    #[test]
    fn video_background_is_a_video_dependency_and_is_rewritten() {
        let mut template: LcdTemplate = serde_json::from_value(serde_json::json!({
            "id": "custom", "name": "Custom", "base_width": 400, "base_height": 400,
            "background": {"type": "video", "path": "background.mp4", "fps": 24},
            "widgets": []
        }))
        .unwrap();
        let dependencies = template_dependencies(&template);
        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0].kind, AssetKind::Video);
        assert!(dependencies[0].owner.contains("background"));
        map_template_paths(&mut template, |path| {
            *path = PathBuf::from("/managed").join(&*path)
        });
        let rewritten = template_dependencies(&template);
        assert_eq!(rewritten[0].path, PathBuf::from("/managed/background.mp4"));
    }

    #[test]
    fn missing_templates_are_reported_with_the_configured_lcd() {
        let config = serde_json::from_value(
            serde_json::json!({"type": "custom", "index": 2, "template_id": "missing"}),
        )
        .unwrap();
        let error = lcd_dependencies(&config, &[]).unwrap_err().to_string();
        assert!(error.contains("LCD[index:2]"));
        assert!(error.contains("template 'missing'"));
    }

    #[test]
    fn preflight_bounds_identifier_expansion_and_path_bytes() {
        let mut lcd: LcdConfig = serde_json::from_value(serde_json::json!({
            "type": "image", "serial": "a".repeat(256), "path": "x".repeat(4096)
        }))
        .unwrap();
        validate_dependency_input(&[lcd.clone()], &[]).unwrap();
        validate_dependency_paths(&lcd_dependencies(&lcd, &[]).unwrap()).unwrap();
        lcd.serial.as_mut().unwrap().push('a');
        assert!(validate_dependency_input(&[lcd.clone()], &[]).is_err());
        lcd.path = Some("x".repeat(4097).into());
        assert!(validate_dependency_paths(&lcd_dependencies(&lcd, &[]).unwrap()).is_err());
    }
}

use crate::command_error::CommandError;
use lianli_shared::ui_message::UiMessage;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;
use tauri::WebviewWindow;
use tauri_plugin_dialog::DialogExt;

const FORMAT: &str = "lianli-linux-fan-curves";
const VERSION: u32 = 1;
const MAX_BYTES: u64 = 64 * 1024;
const MAX_CURVES: usize = 32;
const MAX_POINTS: usize = 32;
const MAX_NAME_CHARS: usize = 64;

/// Only names and points are exchanged. Temperature sources can be shell
/// commands run by the daemon, so they never travel in a curve file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableCurve {
    name: String,
    curve: Vec<(f32, f32)>,
}

#[derive(Serialize, Deserialize)]
struct CurveFile {
    format: String,
    version: u32,
    curves: Vec<PortableCurve>,
}

#[derive(Debug, PartialEq)]
enum CurveFileError {
    TooLarge,
    Invalid(String),
}

fn invalid(detail: impl Into<String>) -> CurveFileError {
    CurveFileError::Invalid(detail.into())
}

fn parse_curve_file(bytes: &[u8]) -> Result<Vec<PortableCurve>, CurveFileError> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(CurveFileError::TooLarge);
    }
    let file: CurveFile =
        serde_json::from_slice(bytes).map_err(|error| invalid(error.to_string()))?;
    if file.format != FORMAT {
        return Err(invalid("not a Lian Li Linux fan curve file"));
    }
    if file.version != VERSION {
        return Err(invalid(format!("unsupported version {}", file.version)));
    }
    if file.curves.is_empty() || file.curves.len() > MAX_CURVES {
        return Err(invalid(format!(
            "expected 1 to {MAX_CURVES} curves, found {}",
            file.curves.len()
        )));
    }
    file.curves.into_iter().map(validate_curve).collect()
}

fn validate_curve(curve: PortableCurve) -> Result<PortableCurve, CurveFileError> {
    let name: String = curve.name.trim().chars().take(MAX_NAME_CHARS).collect();
    if name.is_empty() {
        return Err(invalid("a curve has no name"));
    }
    if curve.curve.is_empty() || curve.curve.len() > MAX_POINTS {
        return Err(invalid(format!("{name} needs 1 to {MAX_POINTS} points")));
    }
    let valid = |&(temp, speed): &(f32, f32)| {
        temp.is_finite()
            && speed.is_finite()
            && (0.0..=120.0).contains(&temp)
            && (0.0..=100.0).contains(&speed)
    };
    if !curve.curve.iter().all(valid) {
        return Err(invalid(format!(
            "{name} has a point outside 0 to 120 °C or 0 to 100%"
        )));
    }
    let mut points = curve.curve;
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(PortableCurve {
        name,
        curve: points,
    })
}

fn read_curve_file(path: &Path) -> Result<Vec<PortableCurve>, CommandError> {
    let unreadable = |error: std::io::Error| {
        CommandError::ui(
            format!("Cannot read the fan curve file: {error}"),
            UiMessage::new("gui.curveFiles.unreadable").param("error", &error),
        )
    };
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(unreadable)?;
    parse_curve_file(&bytes).map_err(|error| match error {
        CurveFileError::TooLarge => CommandError::ui(
            "The fan curve file is larger than 64 KB",
            UiMessage::new("gui.curveFiles.tooLarge"),
        ),
        CurveFileError::Invalid(detail) => CommandError::ui(
            format!("This is not a valid fan curve file: {detail}"),
            UiMessage::new("gui.curveFiles.invalid").param("detail", &detail),
        ),
    })
}

fn require_main(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(CommandError::ui(
            "Export and import fan curves from the main window",
            UiMessage::new("gui.curveFiles.mainWindow"),
        ))
    }
}

#[tauri::command]
pub async fn export_fan_curves(
    window: WebviewWindow,
    app: tauri::AppHandle,
    curves: Vec<PortableCurve>,
    title: Option<String>,
    filter: Option<String>,
) -> Result<bool, CommandError> {
    require_main(&window)?;
    let title = crate::ui_text(title, "Export fan curves");
    let filter = crate::ui_text(filter, "Fan curves (JSON)");
    let json = serde_json::to_vec_pretty(&CurveFile {
        format: FORMAT.into(),
        version: VERSION,
        curves,
    })
    .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(selected) = app
            .dialog()
            .file()
            .set_title(&title)
            .set_file_name("fan-curves.json")
            .add_filter(&filter, &["json"])
            .blocking_save_file()
        else {
            return Ok(false);
        };
        let path = selected.into_path().map_err(|error| error.to_string())?;
        std::fs::write(&path, json).map_err(|error| {
            CommandError::ui(
                format!("Cannot save the fan curve file: {error}"),
                UiMessage::new("gui.curveFiles.writeFailed").param("error", &error),
            )
        })?;
        Ok(true)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn import_fan_curves(
    window: WebviewWindow,
    app: tauri::AppHandle,
    title: Option<String>,
    filter: Option<String>,
) -> Result<Vec<PortableCurve>, CommandError> {
    require_main(&window)?;
    let title = crate::ui_text(title, "Import fan curves");
    let filter = crate::ui_text(filter, "Fan curves (JSON)");
    tauri::async_runtime::spawn_blocking(move || {
        let Some(selected) = app
            .dialog()
            .file()
            .set_title(&title)
            .add_filter(&filter, &["json"])
            .blocking_pick_file()
        else {
            return Ok(Vec::new());
        };
        let path = selected.into_path().map_err(|error| error.to_string())?;
        read_curve_file(&path)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(curves: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "format": FORMAT,
            "version": VERSION,
            "curves": curves,
        }))
        .unwrap()
    }

    #[test]
    fn keeps_names_and_sorted_points_and_drops_sensors_and_commands() {
        let bytes = file(serde_json::json!([{
            "name": "  Quiet  ",
            "curve": [[85, 100], [30, 20], [50, 25]],
            "temp_source": { "type": "command", "cmd": "rm -rf ~" },
            "temp_command": "rm -rf ~",
        }]));
        let curves = parse_curve_file(&bytes).unwrap();
        assert_eq!(
            curves,
            vec![PortableCurve {
                name: "Quiet".into(),
                curve: vec![(30.0, 20.0), (50.0, 25.0), (85.0, 100.0)],
            }]
        );
        let saved = serde_json::to_value(&curves[0]).unwrap();
        assert!(saved.get("temp_source").is_none() && saved.get("temp_command").is_none());
    }

    #[test]
    fn rejects_other_formats_versions_and_counts() {
        let wrong_format = serde_json::to_vec(&serde_json::json!({
            "format": "something-else", "version": 1, "curves": [{ "name": "A", "curve": [[30, 30]] }]
        }))
        .unwrap();
        assert!(matches!(
            parse_curve_file(&wrong_format),
            Err(CurveFileError::Invalid(_))
        ));
        let wrong_version = serde_json::to_vec(&serde_json::json!({
            "format": FORMAT, "version": 2, "curves": [{ "name": "A", "curve": [[30, 30]] }]
        }))
        .unwrap();
        assert!(matches!(
            parse_curve_file(&wrong_version),
            Err(CurveFileError::Invalid(_))
        ));
        assert!(parse_curve_file(&file(serde_json::json!([]))).is_err());
        let many: Vec<_> = (0..=MAX_CURVES)
            .map(|i| serde_json::json!({ "name": format!("C{i}"), "curve": [[30, 30]] }))
            .collect();
        assert!(parse_curve_file(&file(serde_json::json!(many))).is_err());
        let points: Vec<_> = (0..=MAX_POINTS)
            .map(|i| serde_json::json!([i, 50]))
            .collect();
        assert!(
            parse_curve_file(&file(serde_json::json!([{ "name": "A", "curve": points }]))).is_err()
        );
        assert!(parse_curve_file(b"not json").is_err());
    }

    #[test]
    fn rejects_out_of_range_points_blank_names_and_oversized_files() {
        for point in [[30.0, 101.0], [-1.0, 50.0], [121.0, 50.0], [30.0, -5.0]] {
            let bytes = file(serde_json::json!([{ "name": "A", "curve": [point] }]));
            assert!(parse_curve_file(&bytes).is_err(), "{point:?}");
        }
        let blank = file(serde_json::json!([{ "name": "   ", "curve": [[30, 30]] }]));
        assert!(parse_curve_file(&blank).is_err());
        let oversized = vec![b' '; MAX_BYTES as usize + 1];
        assert_eq!(parse_curve_file(&oversized), Err(CurveFileError::TooLarge));
    }

    #[test]
    fn export_round_trips_through_import() {
        let curves = vec![PortableCurve {
            name: "Standard".into(),
            curve: vec![(30.0, 30.0), (85.0, 100.0)],
        }];
        let bytes = serde_json::to_vec(&CurveFile {
            format: FORMAT.into(),
            version: VERSION,
            curves: curves.clone(),
        })
        .unwrap();
        assert_eq!(parse_curve_file(&bytes).unwrap(), curves);
    }
}

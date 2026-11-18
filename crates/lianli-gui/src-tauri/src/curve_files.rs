use crate::command_error::CommandError;
use lianli_shared::fan::{MB_SYNC_KEY, OFF_KEY};
use lianli_shared::ui_message::UiMessage;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tauri::WebviewWindow;
use tauri_plugin_dialog::DialogExt;

const FORMAT: &str = "lianli-linux-fan-curves";
const VERSION: u32 = 1;
const MAX_BYTES: u64 = 64 * 1024;
const MAX_CURVES: usize = 32;
const MAX_POINTS: usize = 32;
const MAX_NAME_CHARS: usize = 64;

/// Sources never travel in a file, since a command source is a shell command the daemon runs.
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
    Invalid { english: String, ui: UiMessage },
}

fn invalid(english: impl Into<String>, ui: UiMessage) -> CurveFileError {
    CurveFileError::Invalid {
        english: english.into(),
        ui,
    }
}

fn parse_curve_file(bytes: &[u8]) -> Result<Vec<PortableCurve>, CurveFileError> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(CurveFileError::TooLarge);
    }
    let file: CurveFile = serde_json::from_slice(bytes).map_err(|error| {
        invalid(
            format!("This is not a valid fan curve file: {error}"),
            UiMessage::new("gui.curveFiles.malformed").param("error", &error),
        )
    })?;
    if file.format != FORMAT {
        return Err(invalid(
            "This is not a Lian Li Linux fan curve file",
            UiMessage::new("gui.curveFiles.wrongFormat"),
        ));
    }
    if file.version != VERSION {
        return Err(invalid(
            format!("Unsupported fan curve file version {}", file.version),
            UiMessage::new("gui.curveFiles.unsupportedVersion").param("version", file.version),
        ));
    }
    if file.curves.is_empty() || file.curves.len() > MAX_CURVES {
        return Err(invalid(
            format!(
                "A fan curve file holds 1 to {MAX_CURVES} curves, this one has {}",
                file.curves.len()
            ),
            UiMessage::new("gui.curveFiles.curveCount")
                .param("max", MAX_CURVES)
                .param("count", file.curves.len()),
        ));
    }
    file.curves.into_iter().map(validate_curve).collect()
}

/// These names mean firmware control or motherboard sync in a fan slot, not a curve.
fn is_reserved_name(name: &str) -> bool {
    name == OFF_KEY || name.starts_with(MB_SYNC_KEY)
}

fn validate_curve(curve: PortableCurve) -> Result<PortableCurve, CurveFileError> {
    let name = curve.name.trim().to_string();
    if name.is_empty() {
        return Err(invalid(
            "A curve in the file has no name",
            UiMessage::new("gui.curveFiles.unnamed"),
        ));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(invalid(
            format!("\"{name}\" is longer than {MAX_NAME_CHARS} characters"),
            UiMessage::new("gui.curveFiles.nameTooLong")
                .param("name", &name)
                .param("max", MAX_NAME_CHARS),
        ));
    }
    if is_reserved_name(&name) {
        return Err(invalid(
            format!("\"{name}\" is reserved for a fan mode"),
            UiMessage::new("gui.curveFiles.reservedName").param("name", &name),
        ));
    }
    if curve.curve.is_empty() || curve.curve.len() > MAX_POINTS {
        return Err(invalid(
            format!("\"{name}\" needs 1 to {MAX_POINTS} points"),
            UiMessage::new("gui.curveFiles.pointCount")
                .param("name", &name)
                .param("max", MAX_POINTS),
        ));
    }
    let valid = |&(temp, speed): &(f32, f32)| {
        temp.is_finite()
            && speed.is_finite()
            && (0.0..=120.0).contains(&temp)
            && (0.0..=100.0).contains(&speed)
    };
    if !curve.curve.iter().all(valid) {
        return Err(invalid(
            format!("\"{name}\" has a point outside 0 to 120 °C or 0 to 100%"),
            UiMessage::new("gui.curveFiles.pointRange").param("name", &name),
        ));
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
    parse_curve_file(&bytes).map_err(command_error)
}

fn command_error(error: CurveFileError) -> CommandError {
    match error {
        CurveFileError::TooLarge => CommandError::ui(
            "The fan curve file is larger than 64 KB",
            UiMessage::new("gui.curveFiles.tooLarge"),
        ),
        CurveFileError::Invalid { english, ui } => CommandError::ui(english, ui),
    }
}

fn encode_curve_file(curves: Vec<PortableCurve>) -> Result<Vec<u8>, CurveFileError> {
    let json = serde_json::to_vec_pretty(&CurveFile {
        format: FORMAT.into(),
        version: VERSION,
        curves,
    })
    .map_err(|error| {
        invalid(
            format!("This is not a valid fan curve file: {error}"),
            UiMessage::new("gui.curveFiles.malformed").param("error", &error),
        )
    })?;
    parse_curve_file(&json)?;
    Ok(json)
}

fn write_replacing(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::Builder::new()
        .prefix(".fan-curves")
        .permissions(std::fs::Permissions::from_mode(0o666))
        .tempfile_in(directory)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
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
    let json = encode_curve_file(curves).map_err(command_error)?;
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
        write_replacing(&path, &json).map_err(|error| {
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
            Err(CurveFileError::Invalid { .. })
        ));
        let wrong_version = serde_json::to_vec(&serde_json::json!({
            "format": FORMAT, "version": 2, "curves": [{ "name": "A", "curve": [[30, 30]] }]
        }))
        .unwrap();
        assert!(matches!(
            parse_curve_file(&wrong_version),
            Err(CurveFileError::Invalid { .. })
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
        for reserved in ["off", "__mb_sync__", "__mb_sync__:hwmon"] {
            let bytes = file(serde_json::json!([{ "name": reserved, "curve": [[30, 30]] }]));
            assert!(parse_curve_file(&bytes).is_err(), "{reserved}");
        }
        let allowed = file(serde_json::json!([{ "name": "Off peak", "curve": [[30, 30]] }]));
        assert!(parse_curve_file(&allowed).is_ok());
        let oversized = vec![b' '; MAX_BYTES as usize + 1];
        assert_eq!(parse_curve_file(&oversized), Err(CurveFileError::TooLarge));
    }

    #[test]
    fn export_round_trips_through_import() {
        let curves = vec![PortableCurve {
            name: "Standard".into(),
            curve: vec![(30.0, 30.0), (85.0, 100.0)],
        }];
        let bytes = encode_curve_file(curves.clone()).unwrap();
        assert_eq!(parse_curve_file(&bytes).unwrap(), curves);
    }

    #[test]
    fn export_refuses_curves_the_importer_would_reject_or_change() {
        let curve = |name: String, points: usize| PortableCurve {
            name,
            curve: (0..points).map(|i| (i as f32, 50.0)).collect(),
        };
        assert!(encode_curve_file(vec![curve("A".into(), MAX_POINTS)]).is_ok());
        assert!(encode_curve_file(vec![curve("A".into(), MAX_POINTS + 1)]).is_err());
        let longest = "é".repeat(MAX_NAME_CHARS);
        let exported = encode_curve_file(vec![curve(longest.clone(), 2)]).unwrap();
        assert_eq!(parse_curve_file(&exported).unwrap()[0].name, longest);
        assert!(encode_curve_file(vec![curve(format!("{longest}x"), 2)]).is_err());
    }

    #[test]
    fn replacing_an_export_leaves_only_the_new_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fan-curves.json");
        std::fs::write(&path, b"old").unwrap();
        write_replacing(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

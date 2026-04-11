//! Render a `preview.png` for a template folder.
//!
//! Usage:
//!   cargo run -p lianli-media --bin render-preview -- <template-dir> [--out <file>]
//!
//! Loads `<template-dir>/template.json`, injects deterministic mock sensor
//! values (seeded per widget id for reproducible PRs), renders one frame at
//! the template's native `base_width × base_height`, and writes the result to
//! `<template-dir>/preview.png` (or `--out` if given).

use anyhow::{anyhow, Context, Result};
use lianli_media::CustomAsset;
use lianli_shared::media::SensorSourceConfig;
use lianli_shared::screen::ScreenInfo;
use lianli_shared::sensors::SensorCategory;
use lianli_shared::systeminfo::SysSensor;
use lianli_shared::template::{LcdTemplate, Widget, WidgetKind};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

const MOCK_CORE_COUNT: usize = 24;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut template_dir: Option<PathBuf> = None;
    let mut out_path: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" | "-o" => {
                out_path = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow!("--out requires a value"))?,
                ));
            }
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            other if template_dir.is_none() => template_dir = Some(PathBuf::from(other)),
            other => return Err(anyhow!("unexpected argument '{other}'")),
        }
    }

    let template_dir = template_dir.ok_or_else(|| {
        print_usage();
        anyhow!("missing template directory")
    })?;

    let template_path = template_dir.join("template.json");
    let raw = std::fs::read_to_string(&template_path)
        .with_context(|| format!("reading {}", template_path.display()))?;
    let mut template: LcdTemplate = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {}", template_path.display()))?;

    stub_sensor_sources(&mut template);
    SysSensor::set_mock_cores(mock_core_values(MOCK_CORE_COUNT));

    let abs_template_dir = std::fs::canonicalize(&template_dir)
        .with_context(|| format!("canonicalizing {}", template_dir.display()))?;
    std::env::set_current_dir(&abs_template_dir)
        .with_context(|| format!("chdir {}", abs_template_dir.display()))?;

    let screen = ScreenInfo {
        width: template.base_width,
        height: template.base_height,
        max_fps: 30,
        jpeg_quality: 100,
        max_payload: usize::MAX,
        device_rotation: 0,
        h264: false,
    };

    let asset = CustomAsset::new(&template, 0.0, &screen, &[])
        .map_err(|e| anyhow!("building custom asset: {e}"))?;
    let frame = asset
        .render_frame(true)
        .map_err(|e| anyhow!("rendering frame: {e}"))?
        .ok_or_else(|| anyhow!("render_frame returned no frame"))?;

    let decoded =
        image::load_from_memory(&frame.data).context("decoding rendered JPEG for PNG re-encode")?;

    let out = out_path.unwrap_or_else(|| abs_template_dir.join("preview.png"));
    decoded
        .save(&out)
        .with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {}", out.display());
    Ok(())
}

fn print_usage() {
    eprintln!("usage: render-preview <template-dir> [--out <file>]");
}

fn stub_sensor_sources(template: &mut LcdTemplate) {
    for widget in template.widgets.iter_mut() {
        let value = mock_value_for_widget(widget);
        if let Some(source) = widget.kind.source_config_mut() {
            *source = SensorSourceConfig::Constant { value };
        }
    }
}

const MOCK_TEMP_C: f32 = 48.0;
const MOCK_USAGE_PCT: f32 = 28.0;

fn mock_value_for_widget(widget: &Widget) -> f32 {
    if let Some(cat) = widget.sensor_category {
        return match cat {
            SensorCategory::CpuTemp | SensorCategory::GpuTemp => MOCK_TEMP_C,
            SensorCategory::CpuUsage | SensorCategory::GpuUsage | SensorCategory::MemUsage => {
                MOCK_USAGE_PCT
            }
        };
    }
    if let WidgetKind::ValueText { unit, .. } = &widget.kind {
        let u = unit.to_lowercase();
        if u.contains("°c") || u == "c" {
            return MOCK_TEMP_C;
        }
        if u.contains('%') {
            return MOCK_USAGE_PCT;
        }
        if u.contains("rpm") {
            return 1400.0;
        }
    }
    MOCK_USAGE_PCT
}

fn mock_core_values(count: usize) -> Vec<u32> {
    (0..count)
        .map(|i| {
            let mut hasher = DefaultHasher::new();
            ("lianli-mock-core", i).hash(&mut hasher);
            let r = (hasher.finish() % 10_000) as f32 / 10_000.0;
            let pct = 5.0 + r * 35.0;
            (pct * 100.0) as u32
        })
        .collect()
}

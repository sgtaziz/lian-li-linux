//! Per-widget renderers.
//!
//! `draw_widget` dispatches to the appropriate widget's `draw` function based
//! on `WidgetKind`, then composites the widget's sub-canvas onto the template
//! frame (with optional rotation).

pub(super) mod bar;
pub(super) mod core_bars;
pub(super) mod image_widget;
pub(super) mod label;
pub(super) mod radial_gauge;
pub(super) mod speedometer;
pub(super) mod value_text;
pub(super) mod video_widget;

use super::helpers::{resolve_font, widget_size_px, ElapsedMs};
use image::{imageops, Rgba, RgbaImage};
use imageproc::geometric_transformations::{rotate_about_center, Interpolation};
use lianli_shared::sensors::{read_sensor_value, ResolvedSensor};
use lianli_shared::template::{Widget, WidgetKind};
use rusttype::Font;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

/// Per-widget render state: resolved sensor + any preloaded media + last-frame memo.
pub(super) struct WidgetState {
    pub resolved_sensor: Option<ResolvedSensor>,
    pub loaded_image: Option<RgbaImage>,
    pub video_frames: Option<Arc<Vec<RgbaImage>>>,
    pub video_frame_duration: Duration,
    pub last_render_text: Option<String>,
    pub last_quantized: i32,
    pub failed: AtomicBool,
}

impl WidgetState {
    pub fn blank() -> Self {
        Self {
            resolved_sensor: None,
            loaded_image: None,
            video_frames: None,
            video_frame_duration: Duration::from_millis(100),
            last_render_text: None,
            last_quantized: i32::MIN,
            failed: AtomicBool::new(false),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_widget(
    frame: &mut RgbaImage,
    widget: &Widget,
    state: &WidgetState,
    uniform_scale: f32,
    offset_x: i32,
    offset_y: i32,
    fonts: &HashMap<PathBuf, Font<'static>>,
    default_font: &Font<'static>,
    elapsed_ms: ElapsedMs,
) {
    let (ww, wh) = widget_size_px(widget, uniform_scale);
    if ww == 0 || wh == 0 {
        return;
    }

    let mut sub = RgbaImage::from_pixel(ww, wh, Rgba([0, 0, 0, 0]));

    match &widget.kind {
        WidgetKind::Label {
            text,
            font,
            font_size,
            color,
            align,
        } => {
            let f = resolve_font(font, fonts, default_font);
            label::draw(
                &mut sub,
                text,
                f,
                *font_size * uniform_scale,
                *color,
                *align,
                ww,
                wh,
            );
        }
        WidgetKind::ValueText {
            font,
            font_size,
            color,
            align,
            value_min,
            value_max,
            ranges,
            ..
        } => {
            let f = resolve_font(font, fonts, default_font);
            value_text::draw(
                &mut sub,
                state,
                f,
                *font_size * uniform_scale,
                *color,
                *align,
                *value_min,
                *value_max,
                ranges,
                ww,
                wh,
            );
        }
        WidgetKind::RadialGauge {
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            inner_radius_pct,
            background_color,
            ranges,
            bg_corner_radius,
            value_corner_radius,
            ..
        } => {
            let raw = state
                .resolved_sensor
                .as_ref()
                .and_then(|s| read_sensor_value(s).ok())
                .unwrap_or(0.0);
            radial_gauge::draw(
                &mut sub,
                raw,
                *value_min,
                *value_max,
                *start_angle,
                *sweep_angle,
                *inner_radius_pct,
                *background_color,
                ranges,
                *bg_corner_radius,
                *value_corner_radius,
            );
        }
        WidgetKind::VerticalBar {
            value_min,
            value_max,
            background_color,
            corner_radius,
            ranges,
            ..
        }
        | WidgetKind::HorizontalBar {
            value_min,
            value_max,
            background_color,
            corner_radius,
            ranges,
            ..
        } => {
            let is_vertical = matches!(widget.kind, WidgetKind::VerticalBar { .. });
            let raw = state
                .resolved_sensor
                .as_ref()
                .and_then(|s| read_sensor_value(s).ok())
                .unwrap_or(0.0);
            bar::draw(
                &mut sub,
                raw,
                *value_min,
                *value_max,
                *background_color,
                *corner_radius * uniform_scale,
                ranges,
                is_vertical,
            );
        }
        WidgetKind::Speedometer {
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            needle_color,
            tick_color,
            tick_count,
            background_color,
            ranges,
            show_gauge,
            show_needle,
            needle_width,
            needle_length_pct,
            needle_border_color,
            needle_border_width,
            ..
        } => {
            let raw = state
                .resolved_sensor
                .as_ref()
                .and_then(|s| read_sensor_value(s).ok())
                .unwrap_or(0.0);
            speedometer::draw(
                &mut sub,
                raw,
                *value_min,
                *value_max,
                *start_angle,
                *sweep_angle,
                *needle_color,
                *tick_color,
                *tick_count,
                *background_color,
                ranges,
                *show_gauge,
                *show_needle,
                *needle_width,
                *needle_length_pct,
                *needle_border_color,
                *needle_border_width,
                uniform_scale,
            );
        }
        WidgetKind::CoreBars {
            orientation,
            background_color,
            show_labels,
            ranges,
        } => {
            core_bars::draw(
                &mut sub,
                *orientation,
                *background_color,
                *show_labels,
                ranges,
                uniform_scale,
                default_font,
            );
        }
        WidgetKind::Image { opacity, .. } => {
            image_widget::draw(&mut sub, state, *opacity);
        }
        WidgetKind::Video { opacity, .. } => {
            video_widget::draw(&mut sub, state, *opacity, elapsed_ms);
        }
    }

    let (ww_i, wh_i) = (ww as i32, wh as i32);
    let tl_x = offset_x + (widget.x * uniform_scale).round() as i32 - ww_i / 2;
    let tl_y = offset_y + (widget.y * uniform_scale).round() as i32 - wh_i / 2;

    if widget.rotation.abs() > 0.5 {
        let rotated = rotate_about_center(
            &sub,
            widget.rotation.to_radians(),
            Interpolation::Bilinear,
            Rgba([0, 0, 0, 0]),
        );
        imageops::overlay(frame, &rotated, tl_x as i64, tl_y as i64);
    } else {
        imageops::overlay(frame, &sub, tl_x as i64, tl_y as i64);
    }
}

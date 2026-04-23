//! Per-widget renderers.
//!
//! `draw_widget` dispatches to the appropriate widget's `draw` function based
//! on `WidgetKind`, then composites the widget's sub-canvas onto the template
//! frame (with optional rotation).

pub(super) mod bar;
pub(super) mod clock_analog;
pub(super) mod clock_digital;
pub(super) mod core_bars;
pub(super) mod image_widget;
pub(super) mod label;
pub(super) mod radial_gauge;
pub(super) mod sparkline;
pub(super) mod speedometer;
pub(super) mod value_text;
pub(super) mod video_widget;

use super::helpers::{resolve_font, widget_size_px, ElapsedMs};
use image::{imageops, Rgba, RgbaImage};
use imageproc::geometric_transformations::{rotate_about_center, Interpolation};
use lianli_shared::sensors::ResolvedSensor;
use lianli_shared::template::{Widget, WidgetKind};
use rusttype::Font;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) struct WidgetState {
    pub resolved_sensor: Option<ResolvedSensor>,
    pub loaded_image: Option<RgbaImage>,
    pub video_frames: Option<Arc<Vec<RgbaImage>>>,
    pub video_frame_duration: Duration,
    pub last_render_text: Option<String>,
    pub last_quantized: i32,
    pub failed: AtomicBool,
    pub history: VecDeque<f32>,
    pub sample_interval: Duration,
    pub last_sample_at: Option<Instant>,
    pub cached_value: f32,
    pub cached_core_usage: Vec<u32>,
    pub last_clock_key: Option<u64>,
    pub last_video_frame_idx: Option<usize>,
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
            history: VecDeque::new(),
            sample_interval: Duration::from_millis(1000),
            last_sample_at: None,
            cached_value: 0.0,
            cached_core_usage: Vec::new(),
            last_clock_key: None,
            last_video_frame_idx: None,
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

    let ss_factor: u32 = match &widget.kind {
        WidgetKind::RadialGauge { .. }
        | WidgetKind::Speedometer { .. }
        | WidgetKind::Sparkline { .. }
        | WidgetKind::ClockAnalog { .. } => 2,
        WidgetKind::VerticalBar { corner_radius, .. }
        | WidgetKind::HorizontalBar { corner_radius, .. } if *corner_radius > 0.1 => 2,
        _ => 1,
    };
    let ss = ss_factor as f32;
    let draw_w = ww * ss_factor;
    let draw_h = wh * ss_factor;
    let base_scale = uniform_scale;
    let uniform_scale = uniform_scale * ss;

    let mut sub = RgbaImage::from_pixel(draw_w, draw_h, Rgba([0, 0, 0, 0]));

    match &widget.kind {
        WidgetKind::Label {
            text,
            font,
            font_size,
            color,
            align,
            letter_spacing,
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
                *letter_spacing * uniform_scale,
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
            letter_spacing,
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
                *letter_spacing * uniform_scale,
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
            radial_gauge::draw(
                &mut sub,
                state.cached_value,
                *value_min,
                *value_max,
                *start_angle,
                *sweep_angle,
                *inner_radius_pct,
                *background_color,
                ranges,
                *bg_corner_radius * ss,
                *value_corner_radius * ss,
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
            bar::draw(
                &mut sub,
                state.cached_value,
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
            speedometer::draw(
                &mut sub,
                state.cached_value,
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
        WidgetKind::Sparkline {
            value_min,
            value_max,
            auto_range,
            line_color,
            line_width,
            fill_color,
            fill_from_ranges,
            range_blend,
            background_color,
            ranges,
            border_color,
            border_width,
            corner_radius,
            padding,
            show_points,
            point_radius,
            show_baseline,
            baseline_value,
            baseline_color,
            baseline_width,
            smooth,
            scroll_rtl,
            show_gridlines,
            gridlines_horizontal,
            gridlines_vertical,
            gridline_color,
            gridline_width,
            show_axis_labels,
            axis_label_count,
            axis_labels_on_right,
            axis_label_format,
            axis_label_font,
            axis_label_size,
            axis_label_color,
            axis_label_padding,
            ..
        } => {
            let af = resolve_font(axis_label_font, fonts, default_font);
            sparkline::draw(
                &mut sub,
                sparkline::DrawArgs {
                    history: &state.history,
                    value_min: *value_min,
                    value_max: *value_max,
                    auto_range: *auto_range,
                    line_color: *line_color,
                    line_width: *line_width * uniform_scale,
                    fill_color: *fill_color,
                    fill_from_ranges: *fill_from_ranges,
                    range_blend: *range_blend,
                    background_color: *background_color,
                    ranges,
                    border_color: *border_color,
                    border_width: *border_width * uniform_scale,
                    corner_radius: *corner_radius * uniform_scale,
                    padding: *padding * uniform_scale,
                    show_points: *show_points,
                    point_radius: *point_radius * uniform_scale,
                    show_baseline: *show_baseline,
                    baseline_value: *baseline_value,
                    baseline_color: *baseline_color,
                    baseline_width: *baseline_width * uniform_scale,
                    smooth: *smooth,
                    scroll_rtl: *scroll_rtl,
                    show_gridlines: *show_gridlines,
                    gridlines_horizontal: *gridlines_horizontal,
                    gridlines_vertical: *gridlines_vertical,
                    gridline_color: *gridline_color,
                    gridline_width: *gridline_width * uniform_scale,
                    show_axis_labels: *show_axis_labels,
                    axis_label_count: *axis_label_count,
                    axis_labels_on_right: *axis_labels_on_right,
                    axis_label_format,
                    axis_label_font: af,
                    axis_label_size: *axis_label_size * uniform_scale,
                    axis_label_color: *axis_label_color,
                    axis_label_padding: *axis_label_padding * uniform_scale,
                },
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
                &state.cached_core_usage,
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
        WidgetKind::ClockDigital {
            format,
            font,
            font_size,
            color,
            align,
            letter_spacing,
        } => {
            let f = resolve_font(font, fonts, default_font);
            clock_digital::draw(
                &mut sub,
                format,
                f,
                *font_size * uniform_scale,
                *color,
                *align,
                ww,
                wh,
                *letter_spacing * uniform_scale,
            );
        }
        WidgetKind::ClockAnalog {
            face_color,
            tick_color,
            minor_tick_color,
            hour_hand_color,
            minute_hand_color,
            second_hand_color,
            hub_color,
            numbers_color,
            numbers_font,
            numbers_font_size,
            show_seconds,
            show_hour_ticks,
            show_minor_ticks,
            show_numbers,
            hour_hand_width,
            minute_hand_width,
            second_hand_width,
            hour_hand_length_pct,
            minute_hand_length_pct,
            second_hand_length_pct,
            hour_tick_length_pct,
            minor_tick_length_pct,
            hour_tick_width,
            minor_tick_width,
            hub_radius,
        } => {
            let nf = resolve_font(numbers_font, fonts, default_font);
            clock_analog::draw(
                &mut sub,
                *face_color,
                *tick_color,
                *minor_tick_color,
                *hour_hand_color,
                *minute_hand_color,
                *second_hand_color,
                *hub_color,
                *numbers_color,
                nf,
                *numbers_font_size,
                *show_seconds,
                *show_hour_ticks,
                *show_minor_ticks,
                *show_numbers,
                *hour_hand_width,
                *minute_hand_width,
                *second_hand_width,
                *hour_hand_length_pct,
                *minute_hand_length_pct,
                *second_hand_length_pct,
                *hour_tick_length_pct,
                *minor_tick_length_pct,
                *hour_tick_width,
                *minor_tick_width,
                *hub_radius,
                uniform_scale,
            );
        }
    }

    let sub = if ss_factor > 1 {
        imageops::resize(&sub, ww, wh, imageops::FilterType::Triangle)
    } else {
        sub
    };

    let (ww_i, wh_i) = (ww as i32, wh as i32);
    let tl_x = offset_x + (widget.x * base_scale).round() as i32 - ww_i / 2;
    let tl_y = offset_y + (widget.y * base_scale).round() as i32 - wh_i / 2;

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

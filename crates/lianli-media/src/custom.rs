//! `CustomAsset` — the data-driven renderer for `MediaType::Custom`.
//!
//! Orchestrates per-widget state (resolved sensors, preloaded images / decoded
//! video frames), composites each widget onto a baked template frame every
//! render tick, and encodes the result as JPEG. Widget drawing lives under
//! [`widgets`], shared helpers under [`helpers`].

mod helpers;
mod widgets;

use crate::common::{apply_orientation, encode_jpeg, render_dimensions, MediaError};
use crate::sensor::FrameInfo;
use crate::video::decode_frames_to_rgba;
use helpers::{
    fit_image, format_sensor_readout, load_font_from_disk, resolve_sensor_source, widget_font_refs,
    widget_sensor_source, widget_size_px, ElapsedMs,
};
use image::imageops::FilterType;
use image::{imageops, DynamicImage, ImageBuffer, Rgb, Rgba, RgbaImage};
use imageproc::drawing::draw_filled_rect_mut;
use imageproc::rect::Rect;
use lianli_shared::fonts::default_font_path;
use lianli_shared::screen::ScreenInfo;
use lianli_shared::sensors::{read_sensor_value, SensorInfo};
use lianli_shared::systeminfo::SysSensor;
use lianli_shared::template::{LcdTemplate, TemplateBackground, WidgetKind};
use parking_lot::Mutex;
use rusttype::Font;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;
use widgets::{draw_widget, WidgetState};

fn default_sample_interval(kind: &WidgetKind, explicit_ms: Option<u64>) -> Duration {
    let default_ms = match kind {
        WidgetKind::ClockAnalog { show_seconds, .. } if *show_seconds => 100,
        WidgetKind::ClockAnalog { .. } | WidgetKind::ClockDigital { .. } => 1000,
        _ => 1000,
    };
    let min_ms = match kind {
        WidgetKind::ClockAnalog { show_seconds, .. } if *show_seconds => 50,
        _ => 100,
    };
    Duration::from_millis(explicit_ms.unwrap_or(default_ms).max(min_ms))
}

pub struct CustomAsset {
    template: LcdTemplate,
    widget_states: Mutex<Vec<WidgetState>>,
    template_image: RgbaImage,
    screen: ScreenInfo,
    orientation: f32,
    update_interval: Duration,
    uniform_scale: f32,
    offset_x: i32,
    offset_y: i32,
    canonical_width: u32,
    canonical_height: u32,
    fonts: HashMap<PathBuf, Font<'static>>,
    default_font: Font<'static>,
    frame_index: AtomicUsize,
    start_instant: Instant,
}

impl std::fmt::Debug for CustomAsset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomAsset")
            .field("template_id", &self.template.id)
            .field("screen", &self.screen)
            .field("orientation", &self.orientation)
            .field("update_interval", &self.update_interval)
            .finish()
    }
}

impl CustomAsset {
    pub fn new(
        template: &LcdTemplate,
        orientation: f32,
        screen: &ScreenInfo,
        all_sensors: &[SensorInfo],
    ) -> Result<Arc<Self>, MediaError> {
        let default_path = default_font_path().ok_or_else(|| {
            MediaError::Sensor("no system font available; install fontconfig or DejaVu Sans".into())
        })?;
        let default_font = load_font_from_disk(&default_path)?;
        let mut fonts: HashMap<PathBuf, Font<'static>> = HashMap::new();
        for w in &template.widgets {
            for fr in widget_font_refs(&w.kind) {
                if let Some(p) = &fr.path {
                    if !fonts.contains_key(p) {
                        match load_font_from_disk(p) {
                            Ok(f) => {
                                fonts.insert(p.clone(), f);
                            }
                            Err(e) => warn!(
                                "template '{}' widget '{}' font '{}' failed: {e}",
                                template.id,
                                w.id,
                                p.display()
                            ),
                        }
                    }
                }
            }
        }

        let (canvas_w, canvas_h) = render_dimensions(screen, orientation);
        let uniform_scale = (canvas_w as f32 / template.base_width as f32)
            .min(canvas_h as f32 / template.base_height as f32)
            .max(0.01);
        let scaled_w = (template.base_width as f32 * uniform_scale).round() as u32;
        let scaled_h = (template.base_height as f32 * uniform_scale).round() as u32;
        let offset_x = ((canvas_w as i32) - scaled_w as i32) / 2;
        let offset_y = ((canvas_h as i32) - scaled_h as i32) / 2;

        let letterbox_rgb = match template.background {
            TemplateBackground::Color { rgb } => [rgb[0], rgb[1], rgb[2]],
            TemplateBackground::Image { .. } => [0, 0, 0],
        };
        let mut composite = RgbaImage::from_pixel(
            canvas_w,
            canvas_h,
            Rgba([letterbox_rgb[0], letterbox_rgb[1], letterbox_rgb[2], 255]),
        );

        match &template.background {
            TemplateBackground::Color { rgb } => {
                let fill = Rgba(*rgb);
                let rect = Rect::at(offset_x, offset_y).of_size(scaled_w, scaled_h);
                draw_filled_rect_mut(&mut composite, rect, fill);
            }
            TemplateBackground::Image { path } => match ::image::open(path) {
                Ok(img) => {
                    let resized = img
                        .resize_exact(scaled_w, scaled_h, FilterType::Lanczos3)
                        .to_rgba8();
                    imageops::overlay(&mut composite, &resized, offset_x as i64, offset_y as i64);
                }
                Err(e) => warn!(
                    "template '{}' background image '{}' failed to load: {e}",
                    template.id,
                    path.display()
                ),
            },
        }

        let mut widget_states: Vec<WidgetState> = Vec::with_capacity(template.widgets.len());

        for widget in &template.widgets {
            let mut state = WidgetState::blank();
            state.sample_interval =
                default_sample_interval(&widget.kind, widget.update_interval_ms);

            if let Some(source) = widget_sensor_source(&widget.kind) {
                state.resolved_sensor = resolve_sensor_source(source, all_sensors);
                if state.resolved_sensor.is_none() {
                    warn!(
                        "template '{}' widget '{}' sensor unavailable — rendering as zero",
                        template.id, widget.id
                    );
                }
            }

            if let WidgetKind::Image { path, fit, .. } = &widget.kind {
                let (ww, wh) = widget_size_px(widget, uniform_scale);
                match ::image::open(path) {
                    Ok(img) => {
                        state.loaded_image = Some(fit_image(img, ww, wh, *fit));
                    }
                    Err(e) => warn!(
                        "template '{}' widget '{}' image '{}' failed: {e}",
                        template.id,
                        widget.id,
                        path.display()
                    ),
                }
            }

            if let WidgetKind::Video { path, .. } = &widget.kind {
                let (ww, wh) = widget_size_px(widget, uniform_scale);
                let fps = widget.fps.unwrap_or(30.0).max(1.0);
                match decode_frames_to_rgba(path, fps, ww.max(1), wh.max(1)) {
                    Ok((frames, durations)) => {
                        let duration = durations
                            .first()
                            .copied()
                            .unwrap_or(Duration::from_millis(33));
                        state.video_frame_duration = duration;
                        state.video_frames = Some(Arc::new(frames));
                    }
                    Err(e) => warn!(
                        "template '{}' widget '{}' video '{}' decode failed: {e}",
                        template.id,
                        widget.id,
                        path.display()
                    ),
                }
            }

            widget_states.push(state);
        }

        let fps = screen.max_fps.max(1);
        let frame_interval =
            Duration::from_nanos(1_000_000_000 / fps as u64).max(Duration::from_millis(16));

        Ok(Arc::new(Self {
            template: template.clone(),
            widget_states: Mutex::new(widget_states),
            template_image: composite,
            screen: *screen,
            orientation,
            update_interval: frame_interval,
            uniform_scale,
            offset_x,
            offset_y,
            canonical_width: canvas_w,
            canonical_height: canvas_h,
            fonts,
            default_font,
            frame_index: AtomicUsize::new(1),
            start_instant: Instant::now(),
        }))
    }

    pub fn update_interval(&self) -> Duration {
        self.update_interval
    }

    pub fn seed_preview_history(&self) {
        let mut states = self.widget_states.lock();
        for (widget, state) in self.template.widgets.iter().zip(states.iter_mut()) {
            if let WidgetKind::Sparkline {
                history_length,
                value_min,
                value_max,
                ..
            } = &widget.kind
            {
                let cap = (*history_length).max(8) as usize;
                let span = (value_max - value_min).abs().max(1.0);
                let base = (value_min + value_max) * 0.5;
                state.history.clear();
                state.history.reserve(cap);
                for i in 0..cap {
                    let t = i as f32 / (cap - 1).max(1) as f32;
                    let phase = t * std::f32::consts::PI * 3.0;
                    let v = base + span * 0.35 * phase.sin();
                    state.history.push_back(v);
                }
            }
        }
    }

    pub fn blank_frame(&self) -> FrameInfo {
        let fill = match self.template.background {
            TemplateBackground::Color { rgb } => Rgb([rgb[0], rgb[1], rgb[2]]),
            TemplateBackground::Image { .. } => Rgb([0, 0, 0]),
        };
        let image = ImageBuffer::from_pixel(self.canonical_width, self.canonical_height, fill);
        let oriented = apply_orientation(image, self.orientation);
        FrameInfo {
            data: encode_jpeg(oriented, &self.screen).unwrap_or_default(),
            frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst),
        }
    }

    pub fn render_frame(&self, force: bool) -> Result<Option<FrameInfo>, MediaError> {
        let now = Instant::now();
        let elapsed_ms = now
            .saturating_duration_since(self.start_instant)
            .as_millis() as u64;

        let mut states = self.widget_states.lock();
        let mut any_dynamic_changed = force;
        for (widget, state) in self.template.widgets.iter().zip(states.iter_mut()) {
            if !widget.visible {
                continue;
            }

            let due = state
                .last_sample_at
                .map(|t| now.saturating_duration_since(t) >= state.sample_interval)
                .unwrap_or(true);

            if let Some(sensor) = &state.resolved_sensor {
                if due {
                    let raw = match read_sensor_value(sensor) {
                        Ok(v) => {
                            state.failed.store(false, Ordering::Relaxed);
                            v
                        }
                        Err(e) => {
                            if !state.failed.swap(true, Ordering::Relaxed) {
                                warn!(
                                    "custom template '{}' widget '{}' sensor read failed: {e}",
                                    self.template.id, widget.id
                                );
                            }
                            0.0
                        }
                    };
                    state.cached_value = raw;
                    state.last_sample_at = Some(now);

                    let (text, quantized) = format_sensor_readout(&widget.kind, raw);
                    let changed = state.last_render_text.as_deref() != Some(text.as_str())
                        || state.last_quantized != quantized;
                    if changed {
                        any_dynamic_changed = true;
                        state.last_render_text = Some(text);
                        state.last_quantized = quantized;
                    }
                    if let WidgetKind::Sparkline { history_length, .. } = &widget.kind {
                        let cap = (*history_length).max(2) as usize;
                        state.history.push_back(raw);
                        while state.history.len() > cap {
                            state.history.pop_front();
                        }
                        any_dynamic_changed = true;
                    }
                }
                continue;
            }

            match &widget.kind {
                WidgetKind::CoreBars { .. } => {
                    if due {
                        let usage = SysSensor::get_core_usage();
                        if usage != state.cached_core_usage {
                            state.cached_core_usage = usage;
                            any_dynamic_changed = true;
                        }
                        state.last_sample_at = Some(now);
                    }
                }
                WidgetKind::ClockDigital { .. } | WidgetKind::ClockAnalog { .. } => {
                    let key = elapsed_ms / state.sample_interval.as_millis().max(1) as u64;
                    if state.last_clock_key != Some(key) {
                        state.last_clock_key = Some(key);
                        state.last_sample_at = Some(now);
                        any_dynamic_changed = true;
                    }
                }
                WidgetKind::Video { .. } => {
                    if let Some(frames) = &state.video_frames {
                        if !frames.is_empty() {
                            let dur_ms = state.video_frame_duration.as_millis().max(1) as u64;
                            let idx = ((elapsed_ms / dur_ms) as usize) % frames.len();
                            if state.last_video_frame_idx != Some(idx) {
                                state.last_video_frame_idx = Some(idx);
                                state.last_sample_at = Some(now);
                                any_dynamic_changed = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if !any_dynamic_changed {
            return Ok(None);
        }

        let mut frame = self.template_image.clone();
        for (widget, state) in self.template.widgets.iter().zip(states.iter()) {
            if !widget.visible {
                continue;
            }
            draw_widget(
                &mut frame,
                widget,
                state,
                self.uniform_scale,
                self.offset_x,
                self.offset_y,
                &self.fonts,
                &self.default_font,
                ElapsedMs::from(elapsed_ms),
            );
        }
        drop(states);

        let rgb = DynamicImage::ImageRgba8(frame).to_rgb8();
        let oriented = apply_orientation(rgb, self.orientation);
        let jpeg = encode_jpeg(oriented, &self.screen)?;

        Ok(Some(FrameInfo {
            data: jpeg,
            frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst),
        }))
    }
}

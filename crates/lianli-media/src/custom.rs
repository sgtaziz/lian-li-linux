//! `CustomAsset` — the data-driven renderer for `MediaType::Custom`.
//!
//! Orchestrates per-widget state (resolved sensors, images and video playback),
//! composites each widget onto a baked template frame every
//! render tick, and encodes the result as JPEG. Widget drawing lives under
//! [`widgets`], shared helpers under [`helpers`].

mod geometry;
mod helpers;
mod history;
mod widgets;

use crate::common::{encode_jpeg_rgba, render_dimensions, MediaError};
use crate::sensor::FrameInfo;
use crate::PreparationControl;
use ab_glyph::FontVec;
use helpers::{
    fast_overlay, fit_image, format_sensor_readout, load_font_from_disk, resolve_sensor_source,
    widget_sensor_source, widget_size_px,
};
use image::imageops::FilterType;
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_filled_rect_mut;
use imageproc::rect::Rect;
use lianli_shared::fonts::default_font_path;
use lianli_shared::screen::ScreenInfo;
use lianli_shared::sensors::{read_sensor_value, SensorInfo};
use lianli_shared::systeminfo::SysSensor;
use lianli_shared::template::{LcdTemplate, TemplateBackground, WidgetKind};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};
use widgets::{draw_widget, WidgetState};

const SENSOR_RESOLVE_RETRY: Duration = Duration::from_secs(2);

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
    sensors: Vec<SensorInfo>,
    widget_states: Mutex<Vec<WidgetState>>,
    template_image: RgbaImage,
    scratch: Mutex<RgbaImage>,
    screen: ScreenInfo,
    orientation: f32,
    update_interval: Duration,
    render_fps: f32,
    uniform_scale: f32,
    offset_x: i32,
    offset_y: i32,
    canonical_width: u32,
    canonical_height: u32,
    fonts: HashMap<PathBuf, FontVec>,
    default_font: FontVec,
    smooth_edges: bool,
    frame_index: AtomicUsize,
    start_instant: Instant,
    _retained_budget: crate::resource_budget::RetainedBudget,
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
        smooth_edges: bool,
        fps: f32,
        control: impl Into<PreparationControl>,
    ) -> Result<Arc<Self>, MediaError> {
        let control = control.into();
        control.check()?;
        let (canvas_w, canvas_h) = render_dimensions(screen, orientation);
        let (uniform_scale, scaled_w, scaled_h) =
            geometry::validate(template, canvas_w, canvas_h, smooth_edges)?;
        let mut retained_budget = crate::resource_budget::RetainedBudget::default();
        retained_budget.reserve(canvas_w as usize * canvas_h as usize * 8)?;
        for widget in &template.widgets {
            let (width, height) = widget_size_px(widget, uniform_scale);
            let copies = if matches!(widget.kind, WidgetKind::Image { .. }) {
                2
            } else {
                1
            };
            retained_budget.reserve(width as usize * height as usize * 4 * copies)?;
        }
        crate::asset_access::validate_dependencies(
            &lianli_shared::media_dependencies::template_dependencies(template),
            &control,
        )?;
        let default_path = default_font_path().ok_or_else(|| {
            MediaError::Sensor("No system font found. Install fontconfig or DejaVu Sans.".into())
        })?;
        let default_font = load_font_from_disk(&default_path)?;
        retained_budget.reserve(default_font.as_slice().len())?;
        let mut fonts: HashMap<PathBuf, FontVec> = HashMap::new();
        for w in &template.widgets {
            control.check()?;
            if let Some(fr) = w.kind.font_ref() {
                if let Some(p) = &fr.path {
                    if !fonts.contains_key(p) {
                        let font = load_font_from_disk(p).map_err(|error| {
                            MediaError::InvalidConfig(format!(
                                "Template '{}' widget '{}' font '{}': {error}",
                                template.id,
                                w.id,
                                p.display()
                            ))
                        })?;
                        retained_budget.reserve(font.as_slice().len())?;
                        fonts.insert(p.clone(), font);
                    }
                }
            }
        }

        for widget in &template.widgets {
            control.check()?;
            let font = widget.kind.font_ref().map_or(&default_font, |reference| {
                helpers::resolve_font(reference, &fonts, &default_font)
            });
            let scale = uniform_scale * geometry::supersampling(&widget.kind, smooth_edges) as f32;
            geometry::validate_font(&widget.kind, scale, font).map_err(|error| {
                MediaError::InvalidConfig(format!(
                    "Template '{}' widget '{}' font: {error}",
                    template.id, widget.id
                ))
            })?;
        }

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
            TemplateBackground::Image { path } => match crate::image::open_image(path) {
                Ok(img) => {
                    let resized = img
                        .resize_exact(scaled_w, scaled_h, FilterType::Lanczos3)
                        .to_rgba8();
                    fast_overlay(&mut composite, &resized, offset_x as i64, offset_y as i64);
                }
                Err(error) => {
                    return Err(MediaError::InvalidConfig(format!(
                        "Template '{}' background image '{}': {error}",
                        template.id,
                        path.display()
                    )))
                }
            },
        }

        let mut widget_states: Vec<WidgetState> = Vec::with_capacity(template.widgets.len());

        for widget in &template.widgets {
            control.check()?;
            let mut state = WidgetState::blank();
            if let WidgetKind::Sparkline { history_length, .. } = &widget.kind {
                state.history.reserve(history::capacity(*history_length));
                retained_budget.reserve(state.history.capacity() * std::mem::size_of::<f32>())?;
            }
            state.sample_interval =
                default_sample_interval(&widget.kind, widget.update_interval_ms);

            if let Some(source) = widget_sensor_source(&widget.kind) {
                state.resolved_sensor = resolve_sensor_source(source, all_sensors);
                if state.resolved_sensor.is_none() {
                    state.next_resolve_at = Some(Instant::now() + SENSOR_RESOLVE_RETRY);
                    warn!(
                        "template '{}' widget '{}' sensor unavailable — rendering as zero until it appears",
                        template.id, widget.id
                    );
                }
            }

            if let WidgetKind::Image { path, fit, .. } = &widget.kind {
                let (ww, wh) = widget_size_px(widget, uniform_scale);
                match crate::image::open_image(path) {
                    Ok(img) => {
                        state.loaded_image = Some(fit_image(img, ww, wh, *fit));
                    }
                    Err(error) => {
                        return Err(MediaError::InvalidConfig(format!(
                            "Template '{}' widget '{}' image '{}': {error}",
                            template.id,
                            widget.id,
                            path.display()
                        )))
                    }
                }
            }

            if let WidgetKind::Video {
                path,
                fit,
                loop_playback,
                ..
            } = &widget.kind
            {
                let (ww, wh) = widget_size_px(widget, uniform_scale);
                let requested = widget.fps.unwrap_or(30.0).min(fps).clamp(1.0, 60.0);
                let decode_fps = if crate::video::widget_animation::is_animation(path) {
                    requested
                } else {
                    crate::video::ffmpeg::cap_fps_cancellable(path, requested, &control)?
                };
                state.video_stream = Some(
                    crate::video::widget_stream::VideoStream::new(
                        path,
                        decode_fps,
                        (ww.max(1), wh.max(1)),
                        *fit,
                        *loop_playback,
                        &control,
                    )
                    .map_err(|error| {
                        if matches!(error, MediaError::Cancelled) {
                            return error;
                        }
                        MediaError::InvalidConfig(format!(
                            "Template '{}' widget '{}' video '{}': {error}",
                            template.id,
                            widget.id,
                            path.display()
                        ))
                    })?,
                );
            }

            widget_states.push(state);
        }

        control.check()?;
        let fps = fps.max(1.0);
        let frame_interval =
            Duration::from_nanos(1_000_000_000 / fps as u64).max(Duration::from_millis(16));

        let scratch = composite.clone();
        Ok(Arc::new(Self {
            _retained_budget: retained_budget,
            template: template.clone(),
            sensors: all_sensors.to_vec(),
            widget_states: Mutex::new(widget_states),
            template_image: composite,
            scratch: Mutex::new(scratch),
            screen: *screen,
            orientation,
            update_interval: frame_interval,
            render_fps: fps,
            uniform_scale,
            offset_x,
            offset_y,
            canonical_width: canvas_w,
            canonical_height: canvas_h,
            fonts,
            default_font,
            smooth_edges,
            frame_index: AtomicUsize::new(1),
            start_instant: Instant::now(),
        }))
    }

    pub fn update_interval(&self) -> Duration {
        self.update_interval
    }

    pub fn render_fps(&self) -> f32 {
        self.render_fps
    }

    pub fn canvas_width(&self) -> u32 {
        self.canonical_width
    }

    pub fn canvas_height(&self) -> u32 {
        self.canonical_height
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
                history::seed(&mut state.history, *history_length, *value_min, *value_max);
            }
        }
    }

    pub fn blank_frame(&self) -> FrameInfo {
        let fill = match self.template.background {
            TemplateBackground::Color { rgb } => Rgba([rgb[0], rgb[1], rgb[2], 255]),
            TemplateBackground::Image { .. } => Rgba([0, 0, 0, 255]),
        };
        let image = RgbaImage::from_pixel(self.canonical_width, self.canonical_height, fill);
        FrameInfo {
            data: encode_jpeg_rgba(
                image.as_raw(),
                self.canonical_width,
                self.canonical_height,
                self.orientation,
                &self.screen,
            )
            .unwrap_or_default(),
            frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst),
        }
    }

    pub fn render_frame(&self, force: bool) -> Result<Option<FrameInfo>, MediaError> {
        let outcome = self.render_frame_rgba_with(force, |bytes| {
            encode_jpeg_rgba(
                bytes,
                self.canonical_width,
                self.canonical_height,
                self.orientation,
                &self.screen,
            )
        })?;
        match outcome {
            None => Ok(None),
            Some(Err(e)) => Err(e),
            Some(Ok(jpeg)) => Ok(Some(FrameInfo {
                data: jpeg,
                frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst),
            })),
        }
    }

    pub fn total_rotation_deg(&self) -> u16 {
        (self.orientation as i32).rem_euclid(360) as u16
    }

    /// Render one frame and hand the raw RGBA bytes to `cb` while still holding the
    /// scratch lock. Avoids a 3.7 MB clone per frame on the hot path. Returns
    /// `Ok(None)` when nothing has changed since the last render and `force` is
    /// false. The bytes are at `(canonical_width, canonical_height)` in the
    /// pre-orientation logical layout — callers needing rotation must apply it.
    pub fn render_frame_rgba_with<R>(
        &self,
        force: bool,
        cb: impl FnOnce(&[u8]) -> R,
    ) -> Result<Option<R>, MediaError> {
        let text_work = crate::text_work::FrameTextWork::begin();
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

            if state.resolved_sensor.is_none() && state.next_resolve_at.is_none_or(|at| now >= at) {
                if let Some(source) = widget_sensor_source(&widget.kind) {
                    state.resolved_sensor = resolve_sensor_source(source, &self.sensors);
                    if state.resolved_sensor.is_some() {
                        state.next_resolve_at = None;
                        debug!(
                            "template '{}' widget '{}' sensor became available",
                            self.template.id, widget.id
                        );
                        any_dynamic_changed = true;
                    } else {
                        state.next_resolve_at = Some(now + SENSOR_RESOLVE_RETRY);
                    }
                }
            }

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
                        history::push(&mut state.history, *history_length, raw);
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
                WidgetKind::Video { path, .. } => {
                    if let Some(stream) = &mut state.video_stream {
                        if stream.advance(now).map_err(|error| {
                            MediaError::Ffmpeg(format!(
                                "Template '{}' widget '{}' video '{}': {error}",
                                self.template.id,
                                widget.id,
                                path.display()
                            ))
                        })? {
                            state.last_video_frame_idx =
                                Some(state.last_video_frame_idx.unwrap_or(0).wrapping_add(1));
                            any_dynamic_changed = true;
                        }
                        continue;
                    }
                }
                _ => {}
            }
        }

        if !any_dynamic_changed {
            return Ok(None);
        }

        let mut scratch = self.scratch.lock();
        scratch
            .as_mut()
            .copy_from_slice(self.template_image.as_raw());
        for (widget, state) in self.template.widgets.iter().zip(states.iter_mut()) {
            if !widget.visible {
                continue;
            }
            draw_widget(
                &mut scratch,
                widget,
                state,
                self.uniform_scale,
                self.offset_x,
                self.offset_y,
                &self.fonts,
                &self.default_font,
                self.smooth_edges,
            );
            if let Err(error) = text_work.check() {
                state.cached_render = None;
                state.cached_render_key = None;
                return Err(error);
            }
        }
        drop(states);

        let result = cb(scratch.as_raw());
        drop(scratch);
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_asset(widgets: Vec<serde_json::Value>) -> CustomAsset {
        let template: LcdTemplate = serde_json::from_value(serde_json::json!({
            "id": "test", "name": "Test", "base_width": 8, "base_height": 8,
            "background": {"type": "color", "rgb": [0,0,0]}, "widgets": widgets
        }))
        .unwrap();
        let font = crate::fonts::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../templates/assets/neon-us88/JetBrainsMonoNL-Medium.ttf"),
        )
        .unwrap();
        CustomAsset {
            widget_states: Mutex::new(
                (0..template.widgets.len())
                    .map(|_| WidgetState::blank())
                    .collect(),
            ),
            template,
            sensors: Vec::new(),
            template_image: RgbaImage::new(8, 8),
            scratch: Mutex::new(RgbaImage::new(8, 8)),
            screen: ScreenInfo::WIRELESS_LCD,
            orientation: 0.0,
            update_interval: Duration::from_millis(100),
            render_fps: 10.0,
            uniform_scale: 1.0,
            offset_x: 0,
            offset_y: 0,
            canonical_width: 8,
            canonical_height: 8,
            fonts: HashMap::new(),
            default_font: font,
            smooth_edges: false,
            frame_index: AtomicUsize::new(0),
            start_instant: Instant::now(),
            _retained_budget: crate::resource_budget::RetainedBudget::default(),
        }
    }

    #[test]
    fn sensor_retry_redraws_then_suppresses_unchanged_frames() {
        let asset = test_asset(vec![serde_json::json!({
            "id": "sensor", "x": 4, "y": 4, "width": 8, "height": 8,
            "kind": {
                "type": "horizontal_bar", "source": {"type": "constant", "value": 50},
                "value_min": 0, "value_max": 100, "background_color": [0, 0, 0, 255]
            }
        })]);
        asset.widget_states.lock()[0].next_resolve_at =
            Some(Instant::now() + Duration::from_secs(3600));
        let initial = asset
            .render_frame_rgba_with(true, |bytes| bytes.to_vec())
            .unwrap()
            .unwrap();
        assert!(asset
            .render_frame_rgba_with(false, |_| panic!("retry ran before its deadline"))
            .unwrap()
            .is_none());

        asset.widget_states.lock()[0].next_resolve_at = Some(Instant::now());
        let resolved = asset
            .render_frame_rgba_with(false, |bytes| bytes.to_vec())
            .unwrap()
            .expect("new sensor value redraws without forcing a frame");
        assert_ne!(initial, resolved);
        assert_eq!(asset.widget_states.lock()[0].cached_value, 50.0);
        asset.widget_states.lock()[0].last_sample_at = None;
        assert!(asset
            .render_frame_rgba_with(false, |_| panic!("unchanged value redrew the frame"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn excessive_text_never_reaches_the_frame_consumer_and_can_recover() {
        let widgets: Vec<_> = (0..20)
            .map(|index| {
                serde_json::json!({
                    "id": format!("label-{index}"), "x": 4, "y": 4, "width": 8, "height": 8,
                    "kind": {"type": "label", "text": "M".repeat(4096), "font_size": 1,
                        "color": [255,255,255,255]}
                })
            })
            .collect();
        let mut asset = test_asset(widgets);
        let error = asset
            .render_frame_rgba_with(true, |_| panic!("partial frame was published"))
            .unwrap_err();
        assert!(error.to_string().contains("per-frame work limit"));
        let states = asset.widget_states.lock();
        assert!(states.iter().any(|state| state.cached_render.is_some()));
        assert!(states
            .iter()
            .any(|state| state.cached_render.is_none() && state.cached_render_key.is_none()));
        drop(states);
        asset.template.widgets.truncate(1);
        asset.widget_states.lock().truncate(1);
        assert_eq!(
            asset
                .render_frame_rgba_with(true, |bytes| bytes.len())
                .unwrap(),
            Some(8 * 8 * 4)
        );
    }
}

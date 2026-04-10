//! `CustomAsset` — the data-driven renderer for `MediaType::Custom`.
//!
//! Takes an [`LcdTemplate`] (built-in or user-authored) and composites a
//! frame on each tick. Every widget type in `WidgetKind` is drawn by a
//! private `draw_*` helper in this file. No widget has its own file yet —
//! each draw function is self-contained and small enough to live here.
//!
//! Template-space coordinates (`base_width × base_height`) are scaled to
//! the target LCD with a single `uniform_scale`, preserving aspect ratio;
//! mismatched aspect ratios letterbox/pillarbox with the template's
//! background fill. The template's `orientation` field (Portrait /
//! Landscape) rotates the full composite once post-render; `LcdConfig`'s
//! per-device `orientation` is applied on top via `apply_orientation` in
//! `common.rs`, same as every other media type.

use crate::common::{
    apply_orientation, encode_jpeg, get_exact_text_metrics, MediaError,
    FONT_DATA as VICTOR_MONO_DATA, FONT_DATA_DIGITAL_7 as DIGITAL7_DATA,
    FONT_DATA_LABEL as JETBRAINS_MONO_DATA,
};
use crate::sensor::FrameInfo;
use crate::video::decode_frames_to_rgba;
use image::imageops::{rotate90, FilterType};
use image::{imageops, DynamicImage, ImageBuffer, Rgb, Rgba, RgbaImage};
use imageproc::drawing::{
    draw_antialiased_line_segment_mut, draw_filled_rect_mut, draw_polygon_mut, draw_text_mut,
};
use imageproc::geometric_transformations::{rotate_about_center, Interpolation};
use imageproc::pixelops::interpolate;
use imageproc::point::Point;
use imageproc::rect::Rect;
use lianli_shared::media::{SensorRange, SensorSourceConfig};
use lianli_shared::screen::ScreenInfo;
use lianli_shared::sensors::{read_sensor_value, resolve_sensor, ResolvedSensor, SensorInfo};
use lianli_shared::systeminfo::SysSensor;
use lianli_shared::template::{
    BarOrientation, BuiltinFont, FontRef, ImageFit, LcdTemplate, TemplateBackground,
    TemplateOrientation, TextAlign, Widget, WidgetKind,
};
use parking_lot::Mutex;
use rusttype::{Font, Scale};
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;

// ---------------------------------------------------------------------------
// CustomAsset
// ---------------------------------------------------------------------------

/// Per-widget runtime state — resolved sensors, cached values, loaded assets.
/// Index-aligned with `CustomAsset.widgets` (clone of the template widgets).
struct WidgetState {
    resolved_sensor: Option<ResolvedSensor>,
    loaded_image: Option<RgbaImage>, // Image widget, pre-resized
    video_frames: Option<Arc<Vec<RgbaImage>>>,
    video_frame_duration: Duration,
    last_render_text: Option<String>, // dirty check for sensor-bound text
    last_quantized: i32,              // dirty check for sensor-bound geometry
    failed: AtomicBool,
}

impl WidgetState {
    fn blank() -> Self {
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

pub struct CustomAsset {
    template: LcdTemplate,
    widget_states: Mutex<Vec<WidgetState>>,
    /// Pre-rendered background (already scaled, letterboxed, and with static
    /// elements like labels and image widgets baked in).
    template_image: RgbaImage,
    screen: ScreenInfo,
    orientation: f32, // LcdConfig.orientation — applied post-composite
    /// Minimum interval across all widgets; drives the render loop frequency.
    update_interval: Duration,
    /// Uniform scale from template-space to composite buffer.
    uniform_scale: f32,
    /// Top-left offset of the template inside the composite buffer (letterbox).
    offset_x: i32,
    offset_y: i32,
    /// Canonical-composite dimensions (before template orientation rotation).
    canonical_width: u32,
    canonical_height: u32,
    // Bundled fonts, loaded once. Custom font files (FontRef::File) are
    // loaded lazily inside draw calls; we don't cache them across frames
    // since the hot path uses built-ins.
    font_victor: Font<'static>,
    font_jetbrains: Font<'static>,
    font_digital7: Font<'static>,
    frame_index: AtomicUsize,
    /// Wall-clock start, used to compute video widget current frame.
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
        let font_victor = Font::try_from_bytes(VICTOR_MONO_DATA)
            .ok_or_else(|| MediaError::Sensor("bundled Victor Mono font failed to load".into()))?;
        let font_jetbrains = Font::try_from_bytes(JETBRAINS_MONO_DATA).ok_or_else(|| {
            MediaError::Sensor("bundled JetBrains Mono font failed to load".into())
        })?;
        let font_digital7 = Font::try_from_bytes(DIGITAL7_DATA)
            .ok_or_else(|| MediaError::Sensor("bundled Digital-7 font failed to load".into()))?;

        // Build a canonical render buffer sized to the LCD. For Landscape
        // templates the base canvas is rotated 90° CW before the uniform
        // scale is computed, so Landscape-authored templates fit wide LCDs
        // without stretching.
        let (src_w, src_h) = match template.orientation {
            TemplateOrientation::Portrait => (template.base_width, template.base_height),
            TemplateOrientation::Landscape => (template.base_height, template.base_width),
        };
        let uniform_scale = (screen.width as f32 / src_w as f32)
            .min(screen.height as f32 / src_h as f32)
            .max(0.01);
        let scaled_w = (src_w as f32 * uniform_scale).round() as u32;
        let scaled_h = (src_h as f32 * uniform_scale).round() as u32;
        let offset_x = ((screen.width as i32) - scaled_w as i32) / 2;
        let offset_y = ((screen.height as i32) - scaled_h as i32) / 2;

        // Letterbox color — fallback to template.background if it's a color,
        // otherwise black (image backgrounds always letterbox onto black).
        let letterbox_rgb = match template.background {
            TemplateBackground::Color { rgb } => rgb,
            TemplateBackground::Image { .. } => [0, 0, 0],
        };
        let mut composite = RgbaImage::from_pixel(
            screen.width,
            screen.height,
            Rgba([letterbox_rgb[0], letterbox_rgb[1], letterbox_rgb[2], 255]),
        );

        // Paint the template background (color or image). Image backgrounds
        // get stretched to the scaled template rectangle via Lanczos3.
        match &template.background {
            TemplateBackground::Color { rgb } => {
                let fill = Rgba([rgb[0], rgb[1], rgb[2], 255]);
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

        // Initialize per-widget runtime state: resolve sensors, pre-load
        // images/videos, and bake static widgets (Label, Image) into the
        // composite background buffer.
        let mut widget_states: Vec<WidgetState> = Vec::with_capacity(template.widgets.len());
        let mut min_interval = Duration::from_millis(1000);

        for widget in &template.widgets {
            let mut state = WidgetState::blank();

            // Resolve the widget's sensor source if any.
            if let Some(source) = widget_sensor_source(&widget.kind) {
                state.resolved_sensor = resolve_sensor_source(source, all_sensors);
                if state.resolved_sensor.is_none() {
                    warn!(
                        "template '{}' widget '{}' sensor unavailable — rendering as zero",
                        template.id, widget.id
                    );
                }
            }

            // Pre-load static images sized to the widget's bounding box.
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

            // Pre-decode video frames. Done once at asset-create time —
            // ffmpeg invocation is slow, but the hot path reduces to a
            // frame-index lookup.
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
                        // Video playback caps the render interval so frames
                        // actually advance on the device.
                        if duration < min_interval {
                            min_interval = duration;
                        }
                    }
                    Err(e) => warn!(
                        "template '{}' widget '{}' video '{}' decode failed: {e}",
                        template.id,
                        widget.id,
                        path.display()
                    ),
                }
            }

            // Sensor-bound widgets pull min_interval down to their configured
            // tick rate (defaulting to 1000 ms).
            if state.resolved_sensor.is_some() {
                let widget_interval =
                    Duration::from_millis(widget.update_interval_ms.unwrap_or(1000).max(100));
                if widget_interval < min_interval {
                    min_interval = widget_interval;
                }
            }

            widget_states.push(state);
        }

        // Bake static widgets (Label, Image) directly into the composite
        // background so they aren't redrawn every tick. Dynamic widgets are
        // drawn live in render_frame().
        for (widget, state) in template.widgets.iter().zip(widget_states.iter()) {
            if !widget.visible {
                continue;
            }
            if is_static_widget(&widget.kind) {
                draw_widget(
                    &mut composite,
                    widget,
                    state,
                    uniform_scale,
                    offset_x,
                    offset_y,
                    &font_victor,
                    &font_jetbrains,
                    &font_digital7,
                    ElapsedMs(0), // unused for static widgets
                );
            }
        }

        Ok(Arc::new(Self {
            template: template.clone(),
            widget_states: Mutex::new(widget_states),
            template_image: composite,
            screen: *screen,
            orientation,
            update_interval: min_interval.max(Duration::from_millis(100)),
            uniform_scale,
            offset_x,
            offset_y,
            canonical_width: screen.width,
            canonical_height: screen.height,
            font_victor,
            font_jetbrains,
            font_digital7,
            frame_index: AtomicUsize::new(1),
            start_instant: Instant::now(),
        }))
    }

    pub fn update_interval(&self) -> Duration {
        self.update_interval
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

        // Pass 1: update sensor values + determine whether anything changed.
        let mut states = self.widget_states.lock();
        let mut any_dynamic_changed = force;
        for (widget, state) in self.template.widgets.iter().zip(states.iter_mut()) {
            if !widget.visible {
                continue;
            }
            if let Some(sensor) = &state.resolved_sensor {
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
                let (text, quantized) = format_sensor_readout(&widget.kind, raw);
                let changed = state.last_render_text.as_deref() != Some(text.as_str())
                    || state.last_quantized != quantized;
                if changed {
                    any_dynamic_changed = true;
                    state.last_render_text = Some(text);
                    state.last_quantized = quantized;
                }
            }
            // Video and CoreBars always "change" between ticks: their frame
            // index / per-core values drive re-renders independently of the
            // sensor dirty check.
            if matches!(
                widget.kind,
                WidgetKind::Video { .. } | WidgetKind::CoreBars { .. }
            ) {
                any_dynamic_changed = true;
            }
        }

        if !any_dynamic_changed {
            return Ok(None);
        }

        // Pass 2: composite dynamic widgets on top of the pre-rendered bg.
        let mut frame = self.template_image.clone();
        for (widget, state) in self.template.widgets.iter().zip(states.iter()) {
            if !widget.visible {
                continue;
            }
            if is_static_widget(&widget.kind) {
                continue;
            }
            let elapsed_ms = now
                .saturating_duration_since(self.start_instant)
                .as_millis() as u64;
            draw_widget(
                &mut frame,
                widget,
                state,
                self.uniform_scale,
                self.offset_x,
                self.offset_y,
                &self.font_victor,
                &self.font_jetbrains,
                &self.font_digital7,
                ElapsedMs(elapsed_ms),
            );
        }
        drop(states);

        // Apply template-level Portrait/Landscape orientation first (rotate
        // 90° CW for Landscape), then the per-device orientation on top.
        let mut rgb = DynamicImage::ImageRgba8(frame).to_rgb8();
        if matches!(self.template.orientation, TemplateOrientation::Landscape) {
            rgb = rotate90(&rgb);
        }
        let oriented = apply_orientation(rgb, self.orientation);
        let jpeg = encode_jpeg(oriented, &self.screen)?;

        Ok(Some(FrameInfo {
            data: jpeg,
            frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst),
        }))
    }
}

// ---------------------------------------------------------------------------
// Widget dispatch
// ---------------------------------------------------------------------------

fn is_static_widget(kind: &WidgetKind) -> bool {
    matches!(kind, WidgetKind::Label { .. } | WidgetKind::Image { .. })
}

fn widget_sensor_source(kind: &WidgetKind) -> Option<&SensorSourceConfig> {
    match kind {
        WidgetKind::ValueText { source, .. }
        | WidgetKind::RadialGauge { source, .. }
        | WidgetKind::VerticalBar { source, .. }
        | WidgetKind::HorizontalBar { source, .. }
        | WidgetKind::Speedometer { source, .. } => Some(source),
        _ => None,
    }
}

fn resolve_sensor_source(
    source: &SensorSourceConfig,
    all_sensors: &[SensorInfo],
) -> Option<ResolvedSensor> {
    if let SensorSourceConfig::Constant { value } = source {
        return Some(ResolvedSensor::Constant(*value));
    }
    let target = source.to_sensor_source();
    let divider = all_sensors
        .iter()
        .find(|s| s.source == target)
        .map(|s| s.divider)
        .unwrap_or(1);
    resolve_sensor(&target, divider)
}

/// Widget bounding-box size in pixels at the current composite scale.
fn widget_size_px(widget: &Widget, uniform_scale: f32) -> (u32, u32) {
    (
        (widget.width * uniform_scale).round().max(1.0) as u32,
        (widget.height * uniform_scale).round().max(1.0) as u32,
    )
}

/// Widget top-left position in pixels at the current composite scale,
/// offset by the letterbox origin. Widgets are positioned by center.
fn widget_origin_px(
    widget: &Widget,
    uniform_scale: f32,
    offset_x: i32,
    offset_y: i32,
) -> (i32, i32) {
    let (ww, wh) = widget_size_px(widget, uniform_scale);
    (
        offset_x + ((widget.x * uniform_scale).round() as i32) - (ww as i32) / 2,
        offset_y + ((widget.y * uniform_scale).round() as i32) - (wh as i32) / 2,
    )
}

/// Format a sensor value according to a ValueText widget's `format`/`unit`,
/// plus a quantized-1000ths key for the dirty check on gauge widgets.
fn format_sensor_readout(kind: &WidgetKind, raw: f32) -> (String, i32) {
    match kind {
        WidgetKind::ValueText { format, unit, .. } => {
            let text = render_value_format(format, raw);
            let quantized = (raw * 10.0).round() as i32;
            (format!("{text}{unit}"), quantized)
        }
        WidgetKind::RadialGauge {
            value_min,
            value_max,
            ..
        }
        | WidgetKind::VerticalBar {
            value_min,
            value_max,
            ..
        }
        | WidgetKind::HorizontalBar {
            value_min,
            value_max,
            ..
        }
        | WidgetKind::Speedometer {
            value_min,
            value_max,
            ..
        } => {
            let span = (value_max - value_min).abs().max(f32::EPSILON);
            let q = (((raw - value_min) / span) * 1000.0).round() as i32;
            (String::new(), q)
        }
        _ => (String::new(), 0),
    }
}

/// Minimal printf-lite for the value text format string. Supports:
/// - `{:.N}` — fixed decimals
/// - `{}` — default (0 decimals)
/// Anything else is passed through unchanged.
fn render_value_format(fmt: &str, value: f32) -> String {
    if let Some(rest) = fmt.strip_prefix("{:.") {
        if let Some(n_str) = rest.strip_suffix("}") {
            if let Ok(n) = n_str.parse::<usize>() {
                return format!("{:.*}", n, value);
            }
        }
    }
    if fmt == "{}" {
        return format!("{:.0}", value);
    }
    // Replace first occurrence of `{}` if embedded; otherwise fall back.
    if let Some(pos) = fmt.find("{}") {
        let mut out = String::with_capacity(fmt.len() + 8);
        out.push_str(&fmt[..pos]);
        out.push_str(&format!("{:.0}", value));
        out.push_str(&fmt[pos + 2..]);
        return out;
    }
    format!("{:.0}", value)
}

/// Fit a source image into a target box using the widget's fit mode.
fn fit_image(src: DynamicImage, target_w: u32, target_h: u32, fit: ImageFit) -> RgbaImage {
    match fit {
        ImageFit::Stretch => src
            .resize_exact(target_w.max(1), target_h.max(1), FilterType::Lanczos3)
            .to_rgba8(),
        ImageFit::Contain => {
            let resized = src.resize(target_w.max(1), target_h.max(1), FilterType::Lanczos3);
            let mut canvas =
                RgbaImage::from_pixel(target_w.max(1), target_h.max(1), Rgba([0, 0, 0, 0]));
            let rgba = resized.to_rgba8();
            let ox = ((target_w as i32) - (rgba.width() as i32)) / 2;
            let oy = ((target_h as i32) - (rgba.height() as i32)) / 2;
            imageops::overlay(&mut canvas, &rgba, ox as i64, oy as i64);
            canvas
        }
        ImageFit::Cover => {
            let resized =
                src.resize_to_fill(target_w.max(1), target_h.max(1), FilterType::Lanczos3);
            resized.to_rgba8()
        }
    }
}

fn resolve_font<'a>(
    font_ref: &FontRef,
    victor: &'a Font<'static>,
    jetbrains: &'a Font<'static>,
    digital7: &'a Font<'static>,
) -> &'a Font<'static> {
    match font_ref {
        FontRef::Builtin { font } => match font {
            BuiltinFont::VictorMono => victor,
            BuiltinFont::JetBrainsMono => jetbrains,
            BuiltinFont::Digital7 => digital7,
        },
        // Custom file fonts fall back to Victor Mono for now — loading them
        // per-frame would be expensive. Commit 4 will cache loaded file fonts
        // at editor save time.
        FontRef::File { .. } => victor,
    }
}

/// Render any widget into its own sub-buffer, then composite onto the main
/// frame with optional rotation. For widgets without rotation the fast path
/// is drawing directly onto `frame` at the final position.
#[allow(clippy::too_many_arguments)]
fn draw_widget(
    frame: &mut RgbaImage,
    widget: &Widget,
    state: &WidgetState,
    uniform_scale: f32,
    offset_x: i32,
    offset_y: i32,
    victor: &Font<'static>,
    jetbrains: &Font<'static>,
    digital7: &Font<'static>,
    elapsed_ms: ElapsedMs,
) {
    let (ww, wh) = widget_size_px(widget, uniform_scale);
    if ww == 0 || wh == 0 {
        return;
    }

    let needs_rotation = widget.rotation.abs() > 0.5;

    // Sub-buffer for the widget's own content. This keeps the geometry math
    // independent of the widget's position/rotation in the parent composite.
    let mut sub = RgbaImage::from_pixel(ww, wh, Rgba([0, 0, 0, 0]));

    match &widget.kind {
        WidgetKind::Label {
            text,
            font,
            font_size,
            color,
            align,
        } => {
            let f = resolve_font(font, victor, jetbrains, digital7);
            draw_text_widget(
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
            format: _,
            unit: _,
            font,
            font_size,
            color,
            align,
            ..
        } => {
            let f = resolve_font(font, victor, jetbrains, digital7);
            let text = state.last_render_text.clone().unwrap_or_default();
            if !text.is_empty() {
                draw_text_widget(
                    &mut sub,
                    &text,
                    f,
                    *font_size * uniform_scale,
                    *color,
                    *align,
                    ww,
                    wh,
                );
            }
        }
        WidgetKind::RadialGauge {
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            inner_radius_pct,
            background_color,
            ranges,
            ..
        } => {
            let raw = state
                .resolved_sensor
                .as_ref()
                .and_then(|s| read_sensor_value(s).ok())
                .unwrap_or(0.0);
            draw_radial_gauge(
                &mut sub,
                raw,
                *value_min,
                *value_max,
                *start_angle,
                *sweep_angle,
                *inner_radius_pct,
                *background_color,
                ranges,
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
            draw_bar(
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
            ..
        } => {
            let raw = state
                .resolved_sensor
                .as_ref()
                .and_then(|s| read_sensor_value(s).ok())
                .unwrap_or(0.0);
            draw_speedometer(
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
                uniform_scale,
            );
        }
        WidgetKind::CoreBars {
            orientation,
            color_cold,
            color_hot,
            background_color,
            show_labels,
        } => {
            draw_core_bars(
                &mut sub,
                *orientation,
                *color_cold,
                *color_hot,
                *background_color,
                *show_labels,
                uniform_scale,
                digital7,
            );
        }
        WidgetKind::Image { opacity, .. } => {
            if let Some(img) = &state.loaded_image {
                blit_with_opacity(&mut sub, img, *opacity);
            }
        }
        WidgetKind::Video { opacity, .. } => {
            if let Some(frames) = &state.video_frames {
                if !frames.is_empty() {
                    let dur_ms = state.video_frame_duration.as_millis().max(1) as u64;
                    let idx = ((elapsed_ms.0 / dur_ms) as usize) % frames.len();
                    blit_with_opacity(&mut sub, &frames[idx], *opacity);
                }
            }
        }
    }

    let (tl_x, tl_y) = widget_origin_px(widget, uniform_scale, offset_x, offset_y);

    if needs_rotation {
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

/// Small wrapper type so callers can't accidentally mix up elapsed-ms with
/// other numeric args in `draw_widget`. Constructed via `.into()` from u64.
#[derive(Copy, Clone)]
struct ElapsedMs(u64);

impl From<u64> for ElapsedMs {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

// ---------------------------------------------------------------------------
// Widget drawing helpers
// ---------------------------------------------------------------------------

fn draw_text_widget(
    sub: &mut RgbaImage,
    text: &str,
    font: &Font<'static>,
    size: f32,
    color: [u8; 3],
    align: TextAlign,
    ww: u32,
    wh: u32,
) {
    if text.is_empty() {
        return;
    }
    let scale = Scale::uniform(size.max(1.0));
    let (tw, th, ox, oy, _ascent) = get_exact_text_metrics(font, text, scale);
    if tw <= 0 || th <= 0 {
        return;
    }
    let x = match align {
        TextAlign::Left => 0,
        TextAlign::Center => ((ww as i32) - tw) / 2,
        TextAlign::Right => (ww as i32) - tw,
    } - ox;
    let y = ((wh as i32) - th) / 2 - oy;
    draw_text_mut(
        sub,
        Rgba([color[0], color[1], color[2], 255]),
        x,
        y,
        scale,
        font,
        text,
    );
}

fn range_color(ranges: &[SensorRange], unit_interval: f32) -> Rgba<u8> {
    if ranges.is_empty() {
        return Rgba([255, 255, 255, 255]);
    }
    let pct = unit_interval.clamp(0.0, 1.0) * 100.0;
    for r in ranges {
        if let Some(max) = r.max {
            if pct <= max {
                return Rgba([r.color[0], r.color[1], r.color[2], 255]);
            }
        } else {
            return Rgba([r.color[0], r.color[1], r.color[2], 255]);
        }
    }
    let last = ranges.last().unwrap();
    Rgba([last.color[0], last.color[1], last.color[2], 255])
}

fn unit_interval(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if span.abs() < f32::EPSILON {
        0.0
    } else {
        ((value - min) / span).clamp(0.0, 1.0)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_radial_gauge(
    sub: &mut RgbaImage,
    value: f32,
    value_min: f32,
    value_max: f32,
    start_angle: f32,
    sweep_angle: f32,
    inner_radius_pct: f32,
    background_color: [u8; 3],
    ranges: &[SensorRange],
) {
    let (w, h) = (sub.width() as f32, sub.height() as f32);
    let center = (w / 2.0, h / 2.0);
    let r_outer = (w.min(h) / 2.0).max(1.0);
    let r_inner = (r_outer * inner_radius_pct.clamp(0.0, 0.99)).max(1.0);

    let bg = Rgba([
        background_color[0],
        background_color[1],
        background_color[2],
        255,
    ]);
    draw_annulus(sub, center, r_inner, r_outer, start_angle, sweep_angle, bg);

    let u = unit_interval(value, value_min, value_max);
    let fill_sweep = sweep_angle * u;
    let color = range_color(ranges, u);
    if fill_sweep.abs() > 0.01 {
        draw_annulus(
            sub,
            center,
            r_inner,
            r_outer,
            start_angle,
            fill_sweep,
            color,
        );
    }
}

/// Fill an annular sector by scanning pixels inside the bounding box and
/// testing their radius + angle. Good-enough for widget-sized gauges without
/// the LUT machinery of `doublegauge.rs`.
fn draw_annulus(
    img: &mut RgbaImage,
    center: (f32, f32),
    r_in: f32,
    r_out: f32,
    start_deg: f32,
    sweep_deg: f32,
    color: Rgba<u8>,
) {
    let r_in_sq = r_in * r_in;
    let r_out_sq = r_out * r_out;
    let start_rad = start_deg.to_radians();
    let sweep_rad = sweep_deg.to_radians();
    let (w, h) = (img.width(), img.height());
    let xmin = (center.0 - r_out).floor().max(0.0) as u32;
    let xmax = ((center.0 + r_out).ceil() as u32).min(w);
    let ymin = (center.1 - r_out).floor().max(0.0) as u32;
    let ymax = ((center.1 + r_out).ceil() as u32).min(h);

    for y in ymin..ymax {
        for x in xmin..xmax {
            let dx = x as f32 - center.0;
            let dy = y as f32 - center.1;
            let d_sq = dx * dx + dy * dy;
            if d_sq < r_in_sq || d_sq > r_out_sq {
                continue;
            }
            let mut theta = dy.atan2(dx) - start_rad;
            // Normalize to [0, 2π)
            while theta < 0.0 {
                theta += 2.0 * PI;
            }
            while theta >= 2.0 * PI {
                theta -= 2.0 * PI;
            }
            let sweep_norm = if sweep_rad >= 0.0 {
                sweep_rad.min(2.0 * PI)
            } else {
                (2.0 * PI) + sweep_rad.max(-2.0 * PI)
            };
            if theta <= sweep_norm {
                img.put_pixel(x, y, color);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_bar(
    sub: &mut RgbaImage,
    value: f32,
    value_min: f32,
    value_max: f32,
    background_color: [u8; 3],
    _corner_radius: f32,
    ranges: &[SensorRange],
    is_vertical: bool,
) {
    let (w, h) = (sub.width(), sub.height());
    let bg = Rgba([
        background_color[0],
        background_color[1],
        background_color[2],
        255,
    ]);
    draw_filled_rect_mut(sub, Rect::at(0, 0).of_size(w, h), bg);

    let u = unit_interval(value, value_min, value_max);
    let color = range_color(ranges, u);
    if u <= 0.0 {
        return;
    }
    if is_vertical {
        let fill_h = ((h as f32) * u).round() as u32;
        if fill_h == 0 {
            return;
        }
        draw_filled_rect_mut(
            sub,
            Rect::at(0, (h - fill_h) as i32).of_size(w, fill_h),
            color,
        );
    } else {
        let fill_w = ((w as f32) * u).round() as u32;
        if fill_w == 0 {
            return;
        }
        draw_filled_rect_mut(sub, Rect::at(0, 0).of_size(fill_w, h), color);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_speedometer(
    sub: &mut RgbaImage,
    value: f32,
    value_min: f32,
    value_max: f32,
    start_angle: f32,
    sweep_angle: f32,
    needle_color: [u8; 3],
    tick_color: [u8; 3],
    tick_count: u32,
    background_color: [u8; 3],
    uniform_scale: f32,
) {
    let (w, h) = (sub.width() as f32, sub.height() as f32);
    let center = (w / 2.0, h / 2.0);
    let r_outer = (w.min(h) / 2.0).max(1.0);
    let r_inner = r_outer * 0.82;
    let bg = Rgba([
        background_color[0],
        background_color[1],
        background_color[2],
        255,
    ]);
    draw_annulus(sub, center, r_inner, r_outer, start_angle, sweep_angle, bg);

    let tick = Rgba([tick_color[0], tick_color[1], tick_color[2], 255]);
    if tick_count > 0 {
        for i in 0..=tick_count {
            let t = i as f32 / tick_count as f32;
            let angle = (start_angle + sweep_angle * t).to_radians();
            let (sx, sy) = (
                center.0 + r_inner * angle.cos(),
                center.1 + r_inner * angle.sin(),
            );
            let (ex, ey) = (
                center.0 + r_outer * angle.cos(),
                center.1 + r_outer * angle.sin(),
            );
            draw_antialiased_line_segment_mut(
                sub,
                (sx as i32, sy as i32),
                (ex as i32, ey as i32),
                tick,
                interpolate,
            );
        }
    }

    let u = unit_interval(value, value_min, value_max);
    let needle_angle = start_angle + sweep_angle * u;
    let needle = Rgba([needle_color[0], needle_color[1], needle_color[2], 255]);
    let start_len = 4.0 * uniform_scale;
    let length = r_inner - 4.0;
    let width = (6.0 * uniform_scale).max(2.0) as i32;
    draw_needle(sub, center, needle_angle, start_len, length, width, needle);
}

fn draw_needle(
    img: &mut RgbaImage,
    center: (f32, f32),
    angle_deg: f32,
    start_len: f32,
    length: f32,
    width: i32,
    color: Rgba<u8>,
) {
    let angle_rad = angle_deg.to_radians();
    let orth = angle_rad + PI / 2.0;
    let base = Point {
        x: center.0 + angle_rad.cos() * start_len,
        y: center.1 + angle_rad.sin() * start_len,
    };
    let tip = Point {
        x: center.0 + angle_rad.cos() * length,
        y: center.1 + angle_rad.sin() * length,
    };
    let half = (width / 2) as f32;
    let p1 = Point {
        x: base.x + orth.cos() * half,
        y: base.y + orth.sin() * half,
    };
    let p2 = Point {
        x: base.x - orth.cos() * half,
        y: base.y - orth.sin() * half,
    };
    let poly = vec![
        Point::new(p1.x as i32, p1.y as i32),
        Point::new(tip.x as i32, tip.y as i32),
        Point::new(p2.x as i32, p2.y as i32),
    ];
    draw_polygon_mut(img, &poly, color);
}

#[allow(clippy::too_many_arguments)]
fn draw_core_bars(
    sub: &mut RgbaImage,
    orientation: BarOrientation,
    color_cold: [u8; 3],
    color_hot: [u8; 3],
    background_color: [u8; 3],
    show_labels: bool,
    uniform_scale: f32,
    font_digital7: &Font<'static>,
) {
    let (w, h) = (sub.width(), sub.height());
    let bg = Rgba([
        background_color[0],
        background_color[1],
        background_color[2],
        255,
    ]);
    draw_filled_rect_mut(sub, Rect::at(0, 0).of_size(w, h), bg);

    let usage = SysSensor::get_core_usage();
    let num_cores = usage.len().max(1);

    match orientation {
        BarOrientation::Horizontal => {
            let total = w as usize;
            let size_per = (total / num_cores).max(1);
            let bar_w = size_per.saturating_sub(2).max(1) as u32;
            for (i, &u10k) in usage.iter().enumerate() {
                let u = (u10k.min(10_000) as f32) / 10_000.0;
                let color = mix_colors(color_cold, color_hot, u);
                let bar_h = ((h as f32) * u).round() as u32;
                let x = (i * size_per) as i32;
                if bar_h > 0 {
                    draw_filled_rect_mut(
                        sub,
                        Rect::at(x, (h - bar_h) as i32).of_size(bar_w, bar_h),
                        color,
                    );
                }
                if show_labels {
                    let label = format!("{}", (i + 1) % 10);
                    draw_text_mut(
                        sub,
                        Rgba([230, 238, 246, 255]),
                        x + 2,
                        (h as i32) - (10.0 * uniform_scale) as i32,
                        Scale::uniform(9.0 * uniform_scale),
                        font_digital7,
                        &label,
                    );
                }
            }
        }
        BarOrientation::Vertical => {
            let total = h as usize;
            let size_per = (total / num_cores).max(1);
            let bar_h = size_per.saturating_sub(2).max(1) as u32;
            for (i, &u10k) in usage.iter().enumerate() {
                let u = (u10k.min(10_000) as f32) / 10_000.0;
                let color = mix_colors(color_cold, color_hot, u);
                let bar_w = ((w as f32) * u).round() as u32;
                let y = (i * size_per) as i32;
                if bar_w > 0 {
                    draw_filled_rect_mut(sub, Rect::at(0, y).of_size(bar_w, bar_h), color);
                }
                if show_labels {
                    let label = format!("{}", (i + 1) % 10);
                    draw_text_mut(
                        sub,
                        Rgba([230, 238, 246, 255]),
                        2,
                        y,
                        Scale::uniform(9.0 * uniform_scale),
                        font_digital7,
                        &label,
                    );
                }
            }
        }
    }
}

fn mix_colors(cold: [u8; 3], hot: [u8; 3], t: f32) -> Rgba<u8> {
    let t = t.clamp(0.0, 1.0);
    let r = (cold[0] as f32 * (1.0 - t) + hot[0] as f32 * t).round() as u8;
    let g = (cold[1] as f32 * (1.0 - t) + hot[1] as f32 * t).round() as u8;
    let b = (cold[2] as f32 * (1.0 - t) + hot[2] as f32 * t).round() as u8;
    Rgba([r, g, b, 255])
}

fn blit_with_opacity(dst: &mut RgbaImage, src: &RgbaImage, opacity: f32) {
    let o = opacity.clamp(0.0, 1.0);
    if o >= 0.999 && src.width() == dst.width() && src.height() == dst.height() {
        imageops::overlay(dst, src, 0, 0);
        return;
    }
    let (dw, dh) = (dst.width(), dst.height());
    let (sw, sh) = (src.width(), src.height());
    let w = sw.min(dw);
    let h = sh.min(dh);
    for y in 0..h {
        for x in 0..w {
            let s = src.get_pixel(x, y);
            let d = dst.get_pixel_mut(x, y);
            let a = (s[3] as f32 / 255.0) * o;
            d[0] = (d[0] as f32 * (1.0 - a) + s[0] as f32 * a).round() as u8;
            d[1] = (d[1] as f32 * (1.0 - a) + s[1] as f32 * a).round() as u8;
            d[2] = (d[2] as f32 * (1.0 - a) + s[2] as f32 * a).round() as u8;
        }
    }
}

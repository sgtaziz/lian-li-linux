mod bitmap_glyphs;
mod gauge;
mod text;

use crate::common::{apply_orientation, encode_jpeg, render_dimensions, MediaError};
use ab_glyph::FontVec;
use gauge::{draw_gauge, GaugeParams};
use image::{ImageBuffer, Rgb, RgbImage, RgbaImage};
use lianli_shared::media::{SensorDescriptor, SensorRange, SensorSourceConfig};
use lianli_shared::screen::ScreenInfo;
use lianli_shared::sensors::SensorInfo;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use text::{draw_sensor_text_fallback, draw_sensor_text_ttf, TextRenderParams};
use tracing::warn;

pub struct FrameInfo {
    pub data: Vec<u8>,
    pub frame_index: usize,
}

#[derive(Debug)]
pub struct SensorAsset {
    label: String,
    unit: String,
    orientation: f32,
    text_color: [u8; 3],
    background_color: [u8; 3],
    gauge_background_color: [u8; 3],
    ranges: Vec<(Option<f32>, [u8; 3])>,
    source: SensorSource,
    update_interval: Duration,
    gauge_start_angle: f32,
    gauge_sweep_angle: f32,
    gauge_outer_radius: f32,
    gauge_thickness: f32,
    bar_corner_radius: f32,
    value_font_size: f32,
    unit_font_size: f32,
    label_font_size: f32,
    font: Option<FontVec>,
    template_image: Option<Arc<RgbImage>>,
    decimal_places: u8,
    value_offset: i32,
    unit_offset: i32,
    label_offset: i32,
    screen: ScreenInfo,
    render_width: u32,
    render_height: u32,
    previous_value: Mutex<String>,
    frame_index: AtomicUsize,
}

impl SensorAsset {
    pub fn new(
        descriptor: &SensorDescriptor,
        orientation: f32,
        screen: &ScreenInfo,
        sensors: &[SensorInfo],
        background_image: Option<&Path>,
        update_interval_ms: u64,
    ) -> Result<Arc<Self>, MediaError> {
        let mut ranges = descriptor.gauge_ranges.clone();
        if ranges.is_empty() {
            ranges = vec![
                SensorRange {
                    max: Some(50.0),
                    color: [0, 200, 0],
                    alpha: 255,
                },
                SensorRange {
                    max: Some(80.0),
                    color: [220, 140, 0],
                    alpha: 255,
                },
                SensorRange {
                    max: None,
                    color: [220, 0, 0],
                    alpha: 255,
                },
            ];
        }
        ranges.sort_by(|a, b| match (a.max, b.max) {
            (Some(a_val), Some(b_val)) => a_val
                .partial_cmp(&b_val)
                .unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });

        let (rw, rh) = render_dimensions(screen, orientation);

        let template_image: Option<Arc<RgbImage>> = background_image
            .filter(|path| !path.as_os_str().is_empty())
            .and_then(|path| match ::image::open(path) {
                Ok(img) => {
                    let resized = img
                        .resize_exact(rw, rh, ::image::imageops::FilterType::Lanczos3)
                        .to_rgb8();
                    Some(Arc::new(resized))
                }
                Err(e) => {
                    warn!(
                        "Failed to load sensor background image '{}': {e}",
                        path.display()
                    );
                    None
                }
            });

        if ranges.last().and_then(|r| r.max).is_some() {
            if let Some(last) = ranges.last().cloned() {
                ranges.push(SensorRange {
                    max: None,
                    color: last.color,
                    alpha: last.alpha,
                });
            }
        }

        let ranges = ranges.into_iter().map(|r| (r.max, r.color)).collect();

        let source = match &descriptor.source {
            SensorSourceConfig::Constant { value } => {
                SensorSource::Constant(value.clamp(0.0, 100.0))
            }
            SensorSourceConfig::Command { .. }
            | SensorSourceConfig::Hwmon { .. }
            | SensorSourceConfig::NvidiaGpu { .. }
            | SensorSourceConfig::AmdGpuUsage { .. }
            | SensorSourceConfig::WirelessCoolant { .. }
            | SensorSourceConfig::CpuUsage
            | SensorSourceConfig::MemUsage
            | SensorSourceConfig::MemUsed
            | SensorSourceConfig::MemFree
            | SensorSourceConfig::NetworkRx { .. }
            | SensorSourceConfig::NetworkTx { .. }
            | SensorSourceConfig::DiskRead { .. }
            | SensorSourceConfig::DiskWrite { .. } => {
                let sensor_source = descriptor.source.to_sensor_source();
                let sensor_info = sensors.iter().find(|s| s.source == sensor_source);
                let divider = sensor_info.map_or(1, |s| s.divider);
                match lianli_shared::sensors::resolve_sensor(&sensor_source, divider) {
                    Some(resolved) => SensorSource::Resolved(resolved),
                    None => return Err(MediaError::Sensor("sensor not found on system".into())),
                }
            }
        };

        let font_path = descriptor
            .font_path
            .clone()
            .or_else(lianli_shared::fonts::default_font_path);
        let font = if let Some(path) = font_path {
            let font_data = std::fs::read(&path)
                .map_err(|e| MediaError::Sensor(format!("Failed to read font file: {e}")))?;
            Some(
                FontVec::try_from_vec(font_data)
                    .map_err(|e| MediaError::Sensor(format!("Failed to parse font file: {e}")))?,
            )
        } else {
            None
        };

        let update_interval = Duration::from_millis(update_interval_ms.clamp(100, 10_000));
        let max_radius = ((rw.min(rh) as f32 / 2.0) - 6.0).max(20.0);
        let gauge_outer_radius = descriptor.gauge_outer_radius.clamp(20.0, max_radius);
        let gauge_thickness = descriptor
            .gauge_thickness
            .clamp(5.0, gauge_outer_radius - 5.0);
        let gauge_start_angle = (descriptor.gauge_start_angle % 360.0 + 360.0) % 360.0;
        let gauge_sweep_angle = descriptor.gauge_sweep_angle.clamp(10.0, 360.0);
        let bar_corner_radius = descriptor.bar_corner_radius.max(0.0);

        Ok(Arc::new(Self {
            label: descriptor.label.clone(),
            unit: descriptor.unit.clone(),
            orientation,
            text_color: descriptor.text_color,
            background_color: descriptor.background_color,
            gauge_background_color: descriptor.gauge_background_color,
            ranges,
            source,
            update_interval,
            gauge_start_angle,
            gauge_sweep_angle,
            gauge_outer_radius,
            gauge_thickness,
            bar_corner_radius,
            value_font_size: descriptor.value_font_size,
            unit_font_size: descriptor.unit_font_size,
            label_font_size: descriptor.label_font_size,
            font,
            template_image,
            decimal_places: descriptor.decimal_places,
            value_offset: descriptor.value_offset,
            unit_offset: descriptor.unit_offset,
            label_offset: descriptor.label_offset,
            screen: *screen,
            render_width: rw,
            render_height: rh,
            previous_value: Mutex::new("N/A".into()),
            frame_index: 1.into(),
        }))
    }

    pub fn update_interval(&self) -> Duration {
        self.update_interval
    }

    /// Render the next frame. Skips encoding when the value text matches the
    /// previous frame and `force` is false. Returns `Ok(None)` when skipped.
    pub fn render_frame(&self, force: bool) -> Result<Option<FrameInfo>, MediaError> {
        let value = self.read_value()?.clamp(0.0, 100.0);

        let value_text = if self.decimal_places > 0 {
            format!("{:.prec$}", value, prec = self.decimal_places as usize)
        } else {
            format!("{:.0}", value.round())
        };

        let mut prev = self.previous_value.lock();
        if value_text == *prev && !force {
            return Ok(None);
        }

        let gauge_color = self.color_for_value(value);
        let w = self.render_width;
        let h = self.render_height;

        let mut image = match &self.template_image {
            Some(tpl) => (**tpl).clone(),
            None => ImageBuffer::from_pixel(w, h, Rgb(self.background_color)),
        };

        draw_gauge(
            &mut image,
            w,
            h,
            GaugeParams {
                value,
                gauge_color,
                ring_color: self.gauge_background_color,
                outer_radius: self.gauge_outer_radius,
                thickness: self.gauge_thickness,
                start_angle: self.gauge_start_angle,
                sweep_angle: self.gauge_sweep_angle,
                corner_radius: self.bar_corner_radius,
            },
        );

        let text_params = TextRenderParams {
            label: &self.label,
            unit: &self.unit,
            color: self.text_color,
            value_size: self.value_font_size,
            unit_size: self.unit_font_size,
            label_size: self.label_font_size,
            value_offset: self.value_offset,
            unit_offset: self.unit_offset,
            label_offset: self.label_offset,
            value_text: &value_text,
        };

        if let Some(font) = &self.font {
            draw_sensor_text_ttf(&mut image, w, h, text_params, font);
        } else {
            draw_sensor_text_fallback(&mut image, w, h, text_params);
        }

        *prev = value_text;

        let oriented = apply_orientation(image, self.orientation);
        let data = encode_jpeg(oriented, &self.screen)?;
        Ok(Some(FrameInfo {
            data,
            frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst),
        }))
    }

    pub fn render_frame_rgba(&self, force: bool) -> Result<Option<RgbaImage>, MediaError> {
        let value = self.read_value()?.clamp(0.0, 100.0);

        let value_text = if self.decimal_places > 0 {
            format!("{:.prec$}", value, prec = self.decimal_places as usize)
        } else {
            format!("{:.0}", value.round())
        };

        let mut prev = self.previous_value.lock();
        if value_text == *prev && !force {
            return Ok(None);
        }

        let gauge_color = self.color_for_value(value);
        let w = self.render_width;
        let h = self.render_height;

        let mut image = match &self.template_image {
            Some(tpl) => (**tpl).clone(),
            None => ImageBuffer::from_pixel(w, h, Rgb(self.background_color)),
        };

        draw_gauge(
            &mut image,
            w,
            h,
            GaugeParams {
                value,
                gauge_color,
                ring_color: self.gauge_background_color,
                outer_radius: self.gauge_outer_radius,
                thickness: self.gauge_thickness,
                start_angle: self.gauge_start_angle,
                sweep_angle: self.gauge_sweep_angle,
                corner_radius: self.bar_corner_radius,
            },
        );

        let text_params = TextRenderParams {
            label: &self.label,
            unit: &self.unit,
            color: self.text_color,
            value_size: self.value_font_size,
            unit_size: self.unit_font_size,
            label_size: self.label_font_size,
            value_offset: self.value_offset,
            unit_offset: self.unit_offset,
            label_offset: self.label_offset,
            value_text: &value_text,
        };

        if let Some(font) = &self.font {
            draw_sensor_text_ttf(&mut image, w, h, text_params, font);
        } else {
            draw_sensor_text_fallback(&mut image, w, h, text_params);
        }

        *prev = value_text;

        let oriented = apply_orientation(image, self.orientation);
        let (ow, oh) = oriented.dimensions();
        let mut rgba = RgbaImage::new(ow, oh);
        for (src, dst) in oriented.pixels().zip(rgba.pixels_mut()) {
            *dst = image::Rgba([src[0], src[1], src[2], 255]);
        }
        self.frame_index.fetch_add(1, Ordering::SeqCst);
        Ok(Some(rgba))
    }

    pub fn blank_frame(&self) -> FrameInfo {
        let image = match &self.template_image {
            Some(tpl) => (**tpl).clone(),
            None => ImageBuffer::from_pixel(
                self.render_width,
                self.render_height,
                Rgb(self.background_color),
            ),
        };
        let oriented = apply_orientation(image, self.orientation);
        FrameInfo {
            data: encode_jpeg(oriented, &self.screen).unwrap_or_default(),
            frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst),
        }
    }

    fn color_for_value(&self, value: f32) -> [u8; 3] {
        for (max, color) in &self.ranges {
            if max.map(|m| value <= m).unwrap_or(true) {
                return *color;
            }
        }
        self.ranges.last().map(|(_, c)| *c).unwrap_or([0, 200, 0])
    }

    fn read_value(&self) -> Result<f32, MediaError> {
        match &self.source {
            SensorSource::Constant(value) => Ok(*value),
            SensorSource::Resolved(resolved) => lianli_shared::sensors::read_sensor_value(resolved)
                .map_err(|e| MediaError::Sensor(e.to_string())),
        }
    }
}

#[derive(Debug)]
enum SensorSource {
    Constant(f32),
    Resolved(lianli_shared::sensors::ResolvedSensor),
}

use super::common::{apply_orientation, encode_jpeg, render_dimensions, MediaError};
use lianli_shared::media::{SensorDescriptor, SensorRange, SensorSourceConfig};
use lianli_shared::screen::ScreenInfo;
use image::{ImageBuffer, Rgb, RgbImage};
use lianli_shared::sensors::SensorInfo;
use rusttype::{point, Font, Scale};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use parking_lot::Mutex;

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
    font: Option<Font<'static>>,
    decimal_places: u8,
    value_offset: i32,
    unit_offset: i32,
    label_offset: i32,
    screen: ScreenInfo,
    render_width: u32,
    render_height: u32,
    previous_value: Mutex<String>,
    // Each time a frame gets redrawn this index is "assigned" to the frame.
    frame_index: AtomicUsize,
}

impl SensorAsset {
    pub fn new(
        descriptor: &SensorDescriptor,
        orientation: f32,
        screen: &ScreenInfo,
        sensors: &[SensorInfo],
    ) -> Result<Arc<Self>, MediaError> {
        let mut ranges = descriptor.gauge_ranges.clone();
        if ranges.is_empty() {
            ranges = vec![
                SensorRange { max: Some(50.0), color: [0, 200, 0] },
                SensorRange { max: Some(80.0), color: [220, 140, 0] },
                SensorRange { max: None, color: [220, 0, 0] },
            ];
        }
        ranges.sort_by(|a, b| match (a.max, b.max) {
            (Some(a_val), Some(b_val)) => a_val.partial_cmp(&b_val).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });

        if ranges.last().and_then(|r| r.max).is_some() {
            if let Some(last) = ranges.last().cloned() {
                ranges.push(SensorRange { max: None, color: last.color });
            }
        }

        let ranges = ranges.into_iter().map(|r| (r.max, r.color)).collect();

        let source = match &descriptor.source {
            SensorSourceConfig::Constant { value } => SensorSource::Constant(value.clamp(0.0, 100.0)),
            SensorSourceConfig::Command { .. }
            | SensorSourceConfig::Hwmon { .. }
            | SensorSourceConfig::NvidiaGpu { .. }
            | SensorSourceConfig::WirelessCoolant { .. }
            | SensorSourceConfig::CpuUsage
            | SensorSourceConfig::MemUsage
            | SensorSourceConfig::MemUsed
            | SensorSourceConfig::MemFree => {
                let sensor_source = descriptor.source.to_sensor_source();
                let sensor_info = sensors.iter().find(|s| s.source == sensor_source);
                let divider = sensor_info.map_or(1, |s| s.divider);
                match lianli_shared::sensors::resolve_sensor(&sensor_source, divider) {
                    Some(resolved) => SensorSource::Resolved(resolved),
                    None => return Err(MediaError::Sensor("sensor not found on system".into())),
                }
            }
        };

        let font = if let Some(font_path) = &descriptor.font_path {
            let font_data = std::fs::read(font_path)
                .map_err(|e| MediaError::Sensor(format!("Failed to read font file: {e}")))?;
            let font = Font::try_from_vec(font_data)
                .ok_or_else(|| MediaError::Sensor("Failed to parse font file".to_string()))?;
            Some(font)
        } else {
            None
        };

        let update_interval = Duration::from_millis(descriptor.update_interval_ms.max(100));
        let (rw, rh) = render_dimensions(screen, orientation);
        let max_radius = (rw.min(rh) as f32 / 2.0) - 6.0;
        let gauge_outer_radius = descriptor.gauge_outer_radius.clamp(20.0, max_radius);
        let gauge_thickness = descriptor.gauge_thickness.clamp(5.0, gauge_outer_radius - 5.0);
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

    /// Force flag: if true, frame gets rendered even if value has not changed. For example when we render the first frame, we set force=true
    /// Returns OK(Empty) in case of "nothing changed", OK(FrameInfo) in case a new frame has been rendered, and Error in case of an error
    pub fn render_frame(&self, force: bool) -> Result<Option<FrameInfo>, MediaError> {

        // First of all, let's check whether we need to render a new frame:
        // if the value to display has not changed, we omit frame rendering

        let value = self.read_value()?.clamp(0.0, 100.0);

        // So let's find out how the value will appear as a string
        let value_text = if self.decimal_places > 0 {
            format!("{:.prec$}", value, prec = self.decimal_places as usize)
        } else {
            format!("{:.0}", value.round())
        };

        // And let's compare it with the previously stored value
        let mut prev = self.previous_value.lock();
        if value_text == *prev  && !force {
            return Ok(None);
        }

        let gauge_color = self.color_for_value(value);
        let w = self.render_width;
        let h = self.render_height;

        let mut image = ImageBuffer::from_pixel(w, h, Rgb(self.background_color));

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

        *prev = value_text; // set the previous value

        let oriented = apply_orientation(image, self.orientation);
        let encoded_jpeg_result: Result<Option<Vec<u8>>, MediaError> = encode_jpeg(oriented, &self.screen).map(Some);

        let frame_result: Result<Option<FrameInfo>, MediaError> = encoded_jpeg_result.map(|opt| {
            opt.map(|data| FrameInfo {
                data,
                frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst),
            })
        });
        return frame_result;
    }

    pub fn blank_frame(&self) -> FrameInfo {
        let image = ImageBuffer::from_pixel(
            self.render_width,
            self.render_height,
            Rgb(self.background_color),
        );
        let oriented = apply_orientation(image, self.orientation);
        let frame_ret = FrameInfo{data: encode_jpeg(oriented, &self.screen).unwrap_or_default(), 
                        frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst) };

        return frame_ret;
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
            SensorSource::Resolved(resolved) => {
                lianli_shared::sensors::read_sensor_value(resolved)
                    .map_err(|e| MediaError::Sensor(e.to_string()))
            }
        }
    }
}

#[derive(Debug)]
enum SensorSource {
    Constant(f32),
    Resolved(lianli_shared::sensors::ResolvedSensor),
}

struct GaugeParams {
    value: f32,
    gauge_color: [u8; 3],
    ring_color: [u8; 3],
    outer_radius: f32,
    thickness: f32,
    start_angle: f32,
    sweep_angle: f32,
    corner_radius: f32,
}

fn draw_gauge(image: &mut RgbImage, width: u32, height: u32, params: GaugeParams) {
    let GaugeParams {
        value, gauge_color, ring_color, outer_radius, thickness,
        start_angle, sweep_angle, corner_radius,
    } = params;
    let cx = (width as f32 - 1.0) / 2.0;
    let cy = (height as f32 - 1.0) / 2.0;
    let max_radius = (width.min(height) as f32 / 2.0) - 4.0;
    let outer = outer_radius.clamp(20.0, max_radius);
    let inner = (outer - thickness.clamp(5.0, outer - 5.0)).max(outer * 0.1);
    let start = (start_angle % 360.0 + 360.0) % 360.0;
    let sweep = sweep_angle.clamp(10.0, 360.0);
    let fill_angle = sweep * (value.clamp(0.0, 100.0) / 100.0);

    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 - cx;
            let dy = cy - y as f32;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist <= outer && dist >= inner {
                let angle = dy.atan2(dx).to_degrees();
                let diff = (start - angle + 360.0) % 360.0;

                if diff <= sweep {
                    let base_color = if diff <= fill_angle {
                        gauge_color
                    } else {
                        ring_color
                    };

                    if corner_radius > 0.0 && diff <= fill_angle && fill_angle > 0.0 {
                        let radial_mid = (inner + outer) / 2.0;
                        let arc_dist_from_start = diff * std::f32::consts::PI / 180.0 * radial_mid;
                        let arc_dist_from_end =
                            (fill_angle - diff) * std::f32::consts::PI / 180.0 * radial_mid;

                        let near_start = arc_dist_from_start < corner_radius;
                        let near_end = arc_dist_from_end < corner_radius;

                        if near_start || near_end {
                            let half_thickness = thickness / 2.0;
                            let bar_center_radius = (inner + outer) / 2.0;
                            let offset_from_center = dist - bar_center_radius;
                            let near_edge =
                                offset_from_center.abs() > half_thickness - corner_radius;

                            if near_edge {
                                let arc_dist = if near_start {
                                    arc_dist_from_start
                                } else {
                                    arc_dist_from_end
                                };

                                if arc_dist < corner_radius {
                                    let x_from_corner = corner_radius - arc_dist;
                                    let y_from_corner = if offset_from_center > 0.0 {
                                        offset_from_center - (half_thickness - corner_radius)
                                    } else {
                                        offset_from_center + (half_thickness - corner_radius)
                                    };
                                    let corner_dist = (x_from_corner * x_from_corner
                                        + y_from_corner * y_from_corner)
                                        .sqrt();
                                    if corner_dist > corner_radius {
                                        image.put_pixel(x, y, Rgb(ring_color));
                                        continue;
                                    } else if corner_dist > corner_radius - 1.0 {
                                        let alpha = (corner_radius - corner_dist).clamp(0.0, 1.0);
                                        let blended = [
                                            (base_color[0] as f32 * alpha + ring_color[0] as f32 * (1.0 - alpha)) as u8,
                                            (base_color[1] as f32 * alpha + ring_color[1] as f32 * (1.0 - alpha)) as u8,
                                            (base_color[2] as f32 * alpha + ring_color[2] as f32 * (1.0 - alpha)) as u8,
                                        ];
                                        image.put_pixel(x, y, Rgb(blended));
                                        continue;
                                    }
                                }
                            }
                        }
                    }

                    image.put_pixel(x, y, Rgb(base_color));
                }
            }
        }
    }
}

struct TextRenderParams<'a> {
    label: &'a str,
    unit: &'a str,
    color: [u8; 3],
    value_size: f32,
    unit_size: f32,
    label_size: f32,
    value_offset: i32,
    unit_offset: i32,
    label_offset: i32,
    value_text: &'a str,
}

fn draw_sensor_text_ttf(
    image: &mut RgbImage,
    width: u32,
    height: u32,
    params: TextRenderParams,
    font: &Font,
) {
    draw_text_centered(image, width, height, params.value_text, params.value_size, params.color, params.value_offset, font);
    draw_text_centered(image, width, height, params.unit, params.unit_size, params.color, params.unit_offset, font);
    draw_text_centered(image, width, height, params.label, params.label_size, params.color, params.label_offset, font);
}

fn draw_text_centered(
    image: &mut RgbImage,
    width: u32,
    height: u32,
    text: &str,
    size: f32,
    color: [u8; 3],
    offset_y: i32,
    font: &Font,
) {
    if size <= 0.0 || text.is_empty() {
        return;
    }

    let scale = Scale::uniform(size);
    let v_metrics = font.v_metrics(scale);

    let glyphs: Vec<_> = font
        .layout(text, scale, point(0.0, v_metrics.ascent))
        .collect();

    let text_width = glyphs
        .iter()
        .rev()
        .filter_map(|g| {
            g.pixel_bounding_box()
                .map(|b| b.min.x as f32 + g.unpositioned().h_metrics().advance_width)
        })
        .next()
        .unwrap_or(0.0);

    let start_x = ((width as f32 - text_width) / 2.0) as i32;
    let start_y = (height as i32 / 2) + offset_y;

    for glyph in glyphs {
        if let Some(bounding_box) = glyph.pixel_bounding_box() {
            glyph.draw(|gx, gy, gv| {
                let x = start_x + bounding_box.min.x + gx as i32;
                let y = start_y + bounding_box.min.y + gy as i32;
                if x >= 0 && x < width as i32 && y >= 0 && y < height as i32 {
                    let px = image.get_pixel_mut(x as u32, y as u32);
                    let alpha = gv;
                    px.0[0] = ((color[0] as f32 * alpha) + (px.0[0] as f32 * (1.0 - alpha))) as u8;
                    px.0[1] = ((color[1] as f32 * alpha) + (px.0[1] as f32 * (1.0 - alpha))) as u8;
                    px.0[2] = ((color[2] as f32 * alpha) + (px.0[2] as f32 * (1.0 - alpha))) as u8;
                }
            });
        }
    }
}

fn draw_sensor_text_fallback(
    image: &mut RgbImage,
    width: u32,
    height: u32,
    params: TextRenderParams,
) {
    let value_scale = (params.value_size / 4.0).max(4.0) as u32;
    let unit_scale = (params.unit_size / 4.0).max(3.0) as u32;
    let label_scale = (params.label_size / 4.0).max(3.0) as u32;

    draw_text_center_bitmap(image, width, height, params.value_text, value_scale, params.color, params.value_offset);
    draw_text_center_bitmap(image, width, height, params.unit, unit_scale, params.color, params.unit_offset);
    draw_text_center_bitmap(image, width, height, params.label, label_scale, params.color, params.label_offset);
}

fn draw_text_center_bitmap(
    image: &mut RgbImage,
    width: u32,
    height: u32,
    text: &str,
    scale: u32,
    color: [u8; 3],
    offset_y: i32,
) {
    if scale == 0 {
        return;
    }
    let glyphs: Vec<[u8; 7]> = text.chars().map(glyph_pattern).collect();
    if glyphs.is_empty() {
        return;
    }
    let glyph_width = 5 * scale;
    let spacing = scale;
    let total_width = (glyphs.len() as u32 * (glyph_width + spacing) - spacing).min(width); // total_width must not be greater than width
    let start_x = ((width - total_width) / 2) as i32;
    let start_y = ((height as i32) / 2) + offset_y - ((7 * scale) as i32 / 2);

    for (i, bitmap) in glyphs.iter().enumerate() {
        let base_x = start_x + i as i32 * (glyph_width as i32 + spacing as i32);
        draw_bitmap_character(image, width, height, base_x, start_y, *bitmap, scale, color);
    }
}

fn draw_bitmap_character(
    image: &mut RgbImage,
    width: u32,
    height: u32,
    base_x: i32,
    base_y: i32,
    bitmap: [u8; 7],
    scale: u32,
    color: [u8; 3],
) {
    for (row, mask) in bitmap.iter().enumerate() {
        for col in 0..5 {
            if (mask >> (4 - col)) & 1 == 1 {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let x = base_x + (col * scale) as i32 + dx as i32;
                        let y = base_y + (row as i32 * scale as i32) + dy as i32;
                        if x >= 0 && x < width as i32 && y >= 0 && y < height as i32 {
                            image.put_pixel(x as u32, y as u32, Rgb(color));
                        }
                    }
                }
            }
        }
    }
}

fn glyph_pattern(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b01110, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b11110, 0b10001, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111],
        'H' => [0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00001, 0b00001, 0b00001, 0b00001, 0b10001, 0b10001, 0b01110],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '%' => [0b11001, 0b11010, 0b00100, 0b01000, 0b10011, 0b01011, 0b00000],
        '°' => [0b01100, 0b10010, 0b10010, 0b01100, 0b00000, 0b00000, 0b00000],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        '_' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b11111],
        ':' => [0b00000, 0b00100, 0b00100, 0b00000, 0b00100, 0b00100, 0b00000],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100],
        ' ' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000],
        _ => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000],
    }
}

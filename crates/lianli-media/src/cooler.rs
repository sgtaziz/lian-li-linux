// Heads-up before you touch anything in here:
//
// Every coordinate in this file is tuned for a 480x480 panel and the bundled
// cooler.png. Scales are screen.dim / 480, label and gauge positions are
// baked-in pixel offsets, and the layout will stretch (not letterbox) on
// non-square panels. Swapping in a background of a different size will land
// widgets in the wrong place — when we let users pick their own backgrounds we
// will need the layout coords to come from the asset, not from this file.
//
// The cooler.png, thermometer.png, and the bundled fonts are also pulled in
// with include_bytes!, which adds ~700 KB to the daemon binary. Same redesign
// that makes backgrounds configurable should move these onto disk too.

use super::common::{
    apply_orientation, encode_jpeg, get_exact_text_metrics, hsl_to_rgb, MediaError,
    FONT_DATA_DIGITAL_7, FONT_DATA_LABEL,
};
use crate::sensor::FrameInfo;
use image::imageops;
use image::imageops::FilterType;
use image::{DynamicImage, ImageBuffer, RgbaImage};
use image::{Rgb, RgbImage, Rgba};
use imageproc::drawing::{
    draw_antialiased_line_segment_mut, draw_filled_circle_mut, draw_filled_rect_mut,
    draw_polygon_mut, draw_text_mut,
};
use imageproc::pixelops::interpolate;
use imageproc::point::Point;
use imageproc::rect::Rect;
use lianli_shared::media::DoublegaugeDescriptor;
use lianli_shared::screen::ScreenInfo;
use lianli_shared::sensors::ResolvedSensor;
use lianli_shared::systeminfo::SysSensor;
use parking_lot::Mutex;
use rusttype::{Font, Scale};
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;


#[derive(Debug, Default)]
pub struct CpuData {
    pub previous_left_text: Option<String>,
    pub previous_right_text: Option<String>,
    // Quantized 0..1000 so the dirty check still fires when the needle/bar
    // moves below the precision of the formatted text.
    pub previous_left_angle_q: i32,
    pub previous_right_angle_q: i32,
    pub previous_usage_per_core: Vec<u32>,
}

#[derive(Debug)]
pub struct CoolerAsset {
    unit1: String,  // Normally %
    unit2: String,  // normally °C
    
    pub gauge_1_min: i32,
    pub gauge_1_max: i32,
    pub value_1_min: i32,
    pub value_1_max: i32,
    pub display_value_1_min: i32,
    pub display_value_1_max: i32,
    pub clamp_1: bool,
    pub decimals_1: usize,

    pub gauge_2_min: i32,
    pub gauge_2_max: i32,
    pub value_2_min: i32,
    pub value_2_max: i32,
    pub display_value_2_min: i32,
    pub display_value_2_max: i32,
    pub clamp_2: bool,
    pub decimals_2: usize,

    pub sensor_1: ResolvedSensor,
    pub sensor_2: ResolvedSensor,

    /// Update interval in ms
    update_interval: Duration,
    orientation: f32,
    sys_data: Mutex<CpuData>,
    screen: ScreenInfo,
    template_image: image::RgbaImage, // Pre-rendered background image

    font_label: Font<'static>,

    sensor_1_failed: AtomicBool,
    sensor_2_failed: AtomicBool,

    // Each time a frame gets redrawn this index is "assigned" to the frame.
    frame_index: AtomicUsize,
}

impl CoolerAsset {
    pub fn new(
        descriptor: &DoublegaugeDescriptor,
        orientation: f32,
        screen: &ScreenInfo, /*,metric: Option<MetricEntry>*/
        sensor_1: ResolvedSensor,
        sensor_2: ResolvedSensor,
    ) -> Result<Arc<Self>, MediaError> {
        let update_interval = Duration::from_millis(100);

        let label1 = &descriptor.label_1;
        let label2 = &descriptor.label_2;

        let label3 = "CPU CORES";
        let unit1 = &descriptor.unit_1;
        let unit2 = &descriptor.unit_2;

        // Load image once during creation
        let data = include_bytes!("../assets/cooler.png");

        let dynamic_img =
            ::image::load_from_memory(data).map_err(|e| MediaError::ImageError(e.to_string()))?;

        // The bundled image is authored for a 480x480 cooler LCD; we rescale below.
        let x_scale = (screen.width as f32) / 480.0;
        let y_scale = (screen.height as f32) / 480.0;

        let font_label =
            Font::try_from_bytes(FONT_DATA_LABEL as &[u8]).expect("Error while loading font");

        let font_digital7 =
            Font::try_from_bytes(FONT_DATA_DIGITAL_7 as &[u8]).expect("Error while loading font");

        let mut resized = dynamic_img.into_rgba8();

        if resized.width() != screen.width || resized.height() != screen.height {
            resized = image::imageops::resize(
                &resized,
                screen.width,
                screen.height,
                FilterType::Lanczos3,
            );
        }

        // Now load the thermometer image

        let data = include_bytes!("../assets/thermometer.png");

        let dynamic_img =
            ::image::load_from_memory(data).map_err(|e| MediaError::ImageError(e.to_string()))?;

        let mut thermometer_image = dynamic_img.into_rgba8();

        if x_scale != 1.0 || y_scale != 1.0 {
            thermometer_image = image::imageops::resize(
                &thermometer_image,
                ((thermometer_image.width() as f32) * x_scale) as u32,
                ((thermometer_image.height() as f32) * y_scale) as u32,
                FilterType::Lanczos3,
            );
        }

        imageops::overlay(
            &mut resized,
            &thermometer_image,
            (300.0 * x_scale).round() as i64,
            (184.0 * y_scale).round() as i64,
        );

        let sys_data = Mutex::new(CpuData::default());
        let scale = Scale::uniform(26.0 * x_scale);

        // Now draw the labels
        let (tw, _, _, _, _) = get_exact_text_metrics(&font_label, label1, scale);

        let box_x = 170.0 * x_scale - tw as f32/2.0;
        let box_y = 220.0 * y_scale;

        let rgb_lightgrey = ::image::Rgba([230, 238, 246, 255]);
        draw_text_mut(
            &mut resized,
            rgb_lightgrey,
            box_x as i32,
            box_y as i32,
            scale,
            &font_label,
            label1,
        );

        let (tw, _, _, _, _) = get_exact_text_metrics(&font_label, label2, scale);

        let box_x = 318.0 * x_scale-tw as f32/2.0;
        let box_y = 155.0 * y_scale;

        draw_text_mut(
            &mut resized,
            rgb_lightgrey,
            box_x as i32,
            box_y as i32,
            scale,
            &font_label,
            &label2,
        );

        let box_x = 188.0 * x_scale;
        let box_y = 322.0 * y_scale;

        draw_text_mut(
            &mut resized,
            rgb_lightgrey,
            box_x as i32,
            box_y as i32,
            scale,
            &font_label,
            &label3,
        );

        // How many cores do we have?
        let usage_per_core = SysSensor::get_core_usage();

        let mut num_cores = usage_per_core.len() as i32;
        if num_cores == 0 {
            // Normally never 0...
            num_cores = 1;
        }

        // We have 228px*x_scale space
        let space = (228.0 * x_scale).round() as i32;
        let size_per_core = space / num_cores;

        let remaining_pixel = space - size_per_core * (num_cores as i32);

        // These pixels will be put in the middle...

        let border_dark = ::image::Rgba([80, 90, 100, 255]);
        let border_shadow = ::image::Rgba([230, 238, 246, 255]);

        let y = (407.0 * y_scale).round() as i32;

        for i in 0..num_cores - 1 {
            let mut x = (129.0 * x_scale) as i32 + i * size_per_core;
            if i >= num_cores / 2 {
                x += remaining_pixel;
            }
            draw_antialiased_line_segment_mut(
                &mut resized,
                (x, y),
                (x, y - 10),
                border_dark,
                interpolate,
            );
            draw_antialiased_line_segment_mut(
                &mut resized,
                (x, y - 10),
                (x + size_per_core - 1, y - 10),
                border_dark,
                interpolate,
            );
            draw_antialiased_line_segment_mut(
                &mut resized,
                (x + size_per_core - 1, y - 10),
                (x + size_per_core - 1, y),
                border_shadow,
                interpolate,
            );
            draw_antialiased_line_segment_mut(
                &mut resized,
                (x + size_per_core - 1, y),
                (x, y),
                border_shadow,
                interpolate,
            );

            let s = ((i + 1) % 10).to_string();

            draw_text_mut(
                &mut resized,
                rgb_lightgrey,
                x + 2,
                y - 8 as i32,
                Scale::uniform(9.0),
                &font_digital7,
                &s,
            );
        }

        Ok(Arc::new(Self {
            unit1: unit1.into(),
            unit2: unit2.into(),

            gauge_1_min: descriptor.gauge_1_min,
            gauge_1_max: descriptor.gauge_1_max,
            value_1_min: descriptor.value_1_min,
            value_1_max: descriptor.value_1_max,
            display_value_1_min: descriptor.display_value_1_min,
            display_value_1_max: descriptor.display_value_1_max,
            clamp_1: descriptor.clamp_1,
            decimals_1: descriptor.decimals_1,

            gauge_2_min: descriptor.gauge_2_min,
            gauge_2_max: descriptor.gauge_2_max,
            value_2_min: descriptor.value_2_min,
            value_2_max: descriptor.value_2_max,
            display_value_2_min: descriptor.display_value_2_min,
            display_value_2_max: descriptor.display_value_2_max,
            clamp_2: descriptor.clamp_2,
            decimals_2: descriptor.decimals_2,
            
            sensor_1,
            sensor_2,
            update_interval,
            orientation,
            sys_data,
            screen: *screen,
            template_image: resized,
            font_label,
            sensor_1_failed: AtomicBool::new(false),
            sensor_2_failed: AtomicBool::new(false),
            frame_index: 1.into(),
        }))
    }

    pub fn update_interval(&self) -> Duration {
        self.update_interval
    }

    /// Force flag: if true, frame gets rendered even if value has not changed. For example when we render the first frame, we set force=true
    /// Returns OK(Empty) in case of "nothing changed", OK(FrameInfo) in case a new frame has been rendered, and Error in case of an error
    pub fn render_frame(&self, force: bool) -> Result<Option<FrameInfo>, MediaError> {
        let mut data = self.sys_data.lock();

        let usage_per_core = SysSensor::get_core_usage();
        // Quantize per-core usage to whole percent (SysSensor reports 0..=10000),
        // otherwise the dirty check below would fire on every sub-percent jitter
        // and re-render at full update rate.
        let usage_per_core_pct: Vec<u32> =
            usage_per_core.iter().map(|u| u / 100).collect();

        let sensor_left_value = read_with_warn(
            "cooler",
            "sensor_1",
            &self.sensor_1,
            &self.sensor_1_failed,
        );
        let sensor_right_value = read_with_warn(
            "cooler",
            "sensor_2",
            &self.sensor_2,
            &self.sensor_2_failed,
        );

        let sensor_left_range = normalize_range(
            sensor_left_value,
            self.gauge_1_min as f32,
            self.gauge_1_max as f32,
        );
        let sensor_right_range = normalize_range(
            sensor_right_value,
            self.gauge_2_min as f32,
            self.gauge_2_max as f32,
        );

        let display_left = map_display_value(
            sensor_left_value,
            self.value_1_min,
            self.value_1_max,
            self.display_value_1_min,
            self.display_value_1_max,
            self.clamp_1,
        );
        let display_right = map_display_value(
            sensor_right_value,
            self.value_2_min,
            self.value_2_max,
            self.display_value_2_min,
            self.display_value_2_max,
            self.clamp_2,
        );

        let left_text = format!(
            "{value:.prec$}{unit}",
            value = display_left,
            prec = self.decimals_1,
            unit = self.unit1,
        );
        let right_text = format!(
            "{value:.prec$}{unit}",
            value = display_right,
            prec = self.decimals_2,
            unit = self.unit2,
        );

        let left_angle_q = (sensor_left_range * 1000.0).round() as i32;
        let right_angle_q = (sensor_right_range * 1000.0).round() as i32;

        if data.previous_left_text.as_deref() == Some(left_text.as_str())
            && data.previous_right_text.as_deref() == Some(right_text.as_str())
            && data.previous_left_angle_q == left_angle_q
            && data.previous_right_angle_q == right_angle_q
            && data.previous_usage_per_core == usage_per_core_pct
            && !force
        {
            return Ok(None);
        }

        let mut frame = self.template_image.clone();

        // Calculate color (120° -> 0°)
        let hue = 120.0 * (1.0 - sensor_left_range);
        let rgb = hsl_to_rgb(hue, 1.0, 0.5);
        let color = Rgba([rgb[0], rgb[1], rgb[2], 255]);

        let font_label = &self.font_label;

        let x_scale = (self.screen.width as f32) / 480.0;
        let y_scale = (self.screen.height as f32) / 480.0;

        let left_anchor_x = (220.0 * x_scale).round() as i32;
        let box_y = (277.0 * y_scale).round() as i32;

        let scale = Scale::uniform(39.0);

        let (tw, _, _, _, ascent) = get_exact_text_metrics(&font_label, &left_text, scale);

        draw_text_mut(
            &mut frame,
            color,
            left_anchor_x - tw,
            box_y - ascent as i32,
            scale,
            &font_label,
            &left_text,
        );

        let right_anchor_x = (374.0 * x_scale).round() as i32;

        // Calculate color  (90° -> 0°)

        let hue = 0.0 + 90.0 * (1.0 - sensor_right_range);
        let rgb = hsl_to_rgb(hue, 1.0, 0.5);
        let color = Rgba([rgb[0], rgb[1], rgb[2], 255]);

        let (tw, _, _, _, ascent) = get_exact_text_metrics(&font_label, &right_text, scale);

        draw_text_mut(
            &mut frame,
            color,
            right_anchor_x - tw,
            box_y - ascent as i32,
            scale,
            &font_label,
            &right_text,
        );

        // Now draw the inner part of the thermometer

        draw_filled_circle_mut(
            &mut frame,
            (
                (317.0 * x_scale).round() as i32,
                (229.0 * y_scale).round() as i32,
            ),
            8,
            color,
        );

        let bar_height = (sensor_right_range * 32.0) as i32;

        if bar_height > 0 {
            draw_filled_rect_mut(
                &mut frame,
                Rect::at(
                    (314.0 * x_scale).round() as i32,
                    (222.0 * y_scale).round() as i32 - bar_height,
                )
                .of_size(7, bar_height as u32),
                color,
            );
        }

        let center_x = (168.0 * x_scale).round() as i32;
        let center_y = (206.0 * y_scale).round() as i32;

        // Dial covers 180deg -> 360deg
        let needle_angle = 180.0 + sensor_left_range * 180.0;

        // Needle should be slightly longer than inner radius
        let needle_start_length = 6.0;
        let needle_length = 54.0 * x_scale;
        let needle_color = Rgba([224, 240, 255, 255]);

        draw_gauge_needle(
            &mut frame,
            (center_x as f32, (center_y + 4) as f32),
            needle_angle,
            needle_start_length,
            needle_length,
            14, // 14 pixel width
            needle_color,
        );

        let num_cores = usage_per_core.len().max(1);
        let chart_width = ((256.0 * x_scale).round() as usize).max(1);
        let size_per_core = (chart_width / num_cores).max(1);
        let bar_width = (size_per_core.saturating_sub(2)).max(1) as u32;
        let spacing = (size_per_core - bar_width as usize) as i32;
        let max_height = (47.0 * y_scale).round() as u32;
        let y_base = (391.0 * y_scale).round() as i32;
        let x_offset = (114.0 * x_scale).round() as i32;

        for (i, &usage) in usage_per_core.iter().enumerate() {
            // 1. limit load (0.0 to 1.0)
            let core_load = (usage/100).clamp(0, 100);

            let clamped_usage = (core_load as f32) / 100.0;

            // 2. Calculate color (120° -> 0°)
            let hue = 120.0 * (1.0 - clamped_usage);
            let rgb = hsl_to_rgb(hue, 1.0, 0.5);
            let color = Rgba([rgb[0], rgb[1], rgb[2], 255]);

            // 3. Determine position and height
            let current_bar_height = (clamped_usage * max_height as f32) as u32;
            let x_pos = x_offset + (i as i32 * (bar_width as i32 + spacing));

            // y_pos is y_base minus height, so that the bars are placed on a line
            let y_pos = y_base - current_bar_height as i32;

            // 4. Now draw...
            if current_bar_height > 0 {
                draw_filled_rect_mut(
                    &mut frame,
                    Rect::at(x_pos, y_pos).of_size(bar_width, current_bar_height),
                    color,
                );
            }

            if max_height - current_bar_height > 0 {
                let background_color = Rgba([40, 40, 40, 255]);
                draw_filled_rect_mut(
                    &mut frame,
                    Rect::at(x_pos, y_base - max_height as i32)
                        .of_size(bar_width, max_height - current_bar_height),
                    background_color,
                );
            }
        }

        let rgb_img: RgbImage = DynamicImage::ImageRgba8(frame).to_rgb8();

        let oriented = apply_orientation(rgb_img, self.orientation);

        // 4. Convert to desired format
        
        let mut s = self.screen.clone();
        s.jpeg_quality = 40;

        let jpeg = encode_jpeg(oriented, &s)?;

        // Only advance the dirty cache once we know the frame actually made it
        // through the encoder; otherwise a transient encode failure would mark
        // these values as already-rendered and skip retries.
        data.previous_left_text = Some(left_text);
        data.previous_right_text = Some(right_text);
        data.previous_left_angle_q = left_angle_q;
        data.previous_right_angle_q = right_angle_q;
        data.previous_usage_per_core = usage_per_core_pct;

        Ok(Some(FrameInfo {
            data: jpeg,
            frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst),
        }))
    }

    pub fn blank_frame(&self) -> FrameInfo {
        let image =
            ImageBuffer::from_pixel(self.screen.width, self.screen.height, Rgb([224, 240, 255]));
        let oriented = apply_orientation(image, self.orientation);
        let frame_ret = FrameInfo {
            data: encode_jpeg(oriented, &self.screen).unwrap_or_default(),
            frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst),
        };
        return frame_ret;
    }

}

/// Map `value` from the inclusive range [min, max] to [0, 1], clamping out-of-range
/// inputs and returning 0 if the range is degenerate (min == max).
fn normalize_range(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if span.abs() < f32::EPSILON {
        return 0.0;
    }
    ((value - min) / span).clamp(0.0, 1.0)
}

fn map_display_value(
    raw: f32,
    value_min: i32,
    value_max: i32,
    display_min: i32,
    display_max: i32,
    clamp: bool,
) -> f32 {
    let span = (value_max - value_min) as f32;
    let display_span = (display_max - display_min) as f32;
    let mapped = if span.abs() < f32::EPSILON {
        display_min as f32
    } else {
        display_min as f32 + ((raw - value_min as f32) / span) * display_span
    };
    if clamp {
        let lo = (display_min as f32).min(display_max as f32);
        let hi = (display_min as f32).max(display_max as f32);
        mapped.clamp(lo, hi)
    } else {
        mapped
    }
}

/// Read a sensor and fall back to 0 on error. Logs the failure once per failure
/// transition (suppresses spam from a 100ms render loop) and re-arms after the
/// next successful read so transient errors aren't lost.
fn read_with_warn(
    asset: &'static str,
    label: &'static str,
    sensor: &ResolvedSensor,
    failed: &AtomicBool,
) -> f32 {
    match lianli_shared::sensors::read_sensor_value(sensor) {
        Ok(value) => {
            failed.store(false, Ordering::Relaxed);
            value
        }
        Err(err) => {
            if !failed.swap(true, Ordering::Relaxed) {
                warn!("{asset} {label} read failed: {err}");
            }
            0.0
        }
    }
}

fn draw_gauge_needle(
    img: &mut RgbaImage,
    center: (f32, f32),
    angle_deg: f32,
    start_length: f32,
    length: f32,
    width: i32, // line thickness
    color: Rgba<u8>,
) {
    let mut dreieck: Vec<Point<f32>> = vec![];
    let angle_rad = angle_deg.to_radians();
    let offset = width / 2;
    let orth_angle = angle_rad + (PI / 2.0);

    let end = Point {
        x: center.0 + angle_rad.cos() * length,
        y: center.1 + angle_rad.sin() * length,
    };
    let center_new = Point {
        x: center.0 + angle_rad.cos() * start_length,
        y: center.1 + angle_rad.sin() * start_length,
    };

    let p1 = Point {
        x: center_new.x + orth_angle.cos() * offset as f32,
        y: center_new.y + orth_angle.sin() * offset as f32,
    };

    let p2 = Point {
        x: center_new.x - orth_angle.cos() * offset as f32,
        y: center_new.y - orth_angle.sin() * offset as f32,
    };

    dreieck.push(p1);
    dreieck.push(end);
    dreieck.push(p2);

    draw_filled_polygon_with_border(img, &dreieck, color, Rgba([174, 10, 16, 255]));
}

fn draw_filled_polygon_with_border(
    img: &mut RgbaImage,
    points: &[Point<f32>],
    fill_color: Rgba<u8>,
    border_color: Rgba<u8>,
) {
    if points.len() < 3 {
        return;
    }

    // 1. convert points into format required by imageproc (i32)
    let poly_points: Vec<Point<i32>> = points
        .iter()
        .map(|p| Point::new(p.x as i32, p.y as i32))
        .collect();

    // 2. At first draw the inner fill
    draw_polygon_mut(img, &poly_points, fill_color);

    // 3. Now draw the border with anti aliasing — iterate over all edges so the
    // final segment back to the first point closes the polygon.
    for i in 0..points.len() {
        let start = points[i];
        let end = points[(i + 1) % points.len()];

        draw_antialiased_line_segment_mut(
            img,
            (start.x as i32, start.y as i32),
            (end.x as i32, end.y as i32),
            border_color,
            interpolate,
        );
    }
}

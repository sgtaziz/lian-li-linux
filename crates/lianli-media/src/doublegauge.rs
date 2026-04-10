// Same caveat as cooler.rs: every coordinate here is tuned for a 400x400 panel
// and the bundled gauge.jpg. Scales are screen.dim / 400, arc centers come
// from the screen midpoint, label and value-text positions are baked-in pixel
// offsets, and the layout will stretch (not letterbox) on non-square panels.
// Swapping the background for one of a different size will misplace the arcs
// and labels — when we let users pick their own, layout coords need to come
// from the asset.
//
// The gauge.jpg and Victor Mono font are pulled in with include_bytes!, part
// of the ~700 KB of assets baked into the daemon binary across this and
// cooler.rs.

use super::common::MediaError;
use super::common::{apply_orientation, encode_jpeg, get_exact_text_metrics, FONT_DATA};
use image::{imageops, DynamicImage, ImageBuffer, Pixel, Rgb, RgbImage, Rgba, RgbaImage};
use crate::sensor::FrameInfo;
use imageproc::drawing::draw_polygon_mut;
use imageproc::drawing::draw_text_mut;
use imageproc::geometric_transformations::{rotate_about_center, Interpolation};
use imageproc::point::Point;
use lianli_shared::media::DoublegaugeDescriptor;
use lianli_shared::screen::ScreenInfo;
use lianli_shared::sensors::ResolvedSensor;
use parking_lot::Mutex;
use rusttype::{Font, Scale};
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

#[derive(Debug)]
struct GaugePixel {
    x: u32,
    y: u32,
    angle: f32,
}

#[derive(Debug)]
pub struct DoublegaugeAsset {
    pub header: String,
    pub gauge_1_min: i32,
    pub gauge_1_max: i32,
    pub value_1_min: i32,
    pub value_1_max: i32,
    pub display_value_1_min: i32,
    pub display_value_1_max: i32,
    pub clamp_1: bool,
    pub unit_1: String,
    pub label_1: String,
    pub decimals_1: usize,

    pub gauge_2_min: i32,
    pub gauge_2_max: i32,
    pub value_2_min: i32,
    pub value_2_max: i32,
    pub display_value_2_min: i32,
    pub display_value_2_max: i32,
    pub clamp_2: bool,
    pub unit_2: String,
    pub label_2: String,
    pub decimals_2: usize,

    pub sensor_1: ResolvedSensor,
    pub sensor_2: ResolvedSensor,

    orientation: f32,
    /// Update interval in ms
    update_interval: Duration,

    template_image: image::RgbaImage, // prepared background image
    full_arc_outer: RgbaImage,        // pre-rendered outer arc
    full_arc_inner: RgbaImage,        // pre-rendered inner arc

    // Lookup-tables for both arcs
    outer_arc_lut: Vec<GaugePixel>,
    inner_arc_lut: Vec<GaugePixel>,
    font: Font<'static>,

    // The whole frame will be rendered only if one of the following two values actually change.
    previous_outer_gauge_value: Mutex<Option<String>>, // previously drawn value for other gauge (as string, but basically a numerical value)
    previous_inner_gauge_value: Mutex<Option<String>>, // previously drawn value for inner gauge (as string, but basically a numerical value)

    sensor_1_failed: AtomicBool,
    sensor_2_failed: AtomicBool,

    /// here we store the screen info like width and height
    screen: ScreenInfo,
    // Each time a frame gets redrawn this index is "assigned" to the frame.
    frame_index: AtomicUsize,
}

impl DoublegaugeAsset {
    pub fn new(
        descriptor: &DoublegaugeDescriptor,
        orientation: f32,
        screen: &ScreenInfo,
        sensor_1: ResolvedSensor,
        sensor_2: ResolvedSensor,
    ) -> Result<Arc<Self>, MediaError> {
        let update_interval = Duration::from_millis(100);

        // Decode image once during init
        let data = include_bytes!("../assets/gauge.jpg");
        let dynamic_img =
            ::image::load_from_memory(data).map_err(|e| MediaError::ImageError(e.to_string()))?;

        let dynamic_img = dynamic_img.resize(
            screen.width,
            screen.height,
            ::image::imageops::FilterType::Lanczos3,
        );

        let x_scale = (screen.width as f32) / 400.0;
        let y_scale = (screen.height as f32) / 400.0;

        let mut template_image = dynamic_img.into_rgb8();

        let font = Font::try_from_bytes(FONT_DATA as &[u8]).expect("Error while loading font");

        let rgb_lightgrey = ::image::Rgb([230, 238, 246]);
        let scale = Scale::uniform(34.0 * x_scale);

        let text = &descriptor.label_1;
        let (tw, _, ox, _, _) = get_exact_text_metrics(&font, text, scale);
        let spacing = 10; // Additional distance in pixel

        let mut x_pos =
            (screen.width as i32 - tw - (text.chars().count() as i32 - 1) * spacing) / 2 - ox; // start position
        let y_pos = (96.0 * y_scale) as i32;

        for c in text.chars() {
            let s = c.to_string();

            // Draw each char
            draw_text_mut(
                &mut template_image,
                rgb_lightgrey,
                x_pos,
                y_pos,
                scale,
                &font,
                &s,
            );

            // Calculate width of current char
            let glyph = font.glyph(c).scaled(scale);
            let h_metrics = glyph.h_metrics();

            // Update x_pos: Width of char + extra spacing
            x_pos += h_metrics.advance_width as i32 + spacing;
        }

        let text = &descriptor.label_2;
        let (tw, _, ox, _, _) = get_exact_text_metrics(&font, text, scale);
        let spacing = 10; // Additional distance in pixel between each label char

        let mut x_pos =
            (screen.width as i32 - tw - (text.chars().count() as i32 - 1) * spacing) / 2 - ox; // Start position
        let y_pos = (210.0 * y_scale) as i32;

        for c in text.chars() {
            let s = c.to_string();

            // Draw each char
            draw_text_mut(
                &mut template_image,
                rgb_lightgrey,
                x_pos,
                y_pos,
                scale,
                &font,
                &s,
            );

            // Calculate width of current char
            let glyph = font.glyph(c).scaled(scale);
            let h_metrics = glyph.h_metrics();

            // Update x_pos: Width of char + extra spacing
            x_pos += h_metrics.advance_width as i32 + spacing;
        }

        let mut rgba_img: RgbaImage = DynamicImage::ImageRgb8(template_image).to_rgba8();

        draw_rotated_text_on_circle(
            &mut rgba_img,
            &descriptor.header,
            &font,
            ((screen.width / 2) as f32, (screen.height / 2) as f32),
            (screen.height / 2 - 40) as f32,
            260.0,
            (50.0 * x_scale) as u32,
            Rgba([0, 0, 0, 255]),
        );

        let mut full_arc_outer = RgbaImage::new(screen.width, screen.height);
        let rgba_green_main = Rgba([40, 255, 137, 220]);
        let center = (screen.width as f32 / 2.0, screen.height as f32 / 2.0);
        let angle_min = -58.0;
        let angle_max = 238.0;

        let r_outer = (screen.width as f32 / 2.0 - 20.0 * x_scale).round();
        let r_inner = (r_outer - 20.0 * x_scale).round();

        draw_smooth_segment_blended(
            &mut full_arc_outer,
            rgba_green_main,
            center,
            r_inner,
            r_outer,
            angle_min,
            angle_max,
        );
        draw_smooth_segment_blended(
            &mut full_arc_outer,
            Rgba([40, 255, 137, 180]),
            center,
            r_outer,
            r_outer + 2.0,
            angle_min,
            angle_max,
        );
        draw_smooth_segment_blended(
            &mut full_arc_outer,
            Rgba([40, 255, 137, 140]),
            center,
            r_outer + 2.0,
            r_outer + 4.0,
            angle_min,
            angle_max,
        );
        draw_smooth_segment_blended(
            &mut full_arc_outer,
            Rgba([40, 255, 137, 140]),
            center,
            r_outer + 4.0,
            r_outer + 6.0,
            angle_min,
            angle_max,
        );
        draw_smooth_segment_blended(
            &mut full_arc_outer,
            Rgba([40, 255, 137, 140]),
            center,
            r_outer + 6.0,
            r_outer + 8.0,
            angle_min,
            angle_max,
        );

        let outer_arc_lut = Self::create_lut(center, r_inner, r_outer + 8.0);

        let mut full_arc_inner = RgbaImage::new(screen.width, screen.height);
        let rgba_blue_main = Rgba([32, 209, 255, 220]);

        let r_outer = (r_inner - 11.0 * x_scale).round();
        let r_inner = (r_outer - 20.0 * x_scale).round();

        draw_smooth_segment_blended(
            &mut full_arc_inner,
            rgba_blue_main,
            center,
            r_inner,
            r_outer,
            angle_min,
            angle_max,
        );
        draw_smooth_segment_blended(
            &mut full_arc_inner,
            Rgba([32, 209, 255, 180]),
            center,
            r_inner - 2.0,
            r_inner,
            angle_min,
            angle_max,
        );
        draw_smooth_segment_blended(
            &mut full_arc_inner,
            Rgba([32, 209, 255, 140]),
            center,
            r_inner - 4.0,
            r_inner - 2.0,
            angle_min,
            angle_max,
        );
        draw_smooth_segment_blended(
            &mut full_arc_inner,
            Rgba([32, 209, 255, 100]),
            center,
            r_inner - 6.0,
            r_inner - 4.0,
            angle_min,
            angle_max,
        );
        draw_smooth_segment_blended(
            &mut full_arc_inner,
            Rgba([32, 209, 255, 80]),
            center,
            r_inner - 8.0,
            r_inner - 6.0,
            angle_min,
            angle_max,
        );

        let inner_arc_lut = Self::create_lut(center, r_inner - 6.0, r_outer);

        Ok(Arc::new(Self {
            header: descriptor.header.clone(),

            gauge_1_min: descriptor.gauge_1_min,
            gauge_1_max: descriptor.gauge_1_max,
            value_1_min: descriptor.value_1_min,
            value_1_max: descriptor.value_1_max,

            display_value_1_min: descriptor.display_value_1_min,
            display_value_1_max: descriptor.display_value_1_max,

            clamp_1: descriptor.clamp_1,
            unit_1: descriptor.unit_1.clone(),
            label_1: descriptor.label_1.clone(),
            decimals_1: descriptor.decimals_1,
            sensor_1: sensor_1,
            sensor_2: sensor_2,

            gauge_2_min: descriptor.gauge_2_min,
            gauge_2_max: descriptor.gauge_2_max,
            value_2_min: descriptor.value_2_min,
            value_2_max: descriptor.value_2_max,

            display_value_2_min: descriptor.display_value_2_min,
            display_value_2_max: descriptor.display_value_2_max,

            clamp_2: descriptor.clamp_2,
            unit_2: descriptor.unit_2.clone(),
            label_2: descriptor.label_2.clone(),
            decimals_2: descriptor.decimals_2,

            update_interval,
            orientation,
            template_image: rgba_img,
            full_arc_outer,
            full_arc_inner,
            outer_arc_lut,
            inner_arc_lut,
            font,
            previous_outer_gauge_value: Mutex::new(Option::None),
            previous_inner_gauge_value: Mutex::new(Option::None),
            sensor_1_failed: AtomicBool::new(false),
            sensor_2_failed: AtomicBool::new(false),
            screen: *screen,
            frame_index: 1.into(),
        }))
    }

    pub fn update_interval(&self) -> Duration {
        self.update_interval
    }

    fn create_lut(center: (f32, f32), r_in: f32, r_out: f32) -> Vec<GaugePixel> {
        let mut lut = Vec::new();
        let r_in_sq = r_in * r_in;
        let r_out_sq = r_out * r_out;

        // Scan the bounding box
        let x_start = (center.0 - r_out) as u32;
        let x_end = (center.0 + r_out) as u32;
        let y_start = (center.1 - r_out) as u32;
        let y_end = (center.1 + r_out) as u32;

        for y in y_start..y_end {
            for x in x_start..x_end {
                let dx = x as f32 - center.0;
                let dy = y as f32 - center.1;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq >= r_in_sq && dist_sq <= r_out_sq {
                    let mut angle = dy.atan2(dx).to_degrees();
                    if angle < -90.0 {
                        angle += 360.0;
                    }

                    lut.push(GaugePixel { x, y, angle });
                }
            }
        }
        // Important: Sort by angle asc (gives speed as we can step out from drawing loop when angle reached)
        lut.sort_by(|a, b| {
            a.angle
                .partial_cmp(&b.angle)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        lut
    }

    /// Force flag: if true, frame gets rendered even if value has not changed. For example when we render the first frame, we set force=true
    /// Returns OK(Empty) in case of "nothing changed", OK(FrameInfo) in case a new frame has been rendered, and Error in case of an error
    pub fn render_frame(&self, force: bool) -> Result<Option<FrameInfo>, MediaError> {
        let metric_outer_gauge_raw = read_with_warn(
            "doublegauge",
            "sensor_1",
            &self.sensor_1,
            &self.sensor_1_failed,
        );

        // Now map metric_outer_gauge: value_1_min to display_value_1_min upto value_1_max to display_value_1_max
        // And if clamp_1 is true, then clamp to display_value_1_min;display_value_1_max

        let mut metric_outer_gauge = self.display_value_1_min as f32
            + map_unit_interval(
                metric_outer_gauge_raw,
                self.value_1_min as f32,
                self.value_1_max as f32,
            ) * (self.display_value_1_max - self.display_value_1_min) as f32;
        if self.clamp_1 {
            metric_outer_gauge = metric_outer_gauge.clamp(
                self.display_value_1_min as f32,
                self.display_value_1_max as f32,
            );
        }

        let metric_outer_gauge_text = format!(
            "{value:.prec$}{unit}",
            value = metric_outer_gauge,
            prec = self.decimals_1,
            unit = self.unit_1
        );

        let metric_inner_gauge_raw = read_with_warn(
            "doublegauge",
            "sensor_2",
            &self.sensor_2,
            &self.sensor_2_failed,
        );

        let mut metric_inner_gauge = self.display_value_2_min as f32
            + map_unit_interval(
                metric_inner_gauge_raw,
                self.value_2_min as f32,
                self.value_2_max as f32,
            ) * (self.display_value_2_max - self.display_value_2_min) as f32;
        if self.clamp_2 {
            metric_inner_gauge = metric_inner_gauge.clamp(
                self.display_value_2_min as f32,
                self.display_value_2_max as f32,
            );
        }

        let metric_inner_gauge_text = format!(
            "{value:.prec$}{unit}",
            value = metric_inner_gauge,
            prec = self.decimals_2,
            unit = self.unit_2
        );
        {
            let prev_outer = self.previous_outer_gauge_value.lock();
            let prev_inner = self.previous_inner_gauge_value.lock();

            if prev_outer.as_deref() == Some(metric_outer_gauge_text.as_str())
                && prev_inner.as_deref() == Some(metric_inner_gauge_text.as_str())
                && !force
            {
                return Ok(None);
            }
        }

        *self.previous_outer_gauge_value.lock() = Some(metric_outer_gauge_text.clone());
        *self.previous_inner_gauge_value.lock() = Some(metric_inner_gauge_text.clone());

        // A pure in-memory-copy-operation (fast)
        let mut frame = self.template_image.clone();

        // Now lets edit the image

        let rgba_green = ::image::Rgba([40, 255, 137, 255]);
        let rgba_blue = ::image::Rgba([32, 209, 255, 255]);

        let x_scale = (self.screen.width as f32) / 400.0;
        let y_scale = (self.screen.height as f32) / 400.0;

        let scale = Scale {
            x: 70.0 * x_scale,
            y: 70.0 * y_scale,
        };

        let (tw, th, ox, oy, _) =
            get_exact_text_metrics(&self.font, &metric_outer_gauge_text, scale);

        let box_x = (self.screen.width as i32) / 2 + (60.0 * x_scale) as i32 - tw;
        let box_y = (self.screen.height as i32 - th) / 2 - (45.0 * y_scale) as i32;

        draw_text_mut(
            &mut frame,
            rgba_green,
            box_x - ox,
            box_y - oy,
            scale,
            &self.font,
            &metric_outer_gauge_text,
        );

        let (tw, _, ox, oy, _) =
            get_exact_text_metrics(&self.font, &metric_inner_gauge_text, scale);

        let box_x = (self.screen.width as i32) / 2 + (60.0 * x_scale) as i32 - tw;
        let box_y = (self.screen.height as i32) / 2 + (46.0 * y_scale) as i32;

        draw_text_mut(
            &mut frame,
            rgba_blue,
            box_x - ox,
            box_y - oy,
            scale,
            &self.font,
            &metric_inner_gauge_text, // Dein Text aus der Struct
        );

        let angle_min = -58.0;
        let angle_max = 238.0;

        // Map raw sensor reading into [0, 1] across the gauge's configured min/max,
        // then linearly interpolate the arc sweep angle.
        let outer_range = map_unit_interval(
            metric_outer_gauge_raw,
            self.gauge_1_min as f32,
            self.gauge_1_max as f32,
        )
        .clamp(0.0, 1.0);
        let angle = angle_min + (angle_max - angle_min) * outer_range;
        self.apply_lut_mask(&mut frame, &self.full_arc_outer, &self.outer_arc_lut, angle);

        let inner_range = map_unit_interval(
            metric_inner_gauge_raw,
            self.gauge_2_min as f32,
            self.gauge_2_max as f32,
        )
        .clamp(0.0, 1.0);
        let angle = angle_min + (angle_max - angle_min) * inner_range;
        self.apply_lut_mask(&mut frame, &self.full_arc_inner, &self.inner_arc_lut, angle);

        let resized: RgbImage = DynamicImage::ImageRgba8(frame).to_rgb8();

        let oriented = apply_orientation(resized, self.orientation);

        let encoded_jpeg_result = encode_jpeg(oriented, &self.screen).map(Some);
        let frame_result: Result<Option<FrameInfo>, MediaError> = encoded_jpeg_result.map(|opt| {
            opt.map(|data| FrameInfo {
                data,
                frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst),
            })
        });
        return frame_result;
    }

    pub fn blank_frame(&self) -> FrameInfo {
        let image =
            ImageBuffer::from_pixel(self.screen.width, self.screen.height, Rgb([255, 0, 0]));
        let oriented = apply_orientation(image, self.orientation);
        let frame_ret = FrameInfo {
            data: encode_jpeg(oriented, &self.screen).unwrap_or_default(),
            frame_index: self.frame_index.fetch_add(1, Ordering::SeqCst),
        };
        return frame_ret;
    }

    fn apply_lut_mask(
        &self,
        target: &mut RgbaImage,
        source: &RgbaImage,
        lut: &[GaugePixel],
        target_angle: f32,
    ) {
        for p in lut {
            if p.angle > target_angle {
                break;
            }
            // Copy directly from the pre-rendered image
            let src_pixel = source.get_pixel(p.x, p.y);
            if src_pixel[3] > 0 {
                // blending is necessary for those semi transparent edges (antialiasing)
                target.get_pixel_mut(p.x, p.y).blend(src_pixel);
            }
        }
    }

}

/// Linearly map `value` from [min, max] to a unit interval (typically [0, 1] but
/// may extend outside if the caller wants to extrapolate). Returns 0 if the
/// range is degenerate (min == max) so we never divide by zero.
fn map_unit_interval(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if span.abs() < f32::EPSILON {
        return 0.0;
    }
    (value - min) / span
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

fn draw_rotated_text_on_circle(
    main_img: &mut RgbaImage,
    text: &str,
    font: &Font,
    center: (f32, f32),
    radius: f32,
    start_angle_deg: f32,
    char_size: u32,
    color: Rgba<u8>,
) {
    let scale = Scale::uniform(char_size as f32);

    let color_gray = Rgba([150, 180, 255, 255]);

    let mut current_angle = start_angle_deg * PI / 180.0;
    let angle_step = 12.0 * PI / 180.0; // angular spacing between characters

    for c in text.chars() {
        // 1. Temporary image for each char (e.g. 40x40 pixel)
        let mut char_img = RgbaImage::new(char_size as u32, char_size + 10 as u32);
        draw_text_mut(
            &mut char_img,
            color_gray,
            13,
            9,
            scale,
            font,
            &c.to_string(),
        );
        draw_text_mut(&mut char_img, color, 10, 5, scale, font, &c.to_string());

        // 2. Rotate char
        // Need to adjust the angle so that the char points outwards
        let rotation_angle = current_angle + (PI / 2.0);
        let rotated_char = rotate_about_center(
            &char_img,
            rotation_angle,
            Interpolation::Bicubic,
            Rgba([0, 0, 0, 0]), // transparent background for the corners
        );

        // 3. Calculate position on circle
        let x = center.0 + radius * current_angle.cos() - (char_size / 2) as f32; // -20 zum Zentrieren des 40px Bildes
        let y = center.1 + radius * current_angle.sin() - (char_size / 2) as f32;

        // 4. Copy to main image
        imageops::overlay(main_img, &rotated_char, x as i64, y as i64);

        current_angle += angle_step;
    }
}

fn draw_smooth_segment_blended(
    img: &mut RgbaImage,
    color: Rgba<u8>,
    center: (f32, f32),
    radius_inner: f32,
    radius_outer: f32,
    start_deg: f32,
    end_deg: f32,
) {
    let mut points = vec![];

    let steps = ((end_deg - start_deg) / 10.0) as i32; // more steps -> smoother arc
    if steps == 0 {
        return;
    }

    for i in 0..=steps {
        let angle = (start_deg + (end_deg - start_deg) * (i as f32 / steps as f32)) * PI / 180.0;
        let x = center.0 + radius_inner * angle.cos();
        let y = center.1 + radius_inner * angle.sin();
        points.push(Point::new(x as i32, y as i32));
    }

    for i in (0..=steps).rev() {
        let angle = (start_deg + (end_deg - start_deg) * (i as f32 / steps as f32)) * PI / 180.0;
        let x = center.0 + radius_outer * angle.cos();
        let y = center.1 + radius_outer * angle.sin();
        points.push(Point::new(x as i32, y as i32));
    }

    // 2. create temporary overlay image (fully transparent)
    let (width, height) = img.dimensions();
    let mut overlay = RgbaImage::new(width, height);

    // 3. Draw the poly onto the overlay (opaque)
    // We're using the RGB values from 'color', but set alpha to 255
    let opaque_color = Rgba([color[0], color[1], color[2], 255]);
    draw_polygon_mut(&mut overlay, &points, opaque_color);

    // 4. Now blend back to main image pixel by pixel
    // Iterate over the pixels which we have drawn right before
    for (x, y, overlay_pixel) in overlay.enumerate_pixels() {
        if overlay_pixel[3] > 0 {
            let base_pixel = img.get_pixel_mut(x, y);

            let blend_color = color;

            // If the polygon method created antialiasing-pixel (semi transparent),
            // we combine these with our alpha

            base_pixel.blend(&blend_color);
        }
    }
}

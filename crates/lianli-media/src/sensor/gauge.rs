use image::{Rgb, RgbImage};

pub(super) struct GaugeParams {
    pub value: f32,
    pub gauge_color: [u8; 3],
    pub ring_color: [u8; 3],
    pub outer_radius: f32,
    pub thickness: f32,
    pub start_angle: f32,
    pub sweep_angle: f32,
    pub corner_radius: f32,
}

pub(super) fn draw_gauge(image: &mut RgbImage, width: u32, height: u32, params: GaugeParams) {
    let GaugeParams {
        value,
        gauge_color,
        ring_color,
        outer_radius,
        thickness,
        start_angle,
        sweep_angle,
        corner_radius,
    } = params;
    let cx = (width as f32 - 1.0) / 2.0;
    let cy = (height as f32 - 1.0) / 2.0;
    let max_radius = ((width.min(height) as f32 / 2.0) - 4.0).max(20.0);
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
                                            (base_color[0] as f32 * alpha
                                                + ring_color[0] as f32 * (1.0 - alpha))
                                                as u8,
                                            (base_color[1] as f32 * alpha
                                                + ring_color[1] as f32 * (1.0 - alpha))
                                                as u8,
                                            (base_color[2] as f32 * alpha
                                                + ring_color[2] as f32 * (1.0 - alpha))
                                                as u8,
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

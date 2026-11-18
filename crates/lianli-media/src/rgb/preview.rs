use std::f32::consts::{FRAC_PI_2, PI, TAU};

use anyhow::Result;
use lianli_shared::rgb::preview::{RgbPreviewElement, RgbPreviewLed, RgbPreviewShape};
use lianli_shared::rgb::{
    RgbDeviceConfig, RgbMode, RgbRegionConfig, RgbRenderFamily, RgbRenderProfile, RgbZoneConfig,
};

pub const MAX_PREVIEW_FRAMES: usize = 240;

/// A TL light bar climbs a 45° chamfer from each side and runs straight along the edge.
const TL_TOP_BAR: [(f32, f32); 4] = [(0.07, 0.27), (0.27, 0.07), (0.73, 0.07), (0.93, 0.27)];
const TL_BAR_STEPS: [usize; 3] = [3, 6, 3];

pub struct PreviewLayout {
    pub elements: Vec<RgbPreviewElement>,
    pub leds: Vec<RgbPreviewLed>,
}

pub struct PreviewFrames {
    pub frames: Vec<Vec<[u8; 3]>>,
    pub interval_ms: u32,
}

#[derive(Default)]
struct Builder {
    elements: Vec<RgbPreviewElement>,
    leds: Vec<RgbPreviewLed>,
}

impl Builder {
    fn element(&mut self, shape: RgbPreviewShape, aspect: f32) -> u8 {
        self.elements.push(RgbPreviewElement { shape, aspect });
        (self.elements.len() - 1) as u8
    }

    fn led(&mut self, element: u8, x: f32, y: f32) {
        self.leds.push(RgbPreviewLed { element, x, y });
    }

    fn ring(&mut self, element: u8, count: usize, radius: f32) {
        for i in 0..count {
            let angle = -FRAC_PI_2 + TAU * i as f32 / count as f32;
            self.led(
                element,
                0.5 + radius * angle.cos(),
                0.5 + radius * angle.sin(),
            );
        }
    }

    fn arc(&mut self, element: u8, count: usize, radius: f32, from: f32, to: f32) {
        for i in 0..count {
            let angle = from + (to - from) * (i as f32 + 0.5) / count as f32;
            self.led(
                element,
                0.5 + radius * angle.cos(),
                0.5 + radius * angle.sin(),
            );
        }
    }

    fn finish(self, led_count: u16) -> Option<PreviewLayout> {
        (self.leds.len() <= usize::from(led_count)).then_some(PreviewLayout {
            elements: self.elements,
            leds: self.leds,
        })
    }
}

fn tl_fan(builder: &mut Builder) {
    let fan = builder.element(RgbPreviewShape::Fan, 1.0);
    for flip in [false, true] {
        let mut first = true;
        for (segment, steps) in TL_TOP_BAR.windows(2).zip(TL_BAR_STEPS) {
            let ((x0, y0), (x1, y1)) = (segment[0], segment[1]);
            for step in usize::from(!first)..=steps {
                let t = step as f32 / steps as f32;
                let y = y0 + (y1 - y0) * t;
                builder.led(fan, x0 + (x1 - x0) * t, if flip { 1.0 - y } else { y });
            }
            first = false;
        }
    }
}

/// Places each physical LED of a bar at its logical position along the sweep, on that bar's row.
fn bars(builder: &mut Builder, element: u8, map: &[Option<usize>], rows: &[(usize, f32)]) {
    let last = map.iter().flatten().copied().max().unwrap_or(1).max(1);
    let mut start = 0;
    for &(count, y) in rows {
        for logical in &map[start..start + count] {
            builder.led(
                element,
                0.1 + 0.8 * logical.unwrap_or(0) as f32 / last as f32,
                y,
            );
        }
        start += count;
    }
}

fn sl_fan(builder: &mut Builder) {
    let fan = builder.element(RgbPreviewShape::Fan, 1.0);
    let map = super::sync_layout::sl_fan_map();
    bars(
        builder,
        fan,
        &map,
        &[(12, 0.14), (8, 0.05), (12, 0.86), (8, 0.95)],
    );
}

fn sl_v4_fan(builder: &mut Builder) {
    let fan = builder.element(RgbPreviewShape::Fan, 1.0);
    let map: Vec<_> = (0..4).flat_map(|_| (0..13).map(Some)).collect();
    bars(
        builder,
        fan,
        &map,
        &[(13, 0.14), (13, 0.05), (13, 0.86), (13, 0.95)],
    );
}

fn sl_inf_fan(builder: &mut Builder, right_attach: bool) {
    let fan = builder.element(RgbPreviewShape::Fan, 1.0);
    let map = super::sync_layout::sl_inf_fan_map(right_attach);
    for (physical, logical) in map[..8].iter().enumerate() {
        let x = 2.0 * logical.unwrap_or(0) as f32 / 7.0 - 1.0;
        let upper = !(3..6).contains(&physical);
        let angle = if upper { -x.acos() } else { x.acos() };
        builder.led(fan, 0.5 + 0.17 * angle.cos(), 0.5 + 0.17 * angle.sin());
    }
    bars(
        builder,
        fan,
        &map[8..],
        &[(10, 0.16), (8, 0.06), (10, 0.84), (8, 0.94)],
    );
}

fn cl_fan(builder: &mut Builder) {
    let fan = builder.element(RgbPreviewShape::Fan, 1.0);
    builder.ring(fan, 8, 0.18);
    let map = super::sync_layout::cl_fan_map();
    for (physical, logical) in map.iter().enumerate().skip(8) {
        let x = 2.0 * logical.unwrap_or(0) as f32 / 7.0 - 1.0;
        let angle = if physical < 16 { -x.acos() } else { x.acos() };
        builder.led(fan, 0.5 + 0.43 * angle.cos(), 0.5 + 0.43 * angle.sin());
    }
}

fn hs2_oled(builder: &mut Builder) {
    let pump = builder.element(RgbPreviewShape::Screen, 2.0);
    let map = super::sync_layout::hs2_oled_map();
    let last = map.iter().flatten().copied().max().unwrap_or(1) as f32;
    let mut gap = 0;
    for (physical, logical) in map.iter().enumerate() {
        match logical {
            Some(logical) => {
                let y = if physical < map.len() / 2 { 0.0 } else { 1.0 };
                builder.led(pump, *logical as f32 / last, y);
            }
            None => {
                gap += 1;
                builder.led(pump, 0.0, gap as f32 / 8.0);
            }
        }
    }
}

fn per_fan(builder: &mut Builder, profile: RgbRenderProfile, draw: impl Fn(&mut Builder)) {
    for _ in 0..profile.fan_count {
        draw(builder);
    }
}

/// Logical index is distance along each half-ring, from mid-left side to mid-right side.
fn universal_ring(builder: &mut Builder) {
    const ASPECT: f32 = 4.0;
    let screen = builder.element(RgbPreviewShape::Screen, ASPECT);
    let map = super::sync_layout::universal_screen_map();
    let last = map.iter().flatten().copied().max().unwrap_or(1) as f32;
    for (physical, logical) in map.into_iter().enumerate() {
        let distance = logical.unwrap_or(0) as f32 / last * (ASPECT + 1.0);
        let (x, from_edge) = if distance < 0.5 {
            (0.0, 0.5 - distance)
        } else if distance <= ASPECT + 0.5 {
            ((distance - 0.5) / ASPECT, 0.0)
        } else {
            (1.0, distance - ASPECT - 0.5)
        };
        let top = !(18..48).contains(&physical);
        builder.led(screen, x, if top { from_edge } else { 1.0 - from_edge });
    }
}

fn screen_edge(builder: &mut Builder, count: usize, aspect: f32) {
    let screen = builder.element(RgbPreviewShape::Screen, aspect);
    let (width, height) = (aspect, 1.0);
    let perimeter = 2.0 * (width + height);
    for i in 0..count {
        let distance = perimeter * (i as f32 + 0.5) / count as f32;
        let (x, y) = if distance < width {
            (distance, 0.0)
        } else if distance < width + height {
            (width, distance - width)
        } else if distance < 2.0 * width + height {
            (2.0 * width + height - distance, height)
        } else {
            (0.0, perimeter - distance)
        };
        builder.led(screen, x / width, y);
    }
}

fn strip_rows(builder: &mut Builder, led_count: u16, per_row: usize) {
    let total = usize::from(led_count);
    let rows = total.div_ceil(per_row);
    let strip = builder.element(RgbPreviewShape::Strip, per_row as f32 / (rows as f32 * 1.6));
    for i in 0..total {
        let (row, column) = (i / per_row, i % per_row);
        builder.led(
            strip,
            (column as f32 + 0.5) / per_row as f32,
            (row as f32 + 0.5) / rows as f32,
        );
    }
}

pub fn layout(profile: RgbRenderProfile) -> Option<PreviewLayout> {
    let mut builder = Builder::default();
    match profile.family {
        RgbRenderFamily::Tl => per_fan(&mut builder, profile, tl_fan),
        RgbRenderFamily::Sl => per_fan(&mut builder, profile, sl_fan),
        RgbRenderFamily::SlV4 => per_fan(&mut builder, profile, sl_v4_fan),
        RgbRenderFamily::SlInf | RgbRenderFamily::SlInfV3 => {
            per_fan(&mut builder, profile, |builder| {
                sl_inf_fan(builder, profile.right_attach)
            })
        }
        RgbRenderFamily::Cl => per_fan(&mut builder, profile, cl_fan),
        RgbRenderFamily::P28 => {
            for _ in 0..profile.fan_count {
                let fan = builder.element(RgbPreviewShape::Fan, 1.0);
                builder.ring(fan, 9, 0.42);
            }
        }
        RgbRenderFamily::HydroShiftII => {
            let pump = builder.element(RgbPreviewShape::Pump, 1.0);
            builder.ring(pump, 24, 0.42);
            per_fan(&mut builder, profile, cl_fan);
        }
        RgbRenderFamily::HydroShiftIIOled if profile.led_count == 35 => hs2_oled(&mut builder),
        RgbRenderFamily::HydroShiftIIOled => {
            let pump = builder.element(RgbPreviewShape::Pump, 1.0);
            let count = usize::from(profile.led_count);
            builder.arc(pump, count, 0.44, -PI / 2.0 + 0.35, 3.0 * PI / 2.0 - 0.35);
        }
        RgbRenderFamily::UniversalScreen if profile.led_count >= 60 => universal_ring(&mut builder),
        RgbRenderFamily::UniversalScreen => {
            screen_edge(&mut builder, usize::from(profile.led_count), 4.0)
        }
        RgbRenderFamily::Strimer => {
            let per_row = if matches!(profile.led_count, 116 | 174) {
                29
            } else {
                22
            };
            strip_rows(&mut builder, profile.led_count, per_row);
        }
        RgbRenderFamily::Lancool217 | RgbRenderFamily::LancoolV150 => {
            let per_row = usize::from(profile.led_count) / 2;
            strip_rows(&mut builder, profile.led_count, per_row);
        }
    }
    builder.finish(profile.led_count)
}

/// Frames as the device would play them, or `None` when the config drives LEDs directly.
pub fn render(
    profile: RgbRenderProfile,
    config: &RgbDeviceConfig,
) -> Result<Option<PreviewFrames>> {
    let Some(regions) = super::regions::resolve(config, profile)? else {
        return Ok(None);
    };
    let animation = super::family::render(profile, &regions)?;
    let step = animation.frames.len().div_ceil(MAX_PREVIEW_FRAMES).max(1);
    let interval_ms = animation.interval_hundredths.div_ceil(160).max(1) * step as u32;
    Ok(Some(PreviewFrames {
        frames: animation.frames.into_iter().step_by(step).collect(),
        interval_ms,
    }))
}

/// One frame of live zone colours, each zone stretched to `per_zone` LEDs when given.
pub fn still_frame(zones: &[Vec<[u8; 3]>], per_zone: Option<u16>, total: usize) -> PreviewFrames {
    let mut frame: Vec<[u8; 3]> = zones
        .iter()
        .flat_map(|colors| {
            let count = per_zone.map_or(colors.len(), usize::from);
            (0..count).map(move |i| {
                colors
                    .get(i * colors.len() / count.max(1))
                    .copied()
                    .unwrap_or([0; 3])
            })
        })
        .collect();
    frame.resize(total, [0; 3]);
    PreviewFrames {
        frames: vec![frame],
        interval_ms: 1_000,
    }
}

/// Approximates firmware per-fan effects by rendering each zone as its own one-fan device.
pub fn render_zones(
    family: RgbRenderFamily,
    leds_per_zone: u16,
    zones: &[RgbZoneConfig],
) -> Result<Option<PreviewFrames>> {
    if zones.is_empty() || zones.iter().any(|zone| zone.effect.mode == RgbMode::Direct) {
        return Ok(None);
    }
    let one_fan = RgbRenderProfile {
        family,
        fan_count: 1,
        led_count: leds_per_zone,
        right_attach: false,
    };
    let mut ordered: Vec<_> = zones.iter().collect();
    ordered.sort_by_key(|zone| zone.zone_index);
    let animations = ordered
        .iter()
        .map(|zone| {
            let region = RgbRegionConfig {
                effect: zone.effect.clone(),
                flip: zone.swap_tb,
            };
            super::family::render(one_fan, std::slice::from_ref(&region))
        })
        .collect::<Result<Vec<_>>>()?;
    let intervals: Vec<u32> = animations
        .iter()
        .map(|animation| animation.interval_hundredths.div_ceil(160).max(1))
        .collect();
    let base = intervals.iter().copied().min().unwrap_or(1);
    let duration = animations
        .iter()
        .zip(&intervals)
        .map(|(animation, &interval)| animation.frames.len() as u32 * interval)
        .max()
        .unwrap_or(base);
    let count = (duration / base).clamp(1, MAX_PREVIEW_FRAMES as u32);
    let interval_ms = duration.div_ceil(count).max(1);
    let frames = (0..count)
        .map(|frame| {
            let at = frame * interval_ms;
            animations
                .iter()
                .zip(&intervals)
                .flat_map(|(animation, &interval)| {
                    let index = (at / interval) as usize % animation.frames.len();
                    animation.frames[index].iter().copied()
                })
                .collect()
        })
        .collect();
    Ok(Some(PreviewFrames {
        frames,
        interval_ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::{RgbEffect, RgbScope};

    fn profile(family: RgbRenderFamily, fan_count: u8, led_count: u16) -> RgbRenderProfile {
        RgbRenderProfile {
            family,
            fan_count,
            led_count,
            right_attach: false,
        }
    }

    #[test]
    fn layouts_place_every_physical_led() {
        for profile in [
            profile(RgbRenderFamily::Tl, 3, 78),
            profile(RgbRenderFamily::Sl, 4, 160),
            profile(RgbRenderFamily::SlV4, 2, 104),
            profile(RgbRenderFamily::SlInf, 3, 132),
            profile(RgbRenderFamily::Cl, 3, 72),
            profile(RgbRenderFamily::P28, 3, 27),
            profile(RgbRenderFamily::HydroShiftII, 3, 96),
            profile(RgbRenderFamily::HydroShiftII, 0, 24),
            profile(RgbRenderFamily::HydroShiftIIOled, 0, 35),
            profile(RgbRenderFamily::UniversalScreen, 0, 60),
            profile(RgbRenderFamily::Strimer, 0, 174),
            profile(RgbRenderFamily::Lancool217, 0, 96),
        ] {
            let layout = layout(profile).unwrap_or_else(|| panic!("{profile:?}"));
            assert_eq!(layout.leds.len(), usize::from(profile.led_count));
            assert!(layout
                .leds
                .iter()
                .all(|led| usize::from(led.element) < layout.elements.len()
                    && (0.0..=1.0).contains(&led.x)
                    && (0.0..=1.0).contains(&led.y)));
        }
    }

    #[test]
    fn leds_sharing_a_logical_position_line_up() {
        use crate::rgb::sync_layout;
        for (profile, map, skip) in [
            (
                profile(RgbRenderFamily::Tl, 1, 26),
                sync_layout::tl_fan_map(),
                0,
            ),
            (
                profile(RgbRenderFamily::Sl, 1, 40),
                sync_layout::sl_fan_map(),
                0,
            ),
            (
                profile(RgbRenderFamily::SlInf, 1, 44),
                sync_layout::sl_inf_fan_map(false),
                8,
            ),
            (
                profile(RgbRenderFamily::UniversalScreen, 0, 60),
                sync_layout::universal_screen_map(),
                0,
            ),
        ] {
            let leds = layout(profile).unwrap().leds;
            for (i, a) in map.iter().enumerate().skip(skip) {
                for (j, b) in map.iter().enumerate().skip(skip) {
                    if a == b {
                        assert!((leds[i].x - leds[j].x).abs() < 1e-6, "{profile:?} {i} {j}");
                    }
                }
            }
        }
    }

    #[test]
    fn tl_bars_are_symmetric_with_leds_on_both_corners() {
        let leds = layout(profile(RgbRenderFamily::Tl, 1, 26)).unwrap().leds;
        for bar in [&leds[..13], &leds[13..]] {
            assert!((bar[6].x - 0.5).abs() < 1e-6);
            assert!((bar[3].y - bar[6].y).abs() < 1e-6);
            assert!((bar[9].y - bar[6].y).abs() < 1e-6);
            for (left, right) in bar.iter().zip(bar.iter().rev()) {
                assert!((left.x + right.x - 1.0).abs() < 1e-6 && (left.y - right.y).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn direct_colours_stretch_each_zone_to_the_drawn_fan() {
        let red = [255, 0, 0];
        let blue = [0, 0, 255];
        let frame = &still_frame(&[vec![red; 20], vec![blue; 20]], Some(26), 52).frames[0];
        assert_eq!(frame.len(), 52);
        assert!(frame[..26].iter().all(|c| *c == red));
        assert!(frame[26..].iter().all(|c| *c == blue));
    }

    #[test]
    fn firmware_zones_render_side_by_side_in_zone_order() {
        let zone = |zone_index: u8, color: [u8; 3]| RgbZoneConfig {
            zone_index,
            effect: RgbEffect {
                mode: RgbMode::Static,
                colors: vec![color],
                ..RgbEffect::default()
            },
            swap_lr: false,
            swap_tb: false,
        };
        let zones = [zone(1, [0, 0, 255]), zone(0, [255, 0, 0])];
        let frames = render_zones(RgbRenderFamily::Tl, 26, &zones)
            .unwrap()
            .unwrap();
        let frame = &frames.frames[0];
        assert_eq!(frame.len(), 52);
        assert!(frame[..26].iter().all(|c| c[0] > 0 && c[2] == 0));
        assert!(frame[26..].iter().all(|c| c[2] > 0 && c[0] == 0));
    }

    #[test]
    fn tl_fans_light_only_their_own_arcs_for_per_fan_static() {
        let tl = profile(RgbRenderFamily::Tl, 3, 78);
        let red = [255, 0, 0];
        let white = [255, 255, 255];
        let config = RgbDeviceConfig {
            device_id: "wireless:test".into(),
            fan_led_count: None,
            mb_rgb_sync: false,
            active_preset: None,
            zones: Vec::new(),
            effect_memory: Vec::new(),
            regions: Some(vec![RgbRegionConfig {
                effect: RgbEffect {
                    mode: RgbMode::Static,
                    colors: vec![red, white, white],
                    scope: RgbScope::All,
                    ..RgbEffect::default()
                },
                flip: false,
            }]),
        };
        let frames = render(tl, &config).unwrap().unwrap();
        let layout = layout(tl).unwrap();
        let frame = &frames.frames[0];
        for (led, color) in layout.leds.iter().zip(frame) {
            let lit_red = color[1] == 0 && color[0] > 0;
            assert_eq!(lit_red, led.element == 0, "{led:?} {color:?}");
        }
    }
}

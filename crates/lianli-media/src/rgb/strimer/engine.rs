pub(super) use crate::rgb::color::scale_byte as scale;
use anyhow::{bail, Result};
use lianli_shared::rgb::{is_brightness_off, RgbEffect, RgbScope};

pub(super) type Color = [u8; 3];
pub(super) type Frame = Vec<Color>;

#[derive(Clone, Copy)]
pub(super) struct Geometry {
    pub led_count: usize,
    pub lanes: usize,
    pub lane_length: usize,
}

pub(super) fn geometry(led_count: usize) -> Result<Geometry> {
    Ok(match led_count {
        88 => Geometry {
            led_count,
            lanes: 4,
            lane_length: 22,
        },
        116 => Geometry {
            led_count,
            lanes: 4,
            lane_length: 29,
        },
        132 => Geometry {
            led_count,
            lanes: 6,
            lane_length: 22,
        },
        174 => Geometry {
            led_count,
            lanes: 6,
            lane_length: 29,
        },
        _ => bail!("Strimer effects require 88, 116, 132 or 174 LEDs"),
    })
}

pub(super) fn lane(scope: RgbScope) -> Option<usize> {
    match scope {
        RgbScope::Segment1 => Some(0),
        RgbScope::Segment2 => Some(1),
        RgbScope::Segment3 => Some(2),
        RgbScope::Segment4 => Some(3),
        RgbScope::Segment5 => Some(4),
        RgbScope::Segment6 => Some(5),
        _ => None,
    }
}

pub(super) fn brightness(effect: &RgbEffect) -> Result<u8> {
    if is_brightness_off(effect.brightness) {
        return Ok(0);
    }
    [0, 64, 128, 192, 255]
        .get(effect.brightness as usize)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("invalid RGB brightness"))
}

pub(super) fn colors(effect: &RgbEffect) -> [Color; 6] {
    let mut colors = [
        [255, 0, 0],
        [0, 255, 0],
        [0, 0, 255],
        [255, 255, 0],
        [0, 255, 255],
        [255, 0, 255],
    ];
    if effect.colors.iter().flatten().any(|&channel| channel != 0) {
        for (target, source) in colors.iter_mut().zip(&effect.colors) {
            *target = clamp(*source);
        }
    }
    colors
}

fn clamp(color: Color) -> Color {
    crate::rgb::color::limit_current(color, 600)
}

pub(super) fn frame(geometry: Geometry) -> Frame {
    vec![[0; 3]; geometry.led_count]
}

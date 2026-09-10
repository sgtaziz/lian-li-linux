pub(crate) use crate::rgb::color::scale;
use anyhow::{ensure, Result};
use lianli_shared::rgb::{is_brightness_off, RgbEffect};

pub(crate) type Color = [u8; 3];

pub(crate) const CENTER_ORDER: [usize; 32] = [
    2, 3, 1, 4, 8, 5, 7, 6, 10, 11, 9, 12, 16, 13, 15, 14, 18, 19, 17, 20, 24, 21, 23, 22, 26, 27,
    25, 28, 32, 29, 31, 30,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Plane {
    Center,
    Outer,
}

pub(crate) struct PlaneAnimation {
    pub frames: Vec<Vec<Color>>,
    pub interval_ticks: u32,
    pub interval_base_ticks: u32,
}

pub(crate) fn validate(effect: &RgbEffect, fans: usize) -> Result<()> {
    ensure!((1..=4).contains(&fans), "CL fan count must be 1..=4");
    ensure!(
        effect.colors.len() <= 4,
        "CL palette supports at most four colors"
    );
    ensure!(
        effect.brightness <= 4 || is_brightness_off(effect.brightness),
        "RGB brightness must be 0..=4"
    );
    ensure!(effect.speed <= 4, "RGB speed must be 0..=4");
    Ok(())
}

pub(crate) fn palette(effect: &RgbEffect, plane: Plane) -> [Color; 4] {
    palette_with_defaults(
        effect,
        match plane {
            Plane::Center => [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]],
            Plane::Outer => [[255, 0, 0], [0, 0, 255], [0, 255, 0], [255, 255, 0]],
        },
    )
}

pub(crate) fn all_palette(effect: &RgbEffect) -> [Color; 4] {
    palette_with_defaults(
        effect,
        [[255, 0, 0], [255, 255, 0], [255, 0, 255], [255, 255, 0]],
    )
}

fn palette_with_defaults(effect: &RgbEffect, mut colors: [Color; 4]) -> [Color; 4] {
    let prepared: Vec<_> = effect.colors.iter().copied().map(clamp_current).collect();
    if prepared.iter().flatten().any(|&channel| channel != 0) {
        for (target, source) in colors.iter_mut().zip(prepared) {
            *target = source;
        }
    }
    colors
}

pub(crate) fn clamp_current(color: Color) -> Color {
    crate::rgb::color::limit_current(color, 600)
}

pub(crate) fn mixed_color(first: Color, second: Color) -> Color {
    clamp_current(saturated_color(first, second))
}

pub(crate) fn saturated_color(first: Color, second: Color) -> Color {
    std::array::from_fn(|index| first[index].saturating_add(second[index]))
}

pub(crate) fn brightness(effect: &RgbEffect) -> u16 {
    if is_brightness_off(effect.brightness) {
        0
    } else {
        [0, 64, 128, 192, 255][effect.brightness as usize]
    }
}

pub(crate) fn place(track: &[Color], plane: Plane, fans: usize) -> Vec<Color> {
    debug_assert_eq!(track.len(), fans * 8);
    let mut output = vec![[0; 3]; fans * 24];
    for fan in 0..fans {
        let source = &track[fan * 8..(fan + 1) * 8];
        let base = fan * 24;
        match plane {
            Plane::Center => output[base..base + 8].copy_from_slice(source),
            Plane::Outer => {
                for (target, color) in output[base + 8..base + 16]
                    .iter_mut()
                    .zip(source.iter().rev())
                {
                    *target = *color;
                }
                output.copy_within(base + 8..base + 16, base + 16);
            }
        }
    }
    output
}

pub(crate) fn place_outer_banks(first: &[Color], second: &[Color], fans: usize) -> Vec<Color> {
    debug_assert_eq!(first.len(), fans * 8);
    debug_assert_eq!(second.len(), fans * 8);
    let mut output = vec![[0; 3]; fans * 24];
    for fan in 0..fans {
        let base = fan * 24;
        let range = fan * 8..(fan + 1) * 8;
        for (target, color) in output[base + 8..base + 16]
            .iter_mut()
            .zip(first[range.clone()].iter().rev())
        {
            *target = *color;
        }
        for (target, color) in output[base + 16..base + 24]
            .iter_mut()
            .zip(second[range].iter().rev())
        {
            *target = *color;
        }
    }
    output
}

pub(crate) fn place_both(center: &[Color], outer: &[Color], fans: usize) -> Vec<Color> {
    let mut output = place(center, Plane::Center, fans);
    let outer = place(outer, Plane::Outer, fans);
    for fan in 0..fans {
        let base = fan * 24;
        output[base + 8..base + 24].copy_from_slice(&outer[base + 8..base + 24]);
    }
    output
}

pub(crate) fn project_p28(frame: &[Color], fans: usize) -> Vec<Color> {
    let mut output = Vec::with_capacity(fans * 9);
    for fan in 0..fans {
        let center = &frame[fan * 24..fan * 24 + 8];
        output.extend_from_slice(center);
        output.push(center[7]);
    }
    output
}

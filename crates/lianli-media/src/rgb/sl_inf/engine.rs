use anyhow::{ensure, Result};
use lianli_shared::rgb::RgbEffect;

pub(super) type Color = [u8; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Plane {
    Outer,
    Inner,
    Center,
}

impl Plane {
    pub(super) fn track_len(self) -> usize {
        match self {
            Self::Outer | Self::Center => 8,
            Self::Inner => 10,
        }
    }
}

pub(super) fn validate(effect: &RgbEffect, fans: usize) -> Result<()> {
    ensure!((1..=4).contains(&fans), "SL-INF fan count must be 1..=4");
    ensure!(
        effect.colors.len() <= 4,
        "SL-INF palette supports at most four colors"
    );
    ensure!(effect.brightness <= 4, "RGB brightness must be 0..=4");
    ensure!(effect.speed <= 4, "RGB speed must be 0..=4");
    Ok(())
}

pub(super) fn palette(effect: &RgbEffect, plane: Plane) -> [Color; 4] {
    let mut colors = match plane {
        Plane::Outer => [[255, 0, 0], [0, 0, 255], [0, 255, 0], [255, 255, 0]],
        Plane::Inner => [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]],
        Plane::Center => [[255, 0, 0], [255, 255, 0], [255, 0, 255], [255, 255, 0]],
    };
    let adjusted: Vec<_> = effect.colors.iter().copied().map(clamp_current).collect();
    if adjusted.iter().flatten().any(|&channel| channel != 0) {
        for (target, source) in colors.iter_mut().zip(adjusted) {
            *target = source;
        }
    }
    colors
}

pub(super) fn drumming_palette(effect: &RgbEffect) -> [Color; 4] {
    let mut colors = [[255, 0, 255], [0, 0, 255], [0, 255, 0], [255, 255, 0]];
    let adjusted: Vec<_> = effect.colors.iter().copied().map(clamp_current).collect();
    if adjusted.iter().flatten().any(|&channel| channel != 0) {
        for (target, source) in colors.iter_mut().zip(adjusted) {
            *target = source;
        }
    }
    colors
}

pub(super) fn clamp_current(mut color: Color) -> Color {
    while color.iter().map(|&channel| u16::from(channel)).sum::<u16>() > 600 {
        color = color.map(|channel| (f64::from(channel) * 0.95) as u8);
    }
    color
}

pub(super) fn mixed_color(first: Color, second: Color) -> Color {
    clamp_current(std::array::from_fn(|channel| {
        first[channel].saturating_add(second[channel])
    }))
}

pub(super) fn center_order(index: usize, pn: u8) -> usize {
    const PN0: [usize; 32] = [
        2, 3, 1, 4, 0, 5, 7, 6, 10, 11, 9, 12, 8, 13, 15, 14, 18, 19, 17, 20, 16, 21, 23, 22, 26,
        27, 25, 28, 24, 29, 31, 30,
    ];
    const PN1: [usize; 32] = [
        6, 7, 5, 0, 4, 1, 3, 2, 14, 15, 13, 8, 12, 9, 11, 10, 22, 23, 21, 16, 20, 17, 19, 18, 30,
        31, 29, 24, 28, 25, 27, 26,
    ];
    if pn == 0 {
        PN0[index]
    } else {
        PN1[index]
    }
}

pub(super) fn brightness(effect: &RgbEffect) -> u16 {
    [0, 64, 128, 192, 255][effect.brightness as usize]
}

pub(super) fn scale(color: Color, factor: u16) -> Color {
    color.map(|channel| ((u16::from(channel) * factor) >> 8) as u8)
}

pub(super) fn place(
    track: &[Color],
    plane: Plane,
    fans: usize,
    pn: u8,
    attachment_order: bool,
) -> Vec<Color> {
    let per_fan = plane.track_len();
    debug_assert_eq!(track.len(), fans * per_fan);
    let mut frame = vec![[0; 3]; fans * 44];
    for fan in 0..fans {
        let source = &track[fan * per_fan..(fan + 1) * per_fan];
        let mut copy = |start: usize| {
            let destination = &mut frame[fan * 44 + start..fan * 44 + start + per_fan];
            if attachment_order && pn == 0 {
                for (target, color) in destination.iter_mut().zip(source.iter().rev()) {
                    *target = *color;
                }
            } else {
                destination.copy_from_slice(source);
            }
        };
        match plane {
            Plane::Center => copy(0),
            Plane::Inner => {
                copy(8);
                copy(26);
            }
            Plane::Outer => {
                copy(18);
                copy(36);
            }
        }
    }
    frame
}

pub(super) fn place_dual(
    first: &[Color],
    second: &[Color],
    plane: Plane,
    fans: usize,
    pn: u8,
) -> Vec<Color> {
    debug_assert_ne!(plane, Plane::Center);
    let per_fan = plane.track_len();
    debug_assert_eq!(first.len(), fans * per_fan);
    debug_assert_eq!(second.len(), fans * per_fan);
    let starts = match plane {
        Plane::Outer => [18, 36],
        Plane::Inner => [8, 26],
        Plane::Center => unreachable!(),
    };
    let mut frame = vec![[0; 3]; fans * 44];
    for fan in 0..fans {
        for (track, start) in [(first, starts[0]), (second, starts[1])] {
            let source = &track[fan * per_fan..(fan + 1) * per_fan];
            let destination = &mut frame[fan * 44 + start..fan * 44 + start + per_fan];
            if pn == 0 {
                for (target, color) in destination.iter_mut().zip(source.iter().rev()) {
                    *target = *color;
                }
            } else {
                destination.copy_from_slice(source);
            }
        }
    }
    frame
}

pub(super) fn copy_plane(target: &mut [Color], source: &[Color], plane: Plane) {
    let target = target.as_chunks_mut::<44>().0;
    let source = source.as_chunks::<44>().0;
    for (target, source) in target.iter_mut().zip(source) {
        match plane {
            Plane::Center => target[..8].copy_from_slice(&source[..8]),
            Plane::Inner => {
                target[8..18].copy_from_slice(&source[8..18]);
                target[26..36].copy_from_slice(&source[26..36]);
            }
            Plane::Outer => {
                target[18..26].copy_from_slice(&source[18..26]);
                target[36..44].copy_from_slice(&source[36..44]);
            }
        }
    }
}

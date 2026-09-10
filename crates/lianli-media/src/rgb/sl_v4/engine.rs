use anyhow::{ensure, Result};
use lianli_shared::rgb::RgbEffect;

pub(super) type Color = [u8; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    Inner,
    Outer,
}

impl Side {
    pub fn leds_per_track(self) -> usize {
        13
    }
}

pub(super) fn validate(effect: &RgbEffect, fans: usize) -> Result<()> {
    ensure!((1..=4).contains(&fans), "SL V4 fan count must be 1..=4");
    ensure!(
        effect.colors.len() <= 4,
        "SL V4 palette supports at most four colors"
    );
    ensure!(effect.brightness <= 4, "RGB brightness must be 0..=4");
    Ok(())
}

pub(super) fn palette(effect: &RgbEffect, side: Side) -> [Color; 4] {
    let defaults = match side {
        Side::Inner => [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]],
        Side::Outer => [[255, 0, 0], [0, 0, 255], [0, 255, 0], [255, 255, 0]],
    };
    palette_with_defaults(effect, defaults)
}

pub(super) fn palette_all(effect: &RgbEffect) -> [Color; 4] {
    palette_with_defaults(
        effect,
        [[255, 0, 0], [255, 255, 0], [255, 0, 255], [255, 255, 0]],
    )
}

fn palette_with_defaults(effect: &RgbEffect, defaults: [Color; 4]) -> [Color; 4] {
    let mut colors = defaults;
    let mut sum = 0u32;
    for (target, source) in colors.iter_mut().zip(&effect.colors) {
        *target = clamp_current(*source);
        sum += target
            .iter()
            .map(|&channel| u32::from(channel))
            .sum::<u32>();
    }
    if sum == 0 {
        defaults
    } else {
        colors
    }
}

pub(super) fn clamp_current(mut color: Color) -> Color {
    while color.iter().map(|&channel| u16::from(channel)).sum::<u16>() > 600 {
        color = color.map(|channel| (f64::from(channel) * 0.95) as u8);
    }
    color
}

pub(super) fn brightness(effect: &RgbEffect) -> u16 {
    [0, 64, 128, 192, 255][effect.brightness as usize]
}

pub(super) fn scale(color: Color, factor: u16) -> Color {
    color.map(|channel| ((u16::from(channel) * factor) >> 8) as u8)
}

pub(super) fn place(track: &[Color], side: Side, fans: usize) -> Vec<Color> {
    place_banks(track, track, side, fans)
}

pub(super) fn place_banks(
    first_bank: &[Color],
    second_bank: &[Color],
    side: Side,
    fans: usize,
) -> Vec<Color> {
    let per_fan = side.leds_per_track();
    debug_assert_eq!(first_bank.len(), fans * per_fan);
    debug_assert_eq!(second_bank.len(), fans * per_fan);
    let mut frame = vec![[0; 3]; fans * 52];
    for fan in 0..fans {
        let first = &first_bank[fan * per_fan..(fan + 1) * per_fan];
        let second = &second_bank[fan * per_fan..(fan + 1) * per_fan];
        let base = fan * 26;
        match side {
            Side::Inner => {
                frame[base..base + 13].copy_from_slice(first);
                frame[base + 26..base + 39].copy_from_slice(second);
            }
            Side::Outer => {
                frame[base + 13..base + 26].copy_from_slice(first);
                frame[base + 39..base + 52].copy_from_slice(second);
            }
        }
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_each_track_to_the_two_sl_v4_banks() {
        let track: Vec<Color> = (0..13).map(|value| [value, 0, 0]).collect();
        let frame = place(&track, Side::Outer, 1);
        assert_eq!(&frame[13..26], track);
        assert_eq!(&frame[39..52], track);
        assert!(frame[..13].iter().all(|color| *color == [0; 3]));
    }

    #[test]
    fn partial_user_palettes_retain_the_native_default_slots() {
        let effect = RgbEffect {
            colors: vec![[12, 34, 56]],
            ..Default::default()
        };
        assert_eq!(
            palette(&effect, Side::Outer),
            [[12, 34, 56], [0, 0, 255], [0, 255, 0], [255, 255, 0]]
        );
        let black = RgbEffect {
            colors: vec![[0, 0, 0]],
            ..Default::default()
        };
        assert_eq!(
            palette(&black, Side::Inner),
            [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]]
        );
    }
}

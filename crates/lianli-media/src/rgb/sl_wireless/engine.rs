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
        match self {
            Self::Inner => 12,
            Self::Outer => 8,
        }
    }
}

pub(super) fn validate(effect: &RgbEffect, fans: usize) -> Result<()> {
    ensure!(
        (1..=4).contains(&fans),
        "SL Wireless fan count must be 1..=4"
    );
    ensure!(
        effect.colors.len() <= 4,
        "SL Wireless palette supports at most four colors"
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
    let mut colors = [[0; 3]; 4];
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
    let mut frame = vec![[0; 3]; fans * 40];
    for fan in 0..fans {
        let first = &first_bank[fan * per_fan..(fan + 1) * per_fan];
        let second = &second_bank[fan * per_fan..(fan + 1) * per_fan];
        let base = fan * 40;
        match side {
            Side::Inner => {
                frame[base..base + 12].copy_from_slice(first);
                frame[base + 20..base + 32].copy_from_slice(second);
            }
            Side::Outer => {
                frame[base + 12..base + 20].copy_from_slice(first);
                frame[base + 32..base + 40].copy_from_slice(second);
            }
        }
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_each_linear_track_to_both_physical_led_banks() {
        let track: Vec<Color> = (0..8).map(|value| [value, 0, 0]).collect();
        let frame = place(&track, Side::Outer, 1);
        assert_eq!(&frame[12..20], track);
        assert_eq!(&frame[32..40], track);
        assert!(frame[..12].iter().all(|color| *color == [0; 3]));
    }

    #[test]
    fn palette_uses_vendor_defaults_only_when_the_whole_input_is_black() {
        let mut effect = RgbEffect {
            colors: vec![[0; 3]; 4],
            ..Default::default()
        };
        assert_eq!(palette(&effect, Side::Outer)[1], [0, 0, 255]);
        assert_eq!(palette(&effect, Side::Inner)[1], [0, 255, 0]);
        assert_eq!(palette_all(&effect)[1], [255, 255, 0]);
        effect.colors[2] = [255; 3];
        assert_eq!(palette(&effect, Side::Outer)[2], [195; 3]);
        assert_eq!(palette(&effect, Side::Outer)[0], [0; 3]);
    }
}

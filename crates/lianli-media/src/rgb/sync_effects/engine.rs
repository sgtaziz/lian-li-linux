pub(super) use crate::rgb::color::scale;
use anyhow::{ensure, Result};
use lianli_shared::rgb::{is_brightness_off, RgbEffect};

pub(super) type Color = [u8; 3];
pub(super) type Frames = Vec<Vec<Color>>;

pub(super) fn validate(effect: &RgbEffect, leds: usize) -> Result<()> {
    ensure!(
        (24..=720).contains(&leds),
        "sync lighting requires 24..=720 logical LEDs"
    );
    ensure!(
        effect.colors.len() <= 6,
        "sync lighting supports at most six colors"
    );
    ensure!(effect.speed <= 4, "RGB speed must be 0..=4");
    ensure!(
        effect.brightness <= 4 || is_brightness_off(effect.brightness),
        "invalid RGB brightness"
    );
    Ok(())
}

pub(super) fn palette(effect: &RgbEffect) -> [Color; 6] {
    let mut colors = [[0; 3]; 6];
    for (target, source) in colors.iter_mut().zip(&effect.colors) {
        *target = clamp_current(*source);
    }
    colors
}

fn clamp_current(color: Color) -> Color {
    crate::rgb::color::limit_current(color, 600)
}

pub(super) fn brightness(effect: &RgbEffect) -> u16 {
    if is_brightness_off(effect.brightness) {
        0
    } else {
        [0, 64, 128, 192, 255][effect.brightness as usize]
    }
}

pub(super) fn solid_frame(leds: usize, color: Color) -> Vec<Color> {
    vec![color; leds]
}

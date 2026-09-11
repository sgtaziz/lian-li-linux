use super::engine::{brightness, palette, place, scale, Color, Plane, CENTER_ORDER};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn rainbow(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let len = fans * 8;
    let stride = [12, 6, 4, 3][fans - 1];
    let bright = brightness(effect);
    let clockwise = !matches!(effect.direction, RgbDirection::CounterClockwise);
    (0..len)
        .map(|frame| {
            let mut source = frame * stride;
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let mut target = if clockwise {
                    len - position - 1
                } else {
                    position
                };
                if plane == Plane::Center {
                    target = CENTER_ORDER[target] - 1;
                }
                track[target] = scale(rainbow_color(source), bright);
                source = (source + stride) % 96;
            }
            place(&track, plane, fans)
        })
        .collect()
}

fn rainbow_color(index: usize) -> Color {
    let pulse = |position: usize| -> u8 {
        match position % 96 {
            0 => 255,
            position @ 1..=31 => (256 - position * 8) as u8,
            32..=64 => 0,
            position => ((position - 64) * 8) as u8,
        }
    };
    [pulse(index), pulse(index + 64), pulse(index + 32)]
}

pub(super) fn rainbow_morph(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let bright = brightness(effect);
    let mut red = 255u8;
    let mut green = 0u8;
    let mut blue = 0u8;
    let mut source = Vec::with_capacity(255);
    for frame in 0..255 {
        source.push(place(
            &vec![scale([red, green, blue], bright); fans * 8],
            plane,
            fans,
        ));
        if frame < 85 {
            red = red.wrapping_sub(3);
            green = green.wrapping_add(3);
            blue = 0;
        } else if frame < 170 {
            red = 0;
            green = green.wrapping_sub(3);
            blue = blue.wrapping_add(3);
        } else {
            red = red.wrapping_add(3);
            green = 0;
            blue = blue.wrapping_sub(3);
        }
    }
    source.into_iter().step_by(2).take(127).collect()
}

pub(super) fn static_color(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let mut track = Vec::with_capacity(fans * 8);
    for color in colors.iter().take(fans) {
        track.extend(std::iter::repeat_n(scale(*color, bright), 8));
    }
    vec![place(&track, plane, fans); 30]
}

pub(super) fn breathing(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let mut level = 0u16;
    let mut frames = Vec::with_capacity(170);
    for frame in 0..170 {
        let intensity = (level * 3) & 0xff;
        let mut track = Vec::with_capacity(fans * 8);
        for color in colors.iter().take(fans) {
            track.extend(std::iter::repeat_n(
                scale(scale(*color, intensity), bright),
                8,
            ));
        }
        frames.push(place(&track, plane, fans));
        level = if frame >= 85 { level - 1 } else { level + 1 };
    }
    frames
}

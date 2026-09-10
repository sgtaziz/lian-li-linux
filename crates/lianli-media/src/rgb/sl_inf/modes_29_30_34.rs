use super::engine::{brightness, center_order, palette, place, place_dual, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn reflect(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    const TAIL: [u16; 8] = [255, 192, 168, 128, 96, 64, 32, 16];
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let len = fans * plane.track_len();
    let half = len / 2;
    let span = half + 2 * fans - 1;
    let mut frames = Vec::with_capacity(4 * (span + span.div_ceil(2)));
    for color in colors {
        for pass in 0..2 {
            let increment = if pass == 0 { 1 } else { 2 };
            for step in (0..span).step_by(increment) {
                let mut track = vec![[0; 3]; len];
                if pass == 0 {
                    for position in 0..half {
                        if position <= step && position + 2 * fans > step {
                            let value = scale(scale(color, TAIL[step - position]), bright);
                            let first = if plane == Plane::Center {
                                center_order(position, pn)
                            } else {
                                position
                            };
                            let second_logical = len - position - 1;
                            let second = if plane == Plane::Center {
                                center_order(second_logical, pn)
                            } else {
                                second_logical
                            };
                            track[first] = value;
                            track[second] = value;
                        }
                    }
                }
                frames.push(place(&track, plane, fans, pn, plane != Plane::Center));
            }
        }
    }
    frames
}

pub(super) fn gradient_ribbon(
    effect: &RgbEffect,
    fans: usize,
    plane: Plane,
    pn: u8,
) -> Vec<Vec<Color>> {
    let bright = brightness(effect);
    let reverse = matches!(effect.direction, RgbDirection::Clockwise);
    let width = plane.track_len();
    let len = fans * width;
    let offsets = match plane {
        Plane::Center => (8, 8),
        Plane::Inner => (4, 12),
        Plane::Outer => (0, 16),
    };
    let maximum = 4 * width;
    let mut frames = Vec::with_capacity(48);
    for frame in 0..48 {
        let make_track = |offset: usize| {
            let mut track = vec![[0; 3]; maximum];
            let mut sample = (frame + offset) % 48;
            for group in 0..8 {
                let value = scale(ribbon_color(sample), bright);
                track[group * width / 2..(group + 1) * width / 2].fill(value);
                sample = (sample + 1) % 48;
            }
            if reverse {
                track.reverse();
            }
            track.truncate(len);
            track
        };
        let first = make_track(offsets.0);
        if plane == Plane::Center {
            frames.push(place(&first, plane, fans, pn, true));
        } else {
            let second = make_track(offsets.1);
            frames.push(place_dual(&first, &second, plane, fans, pn));
        }
    }
    frames
}

fn ribbon_color(index: usize) -> Color {
    const COLORS: [[u8; 3]; 48] = [
        [255, 0, 0],
        [240, 16, 0],
        [224, 32, 0],
        [208, 48, 0],
        [192, 64, 0],
        [176, 80, 0],
        [160, 96, 0],
        [144, 112, 0],
        [128, 128, 0],
        [112, 144, 0],
        [96, 160, 0],
        [80, 176, 0],
        [64, 192, 0],
        [48, 208, 0],
        [32, 224, 0],
        [16, 240, 0],
        [0, 255, 0],
        [0, 240, 16],
        [0, 224, 32],
        [0, 208, 48],
        [0, 192, 64],
        [0, 176, 80],
        [0, 160, 96],
        [0, 144, 112],
        [0, 128, 128],
        [0, 112, 144],
        [0, 96, 160],
        [0, 80, 176],
        [0, 64, 192],
        [0, 48, 208],
        [0, 32, 224],
        [0, 16, 240],
        [0, 0, 255],
        [16, 0, 240],
        [32, 0, 224],
        [48, 0, 208],
        [64, 0, 192],
        [80, 0, 176],
        [96, 0, 160],
        [112, 0, 144],
        [128, 0, 128],
        [144, 0, 112],
        [160, 0, 96],
        [176, 0, 80],
        [192, 0, 64],
        [208, 0, 48],
        [224, 0, 32],
        [240, 0, 16],
    ];
    COLORS[index]
}

pub(super) fn candy_box(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    const OUTER: [[[u8; 3]; 4]; 2] = [
        [[127, 0, 255], [0, 255, 0], [127, 0, 255], [0, 255, 255]],
        [[255, 95, 0], [255, 0, 255], [207, 255, 0], [0, 255, 0]],
    ];
    const INNER: [[[u8; 3]; 4]; 2] = [
        [[0, 255, 0], [255, 0, 255], [0, 255, 255], [127, 0, 255]],
        [[255, 0, 255], [255, 95, 0], [0, 255, 0], [207, 255, 0]],
    ];
    const CENTER: [[[u8; 3]; 4]; 2] = [
        [[255, 0, 47], [0, 0, 255], [255, 0, 255], [0, 255, 0]],
        [[255, 255, 0], [0, 255, 0], [255, 31, 0], [255, 0, 255]],
    ];
    let colors = match plane {
        Plane::Outer => OUTER,
        Plane::Inner => INNER,
        Plane::Center => CENTER,
    };
    let bright = brightness(effect);
    let width = plane.track_len();
    let mut source = Vec::with_capacity(340);
    for pass_colors in &colors {
        for frame in 0..170 {
            let intensity = if frame > 85 {
                (170 - frame) * 3
            } else {
                frame * 3
            } as u16;
            let mut track = vec![[0; 3]; fans * width];
            for fan in 0..fans {
                let value = scale(scale(pass_colors[fan], intensity), bright);
                track[fan * width..(fan + 1) * width].fill(value);
            }
            source.push(place(&track, plane, fans, pn, true));
        }
    }
    source.into_iter().step_by(2).collect()
}

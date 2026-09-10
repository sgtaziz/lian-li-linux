use super::engine::{brightness, palette, place_both, saturated_color, scale, Color, Plane};
use lianli_shared::rgb::RgbEffect;

pub(super) fn wing(effect: &RgbEffect, fans: usize, _plane: Plane) -> Vec<Vec<Color>> {
    let center_colors = palette(effect, Plane::Center);
    let outer_colors = palette(effect, Plane::Outer);
    let bright = brightness(effect);
    let half = fans * 4;
    let end = half + 2 * fans - 1;
    let mut frames = Vec::with_capacity(2 * end);
    for pair in 0..2 {
        for step in 0..end {
            let mut center = vec![[0; 3]; fans * 8];
            center_wing(
                &mut center,
                center_colors[pair * 2],
                center_colors[pair * 2 + 1],
                bright,
                fans,
                step,
            );
            let mut outer = vec![[0; 3]; fans * 8];
            for position in 0..half {
                if position <= step && position + fans > step {
                    outer[half - position - 1] = scale(outer_colors[pair * 2], bright);
                    outer[half + position] = scale(outer_colors[pair * 2 + 1], bright);
                }
            }
            frames.push(place_both(&center, &outer, fans));
        }
    }
    frames
}

fn center_wing(
    track: &mut [Color],
    first: Color,
    second: Color,
    bright: u16,
    fans: usize,
    step: usize,
) {
    if fans == 1 {
        if step < 4 {
            track[4] = scale(saturated_color(first, second), bright);
            if step > 2 {
                track[3] = scale(first, bright);
                track[5] = scale(second, bright);
            }
        }
        return;
    }

    let half = fans * 4;
    let row = if step < half {
        Some(step)
    } else if step < half + fans - 1 {
        Some(half - 1)
    } else {
        None
    };
    if let Some(row) = row {
        for (position, target) in track.iter_mut().enumerate() {
            let marker = wing_marker(fans, row, position);
            *target = match marker {
                1 => scale(first, bright),
                2 => scale(second, bright),
                _ => [0; 3],
            };
        }
        if fans == 3 {
            track[12] = scale(saturated_color(first, second), bright);
            if step > 2 {
                track[11] = scale(first, bright);
            }
        }
    }
}

fn wing_marker(fans: usize, row: usize, position: usize) -> u8 {
    let (first, second) = match fans {
        2 => (
            (WING_4_FIRST[row] >> 8) & 0xffff,
            (WING_4_SECOND[row] >> 8) & 0xffff,
        ),
        3 => (WING_3_FIRST[row], WING_3_SECOND[row]),
        4 => (WING_4_FIRST[row], WING_4_SECOND[row]),
        _ => unreachable!(),
    };
    if first & (1 << position) != 0 {
        1
    } else if second & (1 << position) != 0 {
        2
    } else {
        0
    }
}

const WING_3_FIRST: [u32; 12] = [
    0x800, 0x800, 0xc00, 0xc00, 0xc40, 0xc40, 0xcc0, 0xcc0, 0xcc1, 0xcc1, 0xcc3, 0xcc3,
];

const WING_3_SECOND: [u32; 12] = [
    0x1000, 0x1000, 0x3000, 0x3000, 0x23000, 0x23000, 0x33000, 0x33000, 0x833000, 0x833000,
    0xc33000, 0xc33000,
];

const WING_4_FIRST: [u32; 16] = [
    0x2000, 0x2000, 0x3000, 0x3000, 0x3800, 0x3800, 0x3c00, 0x3c00, 0x3c40, 0x3c40, 0x3cc0, 0x3cc0,
    0x3cc1, 0x3cc1, 0x3cc3, 0x3cc3,
];

const WING_4_SECOND: [u32; 16] = [
    0x40000, 0x40000, 0xc0000, 0xc0000, 0x1c0000, 0x1c0000, 0x3c0000, 0x3c0000, 0x23c0000,
    0x23c0000, 0x33c0000, 0x33c0000, 0x833c0000, 0x833c0000, 0xc33c0000, 0xc33c0000,
];

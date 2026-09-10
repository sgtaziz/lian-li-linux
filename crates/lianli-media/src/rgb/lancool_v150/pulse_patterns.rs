use super::engine::{frame, range, scale, Color, Frame};
use lianli_shared::rgb::RgbScope;

const HEARTBEAT: [u8; 88] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 24, 33, 41, 50, 58, 67, 75, 84,
    92, 101, 109, 118, 126, 135, 143, 152, 160, 169, 177, 186, 194, 203, 211, 220, 228, 237, 245,
    255, 245, 237, 228, 220, 211, 203, 194, 186, 177, 169, 160, 152, 143, 135, 126, 118, 109, 101,
    92, 84, 75, 67, 58, 50, 41, 33, 24, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16,
];

pub(super) fn meteor_contest(
    scope: RgbScope,
    palettes: &[[Color; 4]; 4],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    const FRONT: [u8; 72] = contest_front_levels();
    const REAR: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 254];
    let active = range(scope);
    let width = active.len();
    let side = usize::from(scope == RgbScope::Rear);
    let levels = if side == 0 {
        FRONT.as_slice()
    } else {
        REAR.as_slice()
    };
    (0..width)
        .map(|shift| {
            let mut output = frame();
            for position in 0..width {
                let source = (shift + position) % width;
                let intensity = levels[source];
                let color = usize::from(intensity == 254);
                let destination = if reverse {
                    width - position - 1
                } else {
                    position
                };
                output[active.start + destination] =
                    scale(scale(palettes[side][color], intensity), brightness);
            }
            output
        })
        .collect()
}

const fn contest_front_levels() -> [u8; 72] {
    let mut levels = [0; 72];
    let mut index = 29;
    while index < 36 {
        levels[index] = 255;
        index += 1;
    }
    index = 65;
    while index < 72 {
        levels[index] = 254;
        index += 1;
    }
    levels
}

pub(super) fn return_arc(
    scope: RgbScope,
    palettes: &[[Color; 4]; 4],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let active = range(scope);
    let width = active.len();
    let colors = palettes[usize::from(scope == RgbScope::Rear)];
    let mut frames = Vec::with_capacity(width * 8);
    for &color in &colors {
        for returning in [false, true] {
            for step in 0..width {
                let mut output = frame();
                for position in 0..width {
                    let lit = if returning {
                        position < width - step
                    } else {
                        position < step
                    };
                    let destination = if reverse {
                        width - position - 1
                    } else {
                        position
                    };
                    if lit {
                        output[active.start + destination] = scale(color, brightness);
                    }
                }
                frames.push(output);
            }
        }
    }
    frames
}

pub(super) fn heartbeat(scope: RgbScope, palettes: &[[Color; 4]; 4], brightness: u8) -> Vec<Frame> {
    let color = palettes[usize::from(scope == RgbScope::Rear)][0];
    (0..128)
        .map(|index| {
            let level = HEARTBEAT[if index < 88 { index } else { 0 }];
            vec![scale(scale(color, level), brightness); 88]
        })
        .collect()
}

pub(super) fn heartbeat_runway(palettes: &[[Color; 4]; 4], brightness: u8) -> Vec<Frame> {
    let color = palettes[0][0];
    (0..176)
        .map(|index| {
            let front = HEARTBEAT[if index < 88 { index } else { 0 }];
            let delayed = index.saturating_sub(32);
            let rear = HEARTBEAT[if delayed < 88 { delayed } else { 0 }];
            let mut output = frame();
            output[..72].fill(scale(scale(color, front), brightness));
            output[72..].fill(scale(scale(color, rear), brightness));
            output
        })
        .collect()
}

pub(super) fn disco(
    scope: RgbScope,
    palettes: &[[Color; 4]; 4],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let colors = palettes[usize::from(scope == RgbScope::Rear)];
    (0..16)
        .map(|shift| {
            let mut ring = [[0; 3]; 16];
            for position in 0..16 {
                let destination = if reverse { position } else { 15 - position };
                ring[destination] = scale(colors[((shift + position) % 16) / 4], brightness);
            }
            (0..88).map(|led| ring[led % 16]).collect()
        })
        .collect()
}

pub(super) fn candy_box(brightness: u8) -> Vec<Frame> {
    const FRONT: [Color; 2] = [[127, 0, 255], [255, 95, 0]];
    const REAR: [Color; 2] = [[255, 0, 47], [255, 255, 0]];
    let mut frames = Vec::with_capacity(340);
    for color in 0..2 {
        for step in 0..170 {
            let level = if step > 85 {
                (170 - step) * 3
            } else {
                step * 3
            } as u8;
            let mut output = frame();
            output[..72].fill(scale(scale(FRONT[color], level), brightness));
            output[72..].fill(scale(scale(REAR[color], level), brightness));
            frames.push(output);
        }
    }
    frames
}

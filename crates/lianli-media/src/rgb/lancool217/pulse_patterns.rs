use super::engine::{frame, range, scale, Color, Frame};
use lianli_shared::rgb::RgbScope;

const HEARTBEAT: [u8; 96] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 24, 32, 40, 48, 56, 64,
    72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 152, 160, 168, 176, 184, 192, 200, 208, 216, 224,
    232, 240, 248, 255, 248, 240, 232, 224, 216, 208, 200, 192, 184, 176, 168, 160, 152, 144, 136,
    128, 120, 112, 104, 96, 88, 80, 72, 64, 56, 48, 40, 32, 24, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
];

pub(super) fn meteor_contest(
    scope: RgbScope,
    palettes: &[[Color; 4]; 4],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let active = range(scope);
    let width = active.len();
    let side = usize::from(scope == RgbScope::Rear);
    let head = if side == 0 { 8 } else { 1 };
    (0..width)
        .map(|shift| {
            let mut output = frame();
            for position in 0..width {
                let source = (shift + position) % width;
                let (color, intensity) = if source >= width - head {
                    (1, 254)
                } else if source >= width / 2 - head && source < width / 2 {
                    (0, 255)
                } else {
                    (0, 0)
                };
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
            let level = HEARTBEAT[if index < 96 { index } else { 0 }];
            vec![scale(scale(color, level), brightness); 96]
        })
        .collect()
}

pub(super) fn heartbeat_runway(palettes: &[[Color; 4]; 4], brightness: u8) -> Vec<Frame> {
    let color = palettes[0][0];
    (0..176)
        .map(|index| {
            let front = HEARTBEAT[if index < 96 { index } else { 0 }];
            let delayed = index.saturating_sub(32);
            let rear = HEARTBEAT[if delayed < 96 { delayed } else { 0 }];
            let mut output = frame();
            output[..80].fill(scale(scale(color, front), brightness));
            output[80..].fill(scale(scale(color, rear), brightness));
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
            (0..96).map(|led| ring[led % 16]).collect()
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
            output[..80].fill(scale(scale(FRONT[color], level), brightness));
            output[80..].fill(scale(scale(REAR[color], level), brightness));
            frames.push(output);
        }
    }
    frames
}

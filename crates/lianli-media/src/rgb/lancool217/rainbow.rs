use super::engine::{frame, range, scale, Color, Frame};
use lianli_shared::rgb::RgbScope;

const RAINBOW: [Color; 48] = [
    [255, 0, 0],
    [239, 15, 0],
    [223, 31, 0],
    [207, 47, 0],
    [191, 63, 0],
    [175, 79, 0],
    [159, 95, 0],
    [143, 111, 0],
    [127, 127, 0],
    [111, 143, 0],
    [95, 159, 0],
    [79, 175, 0],
    [63, 191, 0],
    [47, 207, 0],
    [31, 223, 0],
    [15, 239, 0],
    [0, 255, 0],
    [0, 239, 15],
    [0, 223, 31],
    [0, 207, 47],
    [0, 191, 63],
    [0, 175, 79],
    [0, 159, 95],
    [0, 143, 111],
    [0, 127, 127],
    [0, 111, 143],
    [0, 95, 159],
    [0, 79, 175],
    [0, 63, 191],
    [0, 47, 207],
    [0, 31, 223],
    [0, 15, 239],
    [0, 0, 255],
    [15, 0, 239],
    [31, 0, 223],
    [47, 0, 207],
    [63, 0, 191],
    [79, 0, 175],
    [95, 0, 159],
    [111, 0, 143],
    [127, 0, 127],
    [143, 0, 111],
    [159, 0, 95],
    [175, 0, 79],
    [191, 0, 63],
    [207, 0, 47],
    [223, 0, 31],
    [239, 0, 15],
];

pub(super) fn render(scope: RgbScope, brightness: u8, counter_clockwise: bool) -> Vec<Frame> {
    let active = range(scope);
    let mut frames = Vec::with_capacity(48);
    for shift in 0..16 {
        for sub_shift in 0..3 {
            let mut ring = [[0; 3]; 16];
            for position in 0..16 {
                let destination = if counter_clockwise {
                    position
                } else {
                    15 - position
                };
                ring[destination] = scale(
                    RAINBOW[((shift + 1) * 3 + position * 3 + sub_shift) % 48],
                    brightness,
                );
            }
            let mut output = frame();
            for led in active.clone() {
                output[led] = ring[led % 16];
            }
            frames.push(output);
        }
    }
    frames
}

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

pub(super) fn solid(scope: RgbScope, color: Color, brightness: u8) -> Vec<Frame> {
    let mut output = frame();
    for led in range(scope) {
        output[led] = scale(color, brightness);
    }
    vec![output]
}

pub(super) fn rainbow(scope: RgbScope, brightness: u8, counter_clockwise: bool) -> Vec<Frame> {
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

pub(super) fn morph(scope: RgbScope, brightness: u8) -> Vec<Frame> {
    let active = range(scope);
    let (mut red, mut green, mut blue) = (255i16, 0i16, 0i16);
    let mut source = Vec::with_capacity(255);
    for index in 0..255 {
        let color = scale([red as u8, green as u8, blue as u8], brightness);
        let mut output = frame();
        for led in active.clone() {
            output[led] = color;
        }
        source.push(output);
        if index < 85 {
            red -= 3;
            green += 3;
            blue = 0;
        } else if index < 170 {
            red = 0;
            green -= 3;
            blue += 3;
        } else {
            red += 3;
            green = 0;
            blue -= 3;
        }
    }
    (0..127).map(|index| source[index * 2].clone()).collect()
}

pub(super) fn breathing(scope: RgbScope, colors: &[Color; 4], brightness: u8) -> Vec<Frame> {
    let active = range(scope);
    let mut frames = Vec::with_capacity(680);
    for &color in colors {
        for descending in [false, true] {
            for step in 0..85 {
                let intensity = if descending { 255 - step * 3 } else { step * 3 } as u8;
                let color = scale(scale(color, intensity), brightness);
                let mut output = frame();
                for led in active.clone() {
                    output[led] = color;
                }
                frames.push(output);
            }
        }
    }
    frames
}

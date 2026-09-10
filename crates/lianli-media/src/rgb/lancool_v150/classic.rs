use super::engine::{frame, range, scale, Color, Frame};
use lianli_shared::rgb::RgbScope;

const RED: [u8; 54] = [
    255, 241, 227, 213, 199, 185, 171, 157, 143, 129, 115, 101, 87, 73, 59, 45, 31, 17, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 17, 31, 45, 59, 73, 87, 101, 115, 129, 143, 157,
    171, 185, 199, 213, 227, 241,
];
const GREEN: [u8; 54] = [
    0, 17, 31, 45, 59, 73, 87, 101, 115, 129, 143, 157, 171, 185, 199, 213, 227, 241, 255, 241,
    227, 213, 199, 185, 171, 157, 143, 129, 115, 101, 87, 73, 59, 45, 31, 17, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
const BLUE: [u8; 54] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 17, 31, 45, 59, 73, 87, 101, 115, 129,
    143, 157, 171, 185, 199, 213, 227, 241, 255, 241, 227, 213, 199, 185, 171, 157, 143, 129, 115,
    101, 87, 73, 59, 45, 31, 17,
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
    let mut frames = Vec::with_capacity(54);
    for shift in 0..18 {
        for sub_shift in 0..3 {
            let mut ring = [[0; 3]; 18];
            for position in 0..18 {
                let destination = if counter_clockwise {
                    position
                } else {
                    17 - position
                };
                let source = ((shift + 1) * 3 + position * 3 + sub_shift) % 54;
                ring[destination] = scale([RED[source], GREEN[source], BLUE[source]], brightness);
            }
            let mut output = frame();
            for (position, led) in active.clone().enumerate() {
                output[led] = ring[position % 18];
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

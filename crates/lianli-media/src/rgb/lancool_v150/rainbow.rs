use super::engine::{frame, range, scale, Frame};
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

pub(super) fn render(scope: RgbScope, brightness: u8, counter_clockwise: bool) -> Vec<Frame> {
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

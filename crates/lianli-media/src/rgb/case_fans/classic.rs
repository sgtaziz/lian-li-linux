use super::{
    palette::{scale, Color, Frame},
    Layout,
};
use lianli_shared::rgb::RgbScope;

pub(crate) fn solid(layout: Layout, scope: RgbScope, color: Color, brightness: u8) -> Vec<Frame> {
    let mut output = layout.frame();
    for led in layout.range(scope) {
        output[led] = scale(color, brightness);
    }
    vec![output]
}

pub(crate) fn morph(layout: Layout, scope: RgbScope, brightness: u8) -> Vec<Frame> {
    let active = layout.range(scope);
    let (mut red, mut green, mut blue) = (255i16, 0i16, 0i16);
    let mut source = Vec::with_capacity(255);
    for index in 0..255 {
        let color = scale([red as u8, green as u8, blue as u8], brightness);
        let mut output = layout.frame();
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

pub(crate) fn breathing(
    layout: Layout,
    scope: RgbScope,
    colors: &[Color; 4],
    brightness: u8,
) -> Vec<Frame> {
    let active = layout.range(scope);
    let mut frames = Vec::with_capacity(680);
    for &color in colors {
        for descending in [false, true] {
            for step in 0..85 {
                let intensity = if descending { 255 - step * 3 } else { step * 3 } as u8;
                let color = scale(scale(color, intensity), brightness);
                let mut output = layout.frame();
                for led in active.clone() {
                    output[led] = color;
                }
                frames.push(output);
            }
        }
    }
    frames
}

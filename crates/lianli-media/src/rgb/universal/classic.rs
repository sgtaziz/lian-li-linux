use super::geometry::{frame, scale, Color};

const RAINBOW_30: [Color; 30] = [
    [255, 0, 0],
    [229, 21, 0],
    [203, 47, 0],
    [177, 73, 0],
    [151, 99, 0],
    [125, 125, 0],
    [99, 151, 0],
    [73, 177, 0],
    [47, 203, 0],
    [21, 229, 0],
    [0, 255, 0],
    [0, 229, 21],
    [0, 203, 47],
    [0, 177, 73],
    [0, 151, 99],
    [0, 125, 125],
    [0, 99, 151],
    [0, 73, 177],
    [0, 47, 203],
    [0, 21, 229],
    [0, 0, 255],
    [21, 0, 229],
    [47, 0, 203],
    [73, 0, 177],
    [99, 0, 151],
    [125, 0, 125],
    [151, 0, 99],
    [177, 0, 73],
    [203, 0, 47],
    [229, 0, 21],
];

pub(super) fn rainbow(led_count: usize, brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    (0..30)
        .map(|shift| {
            let mut output = frame(led_count);
            for position in 0..30 {
                let destination = if reverse { position } else { 29 - position };
                let color = scale(RAINBOW_30[(shift + position) % 30], brightness);
                output[destination] = color;
                output[30 + destination] = color;
            }
            output
        })
        .collect()
}

pub(super) fn wave(
    led_count: usize,
    colors: &[Color],
    brightness: u8,
    reverse: bool,
) -> Vec<Vec<Color>> {
    const INTENSITY: [u8; 12] = [0, 0, 32, 64, 128, 255, 128, 64, 32, 0, 0, 0];
    let mut frames = Vec::with_capacity(colors.len() * 48);
    for color in colors {
        for _ in 0..4 {
            for shift in 0..12 {
                let mut segment = [[0; 3]; 12];
                for position in 0..12 {
                    let destination = if reverse { position } else { 11 - position };
                    segment[destination] = scale(
                        scale(*color, INTENSITY[(shift + position) % 12]),
                        brightness,
                    );
                }
                let mut output = frame(led_count);
                for repeat in 0..5 {
                    output[repeat * 12..repeat * 12 + 12].copy_from_slice(&segment);
                }
                frames.push(output);
            }
        }
    }
    frames
}

pub(super) fn solid(led_count: usize, color: Color, brightness: u8) -> Vec<Vec<Color>> {
    let mut output = frame(led_count);
    output[..60].fill(scale(color, brightness));
    vec![output]
}

pub(super) fn breathing(led_count: usize, color: Color, brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(170);
    for descending in [false, true] {
        for step in 0..85 {
            let intensity = if descending { 255 - step * 3 } else { step * 3 } as u8;
            let color = scale(scale(color, intensity), brightness);
            let mut output = frame(led_count);
            output[..60].fill(color);
            frames.push(output);
        }
    }
    frames
}

pub(super) fn morph(led_count: usize, brightness: u8) -> Vec<Vec<Color>> {
    let mut source = Vec::with_capacity(255);
    let (mut red, mut green, mut blue) = (255i16, 0i16, 0i16);
    for index in 0..255 {
        source.push(scale([red as u8, green as u8, blue as u8], brightness));
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
    (0..127)
        .map(|index| {
            let mut output = frame(led_count);
            output[..60].fill(source[index * 2]);
            output
        })
        .collect()
}

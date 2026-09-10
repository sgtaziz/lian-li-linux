use super::geometry::{frame, scale, Color, ACTIVE_LEDS};

const RAINBOW_36: [Color; 36] = [
    [255, 0, 0],
    [232, 0, 10],
    [209, 0, 25],
    [186, 0, 48],
    [163, 0, 71],
    [140, 0, 94],
    [117, 0, 117],
    [94, 0, 140],
    [71, 0, 163],
    [48, 0, 186],
    [25, 0, 209],
    [10, 0, 232],
    [0, 0, 255],
    [0, 10, 232],
    [0, 25, 209],
    [0, 48, 186],
    [0, 71, 163],
    [0, 94, 140],
    [0, 117, 117],
    [0, 140, 94],
    [0, 163, 71],
    [0, 186, 48],
    [0, 209, 25],
    [0, 232, 10],
    [0, 255, 0],
    [10, 232, 0],
    [25, 209, 0],
    [48, 186, 0],
    [71, 163, 0],
    [94, 140, 0],
    [117, 117, 0],
    [140, 94, 0],
    [163, 71, 0],
    [186, 48, 0],
    [209, 25, 0],
    [232, 10, 0],
];

pub(super) fn rainbow(brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    (0..35)
        .map(|shift| {
            let mut output = frame();
            for (position, target) in output.iter_mut().take(ACTIVE_LEDS).enumerate() {
                let source = if reverse { position } else { 34 - position };
                *target = scale(RAINBOW_36[(shift + source) % 36], brightness);
            }
            output
        })
        .collect()
}

pub(super) fn wave(colors: &[Color], brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    const INTENSITY: [u8; 12] = [0, 0, 16, 48, 128, 255, 128, 48, 16, 0, 0, 0];
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
                let mut output = frame();
                for position in 0..ACTIVE_LEDS {
                    output[position] = segment[position % 12];
                }
                frames.push(output);
            }
        }
    }
    frames
}

pub(super) fn solid(color: Color, brightness: u8) -> Vec<Vec<Color>> {
    let mut output = frame();
    output[..ACTIVE_LEDS].fill(scale(color, brightness));
    vec![output]
}

pub(super) fn breathing(color: Color, brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(170);
    for descending in [false, true] {
        for step in 0..85 {
            let intensity = if descending { 255 - step * 3 } else { step * 3 } as u8;
            let color = scale(scale(color, intensity), brightness);
            let mut output = frame();
            output[..ACTIVE_LEDS].fill(color);
            frames.push(output);
        }
    }
    frames
}

pub(super) fn morph(brightness: u8) -> Vec<Vec<Color>> {
    let (mut red, mut green, mut blue) = (255i16, 0i16, 0i16);
    (0..255)
        .map(|index| {
            let color = scale([red as u8, green as u8, blue as u8], brightness);
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
            let mut output = frame();
            output[..ACTIVE_LEDS].fill(color);
            output
        })
        .collect()
}

pub(super) fn rainbow_wave(brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    const MASK: [bool; 35] = [
        false, false, false, false, false, true, true, true, true, true, true, true, false, false,
        false, false, false, true, true, true, true, true, true, true, false, false, false, false,
        false, true, true, true, true, true, true,
    ];
    (0..35)
        .map(|shift| {
            let mut generated = [[0; 3]; 35];
            for position in 0..35 {
                if MASK[(shift + position) % 35] {
                    generated[position] =
                        scale(RAINBOW_36[(shift + position * 2) % 36], brightness);
                }
            }
            let mut output = frame();
            for (position, target) in output.iter_mut().take(35).enumerate() {
                let source = if reverse { position } else { 34 - position };
                *target = generated[source];
            }
            output
        })
        .collect()
}

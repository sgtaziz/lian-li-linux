use super::engine::{frame, scale, Color};

pub(super) fn runway(colors: &[Color; 4], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(54);
    for reverse in [false, true] {
        for step in 0..27 {
            let mut output = frame();
            for position in 0..24 {
                let color = if position <= step && position + 3 > step {
                    colors[0]
                } else {
                    colors[1]
                };
                let destination = if reverse { 23 - position } else { position };
                output[destination] = scale(color, brightness);
            }
            frames.push(output);
        }
    }
    frames
}

pub(super) fn meteor(colors: &[Color; 4], brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    const TAIL: [u8; 12] = [6, 8, 16, 24, 32, 48, 64, 96, 120, 150, 200, 255];
    let mut source = Vec::with_capacity(144);
    for color in colors {
        for step in 0..36 {
            let mut output = frame();
            let mut tail_index = 0;
            for position in 0..24 {
                let intensity = if position <= step && position + 12 > step {
                    let value = TAIL[tail_index];
                    tail_index += 1;
                    value
                } else {
                    0
                };
                let destination = if reverse { 23 - position } else { position };
                output[destination] = scale(scale(*color, intensity), brightness);
            }
            source.push(output);
        }
    }
    (0..60)
        .map(|index| source[(index as f64 * 2.4) as usize].clone())
        .collect()
}

pub(super) fn tai_chi(colors: &[Color; 4], brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    const INTENSITY: [u8; 24] = [
        255, 230, 205, 180, 155, 130, 105, 80, 55, 30, 20, 10, 255, 230, 205, 180, 155, 130, 105,
        80, 55, 30, 20, 10,
    ];
    (0..24)
        .map(|shift| {
            let mut output = frame();
            for position in 0..24 {
                let source = (shift + position) % 24;
                let destination = if reverse { position } else { 23 - position };
                output[destination] = scale(
                    scale(colors[usize::from(source >= 12)], INTENSITY[source]),
                    brightness,
                );
            }
            output
        })
        .collect()
}

pub(super) fn voice(color: Color, brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    let color = scale(color, brightness);
    let mut frames = Vec::with_capacity(48);
    for extent in [12, 8, 4] {
        for shrink in [false, true] {
            for step in 0..extent {
                let mut output = frame();
                for position in 0..12 {
                    let destination = if reverse { 11 - position } else { position };
                    let dark = if shrink {
                        position > extent - step
                    } else {
                        position > step
                    };
                    output[destination] = if dark { [0; 3] } else { color };
                }
                for position in 0..12 {
                    output[position + 12] = output[11 - position];
                }
                frames.push(output);
            }
        }
    }
    frames
}

pub(super) fn pump(color: Color, brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    const TAIL: [u8; 8] = [16, 24, 32, 64, 96, 128, 196, 255];
    let mut frames = Vec::with_capacity(20);
    for step in 0..20 {
        let mut output = frame();
        let mut tail_index = 0;
        for position in 0..12 {
            let intensity = if position <= step && position + 8 > step {
                let value = TAIL[tail_index];
                tail_index += 1;
                value
            } else {
                0
            };
            let destination = if reverse { 11 - position } else { position };
            output[destination] = scale(scale(color, intensity), brightness);
        }
        for position in 0..12 {
            output[position + 12] = output[11 - position];
        }
        frames.push(output);
    }
    frames
}

pub(super) fn bounce(colors: &[Color; 4], brightness: u8) -> Vec<Vec<Color>> {
    const TAIL: [u8; 8] = [16, 24, 32, 64, 96, 128, 196, 255];
    let mut frames = Vec::with_capacity(80);
    for (color_index, color) in colors.iter().enumerate() {
        for step in 0..20 {
            let mut output = frame();
            let mut tail_index = 0;
            for position in 0..12 {
                let intensity = if position <= step && position + 8 > step {
                    let value = TAIL[tail_index];
                    tail_index += 1;
                    value
                } else {
                    0
                };
                let destination = if color_index % 2 == 1 {
                    11 - position
                } else {
                    position
                };
                output[destination] = scale(scale(*color, intensity), brightness);
            }
            for position in 0..12 {
                output[position + 12] = output[11 - position];
            }
            frames.push(output);
        }
    }
    frames
}

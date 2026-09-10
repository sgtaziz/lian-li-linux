type Color = [u8; 3];

fn scale(color: Color, value: u16) -> Color {
    color.map(|channel| ((u16::from(channel) * value) >> 8) as u8)
}

pub(super) const METEOR_TAILS: [&[u16]; 4] = [
    &[32, 255],
    &[16, 64, 128, 255],
    &[8, 16, 32, 64, 128, 255],
    &[6, 10, 16, 32, 64, 96, 168, 255],
];

pub(super) fn runway(
    len: usize,
    head: usize,
    colors: [Color; 2],
    project: impl Fn(&[Color]) -> Vec<Color>,
) -> Vec<Vec<Color>> {
    let span = len + head - 1;
    let mut frames = Vec::with_capacity(span * 2);
    let mut track = vec![[0; 3]; len];
    for reverse in [false, true] {
        for step in 0..span {
            for position in 0..len {
                let color = if position <= step && position + head > step {
                    colors[0]
                } else {
                    colors[1]
                };
                track[if reverse {
                    len - position - 1
                } else {
                    position
                }] = color;
            }
            frames.push(project(&track));
        }
    }
    frames
}

pub(super) fn meteor(
    len: usize,
    colors: &[Color],
    tail: &[u16],
    bright: u16,
    reverse: bool,
    project: impl Fn(&[Color]) -> Vec<Color>,
) -> Vec<Vec<Color>> {
    let span = len + tail.len() - 1;
    let mut frames = Vec::with_capacity(span * colors.len());
    let mut track = vec![[0; 3]; len];
    for &color in colors {
        for step in 0..span {
            let mut tail_index = 0;
            for position in 0..len {
                let color = if position <= step && position + tail.len() > step {
                    let color = scale(scale(color, tail[tail_index]), bright);
                    tail_index += 1;
                    color
                } else {
                    [0; 3]
                };
                track[if reverse {
                    len - position - 1
                } else {
                    position
                }] = color;
            }
            frames.push(project(&track));
        }
    }
    frames
}

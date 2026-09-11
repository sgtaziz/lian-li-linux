use crate::rgb::color::{scale, Color};

pub(crate) fn mixing(
    track_len: usize,
    head: usize,
    colors: [Color; 4],
    mixed: Color,
    bright: u16,
    project: impl Fn(&[Color]) -> Vec<Color>,
) -> Vec<Vec<Color>> {
    let half = track_len / 2;
    let span = half + head - 1;
    let mut delay = 0;
    let mut frames = Vec::new();
    for phase in 0..3 {
        let mut step = 0;
        while step < span {
            if delay < 10 {
                delay += 1;
                step = 0;
            }
            let mut track = vec![[0; 3]; track_len];
            for position in 0..half {
                let blended =
                    (phase == 1 && position < step) || (phase == 2 && position + head > step);
                let moving = position < step && position + head > step;
                let first = if blended {
                    mixed
                } else if moving {
                    colors[0]
                } else {
                    [0; 3]
                };
                let second = if blended {
                    mixed
                } else if moving {
                    colors[if phase == 0 { 1 } else { 2 }]
                } else {
                    [0; 3]
                };
                let first_target = if phase == 0 {
                    position
                } else {
                    half - position - 1
                };
                let second_target = if phase == 0 {
                    half - position - 1
                } else {
                    position
                };
                track[first_target] = scale(first, bright);
                track[half + second_target] = scale(second, bright);
            }
            frames.push(project(&track));
            step += if phase == 0 { 1 } else { 2 };
        }
    }
    frames
}

pub(crate) fn stack(
    len: usize,
    width: usize,
    stack_count: usize,
    colors: [Color; 4],
    reverse: bool,
    project: impl Fn(&[Color]) -> Vec<Color>,
) -> Vec<Vec<Color>> {
    let mut source = Vec::new();
    let mut track = vec![[0; 3]; len];
    for color in colors {
        let mut stacked = 0;
        for _ in 0..stack_count {
            for step in 0..len - stacked {
                for position in 0..len - stacked {
                    let target = if reverse {
                        len - position - 1
                    } else {
                        position
                    };
                    track[target] = if position <= step && position + width > step {
                        color
                    } else {
                        [0; 3]
                    };
                }
                source.push(project(&track));
            }
            stacked += width;
        }
        for step in 0..len {
            for position in 0..len {
                let target = if reverse {
                    len - position - 1
                } else {
                    position
                };
                track[target] = if position > step { color } else { [0; 3] };
            }
            source.push(project(&track));
        }
    }
    source
}

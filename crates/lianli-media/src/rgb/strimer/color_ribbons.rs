use super::engine::{frame, scale, Color, Frame, Geometry};

pub(super) fn transformation(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let mut output = Vec::with_capacity(geometry.lane_length * 4);
    for color in 0..4 {
        for edge in 0..geometry.lane_length {
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                for position in 0..geometry.lane_length {
                    let mut selected = color;
                    if position > edge {
                        selected = (selected + 3) % 4;
                    }
                    if lane > 0 && lane + 1 < geometry.lanes {
                        selected = (selected + 3) % 4;
                    }
                    let flip = (lane == 0 || lane + 1 == geometry.lanes) != reverse;
                    let physical = lane * geometry.lane_length
                        + if flip {
                            geometry.lane_length - position - 1
                        } else {
                            position
                        };
                    current[physical] = scale(colors[selected], brightness);
                }
            }
            output.push(current);
        }
    }
    output
}

pub(super) fn gradient_ribbon(geometry: Geometry, brightness: u8, reverse: bool) -> Vec<Frame> {
    (0..192)
        .map(|shift| {
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                let mut wheel = (shift + lane * 4) % 192;
                for position in 0..geometry.lane_length {
                    current[lane * geometry.lane_length + position] =
                        scale(rainbow(wheel), brightness);
                    wheel = (wheel + 1) % 192;
                }
            }
            if reverse {
                current.reverse();
            }
            current
        })
        .collect()
}

fn rainbow(position: usize) -> Color {
    let red = if position == 0 {
        255
    } else if position <= 64 {
        (256 - position * 4) as u8
    } else if position >= 129 {
        ((position - 128) * 4) as u8
    } else {
        0
    };
    let green = if position <= 64 {
        (position * 4).min(255) as u8
    } else if position <= 128 {
        ((128 - position) * 4) as u8
    } else {
        0
    };
    let blue = if position <= 64 {
        0
    } else if position <= 128 {
        ((position - 64) * 4).min(255) as u8
    } else {
        ((192 - position) * 4) as u8
    };
    [red, green, blue]
}

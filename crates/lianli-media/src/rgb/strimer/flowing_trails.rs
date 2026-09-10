use super::engine::{frame, scale, Color, Frame, Geometry};

pub(super) fn drizzling(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let (mut selected, offsets): ([usize; 6], [usize; 6]) = match geometry.led_count {
        88 => ([0, 2, 4, 1, 0, 0], [3, 19, 40, 18, 0, 0]),
        116 => ([0, 2, 4, 1, 0, 0], [7, 18, 55, 36, 0, 0]),
        132 => ([0, 2, 4, 1, 5, 1], [3, 19, 40, 18, 5, 25]),
        174 => ([0, 2, 4, 1, 5, 3], [7, 18, 55, 36, 47, 29]),
        _ => unreachable!("validated geometry"),
    };
    let cycle = geometry.lane_length * 2;
    let second = geometry.lane_length.div_ceil(2);
    let mut output = Vec::with_capacity(cycle * 6);
    for _ in 0..6 {
        for step in 0..cycle {
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                if (step + offsets[lane]) % cycle == geometry.lane_length {
                    selected[lane] = (selected[lane] + 1) % 6;
                }
                for position in 0..geometry.lane_length {
                    let pattern = (position + offsets[lane] + step) % cycle;
                    let intensity = match pattern {
                        0 => 255,
                        1 => 128,
                        2 => 32,
                        3 => 8,
                        value if value == second => 255,
                        value if value == second + 1 => 128,
                        value if value == second + 2 => 32,
                        value if value == second + 3 => 8,
                        _ => 0,
                    };
                    current[lane * geometry.lane_length + position] =
                        scale(scale(colors[selected[lane]], intensity), brightness);
                }
            }
            if reverse {
                current.reverse();
            }
            output.push(current);
        }
    }
    output
}

pub(super) fn endless(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    let mut output = Vec::with_capacity(geometry.lane_length * 8);
    let active_lanes = if geometry.lanes == 4 { [0, 3] } else { [1, 4] };
    for color in colors.iter().take(2) {
        for flip_whole in [false, true] {
            for step in 0..geometry.lane_length {
                let mut current = frame(geometry);
                for lane in active_lanes {
                    for position in 0..=step {
                        current[lane * geometry.lane_length + position] = scale(*color, brightness);
                    }
                }
                if flip_whole {
                    current.reverse();
                }
                output.push(current);
            }
            for step in 0..geometry.lane_length {
                let mut current = frame(geometry);
                for lane in 0..geometry.lanes {
                    for position in 0..geometry.lane_length {
                        let intensity = if position <= step {
                            endless_intensity(geometry.lane_length, step - position)
                        } else {
                            0
                        };
                        current
                            [lane * geometry.lane_length + geometry.lane_length - position - 1] =
                            scale(scale(*color, intensity), brightness);
                    }
                }
                if flip_whole {
                    current.reverse();
                }
                output.push(current);
            }
        }
    }
    output
}

fn endless_intensity(width: usize, age: usize) -> u8 {
    if width == 22 {
        const LEVELS: [u8; 22] = [
            5, 30, 55, 80, 105, 130, 155, 180, 205, 230, 255, 255, 230, 205, 180, 155, 130, 105,
            80, 55, 30, 5,
        ];
        LEVELS[age]
    } else {
        let distance = age.abs_diff(14);
        (255 - distance * 17) as u8
    }
}

use super::engine::{frame, scale, Color, Frame, Geometry};

pub(super) fn ping_pong(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    let span = geometry.lane_length + 2;
    let mut output = Vec::with_capacity(span * 6);
    for (color, selected) in colors.iter().enumerate() {
        for step in 0..span {
            let table_step = if color % 2 == 0 {
                step
            } else {
                span - step - 1
            };
            let pattern_step = moving_pattern_step(table_step);
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                for position in 0..geometry.lane_length {
                    if position <= pattern_step && position + 4 > pattern_step {
                        current[lane * geometry.lane_length + position] =
                            scale(*selected, brightness);
                    }
                }
            }
            output.push(current);
        }
    }
    output
}

pub(super) fn runway(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    let span = geometry.lane_length + 2;
    let mut output = Vec::with_capacity(span * 2);
    for pass in 0..2 {
        for step in 0..span {
            let table_step = if pass == 0 { step } else { span - step - 1 };
            let pattern_step = moving_pattern_step(table_step);
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                for position in 0..geometry.lane_length {
                    let selected =
                        usize::from(!(position <= pattern_step && position + 4 > pattern_step));
                    current[lane * geometry.lane_length + position] =
                        scale(colors[selected], brightness);
                }
            }
            output.push(current);
        }
    }
    output
}

fn moving_pattern_step(table_step: usize) -> usize {
    if table_step < 20 {
        table_step
    } else {
        table_step + 1
    }
}

pub(super) fn tide(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    let span = geometry.lane_length.div_ceil(2);
    let mut output = Vec::with_capacity(span * 6);
    for color in 0..6 {
        for step in 0..span {
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                for position in 0..geometry.lane_length {
                    let selected =
                        if position <= step || position + step + 1 >= geometry.lane_length {
                            color
                        } else {
                            (color + 5) % 6
                        };
                    current[lane * geometry.lane_length + position] =
                        scale(colors[selected], brightness);
                }
            }
            output.push(current);
        }
    }
    output
}

pub(super) fn blow_up(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    let expansion = geometry.lane_length.div_ceil(2);
    let mut output = Vec::with_capacity((expansion + 35) * 6);
    for color in colors {
        for step in 0..expansion {
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                for position in 0..geometry.lane_length {
                    let left_center = (geometry.lane_length - 1) / 2;
                    if position >= left_center - step && position <= geometry.lane_length / 2 + step
                    {
                        current[lane * geometry.lane_length + position] = scale(*color, brightness);
                    }
                }
            }
            output.push(current);
        }
        for fade in 0..35 {
            let intensity = if fade < 25 { (24 - fade) * 10 } else { 0 } as u8;
            let pixel = scale(scale(*color, intensity), brightness);
            output.push(vec![pixel; geometry.led_count]);
        }
    }
    output
}

pub(super) fn meteor(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    const TAIL: [u8; 8] = [255, 128, 64, 32, 16, 8, 4, 2];
    let span = geometry.lane_length + TAIL.len();
    let mut output = Vec::with_capacity(span * 6);
    for color in colors {
        for step in 0..span {
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                for position in 0..geometry.lane_length {
                    if let Some(offset) =
                        step.checked_sub(position).filter(|&item| item < TAIL.len())
                    {
                        let physical = lane * geometry.lane_length
                            + if reverse {
                                position
                            } else {
                                geometry.lane_length - position - 1
                            };
                        current[physical] = scale(scale(*color, TAIL[offset]), brightness);
                    }
                }
            }
            output.push(current);
        }
    }
    output
}

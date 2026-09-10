use super::engine::{frame, scale, Color, Frame, Geometry};

pub(super) fn cross_over(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let mut output = Vec::with_capacity(geometry.lane_length * 2);
    for shift in 0..geometry.lane_length * 2 {
        let mut logical = frame(geometry);
        for pair in 0..geometry.lanes / 2 {
            let iterations = if geometry.led_count == 88 && pair == 0 {
                58
            } else {
                geometry.lane_length * 2
            };
            for position in 0..iterations {
                let pattern = (shift + position) % (geometry.lane_length * 2);
                let color = pair * 2 + usize::from(pattern >= geometry.lane_length);
                let pair_position = if position < geometry.lane_length {
                    position
                } else {
                    geometry.lane_length * 3 - position - 1
                };
                logical[pair * geometry.lane_length * 2 + pair_position] =
                    scale(colors[color], brightness);
            }
        }
        if reverse {
            for lane in logical.chunks_exact_mut(geometry.lane_length) {
                lane.reverse();
            }
        }
        output.push(logical);
    }
    output
}

pub(super) fn bullet_stack(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let frame_count = if geometry.lane_length == 22 { 226 } else { 400 };
    let mut positions = [5usize, 2, 7, 4, 0, 3];
    let mut filled = [0usize; 6];
    let mut color = 0usize;
    let mut output = Vec::with_capacity(frame_count);
    while output.len() < frame_count {
        for _ in 0..geometry.lane_length {
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                for position in 0..geometry.lane_length {
                    let selected = if position == positions[lane] {
                        Some(color)
                    } else if filled[lane] >= geometry.lane_length
                        || position >= geometry.lane_length - filled[lane]
                    {
                        Some((color + 5) % 6)
                    } else {
                        None
                    };
                    let physical_position = if reverse {
                        geometry.lane_length - position - 1
                    } else {
                        position
                    };
                    if let Some(selected) = selected {
                        current[lane * geometry.lane_length + physical_position] =
                            scale(colors[selected], brightness);
                    }
                }
                positions[lane] += 1;
                if positions[lane] < geometry.lane_length
                    && filled[lane] < geometry.lane_length - 1
                    && filled[lane] < geometry.lane_length - 1 - positions[lane]
                {
                    continue;
                }
                positions[lane] = 0;
                filled[lane] += 1;
                if lane + 1 == geometry.lanes {
                    color = (color + 1) % 6;
                }
            }
            output.push(current);
            if output.len() == frame_count {
                return output;
            }
        }
    }
    output
}

use super::engine::{frame, scale, Color, Frame, Geometry};

const FIRST_HALF_COLORS: [[usize; 4]; 3] = [[0, 5, 4, 1], [2, 1, 0, 3], [4, 3, 2, 5]];
const SECOND_HALF_COLORS: [[usize; 4]; 3] = [[0, 1, 0, 1], [2, 3, 2, 3], [4, 5, 4, 5]];

pub(super) fn parallel(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let half = geometry.led_count / 2;
    let mut output = Vec::with_capacity(geometry.led_count * 3);
    for cycle in 0..3 {
        for step in 0..geometry.led_count {
            let first_half = step < half;
            let edge_length = if first_half {
                step
            } else {
                geometry.led_count - step
            };
            let indices = if first_half {
                FIRST_HALF_COLORS[cycle]
            } else {
                SECOND_HALF_COLORS[cycle]
            };
            let lengths = [
                edge_length,
                half - edge_length,
                half - edge_length,
                edge_length,
            ];
            let mut logical = Vec::with_capacity(geometry.led_count);
            for (&color, &length) in indices.iter().zip(&lengths) {
                logical.extend(std::iter::repeat_n(
                    scale(colors[color], brightness),
                    length,
                ));
            }
            let mut current = frame(geometry);
            for lane_index in 0..geometry.lanes {
                let special = if geometry.lanes == 4 {
                    lane_index == 1 || lane_index == 2
                } else {
                    lane_index == 1 || lane_index == 4
                };
                let mirror = if reverse {
                    if first_half {
                        special
                    } else {
                        !special
                    }
                } else if first_half {
                    !special
                } else {
                    special
                };
                for position in 0..geometry.lane_length {
                    let source_position = if mirror {
                        geometry.lane_length - position - 1
                    } else {
                        position
                    };
                    let base = lane_index * geometry.lane_length;
                    current[base + position] = logical[base + source_position];
                }
            }
            output.push(current);
        }
    }
    output
}

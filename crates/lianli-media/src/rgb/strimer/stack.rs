use super::engine::{frame, scale, Color, Frame, Geometry};

pub(super) fn stack(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let increment = if geometry.lane_length == 22 { 2 } else { 3 };
    let levels = if geometry.lane_length == 22 { 11 } else { 10 };
    let mut lane = vec![[0; 3]; geometry.lane_length];
    let mut output = Vec::with_capacity(if geometry.lane_length == 22 { 792 } else { 930 });
    for color in colors {
        for level in 0..levels {
            let active = geometry.lane_length - level * increment;
            for step in 0..active {
                for position in 0..active {
                    let physical = if reverse {
                        geometry.lane_length - position - 1
                    } else {
                        position
                    };
                    lane[physical] = if position <= step && position + 3 > step {
                        scale(*color, brightness)
                    } else {
                        [0; 3]
                    };
                }
                let mut current = frame(geometry);
                for destination in current.chunks_exact_mut(geometry.lane_length) {
                    destination.copy_from_slice(&lane);
                }
                output.push(current);
            }
        }
    }
    output
}

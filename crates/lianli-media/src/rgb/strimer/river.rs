use super::engine::{frame, scale, Color, Frame, Geometry};

const SHORT_PATTERN: [bool; 22] = [
    false, false, false, false, false, true, true, false, false, false, false, false, true, true,
    false, false, false, false, false, false, true, true,
];
const LONG_PATTERN: [bool; 29] = [
    false, false, false, false, false, false, false, true, true, true, false, false, false, false,
    false, false, false, true, true, true, false, false, false, false, false, false, true, true,
    true,
];

pub(super) fn river(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let pattern = if geometry.lane_length == 22 {
        &SHORT_PATTERN[..]
    } else {
        &LONG_PATTERN[..]
    };
    (0..geometry.lane_length)
        .map(|shift| {
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                for position in 0..geometry.lane_length {
                    let selected = usize::from(pattern[(position + shift) % pattern.len()]);
                    let flip = (lane % 2 == 1) != reverse;
                    let physical = lane * geometry.lane_length
                        + if flip {
                            geometry.lane_length - position - 1
                        } else {
                            position
                        };
                    current[physical] = scale(colors[selected], brightness);
                }
            }
            current
        })
        .collect()
}

use super::engine::{frame, scale, Color, Frame, Geometry};

pub(super) fn contest(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let cycle = geometry.lane_length * 18;
    (0..cycle)
        .map(|shift| {
            let mut physical = frame(geometry);
            for led in 0..geometry.led_count {
                let pattern = (shift + led) % cycle;
                let group = pattern / geometry.lane_length;
                let pixel = if group.is_multiple_of(3) {
                    scale(colors[group / 3], brightness)
                } else {
                    [0; 3]
                };
                let lane = led / geometry.lane_length;
                let position = led % geometry.lane_length;
                let destination = lane * geometry.lane_length
                    + if lane % 2 == 1 {
                        geometry.lane_length - position - 1
                    } else {
                        position
                    };
                physical[destination] = pixel;
            }
            if reverse {
                physical.reverse();
            }
            physical
        })
        .collect()
}

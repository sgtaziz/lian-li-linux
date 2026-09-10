use super::engine::{frame, scale, Color, Frame, Geometry};

pub(super) fn color_transfer(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let mut frames = Vec::with_capacity(geometry.lanes * 6);
    for color in 0..6 {
        for completed_lanes in 1..=geometry.lanes {
            let boundary = completed_lanes * geometry.lane_length;
            let mut output = frame(geometry);
            for position in 0..geometry.led_count {
                let source = if position < boundary {
                    color
                } else {
                    (color + 5) % 6
                };
                let destination = if reverse {
                    geometry.led_count - position - 1
                } else {
                    position
                };
                output[destination] = scale(colors[source], brightness);
            }
            frames.push(output);
        }
    }
    frames
}

pub(super) fn fade_out(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    const FOUR: [[u8; 4]; 35] = [
        [0, 255, 255, 0],
        [0, 255, 255, 0],
        [0, 255, 255, 0],
        [0, 255, 255, 0],
        [0, 255, 255, 0],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 238, 238, 255],
        [255, 221, 221, 255],
        [255, 204, 204, 255],
        [255, 187, 187, 255],
        [255, 170, 170, 255],
        [238, 153, 153, 238],
        [221, 136, 136, 221],
        [204, 119, 119, 204],
        [187, 102, 102, 187],
        [170, 85, 85, 170],
        [153, 68, 68, 153],
        [136, 51, 51, 136],
        [119, 34, 34, 119],
        [102, 17, 17, 102],
        [85, 0, 0, 85],
        [68, 0, 0, 68],
        [51, 0, 0, 51],
        [34, 0, 0, 34],
        [17, 0, 0, 17],
        [0, 0, 0, 0],
    ];
    const SIX: [[u8; 6]; 40] = [
        [0, 0, 255, 255, 0, 0],
        [0, 0, 255, 255, 0, 0],
        [0, 0, 255, 255, 0, 0],
        [0, 0, 255, 255, 0, 0],
        [0, 0, 255, 255, 0, 0],
        [0, 255, 255, 255, 255, 0],
        [0, 255, 255, 255, 255, 0],
        [0, 255, 255, 255, 255, 0],
        [0, 255, 255, 255, 255, 0],
        [0, 255, 255, 255, 255, 0],
        [255, 255, 255, 255, 255, 255],
        [255, 255, 255, 255, 255, 255],
        [255, 255, 255, 255, 255, 255],
        [255, 255, 255, 255, 255, 255],
        [255, 255, 255, 255, 255, 255],
        [255, 255, 238, 238, 255, 255],
        [255, 255, 221, 221, 255, 255],
        [255, 255, 204, 204, 255, 255],
        [255, 255, 187, 187, 255, 255],
        [255, 255, 170, 170, 255, 255],
        [255, 238, 153, 153, 238, 255],
        [255, 221, 136, 136, 221, 255],
        [255, 204, 119, 119, 204, 255],
        [255, 187, 102, 102, 187, 255],
        [255, 170, 85, 85, 170, 255],
        [238, 153, 68, 68, 153, 238],
        [221, 136, 51, 51, 136, 221],
        [204, 119, 34, 34, 119, 204],
        [187, 102, 17, 17, 102, 187],
        [170, 85, 0, 0, 85, 170],
        [153, 68, 0, 0, 68, 153],
        [136, 51, 0, 0, 51, 136],
        [119, 34, 0, 0, 34, 119],
        [102, 17, 0, 0, 17, 102],
        [85, 0, 0, 0, 0, 85],
        [68, 0, 0, 0, 0, 68],
        [51, 0, 0, 0, 0, 51],
        [34, 0, 0, 0, 0, 34],
        [17, 0, 0, 0, 0, 17],
        [0, 0, 0, 0, 0, 0],
    ];
    let steps = if geometry.lanes == 4 { 35 } else { 40 };
    let mut frames = Vec::with_capacity(steps * 3);
    for pair in 0..3 {
        for step in 0..steps {
            let mut output = frame(geometry);
            for lane in 0..geometry.lanes {
                let color = pair * 2 + usize::from(lane >= geometry.lanes / 2);
                let level = if geometry.lanes == 4 {
                    FOUR[step][lane]
                } else {
                    SIX[step][lane]
                };
                let pixel = scale(scale(colors[color], level), brightness);
                output[lane * geometry.lane_length..(lane + 1) * geometry.lane_length].fill(pixel);
            }
            frames.push(output);
        }
    }
    frames
}

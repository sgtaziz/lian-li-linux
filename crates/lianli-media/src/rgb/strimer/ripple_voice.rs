use super::engine::{frame, scale, Color, Frame, Geometry};

pub(super) fn ripple(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    let (frames_per_color, growth_frames) = match (geometry.lanes, geometry.lane_length) {
        (4, 22) => (34, 14),
        (4, 29) => (40, 20),
        (6, 22) => (44, 19),
        (6, 29) => (50, 25),
        _ => unreachable!(),
    };
    let mut output = Vec::with_capacity(frames_per_color * 6);
    for color in colors {
        let mut sizes = [0usize; 6];
        let mut intensities = [255u8; 6];
        for step in 0..frames_per_color {
            if step < growth_frames {
                grow_ripple(geometry, &mut sizes);
            } else {
                fade_ripple(geometry.lanes, &mut intensities);
            }
            let mut current = frame(geometry);
            for lane_index in 0..geometry.lanes {
                let start = (geometry.lane_length - sizes[lane_index]) / 2;
                let pixel = scale(scale(*color, intensities[lane_index]), brightness);
                current[lane_index * geometry.lane_length + start
                    ..lane_index * geometry.lane_length + start + sizes[lane_index]]
                    .fill(pixel);
            }
            output.push(current);
        }
    }
    output
}

fn grow_ripple(geometry: Geometry, sizes: &mut [usize; 6]) {
    if geometry.lanes == 4 {
        let start = if geometry.lane_length == 22 { 2 } else { 1 };
        let trigger = if geometry.lane_length == 22 { 10 } else { 13 };
        if sizes[1] == 0 {
            sizes[1] = start;
            sizes[2] = start;
        } else if sizes[1] < geometry.lane_length {
            sizes[1] += 2;
            sizes[2] += 2;
        }
        if sizes[1] == trigger {
            sizes[0] = start;
            sizes[3] = start;
        }
        if sizes[0] > 0 && sizes[0] < geometry.lane_length {
            sizes[0] += 2;
            sizes[3] += 2;
        }
        return;
    }

    let start = if geometry.lane_length == 22 { 2 } else { 1 };
    let trigger = if geometry.lane_length == 22 { 10 } else { 11 };
    if sizes[2] == 0 {
        sizes[2] = start;
        sizes[3] = start;
    } else if sizes[2] == trigger {
        sizes[1] = start;
        sizes[4] = start;
        sizes[2] += 2;
        sizes[3] += 2;
    } else if sizes[2] < geometry.lane_length {
        sizes[2] += 2;
        sizes[3] += 2;
    }
    if sizes[1] > 0 {
        if sizes[1] == trigger {
            sizes[0] = start;
            sizes[5] = start;
            sizes[1] += 2;
            sizes[4] += 2;
        } else if sizes[1] < geometry.lane_length {
            sizes[1] += 2;
            sizes[4] += 2;
        }
    }
    if sizes[0] > 0 && sizes[0] < geometry.lane_length {
        sizes[0] += 2;
        sizes[5] += 2;
    }
}

fn fade_ripple(lanes: usize, intensities: &mut [u8; 6]) {
    if lanes == 4 {
        if intensities[1] > 0 {
            intensities[1] -= 17;
            intensities[2] -= 17;
        }
        if intensities[1] == 170 {
            intensities[0] -= 17;
            intensities[3] -= 17;
        }
        if intensities[0] != 255 && intensities[0] > 0 {
            intensities[0] -= 17;
            intensities[3] -= 17;
        }
        return;
    }

    if intensities[2] > 0 {
        intensities[2] -= 17;
        intensities[3] -= 17;
    }
    if intensities[2] == 170 {
        intensities[1] -= 17;
        intensities[4] -= 17;
    }
    if intensities[1] != 255 {
        if intensities[1] > 0 {
            intensities[1] -= 17;
            intensities[4] -= 17;
        }
        if intensities[1] == 170 {
            intensities[0] -= 17;
            intensities[5] -= 17;
        }
    }
    if intensities[0] != 255 && intensities[0] > 0 {
        intensities[0] -= 17;
        intensities[5] -= 17;
    }
}

pub(super) fn voice(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    let mut sizes = match (geometry.lanes, geometry.lane_length) {
        (4, 22) => [4, 20, 8, 16, 0, 0],
        (4, 29) => [3, 25, 5, 13, 0, 0],
        (6, 22) => [4, 20, 8, 16, 6, 12],
        (6, 29) => [3, 25, 5, 13, 7, 19],
        _ => unreachable!(),
    };
    let mut shrinking = [false; 6];
    let frame_count = if geometry.lane_length == 22 { 19 } else { 27 };
    let minimum = if geometry.lane_length == 22 { 2 } else { 1 };
    let mut output = Vec::with_capacity(frame_count);
    for _ in 0..frame_count {
        let mut current = frame(geometry);
        for lane_index in 0..geometry.lanes {
            if !shrinking[lane_index] {
                if sizes[lane_index] < geometry.lane_length {
                    sizes[lane_index] += 2;
                } else {
                    shrinking[lane_index] = true;
                    sizes[lane_index] -= 2;
                }
            } else if sizes[lane_index] > minimum {
                sizes[lane_index] -= 2;
            } else {
                shrinking[lane_index] = false;
                sizes[lane_index] += 2;
            }
            let start = (geometry.lane_length - sizes[lane_index]) / 2;
            current[lane_index * geometry.lane_length + start
                ..lane_index * geometry.lane_length + start + sizes[lane_index]]
                .fill(scale(colors[lane_index], brightness));
        }
        output.push(current);
    }
    output
}

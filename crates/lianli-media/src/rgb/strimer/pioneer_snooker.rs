use super::engine::{frame, scale, Color, Frame, Geometry};

pub(super) fn pioneer(geometry: Geometry, color: Color, brightness: u8) -> Vec<Frame> {
    let mut sizes = match geometry.lanes {
        4 => [0, geometry.lane_length, geometry.lane_length, 0, 0, 0],
        6 => [0, 0, geometry.lane_length, geometry.lane_length, 0, 0],
        _ => unreachable!(),
    };
    let pixel = scale(color, brightness);
    let mut output = Vec::with_capacity(geometry.lane_length);
    for step in 0..geometry.lane_length {
        let mut current = frame(geometry);
        for lane_index in 0..geometry.lanes {
            let start = (geometry.lane_length - sizes[lane_index]) / 2;
            current[lane_index * geometry.lane_length + start
                ..lane_index * geometry.lane_length + start + sizes[lane_index]]
                .fill(pixel);
        }
        update_pioneer(geometry, step, &mut sizes);
        output.push(current);
    }
    output
}

fn update_pioneer(geometry: Geometry, step: usize, sizes: &mut [usize; 6]) {
    let first_half_end = if geometry.lane_length == 22 { 10 } else { 13 };
    if geometry.lanes == 4 {
        if step < first_half_end {
            sizes[1] -= 2;
            sizes[2] -= 2;
        } else if step == first_half_end {
            sizes[..4].fill(if geometry.lane_length == 22 { 4 } else { 5 });
        } else if step < geometry.lane_length - 3 {
            sizes[..4].iter_mut().for_each(|size| *size += 2);
        } else if step < geometry.lane_length - 1 {
            sizes[..4].fill(geometry.lane_length);
        } else {
            sizes[..4].copy_from_slice(&[0, geometry.lane_length, geometry.lane_length, 0]);
        }
        return;
    }

    if step < first_half_end {
        sizes[2] -= 2;
        sizes[3] -= 2;
    } else if step == first_half_end {
        let middle = if geometry.lane_length == 22 { 4 } else { 5 };
        sizes[1..5].fill(middle);
    } else if step == first_half_end + 1 {
        sizes.fill(if geometry.lane_length == 22 { 6 } else { 7 });
    } else if step < geometry.lane_length - 3 {
        sizes.iter_mut().for_each(|size| *size += 2);
    } else if step < geometry.lane_length - 2 {
        sizes.fill(geometry.lane_length);
    } else if step < geometry.lane_length - 1 {
        sizes.copy_from_slice(&[
            0,
            geometry.lane_length,
            geometry.lane_length,
            geometry.lane_length,
            geometry.lane_length,
            0,
        ]);
    } else {
        sizes.copy_from_slice(&[0, 0, geometry.lane_length, geometry.lane_length, 0, 0]);
    }
}

pub(super) fn snooker(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    let half_cycle = if geometry.lane_length == 22 { 13 } else { 17 };
    let cycle = half_cycle * 2;
    let group_size = if geometry.lane_length == 22 { 3 } else { 4 };
    let mut output = Vec::with_capacity(cycle * 6);
    for color in colors {
        let pixel = scale(*color, brightness);
        for step in 0..cycle {
            let table_step = if step < half_cycle {
                step
            } else {
                cycle - step - 1
            };
            let mut pattern = vec![false; geometry.lane_length];
            if table_step + 1 == half_cycle {
                let start = (geometry.lane_length - (group_size - 1)) / 2;
                pattern[start..start + group_size - 1].fill(true);
            } else {
                let start = table_step.saturating_sub(group_size - 1);
                let length = (table_step + 1).min(group_size);
                pattern[start..start + length].fill(true);
                let opposite = geometry.lane_length - start - length;
                pattern[opposite..opposite + length].fill(true);
            }
            let mut current = frame(geometry);
            for lane_index in 0..geometry.lanes {
                for (position, &selected) in pattern.iter().enumerate() {
                    if selected {
                        current[lane_index * geometry.lane_length + position] = pixel;
                    }
                }
            }
            output.push(current);
        }
    }
    output
}

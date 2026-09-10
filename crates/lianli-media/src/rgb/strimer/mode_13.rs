use super::engine::{frame, scale, Color, Frame, Geometry};

pub(super) fn shuttle_run(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    let forward_delays: &[usize] = if geometry.lanes == 4 {
        &[0, 10, 20, 30]
    } else {
        &[0, 10, 20, 30, 20, 30]
    };
    let backward_delays: &[usize] = if geometry.lanes == 4 {
        &[30, 20, 10, 0]
    } else {
        &[30, 20, 30, 20, 10, 0]
    };
    let mut output = Vec::with_capacity(120);
    for (pass, delays) in [forward_delays, backward_delays].into_iter().enumerate() {
        for step in 0..60 {
            let mut current = frame(geometry);
            for (lane, &delay) in delays.iter().enumerate() {
                let intensity = pulse(step, delay);
                let first_group = if geometry.lanes == 4 {
                    lane < 2
                } else {
                    lane < 2 || lane + 2 >= geometry.lanes
                };
                let color = usize::from(first_group == (pass == 1));
                let pixel = scale(scale(colors[color], intensity), brightness);
                current[lane * geometry.lane_length..(lane + 1) * geometry.lane_length].fill(pixel);
            }
            output.push(current);
        }
    }
    output
}

fn pulse(step: usize, delay: usize) -> u8 {
    let Some(local) = step.checked_sub(delay) else {
        return 0;
    };
    match local {
        0..=14 => ((local + 1) * 17) as u8,
        15..=29 => ((29 - local) * 17) as u8,
        _ => 0,
    }
}

use super::{palette::Frame, Layout};
use crate::rgb::Animation;
use lianli_shared::rgb::RgbMode;

pub(crate) struct RegionAnimation {
    pub(crate) frames: Vec<Frame>,
    pub(crate) interval_hundredths: u32,
    pub(crate) mode: RgbMode,
}

pub(crate) fn combine(
    layout: Layout,
    mut front: RegionAnimation,
    mut rear: RegionAnimation,
) -> Animation {
    if front.mode == RgbMode::Meteor && rear.mode == RgbMode::Meteor {
        rear.frames = stretch_rear_meteor(&rear.frames);
    }
    let front_dominant = front.frames.len() >= rear.frames.len();
    let maximum = front.frames.len().max(rear.frames.len());
    repeat_if_short(&mut front.frames, maximum);
    repeat_if_short(&mut rear.frames, maximum);
    let interval_hundredths = rear.interval_hundredths;
    let ticks = interval_hundredths / 100;

    let mut frames = Vec::with_capacity(maximum);
    for index in 0..maximum {
        let front_index = frame_index(index, maximum, front.frames.len(), front_dominant, ticks);
        let rear_index = frame_index(index, maximum, rear.frames.len(), !front_dominant, ticks);
        let mut output = if front_dominant {
            front.frames[index].clone()
        } else {
            rear.frames[index].clone()
        };
        output[..layout.front_leds]
            .copy_from_slice(&front.frames[front_index][..layout.front_leds]);
        output[layout.front_leds..].copy_from_slice(&rear.frames[rear_index][layout.front_leds..]);
        frames.push(output);
    }
    Animation {
        frames,
        interval_hundredths,
        secondary: None,
    }
}

fn stretch_rear_meteor(frames: &[Frame]) -> Vec<Frame> {
    let mut repeated = Vec::with_capacity(192);
    for color in 0..4 {
        for _ in 0..2 {
            repeated.extend_from_slice(&frames[color * 24..color * 24 + 24]);
        }
    }
    (0..360)
        .map(|index| repeated[(index as f32 / 1.875f32) as usize].clone())
        .collect()
}

fn repeat_if_short(frames: &mut Vec<Frame>, maximum: usize) {
    if maximum / 2 >= frames.len() {
        let source = frames.clone();
        let repeats = maximum / source.len();
        frames.clear();
        for _ in 0..repeats {
            frames.extend(source.iter().cloned());
        }
    }
}

fn frame_index(index: usize, maximum: usize, length: usize, dominant: bool, ticks: u32) -> usize {
    if dominant || length == maximum {
        return index;
    }
    let divisor = ticks as usize * maximum / length;
    (index * ticks as usize / divisor).min(length - 1)
}

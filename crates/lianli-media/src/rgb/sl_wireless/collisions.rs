use super::engine::{brightness, palette, place, scale, Color, Side};
use lianli_shared::rgb::RgbEffect;

pub(super) fn ripple(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    crate::rgb::effects::collisions::ripple(fans * side.leds_per_track(), fans, colors, |track| {
        place(track, side, fans)
    })
}

pub(super) fn collide(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    crate::rgb::effects::collisions::collide(fans * side.leds_per_track(), fans, colors, |track| {
        place(track, side, fans)
    })
}

pub(super) fn reflect(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    crate::rgb::effects::collisions::reflect(fans * side.leds_per_track(), fans, colors, |track| {
        place(track, side, fans)
    })
}

pub(super) fn electric_current(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    crate::rgb::effects::collisions::electric_current(
        fans * side.leds_per_track(),
        fans,
        colors,
        |track| place(track, side, fans),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::RgbMode;

    fn effect(mode: RgbMode) -> RgbEffect {
        RgbEffect {
            mode,
            colors: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]],
            brightness: 4,
            ..Default::default()
        }
    }

    #[test]
    fn ripple_emits_four_inward_pairs_per_color() {
        let frames = ripple(&effect(RgbMode::Ripple), 1, Side::Outer);
        assert_eq!(frames.len(), 64);
        assert_eq!(frames[0][15], [254, 0, 0]);
        assert_eq!(frames[0][16], [254, 0, 0]);
        assert_eq!(frames[1][15], [0; 3]);
        assert_eq!(frames[3][15], [254, 0, 0]);
    }

    #[test]
    fn collide_changes_color_at_the_native_turnaround() {
        let frames = collide(&effect(RgbMode::Collide), 1, Side::Outer);
        assert_eq!(frames.len(), 40);
        assert!(frames[0].iter().all(|color| *color == [0; 3]));
        assert_eq!(frames[1][12], [254, 0, 0]);
        assert_eq!(frames[4][15], [0, 254, 0]);
        assert_eq!(frames[4][16], [0, 254, 0]);
    }

    #[test]
    fn reflect_keeps_the_native_blank_return_pass() {
        let frames = reflect(&effect(RgbMode::Reflect), 1, Side::Inner);
        assert_eq!(frames.len(), 56);
        assert_eq!(frames[0][0], [254, 0, 0]);
        assert_eq!(frames[0][11], [254, 0, 0]);
        assert!(frames[7..14]
            .iter()
            .all(|frame| frame.iter().all(|color| *color == [0; 3])));
    }

    #[test]
    fn electric_current_repeats_the_startup_hold_for_each_color() {
        let frames = electric_current(&effect(RgbMode::ElectricCurrent), 1, Side::Outer);
        assert_eq!(frames.len(), 68);
        assert!(frames[..10]
            .iter()
            .all(|frame| frame.iter().all(|color| *color == [0; 3])));
        assert_eq!(frames[10][12], [254, 0, 0]);
        assert_eq!(frames[10][19], [254, 0, 0]);
        assert!(frames[17..27]
            .iter()
            .all(|frame| frame.iter().all(|color| *color == [0; 3])));
    }
}

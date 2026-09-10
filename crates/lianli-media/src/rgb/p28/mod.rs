use super::{cl, Animation};
use anyhow::{ensure, Result};
use cl::engine::{project_p28, Plane};
use lianli_shared::rgb::{RgbMode, RgbRegionConfig, RgbScope};

pub const MODES: &[RgbMode] = cl::MODES;

pub fn render(regions: &[RgbRegionConfig], fans: usize) -> Result<Animation> {
    ensure!((1..=4).contains(&fans), "P28 fan count must be 1..=4");
    ensure!(regions.len() == 1, "P28 requires one whole-lighting effect");
    let region = &regions[0];
    ensure!(!region.flip, "P28 does not support region flipping");
    ensure!(
        matches!(region.effect.scope, RgbScope::All | RgbScope::Center),
        "P28 effects support the whole lighting zone only"
    );
    cl::engine::validate(&region.effect, fans)?;
    let source = cl::render_plane(&region.effect, fans, Plane::Center)?;
    Ok(Animation {
        frames: source
            .frames
            .iter()
            .map(|frame| project_p28(frame, fans))
            .collect(),
        interval_hundredths: source.interval_ticks * 100,
        secondary: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::{RgbDirection, RgbEffect};

    fn region(mode: RgbMode) -> RgbRegionConfig {
        RgbRegionConfig {
            effect: RgbEffect {
                mode,
                colors: vec![[255, 128, 1], [1, 200, 99]],
                brightness: 4,
                speed: 2,
                scope: RgbScope::All,
                ..Default::default()
            },
            flip: false,
        }
    }

    #[test]
    fn projects_cl_center_and_duplicates_its_last_led() {
        let animation = render(&[region(RgbMode::Static)], 2).unwrap();
        assert_eq!(animation.frames.len(), 30);
        assert_eq!(animation.interval_hundredths, 10_000);
        assert_eq!(&animation.frames[0][..9], &[[254, 127, 0]; 9]);
        assert_eq!(&animation.frames[0][9..18], &[[0, 199, 98]; 9]);
    }

    #[test]
    fn rainbow_uses_cl_center_wiring_order_and_direction() {
        let mut clockwise = region(RgbMode::Rainbow);
        clockwise.effect.colors.clear();
        let cw = render(&[clockwise.clone()], 1).unwrap();
        assert_eq!(cw.frames.len(), 8);
        assert_eq!(cw.interval_hundredths, 17_500);
        assert_eq!(cw.frames[0][0], [0, 31, 223]);
        assert_eq!(cw.frames[0][1], [159, 0, 95]);
        assert_eq!(cw.frames[0][8], cw.frames[0][7]);

        clockwise.effect.direction = RgbDirection::CounterClockwise;
        let ccw = render(&[clockwise], 1).unwrap();
        assert_eq!(ccw.frames[0][0], [63, 191, 0]);
        assert_ne!(cw.frames[0], ccw.frames[0]);
    }

    #[test]
    fn morph_and_breathing_keep_vendor_frame_selection_and_integer_scaling() {
        let morph = render(&[region(RgbMode::RainbowMorph)], 1).unwrap();
        assert_eq!(morph.frames.len(), 127);
        assert_eq!(morph.frames[0][0], [254, 0, 0]);
        assert_eq!(morph.frames[1][0], [248, 5, 0]);

        let breathing = render(&[region(RgbMode::Breathing)], 1).unwrap();
        assert_eq!(breathing.frames.len(), 170);
        assert_eq!(breathing.frames[0][0], [0, 0, 0]);
        assert_eq!(breathing.frames[1][0], [1, 0, 0]);
        assert_eq!(breathing.frames[85][0], [253, 126, 0]);
    }

    #[test]
    fn modes_five_through_ten_keep_native_lengths_and_boundaries() {
        let runway = render(&[region(RgbMode::Runway)], 1).unwrap();
        assert_eq!(runway.frames.len(), 18);
        assert_eq!(runway.frames[0][0], [254, 127, 0]);
        assert_eq!(runway.frames[0][1], [0, 199, 98]);
        assert_eq!(runway.frames[9][7], [254, 127, 0]);

        let meteor = render(&[region(RgbMode::Meteor)], 1).unwrap();
        assert_eq!(meteor.frames.len(), 36);
        assert_eq!(meteor.frames[0][0], [30, 15, 0]);
        assert!(meteor.frames[0][1..].iter().all(|&color| color == [0; 3]));

        let twinkle = render(&[region(RgbMode::Twinkle)], 1).unwrap();
        assert_eq!(twinkle.frames.len(), 200);
        assert_eq!(twinkle.frames[4][7], [3, 1, 0]);
        assert_eq!(twinkle.frames[4][8], twinkle.frames[4][7]);

        let tai_chi = render(&[region(RgbMode::TaiChi)], 1).unwrap();
        assert_eq!(tai_chi.frames.len(), 8);
        assert_eq!(&tai_chi.frames[0][..4], &[[254, 127, 0]; 4]);
        assert_eq!(&tai_chi.frames[0][4..8], &[[0, 199, 98]; 4]);

        let cycle = render(&[region(RgbMode::ColorCycle)], 1).unwrap();
        assert_eq!(cycle.frames.len(), 32);
        assert_eq!(cycle.frames[0][0], [254, 127, 0]);
        assert_eq!(cycle.frames[0][1], [254, 254, 0]);

        let mop_up = render(&[region(RgbMode::MopUp)], 1).unwrap();
        assert_eq!(mop_up.frames.len(), 40);
        assert_eq!(mop_up.frames[0][0], [254, 127, 0]);
        assert!(mop_up.frames[0][1..].iter().all(|&color| color == [0; 3]));
    }

    #[test]
    fn meteor_variants_keep_native_tables_and_direction() {
        let rainbow = render(&[region(RgbMode::MeteorRainbow)], 1).unwrap();
        assert_eq!(rainbow.frames.len(), 9);
        assert_eq!(rainbow.frames[0][0], [127, 0, 63]);
        assert!(rainbow.frames[0][1..].iter().all(|&color| color == [0; 3]));

        let colorful = render(&[region(RgbMode::ColorfulMeteor)], 1).unwrap();
        assert_eq!(colorful.frames.len(), 9);
        assert_eq!(colorful.frames[0][0], [30, 0, 0]);
        assert_eq!(colorful.frames[2][1], [0, 30, 0]);

        let mut reversed = region(RgbMode::ColorfulMeteor);
        reversed.effect.direction = RgbDirection::CounterClockwise;
        let reversed = render(&[reversed], 1).unwrap();
        assert_eq!(reversed.frames[0][7], [30, 0, 0]);
        assert_eq!(reversed.frames[0][8], [30, 0, 0]);
    }

    #[test]
    fn lottery_warning_and_voice_preserve_center_plane_boundaries() {
        let lottery = render(&[region(RgbMode::Lottery)], 1).unwrap();
        assert_eq!(lottery.frames.len(), 8);
        assert_eq!(&lottery.frames[0][..7], &[[0, 199, 98]; 7]);
        assert_eq!(&lottery.frames[0][7..9], &[[254, 127, 0]; 2]);

        let warning = render(&[region(RgbMode::Warning)], 1).unwrap();
        assert_eq!(warning.frames.len(), 64);
        assert!(warning.frames[0]
            .iter()
            .all(|&color| color == [254, 127, 0]));
        assert!(warning.frames[8].iter().all(|&color| color == [0; 3]));

        let voice = render(&[region(RgbMode::Voice)], 1).unwrap();
        assert_eq!(voice.frames.len(), 120);
        assert_eq!(voice.frames[0][0], [31, 15, 0]);
        assert_eq!(voice.frames[0][1], [254, 127, 0]);
        assert!(voice.frames[8][..8]
            .iter()
            .all(|&color| color == [254, 127, 0]));
    }

    #[test]
    fn mixing_tide_and_scan_keep_vendor_phase_lengths_and_wiring() {
        let mixing = render(&[region(RgbMode::Mixing)], 1).unwrap();
        assert_eq!(mixing.frames.len(), 11);
        assert!(mixing.frames[0].iter().all(|&color| color == [0; 3]));
        assert_eq!(mixing.frames[1][0], [254, 127, 0]);
        assert_eq!(mixing.frames[1][7], [0, 199, 98]);
        assert_eq!(mixing.frames[6][3], [241, 241, 94]);

        let tide = render(&[region(RgbMode::Tide)], 1).unwrap();
        assert_eq!(tide.frames.len(), 20);
        assert_eq!(tide.frames[0][0], [254, 127, 0]);
        assert_eq!(tide.frames[0][1], [254, 254, 0]);
        assert_eq!(tide.frames[0][5], [254, 127, 0]);
        assert!(tide.frames[4][..9]
            .iter()
            .all(|&color| color == [254, 127, 0]));

        let scan = render(&[region(RgbMode::Scan)], 1).unwrap();
        assert_eq!(scan.frames.len(), 20);
        assert_eq!(scan.frames[0][1], [254, 127, 0]);
        assert_eq!(scan.frames[10][5], [254, 127, 0]);
        assert!(scan.frames[0]
            .iter()
            .enumerate()
            .all(|(index, &color)| index == 1 || color == [0; 3]));
    }

    #[test]
    fn paired_meteors_and_arcs_keep_native_patterns() {
        let double = render(&[region(RgbMode::DoubleMeteor)], 1).unwrap();
        assert_eq!(double.frames.len(), 16);
        assert_eq!(double.frames[0][0], [254, 127, 0]);
        assert_eq!(&double.frames[0][7..9], &[[254, 127, 0]; 2]);
        assert_eq!(double.frames[4][3], [0, 199, 98]);
        assert_eq!(double.frames[4][4], [0, 199, 98]);

        let contest = render(&[region(RgbMode::MeteorContest)], 1).unwrap();
        assert_eq!(contest.frames.len(), 8);
        assert_eq!(contest.interval_hundredths, 12_500);
        assert_eq!(contest.frames[0][3], [253, 126, 0]);
        assert_eq!(&contest.frames[0][7..9], &[[0, 197, 97]; 2]);

        let mix = render(&[region(RgbMode::MeteorMix)], 1).unwrap();
        assert_eq!(mix.frames.len(), 8);
        assert_eq!(mix.frames[0][0], [254, 127, 0]);
        assert_eq!(&mix.frames[0][7..9], &[[0, 199, 98]; 2]);

        let return_arc = render(&[region(RgbMode::ReturnArc)], 1).unwrap();
        assert_eq!(return_arc.frames.len(), 64);
        assert!(return_arc.frames[0].iter().all(|&color| color == [0; 3]));
        assert!(return_arc.frames[8]
            .iter()
            .all(|&color| color == [254, 127, 0]));

        let double_arc = render(&[region(RgbMode::DoubleArc)], 1).unwrap();
        assert_eq!(double_arc.frames.len(), 32);
        assert!(double_arc.frames[0].iter().all(|&color| color == [0; 3]));
        assert!(double_arc.frames[4]
            .iter()
            .all(|&color| color == [254, 127, 0]));
    }

    #[test]
    fn door_and_heartbeats_keep_native_masks_and_delays() {
        let door = render(&[region(RgbMode::Door)], 1).unwrap();
        assert_eq!(door.frames.len(), 32);
        assert_eq!(door.frames[0][0], [0; 3]);
        assert_eq!(door.frames[0][1], [254, 127, 0]);
        assert_eq!(door.frames[0][2], [254, 127, 0]);
        assert_eq!(door.frames[0][5], [254, 127, 0]);
        assert_eq!(door.frames[0][6], [254, 127, 0]);
        assert!(door.frames[7].iter().all(|&color| color == [0; 3]));

        let heart = render(&[region(RgbMode::HeartBeat)], 1).unwrap();
        assert_eq!(heart.frames.len(), 128);
        assert_eq!(heart.frames[0][0], [14, 7, 0]);
        assert_eq!(heart.frames[47][0], [253, 126, 0]);
        assert_eq!(heart.frames[96][0], [14, 7, 0]);

        let runway = render(&[region(RgbMode::HeartBeatRunway)], 2).unwrap();
        assert_eq!(runway.frames.len(), 180);
        assert_eq!(runway.interval_hundredths, 5_000);
        assert_eq!(runway.frames[47][0], [253, 126, 0]);
        assert_eq!(runway.frames[47][9], [14, 7, 0]);

        let mut reversed = region(RgbMode::HeartBeatRunway);
        reversed.effect.direction = RgbDirection::CounterClockwise;
        let reversed = render(&[reversed], 2).unwrap();
        assert_eq!(reversed.frames[47][0], [14, 7, 0]);
        assert_eq!(reversed.frames[47][9], [253, 126, 0]);
    }

    #[test]
    fn disco_current_reflect_and_ribbon_keep_native_boundaries() {
        let disco = render(&[region(RgbMode::Disco)], 1).unwrap();
        assert_eq!(disco.frames.len(), 8);
        assert_eq!(disco.interval_hundredths, 10_000);
        assert_eq!(disco.frames[0][0], [254, 254, 0]);
        assert_eq!(disco.frames[0][7], [254, 127, 0]);
        assert_eq!(disco.frames[0][8], disco.frames[0][7]);

        let current = render(&[region(RgbMode::ElectricCurrent)], 1).unwrap();
        assert_eq!(current.frames.len(), 68);
        assert_eq!(current.interval_hundredths, 10_000);
        assert!(current.frames[..10]
            .iter()
            .flatten()
            .all(|&color| color == [0; 3]));
        assert_eq!(current.frames[10][0], [254, 127, 0]);
        assert_eq!(current.frames[10][7], [254, 127, 0]);

        let reflect = render(&[region(RgbMode::Reflect)], 1).unwrap();
        assert_eq!(reflect.frames.len(), 40);
        assert_eq!(reflect.frames[0][0], [253, 126, 0]);
        assert_eq!(reflect.frames[0][7], [253, 126, 0]);
        assert!(reflect.frames[5..10]
            .iter()
            .flatten()
            .all(|&color| color == [0; 3]));

        let ribbon = render(&[region(RgbMode::GradientRibbon)], 1).unwrap();
        assert_eq!(ribbon.frames.len(), 48);
        assert_eq!(ribbon.frames[0][0], [15, 239, 0]);
        assert_eq!(ribbon.frames[0][4], [31, 223, 0]);
        assert_eq!(ribbon.frames[0][8], ribbon.frames[0][7]);

        let mut reversed = region(RgbMode::GradientRibbon);
        reversed.effect.direction = RgbDirection::CounterClockwise;
        let reversed = render(&[reversed], 1).unwrap();
        assert_eq!(reversed.frames[0][0], [127, 127, 0]);
        assert_eq!(reversed.frames[0][4], [111, 143, 0]);
    }

    #[test]
    fn wing_drumming_boomerang_and_candy_box_keep_native_tables() {
        let wing = render(&[region(RgbMode::Wing)], 1).unwrap();
        assert_eq!(wing.frames.len(), 10);
        assert_eq!(wing.interval_hundredths, 10_000);
        assert_eq!(wing.frames[0][4], [254, 254, 99]);
        assert_eq!(wing.frames[3][3], [254, 127, 0]);
        assert_eq!(wing.frames[3][4], [254, 254, 99]);
        assert_eq!(wing.frames[3][5], [0, 199, 98]);
        assert!(wing.frames[4].iter().all(|&color| color == [0; 3]));

        let wing_two = render(&[region(RgbMode::Wing)], 2).unwrap();
        assert_eq!(wing_two.frames.len(), 22);
        assert_eq!(wing_two.frames[0][5], [254, 127, 0]);
        assert_eq!(wing_two.frames[0][11], [0, 199, 98]);

        let drumming = render(&[region(RgbMode::Drumming)], 1).unwrap();
        assert_eq!(drumming.frames.len(), 108);
        assert_eq!(drumming.frames[0][0], [49, 24, 0]);
        assert!(drumming.frames[18].iter().all(|&color| color == [0; 3]));
        assert_eq!(drumming.frames[72][0], [49, 24, 0]);

        let boomerang = render(&[region(RgbMode::Boomerang)], 1).unwrap();
        assert_eq!(boomerang.frames.len(), 18);
        assert_eq!(boomerang.frames[0][1], [254, 127, 0]);
        assert_eq!(boomerang.frames[0][5], [0, 199, 98]);
        assert!(boomerang.frames[8].iter().all(|&color| color == [0; 3]));
        assert_eq!(boomerang.frames[9][1], [0, 199, 98]);
        assert_eq!(boomerang.frames[9][5], [254, 127, 0]);

        let candy = render(&[region(RgbMode::CandyBox)], 1).unwrap();
        assert_eq!(candy.frames.len(), 340);
        assert!(candy.frames[0].iter().all(|&color| color == [0; 3]));
        assert_eq!(candy.frames[85][0], [253, 0, 45]);
        assert!(candy.frames[170].iter().all(|&color| color == [0; 3]));
    }
}

use super::rgb_packets::validate_rgb_ack;
use super::{checked_frame_count, rgb_flash_header, ReceiverParams};
use lianli_shared::rgb::{RgbPlaybackTiming, RgbRenderFamily, RgbRenderProfile};

#[test]
fn accepts_expected_command() {
    assert!(validate_rgb_ack(&[0x18], 0x18).is_ok());
}

#[test]
fn rejects_empty_or_wrong_command() {
    assert!(validate_rgb_ack(&[], 0x18).is_err());
    assert!(validate_rgb_ack(&[0x19], 0x18).is_err());
}

#[test]
fn flash_header_preserves_fractional_and_secondary_timing() {
    let timing = RgbPlaybackTiming {
        interval_hundredths: 1_430,
        secondary_interval_ticks: 55,
        secondary_frame_count: 123,
        outer_longest: true,
    };
    let header = rgb_flash_header(0x01_02_03, 88, 0x11_22_33_44, 300, timing).unwrap();

    assert_eq!(header[0], 0x18);
    assert_eq!(&header[16..20], &[0x11, 0x22, 0x33, 0x44]);
    assert_eq!(&header[22..26], &[0, 1, 2, 3]);
    assert_eq!(&header[27..29], &[1, 44]);
    assert_eq!(header[29], 88);
    assert_eq!(&header[34..37], &[0, 14, 30]);
    assert_eq!(&header[37..39], &[0, 55]);
    assert_eq!(header[39], 1);
    assert_eq!(&header[40..42], &[0, 123]);
}

#[test]
fn frame_count_uses_full_protocol_field() {
    assert_eq!(checked_frame_count(120).unwrap(), 120);
    assert_eq!(checked_frame_count(u16::MAX as usize).unwrap(), u16::MAX);
    assert!(checked_frame_count(0).is_err());
    assert!(checked_frame_count(u16::MAX as usize + 1).is_err());
}

#[test]
fn receiver_pids_select_verified_render_families_and_led_counts() {
    for (pid, family, leds_per_fan) in [
        (0x0101, RgbRenderFamily::Tl, 26),
        (0x0102, RgbRenderFamily::Tl, 26),
        (0x0103, RgbRenderFamily::SlInfV3, 44),
        (0x0104, RgbRenderFamily::SlInfV3, 44),
        (0x0105, RgbRenderFamily::P28, 9),
        (0x0106, RgbRenderFamily::SlV4, 52),
        (0x0107, RgbRenderFamily::Cl, 24),
    ] {
        let params = ReceiverParams::from_pid(pid).unwrap();
        let render_family = ReceiverParams::render_family(pid).unwrap();
        assert_eq!(
            params.render_profile(render_family, 3),
            Some(RgbRenderProfile {
                family,
                fan_count: 3,
                led_count: 3 * leds_per_fan,
                right_attach: false,
            })
        );
        assert_eq!(params.render_profile(render_family, 0), None);
        assert_eq!(params.render_profile(render_family, 5), None);
    }
}

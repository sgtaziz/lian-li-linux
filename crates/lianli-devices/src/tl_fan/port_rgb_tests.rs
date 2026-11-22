use super::*;
use crate::tl_fan::TlFanHandshake;
use lianli_transport::{error::TransportError, HidTransport};
use parking_lot::Mutex;

struct RecordingHid(Arc<Mutex<Vec<Vec<u8>>>>);
impl HidTransport for RecordingHid {
    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, TransportError> {
        self.0.lock().push(data.to_vec());
        Ok(data.len())
    }
    fn read_timeout(&mut self, _: &mut [u8], _: i32) -> std::result::Result<usize, TransportError> {
        Ok(0)
    }
    fn send_feature_report(&mut self, _: &[u8]) -> std::result::Result<usize, TransportError> {
        unreachable!()
    }
    fn get_feature_report(&mut self, _: &mut [u8]) -> std::result::Result<usize, TransportError> {
        unreachable!()
    }
    fn get_input_report(&mut self, _: &mut [u8]) -> std::result::Result<usize, TransportError> {
        unreachable!()
    }
    fn read_flush(&mut self) {}
}

#[test]
fn individual_animation_targets_one_fan_and_group_animation_targets_both_sides_once() {
    let packets = Arc::new(Mutex::new(Vec::new()));
    let controller = Arc::new(TlFanController {
        device: Arc::new(Mutex::new(Box::new(RecordingHid(packets.clone())))),
        last_handshake: Mutex::new(None),
        merge_layout: Mutex::new(None),
    });
    let port = TlFanPortDevice::new(controller, 2, 3);
    let mut effect = RgbEffect {
        mode: RgbMode::Wave,
        scope: RgbScope::Fan,
        ..Default::default()
    };
    port.set_zone_effect(1, &effect).unwrap();
    {
        let packets = packets.lock();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0][1], 0xa3);
        assert_eq!(&packets[0][6..9], &[0x20, 0x21, 21]);
    }
    packets.lock().clear();
    effect.scope = RgbScope::All;
    port.set_all_effects(&effect).unwrap();
    {
        let packets = packets.lock();
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0][1], 0xb0);
        assert_eq!(packets[1][1], 0xb0);
        assert_eq!(&packets[0][6..9], &[0, 16, 21]);
        assert_eq!(&packets[1][6..9], &[0, 17, 21]);
    }
    effect.scope = RgbScope::Fan;
    effect.mode = RgbMode::Breathing;
    assert!(!port.zone_effect_modes().contains(&effect.mode));
    assert!(port.set_zone_effect(1, &effect).is_err());
    assert!(port.set_zone_effect(3, &effect).is_err());
}

fn merge_fixture() -> (Arc<Mutex<Vec<Vec<u8>>>>, TlFanPortDevice) {
    let packets = Arc::new(Mutex::new(Vec::new()));
    let controller = Arc::new(TlFanController {
        device: Arc::new(Mutex::new(Box::new(RecordingHid(packets.clone())))),
        last_handshake: Mutex::new(Some(TlFanHandshake {
            port_fan_counts: [2, 0, 1, 0],
            fans: Vec::new(),
        })),
        merge_layout: Mutex::new(None),
    });
    (packets.clone(), TlFanPortDevice::new(controller, 2, 1))
}

fn effect(mode: RgbMode, colors: Vec<[u8; 3]>) -> RgbEffect {
    RgbEffect {
        mode,
        colors,
        ..Default::default()
    }
}

#[test]
fn whole_fan_merge_groups_both_sides_in_port_order() {
    let (packets, port) = merge_fixture();
    let merge = RgbPortMerge {
        enabled: true,
        effect: effect(RgbMode::Wave, vec![]),
        port_order: vec![2],
        ..Default::default()
    };
    port.apply_port_merge(Some(&merge)).unwrap();
    port.apply_port_merge(Some(&merge)).unwrap();
    {
        let packets = packets.lock();
        assert_eq!(packets.len(), 4);
        assert_eq!(packets[0][1], 0xad);
        assert_eq!(&packets[0][6..11], &[0, 3, 0xe0, 0xc0, 0xc1]);
        assert_eq!(packets[1][1], 0xb0);
        assert_eq!(&packets[1][6..9], &[0, 0, 21]);
        assert_eq!(packets[2..], packets[..2]);
    }
    assert!(port
        .set_all_effects(&effect(RgbMode::Rainbow, vec![]))
        .is_err());
    let breathing = RgbPortMerge {
        effect: effect(RgbMode::Breathing, vec![]),
        ..merge
    };
    assert!(port.apply_port_merge(Some(&breathing)).is_err());

    packets.lock().clear();
    port.apply_port_merge(None).unwrap();
    port.apply_port_merge(None).unwrap();
    let groups: Vec<u8> = packets.lock().iter().map(|packet| packet[6]).collect();
    assert_eq!(groups, [0, 1, 16, 17]);
}

#[test]
fn merged_static_and_side_effects_split_tops_and_bottoms() {
    let (packets, port) = merge_fixture();
    let red = [255, 0, 0];
    let blue = [0, 0, 255];
    port.apply_port_merge(Some(&RgbPortMerge {
        enabled: true,
        effect: effect(RgbMode::Static, vec![red, blue]),
        ..Default::default()
    }))
    .unwrap();
    {
        let packets = packets.lock();
        assert_eq!(packets.len(), 4);
        assert_eq!(&packets[0][6..11], &[0, 3, 0x80, 0x81, 0xa0]);
        assert_eq!(&packets[1][6..11], &[1, 3, 0x40, 0x41, 0x60]);
        assert_eq!(&packets[2][6..9], &[0, 1, 3]);
        assert_eq!(&packets[2][11..14], &blue);
        assert_eq!(packets[2][25], 1);
        assert_eq!(&packets[3][6..9], &[0, 0, 3]);
        assert_eq!(&packets[3][11..14], &red);
    }

    packets.lock().clear();
    let mut sides = RgbPortMerge {
        enabled: true,
        split_sides: true,
        sync_sides: true,
        top: effect(RgbMode::Tide, vec![red, blue]),
        bottom: effect(RgbMode::Breathing, vec![]),
        ..Default::default()
    };
    port.apply_port_merge(Some(&sides)).unwrap();
    {
        let packets = packets.lock();
        assert_eq!(packets.len(), 4);
        assert_eq!(&packets[0][6..8], &[0, 3]);
        assert_eq!(&packets[1][6..8], &[1, 3]);
        assert_eq!(&packets[2][6..9], &[0, 1, 9]);
        assert_eq!(&packets[3][6..9], &[0, 0, 9]);
    }
    sides.sync_sides = false;
    assert!(port.apply_port_merge(Some(&sides)).is_err());
    sides.bottom = effect(RgbMode::Wave, vec![]);
    assert!(port.apply_port_merge(Some(&sides)).is_err());
}

#[test]
fn port_reverse_sends_the_port_direction_flag() {
    let (packets, port) = merge_fixture();
    port.set_port_reverse(true).unwrap();
    port.set_port_reverse(false).unwrap();
    let packets = packets.lock();
    assert_eq!(packets[0][1], 0xaf);
    assert_eq!(&packets[0][6..8], &[0x20, 1]);
    assert_eq!(&packets[1][6..8], &[0x20, 0]);
}

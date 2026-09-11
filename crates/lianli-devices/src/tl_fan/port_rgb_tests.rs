use super::*;
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

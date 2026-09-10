use super::controller::WirelessController;
use super::convergence::AckSignal;
use super::discovery::DiscoveredDevice;
use super::discovery::ACK_FRESHNESS;
use super::fan_type::WirelessFanType;
use super::{AIO_PARAM_LEN, RF_AIO_PARAMS, RF_AIO_SWITCH_WIRELESS, RF_DATA_SIZE, RF_SELECT};
use anyhow::Result;
use std::time::Instant;
use tracing::debug;

impl WirelessController {
    pub fn switch_to_wireless_theme(&self, mac: &[u8; 6]) -> Result<u8> {
        let device = self.device_by_mac_snapshot(mac)?;
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        let sequence = self.bump_target_cmd_seq(mac, device.cmd_seq);
        let rf_data = wireless_theme_packet(
            &device,
            &master_mac,
            master_ch,
            self.next_slot_index(&device),
            sequence,
        );
        self.enqueue_rf_command(
            &device,
            rf_data,
            AckSignal::CmdSeq(sequence),
            "AIO wireless theme",
        )?;
        Ok(sequence)
    }

    pub fn wireless_theme_acked(&self, mac: &[u8; 6], sequence: u8, sent_at: Instant) -> bool {
        self.device_health.lock().get(mac).is_some_and(|health| {
            health.raw_seen >= sent_at
                && health.raw_seen.elapsed() <= ACK_FRESHNESS
                && health.published.cmd_seq == sequence
        })
    }

    /// Send the 32-byte aio_param block. Carries pump speed, on-screen sensor
    /// values + enables, text colors, LCD brightness, rotation, theme index,
    /// loop interval.
    pub fn set_aio_params(&self, mac: &[u8; 6], aio_param: &[u8; AIO_PARAM_LEN]) -> Result<()> {
        let device = self.device_by_mac_snapshot(mac)?;
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();
        let slot_index = self.next_slot_index(&device);

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_AIO_PARAMS;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;
        rf_data[15] = master_ch;
        rf_data[16] = slot_index;
        rf_data[18..18 + AIO_PARAM_LEN].copy_from_slice(aio_param);

        self.tx_recover(|handle| self.send_rf_packet(handle, &device, &rf_data))?;

        debug!(
            "set_aio_params sent to {} (pump_timer={}, theme={})",
            device.mac_str(),
            u16::from_be_bytes([aio_param[28], aio_param[29]]),
            aio_param[27]
        );
        Ok(())
    }
}

fn wireless_theme_packet(
    device: &DiscoveredDevice,
    master_mac: &[u8; 6],
    channel: u8,
    slot: u8,
    sequence: u8,
) -> Vec<u8> {
    let mut packet = vec![0; RF_DATA_SIZE];
    packet[0] = RF_SELECT;
    packet[1] = RF_AIO_SWITCH_WIRELESS;
    packet[2..8].copy_from_slice(&device.mac);
    packet[8..14].copy_from_slice(master_mac);
    packet[14] = device.rx_type;
    packet[15] = channel;
    packet[16] = slot;
    packet[17] = sequence;
    packet
}

/// Map pump target RPM → firmware PWM timer value for the given AIO variant.
/// Returns `None` for non-AIO device types.
pub fn pump_rpm_to_timer(rpm: u32, variant: WirelessFanType) -> Option<u16> {
    match variant {
        WirelessFanType::WaterBlock => Some(circle_pump_timer(rpm)),
        WirelessFanType::WaterBlock2 => Some(square_pump_timer(rpm)),
        _ => None,
    }
}

fn circle_pump_timer(rpm: u32) -> u16 {
    let rpm = rpm.clamp(1600, 2500) as f32;
    let t = if rpm <= 1720.0 {
        1500.0 - (rpm - 1600.0) * 1.667
    } else if rpm <= 1870.0 {
        1300.0 - (rpm - 1720.0) * 2.0
    } else if rpm <= 2000.0 {
        1000.0 - (rpm - 1870.0) * 1.23
    } else if rpm <= 2300.0 {
        840.0 - (rpm - 2000.0) * 2.0
    } else if rpm <= 2400.0 {
        240.0 - (rpm - 2300.0) * 1.8
    } else {
        60.0 - (rpm - 2400.0) * 0.5
    };
    t.clamp(0.0, u16::MAX as f32) as u16
}

fn square_pump_timer(rpm: u32) -> u16 {
    let rpm = rpm.clamp(1600, 3200) as f32;
    let t = if rpm <= 1800.0 {
        1590.0 - (rpm - 1600.0) * 0.95
    } else if rpm <= 2000.0 {
        1400.0 - (rpm - 1800.0)
    } else if rpm <= 2200.0 {
        1200.0 - (rpm - 2000.0)
    } else if rpm <= 2400.0 {
        1000.0 - (rpm - 2200.0)
    } else if rpm <= 2600.0 {
        800.0 - (rpm - 2400.0)
    } else if rpm <= 2800.0 {
        580.0 - (rpm - 2600.0) * 1.11
    } else if rpm <= 3000.0 {
        330.0 - (rpm - 2800.0) * 1.2
    } else {
        90.0 - (rpm - 3000.0) * 0.45
    };
    t.clamp(0.0, u16::MAX as f32) as u16
}

#[cfg(test)]
mod aio_tests {
    use super::*;

    #[test]
    fn wireless_theme_packet_carries_slot_sequence_and_requires_fresh_ack() {
        let mut record = [0; 42];
        record[..6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        record[13] = 2;
        record[18] = 10;
        record[41] = 0x1c;
        let device = super::super::discovery::parse_device_record(&record, 0).unwrap();
        let packet = wireless_theme_packet(&device, &[9; 6], 8, 3, 7);
        assert_eq!(
            &packet[..18],
            &[0x12, 0x19, 1, 2, 3, 4, 5, 6, 9, 9, 9, 9, 9, 9, 2, 8, 3, 7]
        );
        assert_eq!(packet.len(), 240);
        assert!(packet[18..].iter().all(|&byte| byte == 0));

        let controller = WirelessController::new();
        let mac = device.mac;
        let mut health = super::super::discovery::DeviceHealth::new(device);
        health.published.cmd_seq = 7;
        let sent_at = Instant::now();
        health.raw_seen = sent_at - std::time::Duration::from_millis(1);
        controller.device_health.lock().insert(mac, health);
        assert!(!controller.wireless_theme_acked(&mac, 7, sent_at));
        controller
            .device_health
            .lock()
            .get_mut(&mac)
            .unwrap()
            .raw_seen = Instant::now();
        assert!(controller.wireless_theme_acked(&mac, 7, sent_at));
        assert!(!controller.wireless_theme_acked(&mac, 8, sent_at));
        controller
            .device_health
            .lock()
            .get_mut(&mac)
            .unwrap()
            .raw_seen = sent_at - ACK_FRESHNESS;
        assert!(!controller.wireless_theme_acked(&mac, 7, sent_at - ACK_FRESHNESS));
    }

    #[test]
    fn circle_curve_clamps_to_range() {
        assert_eq!(circle_pump_timer(1000), circle_pump_timer(1600));
        assert_eq!(circle_pump_timer(5000), circle_pump_timer(2500));
    }

    #[test]
    fn circle_curve_spans_each_segment() {
        assert_eq!(circle_pump_timer(1600), 1500);
        assert_eq!(circle_pump_timer(1700), 1333);
        assert_eq!(circle_pump_timer(1800), 1140);
        assert_eq!(circle_pump_timer(1900), 963);
        assert_eq!(circle_pump_timer(2100), 640);
        assert_eq!(circle_pump_timer(2350), 150);
        assert_eq!(circle_pump_timer(2450), 35);
        assert_eq!(circle_pump_timer(2500), 10);
    }

    #[test]
    fn square_curve_clamps_to_range() {
        assert_eq!(square_pump_timer(100), square_pump_timer(1600));
        assert_eq!(square_pump_timer(9999), square_pump_timer(3200));
    }

    #[test]
    fn square_curve_spans_each_segment() {
        assert_eq!(square_pump_timer(1600), 1590);
        assert_eq!(square_pump_timer(1700), 1495);
        assert_eq!(square_pump_timer(1900), 1300);
        assert_eq!(square_pump_timer(2100), 1100);
        assert_eq!(square_pump_timer(2300), 900);
        assert_eq!(square_pump_timer(2500), 700);
        assert_eq!(square_pump_timer(2700), 469);
        assert_eq!(square_pump_timer(2900), 210);
        assert_eq!(square_pump_timer(3100), 45);
        assert_eq!(square_pump_timer(3200), 0);
    }

    #[test]
    fn pump_rpm_to_timer_dispatches_by_variant() {
        assert_eq!(
            pump_rpm_to_timer(2000, WirelessFanType::WaterBlock),
            Some(circle_pump_timer(2000))
        );
        assert_eq!(
            pump_rpm_to_timer(2000, WirelessFanType::WaterBlock2),
            Some(square_pump_timer(2000))
        );
        assert_eq!(pump_rpm_to_timer(2000, WirelessFanType::Slv3Led), None);
    }

    #[test]
    fn pump_rpm_range_per_variant() {
        assert_eq!(
            WirelessFanType::WaterBlock.pump_rpm_range(),
            Some((1600, 2500))
        );
        assert_eq!(
            WirelessFanType::WaterBlock2.pump_rpm_range(),
            Some((1600, 3200))
        );
        assert_eq!(WirelessFanType::Slv3Led.pump_rpm_range(), None);
    }
}

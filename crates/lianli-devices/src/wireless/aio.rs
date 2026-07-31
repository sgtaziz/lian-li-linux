use super::controller::WirelessController;
use super::discovery::DiscoveredDevice;
use super::fan_type::WirelessFanType;
use super::{
    AIO_PARAM_LEN, RF_AIO_PARAMS, RF_AIO_SWITCH_WIRELESS, RF_CHUNKS, RF_CHUNK_SIZE, RF_DATA_SIZE,
    RF_SELECT, USB_CMD_SEND_RF,
};
use anyhow::{Context, Result};
use lianli_transport::usb::{RusbBulk, USB_TIMEOUT};
use std::thread;
use std::time::Duration;
use tracing::debug;

impl WirelessController {
    /// Signal an AIO device to start honouring RF-driven theme / pump state.
    /// Must be sent once per AIO MAC after discovery, before the first `set_aio_params`.
    /// Idempotent — safe to re-invoke on reconnects.
    pub fn switch_to_wireless_theme(&self, mac: &[u8; 6]) -> Result<()> {
        let device = self.device_by_mac_snapshot(mac)?;
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_AIO_SWITCH_WIRELESS;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;
        rf_data[15] = master_ch;

        self.tx_recover(|handle| {
            for _ in 0..10 {
                send_rf_frame_via(handle, &device, &rf_data)?;
                thread::sleep(Duration::from_millis(2));
            }
            Ok(())
        })?;

        debug!("switch_to_wireless_theme sent to {}", device.mac_str());
        Ok(())
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

        self.tx_recover(|handle| send_rf_frame_via(handle, &device, &rf_data))?;

        debug!(
            "set_aio_params sent to {} (pump_timer={}, theme={})",
            device.mac_str(),
            u16::from_be_bytes([aio_param[28], aio_param[29]]),
            aio_param[27]
        );
        Ok(())
    }
}

fn send_rf_frame_via(handle: &RusbBulk, device: &DiscoveredDevice, rf_data: &[u8]) -> Result<()> {
    assert_eq!(rf_data.len(), RF_DATA_SIZE);
    for chunk_idx in 0..RF_CHUNKS as u8 {
        let mut packet = vec![0u8; 64];
        packet[0] = USB_CMD_SEND_RF;
        packet[1] = chunk_idx;
        packet[2] = device.channel;
        packet[3] = device.rx_type;

        let start = chunk_idx as usize * RF_CHUNK_SIZE;
        let end = start + RF_CHUNK_SIZE;
        packet[4..64].copy_from_slice(&rf_data[start..end]);

        handle
            .write(&packet, USB_TIMEOUT)
            .context("sending RF packet chunk")?;
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
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

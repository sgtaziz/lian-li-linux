//! Strimer Plus wired LED strip driver (VID=0x0CF2, PID=0xA200).
//!
//! Uses HID feature reports for control commands and output reports for color
//! data. 40 ms inter-command delay (double the ENE6K77 family's 20 ms).
//! 12 ports across 2 channels, up to 27 LEDs per port.

use crate::traits::RgbDevice;
use anyhow::{bail, Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope, RgbZoneInfo};
use lianli_transport::RusbHid;
use parking_lot::Mutex;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::{debug, info};

const REPORT_ID: u8 = 0xE0;
const CMD_DELAY: Duration = Duration::from_millis(40);

const PORT_COUNT: u8 = 12;
const MAX_LEDS_PER_PORT: u16 = 27;

pub struct StrimerPlusController {
    device: Arc<Mutex<RusbHid>>,
    firmware: Mutex<Option<String>>,
}

impl StrimerPlusController {
    pub fn new(device: Arc<Mutex<RusbHid>>) -> Result<Self> {
        let ctrl = Self {
            device,
            firmware: Mutex::new(None),
        };
        ctrl.read_firmware().ok();
        Ok(ctrl)
    }

    pub fn firmware_str(&self) -> Option<String> {
        self.firmware.lock().clone()
    }

    fn read_firmware(&self) -> Result<()> {
        self.send_feature(&[REPORT_ID, 0x50, 0x00])?;
        let data = self.read_input(5)?;
        let fw = format!(
            "v{}.{} (cust={:#04x} proj={:#04x} major={:#04x} minor={:#04x})",
            ((data[4] >> 4) as u16 + 1) / 10,
            ((data[4] & 0x0F) as i16 - 3).max(0) as u16,
            data[0],
            data[1],
            data[2],
            data[3]
        );
        info!("Strimer Plus firmware: {fw}");
        *self.firmware.lock() = Some(fw);
        Ok(())
    }

    fn set_effect_setting(
        &self,
        port: u8,
        mode: u8,
        speed: u8,
        dir: u8,
        brightness: u8,
    ) -> Result<()> {
        self.send_feature(&[
            REPORT_ID,
            0x10 | (port & 0x0F),
            mode,
            speed,
            dir,
            brightness,
        ])
    }

    fn set_effect_enable(&self, port: u8) -> Result<()> {
        self.send_feature(&[REPORT_ID, 0x20 | (port & 0x0F), 0x00, 0x00])
    }

    fn set_effect_enable_multi(&self, bitmap: u16) -> Result<()> {
        self.send_feature(&[
            REPORT_ID,
            0x20 | 0x0C,
            (bitmap >> 8) as u8,
            (bitmap & 0xFF) as u8,
        ])
    }

    fn set_color_setting(&self, port: u8, colors: &[[u8; 3]]) -> Result<()> {
        let mut buf = vec![REPORT_ID, 0x30 | (port & 0x0F)];
        for c in colors {
            buf.push(c[0]); // R
            buf.push(c[2]); // B (firmware quirk: B before G)
            buf.push(c[1]); // G
        }
        self.send_output(&buf)
    }

    fn send_feature(&self, data: &[u8]) -> Result<()> {
        let mut dev = self.device.lock();
        dev.send_feature_report(data)
            .context("Strimer Plus: send feature report")?;
        drop(dev);
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    fn send_output(&self, data: &[u8]) -> Result<()> {
        let mut dev = self.device.lock();
        dev.write(data)
            .context("Strimer Plus: send output report")?;
        drop(dev);
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    fn read_input(&self, expected_len: usize) -> Result<Vec<u8>> {
        let mut dev = self.device.lock();
        let mut buf = vec![0u8; 65];
        buf[0] = REPORT_ID;
        let n = dev
            .get_input_report(&mut buf)
            .context("Strimer Plus: get input report")?;
        if n < expected_len {
            bail!("Strimer Plus: expected {expected_len} bytes, got {n}");
        }
        Ok(buf[1..=expected_len].to_vec())
    }

    fn map_mode(&self, mode: RgbMode) -> u8 {
        match mode {
            RgbMode::Off => 0,
            RgbMode::Static => 1,
            RgbMode::Breathing => 2,
            RgbMode::RainbowMorph => 4,
            RgbMode::Rainbow => 5,
            RgbMode::Wave => 24,
            RgbMode::Snooker => 25,
            RgbMode::Mixing => 26,
            RgbMode::PingPong => 27,
            RgbMode::Runway => 28,
            RgbMode::Paint => 29,
            RgbMode::Tide => 30,
            RgbMode::BlowUp => 31,
            RgbMode::Meteor => 32,
            RgbMode::ShockWave => 33,
            RgbMode::Ripple => 34,
            RgbMode::Voice => 35,
            RgbMode::BulletStack => 36,
            RgbMode::Drizzling => 37,
            RgbMode::FadeOut => 38,
            RgbMode::ColorTransfer => 39,
            RgbMode::CrossOver => 40,
            RgbMode::Twinkle => 41,
            RgbMode::Contest => 42,
            RgbMode::Parallel => 43,
            _ => 1,
        }
    }

    fn map_speed(speed: u8) -> u8 {
        match speed {
            0 => 2,
            1 => 1,
            2 => 0,
            3 => 255,
            4 => 254,
            _ => 0,
        }
    }

    fn map_brightness(brightness: u8) -> u8 {
        match brightness {
            0 => 4,
            1 => 3,
            2 => 2,
            3 => 1,
            4 => 0,
            _ => 2,
        }
    }
}

impl RgbDevice for StrimerPlusController {
    fn device_name(&self) -> String {
        "Strimer Plus".to_string()
    }

    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![
            RgbMode::Off,
            RgbMode::Static,
            RgbMode::Breathing,
            RgbMode::RainbowMorph,
            RgbMode::Rainbow,
            RgbMode::Wave,
            RgbMode::Snooker,
            RgbMode::Mixing,
            RgbMode::PingPong,
            RgbMode::Runway,
            RgbMode::Paint,
            RgbMode::Tide,
            RgbMode::BlowUp,
            RgbMode::Meteor,
            RgbMode::ShockWave,
            RgbMode::Ripple,
            RgbMode::Voice,
            RgbMode::BulletStack,
            RgbMode::Drizzling,
            RgbMode::FadeOut,
            RgbMode::ColorTransfer,
            RgbMode::CrossOver,
            RgbMode::Twinkle,
            RgbMode::Contest,
            RgbMode::Parallel,
        ]
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        (0..PORT_COUNT)
            .map(|port| RgbZoneInfo {
                name: format!("Port {}", port + 1),
                led_count: MAX_LEDS_PER_PORT,
            })
            .collect()
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        vec![vec![]; PORT_COUNT as usize]
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        if zone >= PORT_COUNT {
            bail!("Port {zone} out of range (0-{})", PORT_COUNT - 1);
        }
        let mode = self.map_mode(effect.mode);
        let speed = Self::map_speed(effect.speed);
        let dir = match effect.direction {
            lianli_shared::rgb::RgbDirection::CounterClockwise => 1,
            _ => 0,
        };
        let brightness = Self::map_brightness(effect.brightness);

        self.set_effect_setting(zone, mode, speed, dir, brightness)?;

        // Push color data
        if !effect.colors.is_empty() {
            let colors: Vec<[u8; 3]> =
                if matches!(effect.mode, RgbMode::Static | RgbMode::Breathing) {
                    // Replicate first color ×27 for static/breathing
                    vec![effect.colors[0]; MAX_LEDS_PER_PORT as usize]
                } else {
                    effect.colors.iter().map(|c| [c[0], c[1], c[2]]).collect()
                };
            self.set_color_setting(zone, &colors)?;
        }

        // Enable the effect
        self.set_effect_enable(zone)?;
        debug!(
            "Strimer Plus port {zone}: mode={mode} speed={speed} dir={dir} brightness={brightness}"
        );
        Ok(())
    }

    fn supports_mb_rgb_sync(&self) -> bool {
        true
    }

    fn set_mb_rgb_sync(&self, enabled: bool) -> Result<()> {
        // MB ARGB sync uses mode byte 0x40 as a special selector
        self.set_effect_setting(0, 0x40, enabled as u8, 0, 0)?;
        self.set_effect_enable_multi(0)?;
        debug!("Strimer Plus MB RGB sync: {enabled}");
        Ok(())
    }
}

pub struct StrimerPlusDriver;

impl crate::registry::DeviceDriver for StrimerPlusDriver {
    fn family(&self) -> lianli_shared::device_id::DeviceFamily {
        lianli_shared::device_id::DeviceFamily::StrimerPlus
    }

    fn open(
        &self,
        ctx: &crate::registry::OpenContext,
    ) -> anyhow::Result<crate::registry::OpenedDevice> {
        let backend: crate::registry::SharedHid = crate::detect::open_hid_with_reopener(
            ctx.device.clone(),
            ctx.hid_usage_page,
            ctx.vid,
            ctx.pid,
            ctx.bus,
            ctx.device.port_numbers().unwrap_or_default(),
        )?;
        let ctrl = Arc::new(StrimerPlusController::new(backend.clone())?);
        let firmware = ctrl.firmware_str();
        Ok(crate::registry::OpenedDevice {
            id: ctx.device_id(),
            family: lianli_shared::device_id::DeviceFamily::StrimerPlus,
            capabilities: lianli_shared::device_id::DeviceFamily::StrimerPlus.capabilities(),
            transport_kind: lianli_shared::device_id::TransportKind::Hid,
            model_name: "Strimer Plus".to_string(),
            firmware,
            fan: None,
            lcd: None,
            rgb: vec![(
                String::new(),
                Box::new(ctrl) as Box<dyn crate::traits::RgbDevice>,
            )],
            aio: None,
            shared_hid: Some(backend),
        })
    }
}

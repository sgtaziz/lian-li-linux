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
const PORTS_PER_CHANNEL: u8 = 6;
const MAX_LEDS_PER_PORT: u16 = 27;

/// 6-color rainbow palette injected for Rainbow / RainbowMorph / BulletStack /
/// Twinkle modes.
const DEFAULT_COLORS: [[u8; 3]; 6] = [
    [255, 0, 0],   // Red
    [255, 105, 0], // Orange
    [255, 215, 0], // Gold
    [0, 255, 0],   // Green
    [0, 0, 255],   // Blue
    [170, 0, 255], // Purple
];

/// Mode bytes that span an entire channel ("Complete" modes).
/// When the base port of a channel uses one of these, only that port needs
/// to be enabled — the firmware drives all ports of the channel as one strip.
const COMPLETE_MODE_BYTES: &[u8] = &[
    33, // ShockWave
    34, // Ripple
    35, // Voice
    36, // BulletStack
    37, // Drizzling
    38, // FadeOut
    39, // ColorTransfer
    40, // CrossOver
    41, // Twinkle
    42, // Contest
    43, // Parallel
];

fn is_complete_mode_byte(mode: u8) -> bool {
    COMPLETE_MODE_BYTES.contains(&mode)
}

/// Normalize colors for a given mode:
/// - Rainbow / RainbowMorph / BulletStack / Twinkle → 6-color rainbow palette
/// - Static / Breathing → replicate first color ×27
/// - All others → pass through as-is
fn normalize_colors(mode: RgbMode, colors: &[[u8; 3]]) -> Vec<[u8; 3]> {
    match mode {
        RgbMode::Rainbow | RgbMode::RainbowMorph | RgbMode::BulletStack | RgbMode::Twinkle => {
            DEFAULT_COLORS.to_vec()
        }
        RgbMode::Static | RgbMode::Breathing => {
            let first = colors.first().copied().unwrap_or([255, 255, 255]);
            vec![first; MAX_LEDS_PER_PORT as usize]
        }
        _ => colors.to_vec(),
    }
}

pub struct StrimerPlusController {
    device: Arc<Mutex<RusbHid>>,
    firmware: Mutex<Option<String>>,
    /// Tracked mode byte per port, for channel-wide enable bitmap logic.
    port_modes: Mutex<[u8; PORT_COUNT as usize]>,
    /// Detected port count on channel 2 (4 or 6, or 6 if unknown).
    channel2_port_count: Mutex<u8>,
}

impl StrimerPlusController {
    pub fn new(device: Arc<Mutex<RusbHid>>) -> Result<Self> {
        let ctrl = Self {
            device,
            firmware: Mutex::new(None),
            port_modes: Mutex::new([1u8; PORT_COUNT as usize]),
            channel2_port_count: Mutex::new(PORTS_PER_CHANNEL),
        };
        ctrl.read_firmware().ok();
        ctrl.read_channel2_port_count().ok();
        Ok(ctrl)
    }

    pub fn firmware_str(&self) -> Option<String> {
        self.firmware.lock().clone()
    }

    fn read_firmware(&self) -> Result<()> {
        self.send_feature(&[REPORT_ID, 0x50, 0x00])?;
        let data = self.read_input(5)?;

        // Validity check: CustomerID=0xE0, ProjectID=0x52, MajorID=0xFF, MinorID=0x40
        let valid = data[0] == 0xE0 && data[1] == 0x52 && data[2] == 0xFF && data[3] == 0x40;

        let fw = if valid {
            // Version formula:
            //   hi = (FineTune >> 4) & 0xF
            //   lo = FineTune & 0xF
            //   if lo > 9 → invalid
            //   n = 10 * (hi + 1) + (lo - 3)
            //   major = n / 10, minor = n % 10
            let fine_tune = data[4];
            let hi = (fine_tune >> 4) & 0x0F;
            let lo = fine_tune & 0x0F;
            if lo > 9 {
                format!("v0.0 (invalid fine-tune {fine_tune:#04x})")
            } else {
                let n = 10 * (hi as u32 + 1) + (lo as u32 - 3);
                format!("v{}.{}", n / 10, n % 10)
            }
        } else {
            format!(
                "v0.0 (unrecognized: cust={:#04x} proj={:#04x} major={:#04x} minor={:#04x})",
                data[0], data[1], data[2], data[3]
            )
        };
        info!("Strimer Plus firmware: {fw}");
        *self.firmware.lock() = Some(fw);
        Ok(())
    }

    /// Query how many ports are physically connected on channel 2.
    /// Returns 4 or 6. Falls back to 6 on error.
    fn read_channel2_port_count(&self) -> Result<u8> {
        self.send_feature(&[REPORT_ID, 0x50, 0x01])?;
        let data = self.read_input(1)?;
        let count = data[0];
        if count == 4 || count == 6 {
            *self.channel2_port_count.lock() = count;
            debug!("Strimer Plus channel 2 port count: {count}");
            Ok(count)
        } else {
            bail!("unexpected channel 2 port count: {count}");
        }
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

    /// Build the port-enable bitmap for a given channel, based on the tracked
    /// mode of the channel's base port. Complete modes enable only the base
    /// Complete modes enable only the base port; Individual modes enable all.
    fn channel_enable_bitmap(&self, channel: u8) -> u16 {
        let base = channel * PORTS_PER_CHANNEL;
        let count = if channel == 1 {
            *self.channel2_port_count.lock()
        } else {
            PORTS_PER_CHANNEL
        };
        let modes = self.port_modes.lock();
        let mut bitmap = 0u16;
        if is_complete_mode_byte(modes[base as usize]) {
            // Complete mode: only base port
            bitmap |= 1 << base;
        } else {
            // Individual mode: all ports of this channel
            for p in 0..count {
                bitmap |= 1 << (base + p);
            }
        }
        bitmap
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
        if lianli_shared::rgb::is_brightness_off(brightness) {
            return 255;
        }
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
        let ch2_count = *self.channel2_port_count.lock();
        let total_ports = PORTS_PER_CHANNEL + ch2_count;
        (0..total_ports)
            .map(|port| RgbZoneInfo {
                name: format!("Port {}", port + 1),
                led_count: MAX_LEDS_PER_PORT,
            })
            .collect()
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        let ch2_count = *self.channel2_port_count.lock();
        let total_ports = PORTS_PER_CHANNEL + ch2_count;
        vec![vec![]; total_ports as usize]
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        let total_ports = PORTS_PER_CHANNEL + *self.channel2_port_count.lock();
        if zone >= total_ports {
            bail!("Port {zone} out of range (0-{})", total_ports - 1);
        }
        let mode = self.map_mode(effect.mode);
        let speed = Self::map_speed(effect.speed);
        let dir = match effect.direction {
            lianli_shared::rgb::RgbDirection::CounterClockwise => 1,
            _ => 0,
        };
        let brightness = Self::map_brightness(effect.brightness);

        // Track the mode for channel bitmap logic.
        self.port_modes.lock()[zone as usize] = mode;

        self.set_effect_setting(zone, mode, speed, dir, brightness)?;

        // Push normalized color data.
        let input_colors: Vec<[u8; 3]> = effect.colors.iter().map(|c| [c[0], c[1], c[2]]).collect();
        let colors = normalize_colors(effect.mode, &input_colors);
        if !colors.is_empty() {
            self.set_color_setting(zone, &colors)?;
        }

        // Build channel-aware enable bitmap. The bitmap covers both channels:
        // each channel's enable set is derived from its base port's tracked mode.
        let bitmap = self.channel_enable_bitmap(0) | self.channel_enable_bitmap(1);
        self.set_effect_enable_multi(bitmap)?;

        debug!(
            "Strimer Plus port {zone}: mode={mode} speed={speed} dir={dir} brightness={brightness} bitmap={bitmap:#06x}"
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

    fn supports_merge_lighting(&self) -> bool {
        true
    }

    fn start_merge_lighting(&self) -> Result<()> {
        // Enable all connected ports at once
        let bitmap = self.channel_enable_bitmap(0) | self.channel_enable_bitmap(1);
        self.set_effect_enable_multi(bitmap)?;
        debug!("Strimer Plus merge lighting started (bitmap={bitmap:#06x})");
        Ok(())
    }

    fn stop_merge_lighting(&self) -> Result<()> {
        // Revert to per-port mode (only port 0)
        self.set_effect_enable(0)?;
        debug!("Strimer Plus merge lighting stopped");
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
                Arc::new(ctrl) as Arc<dyn crate::traits::RgbDevice>,
            )],
            aio: None,
            shared_hid: Some(backend),
        })
    }
}

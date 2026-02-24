use anyhow::Result;
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbZoneInfo};
use lianli_shared::screen::ScreenInfo;

/// A device that can control fan speeds.
pub trait FanDevice: Send + Sync {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()>;
    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()>;
    fn read_fan_rpm(&self) -> Result<Vec<u16>>;
    fn fan_slot_count(&self) -> u8;

    /// Per-port fan counts: `(port_index, fan_count)`.
    /// Default: single port with `fan_slot_count()` fans.
    fn fan_port_info(&self) -> Vec<(u8, u8)> {
        vec![(0, self.fan_slot_count())]
    }

    /// Whether individual fans can be set to different speeds.
    /// Default: true (per-fan). Override to false for per-port devices.
    fn per_fan_control(&self) -> bool {
        true
    }

    /// Whether this device supports motherboard RPM sync (hardware passthrough).
    fn supports_mb_sync(&self) -> bool {
        false
    }

    /// Enable or disable motherboard RPM sync for a port.
    /// Only meaningful for devices where `supports_mb_sync()` returns true.
    fn set_mb_rpm_sync(&self, _port: u8, _sync: bool) -> Result<()> {
        anyhow::bail!("MB RPM sync not supported by this device")
    }
}

/// A device with an LCD screen.
pub trait LcdDevice: Send + Sync {
    fn screen_info(&self) -> &ScreenInfo;
    fn send_jpeg_frame(&mut self, jpeg_data: &[u8]) -> Result<()>;
    fn set_brightness(&self, brightness: u8) -> Result<()>;
    fn set_rotation(&self, degrees: u16) -> Result<()>;
    fn initialize(&mut self) -> Result<()>;
}

/// An AIO device with pump, fans, and optionally LCD.
pub trait AioDevice: FanDevice {
    fn set_pump_speed(&self, duty: u8) -> Result<()>;
    fn read_pump_rpm(&self) -> Result<u16>;
    fn read_coolant_temp(&self) -> Result<f32>;
}

/// A device that can control RGB/LED effects.
///
/// Two control modes:
/// - **Effect mode**: Set a hardware-native effect (mode + colors + speed + brightness).
///   Used by wired devices (TL Fan, ENE 6K77, Galahad2) and the native GUI.
/// - **Direct mode**: Set per-LED colors directly. Used by OpenRGB `UpdateLEDs`.
///   For wired devices, maps to Static mode. For wireless, streams RGB frames via RF.
pub trait RgbDevice: Send + Sync {
    /// Human-readable device name (e.g., "UNI FAN TL Controller").
    fn device_name(&self) -> String;

    /// Supported LED effect modes for this device.
    fn supported_modes(&self) -> Vec<RgbMode>;

    /// Information about each independently controllable LED zone.
    fn zone_info(&self) -> Vec<RgbZoneInfo>;

    /// Total LED count across all zones.
    fn total_led_count(&self) -> u16 {
        self.zone_info().iter().map(|z| z.led_count).sum()
    }

    /// Set a zone's effect mode + parameters.
    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()>;

    /// Set all zones to the same effect.
    fn set_all_effects(&self, effect: &RgbEffect) -> Result<()> {
        for zone in 0..self.zone_info().len() as u8 {
            self.set_zone_effect(zone, effect)?;
        }
        Ok(())
    }

    /// Set per-LED colors directly for a zone (used by OpenRGB `UpdateLEDs`).
    ///
    /// `colors` is a slice of RGB triplets, one per LED in the zone.
    /// Default implementation maps to Static mode with the first color.
    fn set_direct_colors(&self, zone: u8, colors: &[[u8; 3]]) -> Result<()> {
        let color = colors.first().copied().unwrap_or([255, 255, 255]);
        let effect = RgbEffect {
            mode: RgbMode::Static,
            colors: vec![color],
            ..RgbEffect::default()
        };
        self.set_zone_effect(zone, &effect)
    }

    /// Whether this device supports true per-LED direct color control.
    /// When false, `set_direct_colors` maps to Static mode with the first color.
    fn supports_direct(&self) -> bool {
        false
    }

    /// Whether this device supports motherboard ARGB sync (passthrough from MB header).
    fn supports_mb_rgb_sync(&self) -> bool {
        false
    }

    /// Enable or disable motherboard ARGB sync.
    /// When enabled, the device reads ARGB from the motherboard header instead of using
    /// software-controlled effects.
    fn set_mb_rgb_sync(&self, _enabled: bool) -> Result<()> {
        anyhow::bail!("MB RGB sync not supported by this device")
    }
}

use anyhow::Result;
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope, RgbZoneInfo};
use lianli_shared::screen::ScreenInfo;
use std::sync::Arc;

/// A device that can control fan speeds.
///
/// `duty` values passed to `set_fan_speed`, `set_fan_speeds`, and `set_pump_speed`
/// are raw PWM in the range 0-255. Implementations scale to device-native units.
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

    /// Whether this device has a controllable pump.
    fn has_pump_control(&self) -> bool {
        false
    }

    /// Only for devices where `has_pump_control()` is true.
    fn set_pump_speed(&self, _duty: u8) -> Result<()> {
        anyhow::bail!("Pump speed control not supported by this device")
    }

    /// Poll coolant temperature from wired AIO devices (HydroShift LCD, Galahad2).
    /// Returns `None` if the device doesn't have a coolant sensor.
    /// Ref: PR #103 — exposes wired coolant telemetry to fan curves.
    fn poll_coolant_temp(&self) -> Option<f32> {
        None
    }

    /// Whether this device exposes a per-port daisy-chain "fan quantity" override.
    fn supports_fan_quantity(&self) -> bool {
        false
    }

    /// Maximum fan count per port for the quantity override (only meaningful
    /// when `supports_fan_quantity()` is true).
    fn max_fan_quantity_per_port(&self) -> u8 {
        1
    }

    /// Set the per-port daisy-chain fan quantity.
    fn set_port_fan_quantity(&self, _port: u8, _quantity: u8) -> Result<()> {
        anyhow::bail!("Fan quantity override not supported by this device")
    }

    fn stop_pwm(&self) -> u8 {
        0
    }
}

/// Blanket forwarding impl so any `Arc<T>` can be used as a `FanDevice`
/// without per-driver boilerplate. Required for fan controllers that share
/// their USB handle with a sibling RGB controller (e.g. `TlFanController`
/// paired with `TlFanPortDevice`).
impl<T: FanDevice + ?Sized> FanDevice for Arc<T> {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        (**self).set_fan_speed(slot, duty)
    }
    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        (**self).set_fan_speeds(duties)
    }
    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        (**self).read_fan_rpm()
    }
    fn fan_slot_count(&self) -> u8 {
        (**self).fan_slot_count()
    }
    fn fan_port_info(&self) -> Vec<(u8, u8)> {
        (**self).fan_port_info()
    }
    fn per_fan_control(&self) -> bool {
        (**self).per_fan_control()
    }
    fn supports_mb_sync(&self) -> bool {
        (**self).supports_mb_sync()
    }
    fn set_mb_rpm_sync(&self, port: u8, sync: bool) -> Result<()> {
        (**self).set_mb_rpm_sync(port, sync)
    }
    fn has_pump_control(&self) -> bool {
        (**self).has_pump_control()
    }
    fn set_pump_speed(&self, duty: u8) -> Result<()> {
        (**self).set_pump_speed(duty)
    }
    fn poll_coolant_temp(&self) -> Option<f32> {
        (**self).poll_coolant_temp()
    }
    fn supports_fan_quantity(&self) -> bool {
        (**self).supports_fan_quantity()
    }
    fn max_fan_quantity_per_port(&self) -> u8 {
        (**self).max_fan_quantity_per_port()
    }
    fn set_port_fan_quantity(&self, port: u8, quantity: u8) -> Result<()> {
        (**self).set_port_fan_quantity(port, quantity)
    }
    fn stop_pwm(&self) -> u8 {
        (**self).stop_pwm()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    NoChange,
    Recovered,
}

/// A device with an LCD screen.
pub trait LcdDevice: Send + Sync {
    fn screen_info(&self) -> &ScreenInfo;
    fn send_jpeg_frame(&mut self, jpeg_data: &[u8]) -> Result<()>;
    fn send_static_frame(&mut self, jpeg_data: &[u8]) -> Result<()> {
        self.send_jpeg_frame(jpeg_data)
    }
    fn set_brightness(&self, brightness: u8) -> Result<()>;
    fn set_rotation(&self, degrees: u16) -> Result<()>;
    fn initialize(&mut self) -> Result<()>;
    fn check_and_recover_lcd(&mut self) -> Result<RecoveryAction> {
        Ok(RecoveryAction::NoChange)
    }
    fn supports_c_command(&self) -> bool {
        false
    }
    fn firmware_version_str(&self) -> Option<&str> {
        None
    }
    fn set_use_c_command(&mut self, _enable: bool) {}
    fn try_read_firmware(&mut self) -> Result<()> {
        Ok(())
    }
    fn stream_h264_reader(
        &mut self,
        _reader: &mut dyn std::io::Read,
        _stop: &std::sync::atomic::AtomicBool,
        _fps: f32,
    ) -> Result<()> {
        anyhow::bail!("h264 streaming not supported by this device")
    }
}

/// An AIO device with pump, fans, and optionally LCD.
pub trait AioDevice: FanDevice {
    fn read_pump_rpm(&self) -> Result<u16>;
    fn read_coolant_temp(&self) -> Result<f32>;
}

impl<T: AioDevice> AioDevice for std::sync::Arc<T> {
    fn read_pump_rpm(&self) -> Result<u16> {
        (**self).read_pump_rpm()
    }
    fn read_coolant_temp(&self) -> Result<f32> {
        (**self).read_coolant_temp()
    }
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

    /// Supported scopes per zone. Return empty vec for zones with only "All".
    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        vec![]
    }

    /// Whether this device supports fan direction (swap left/right, swap top/bottom).
    fn supports_direction(&self) -> bool {
        false
    }

    /// Set fan direction (orientation) for a specific zone.
    fn set_fan_direction(&self, _zone: u8, _swap_lr: bool, _swap_tb: bool) -> Result<()> {
        anyhow::bail!("Fan direction not supported by this device")
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

    /// Whether this device supports merge-lighting (cross-group synchronized animation).
    fn supports_merge_lighting(&self) -> bool {
        false
    }

    /// Enter merge-lighting mode. After this call, effects applied to zone 0
    /// propagate across all groups as a single synchronized animation.
    fn start_merge_lighting(&self) -> Result<()> {
        anyhow::bail!("Merge lighting not supported by this device")
    }

    /// Exit merge-lighting mode. Restores per-group independent control.
    fn stop_merge_lighting(&self) -> Result<()> {
        anyhow::bail!("Merge lighting not supported by this device")
    }

    fn ping(&self, _zone: u8) -> Result<()> {
        anyhow::bail!("Ping not supported by this device")
    }
}

/// Blanket forwarding impl so any `Arc<T>` can be used as an `RgbDevice`
/// without per-driver boilerplate.
impl<T: RgbDevice + ?Sized> RgbDevice for Arc<T> {
    fn device_name(&self) -> String {
        (**self).device_name()
    }
    fn supported_modes(&self) -> Vec<RgbMode> {
        (**self).supported_modes()
    }
    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        (**self).zone_info()
    }
    fn total_led_count(&self) -> u16 {
        (**self).total_led_count()
    }
    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        (**self).set_zone_effect(zone, effect)
    }
    fn set_all_effects(&self, effect: &RgbEffect) -> Result<()> {
        (**self).set_all_effects(effect)
    }
    fn set_direct_colors(&self, zone: u8, colors: &[[u8; 3]]) -> Result<()> {
        (**self).set_direct_colors(zone, colors)
    }
    fn supports_direct(&self) -> bool {
        (**self).supports_direct()
    }
    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        (**self).supported_scopes()
    }
    fn supports_direction(&self) -> bool {
        (**self).supports_direction()
    }
    fn set_fan_direction(&self, zone: u8, swap_lr: bool, swap_tb: bool) -> Result<()> {
        (**self).set_fan_direction(zone, swap_lr, swap_tb)
    }
    fn supports_mb_rgb_sync(&self) -> bool {
        (**self).supports_mb_rgb_sync()
    }
    fn set_mb_rgb_sync(&self, enabled: bool) -> Result<()> {
        (**self).set_mb_rgb_sync(enabled)
    }
    fn supports_merge_lighting(&self) -> bool {
        (**self).supports_merge_lighting()
    }
    fn start_merge_lighting(&self) -> Result<()> {
        (**self).start_merge_lighting()
    }
    fn stop_merge_lighting(&self) -> Result<()> {
        (**self).stop_merge_lighting()
    }
    fn ping(&self, zone: u8) -> Result<()> {
        (**self).ping(zone)
    }
}

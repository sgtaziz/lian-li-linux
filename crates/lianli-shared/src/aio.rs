use crate::fan::FanSpeed;
use crate::media::SensorSourceConfig;
use serde::{Deserialize, Serialize};

// ─── Pump envelopes ───────────────────────────────────────────────────
//
// Per-variant RPM→PWM translation tables. The firmware translates a target
// RPM to a PWM percentage via piecewise-linear interpolation, clamping to
// [min_rpm, max_rpm] first. Rust's wired AIO drivers send raw PWM duty, so
// the primary use of these tables today is to clamp the PWM floor (prevent
// stall) and to provide the data for future RPM-target control.

/// Per-variant pump RPM envelope and RPM→PWM translation table.
#[derive(Debug, Clone, Copy)]
pub struct PumpEnvelope {
    pub min_rpm: u16,
    pub max_rpm: u16,
    /// Piecewise-linear map: `(rpm, pwm_percent)`. Must be sorted by rpm.
    /// First entry's pwm is the minimum safe PWM for this variant.
    pub rpm_to_pwm: &'static [(u16, u8)],
}

impl PumpEnvelope {
    /// Translate a target RPM to a PWM percentage (0-100).
    ///
    /// Clamps `rpm` to `[min_rpm, max_rpm]`, then interpolates via the
    /// piecewise-linear table.
    pub fn rpm_to_pwm(&self, rpm: u16) -> u8 {
        let clamped = rpm.clamp(self.min_rpm, self.max_rpm);
        let table = self.rpm_to_pwm;

        if clamped <= table[0].0 {
            return table[0].1;
        }
        for window in table.windows(2) {
            let (r0, p0) = (window[0].0, window[0].1);
            let (r1, p1) = (window[1].0, window[1].1);
            if clamped <= r1 {
                if r1 == r0 {
                    return p1;
                }
                let t = (clamped - r0) as f32 / (r1 - r0) as f32;
                return (p0 as f32 + t * (p1 as f32 - p0 as f32)).round() as u8;
            }
        }
        table.last().map(|e| e.1).unwrap_or(100)
    }

    /// Minimum safe PWM percentage (prevents pump stall).
    pub fn min_pwm(&self) -> u8 {
        self.rpm_to_pwm[0].1
    }

    // ── Galahad2 Trinity ──

    /// Galahad2 Trinity Performance (PID 0x7371).
    pub const GALAHAD2_PERFORMANCE: Self = Self {
        min_rpm: 2200,
        max_rpm: 4200,
        rpm_to_pwm: &[(2200, 30), (4200, 100)],
    };

    /// Galahad2 Trinity Regular (PID 0x7373).
    pub const GALAHAD2_REGULAR: Self = Self {
        min_rpm: 2200,
        max_rpm: 3200,
        rpm_to_pwm: &[(2200, 30), (3200, 100)],
    };

    // ── HydroShift LCD / Galahad2 LCD ──

    /// HydroShift LCD base (PID 0x7398) / Galahad2 LCD (0x7391).
    pub const HYDROSHIFT_LCD: Self = Self {
        min_rpm: 2200,
        max_rpm: 3800,
        rpm_to_pwm: &[(2200, 50), (2600, 60), (3000, 70), (3400, 85), (3800, 100)],
    };

    /// Galahad2 Vision (PID 0x7395). 29-point RPM→PWM phase table.
    pub const GALAHAD2_VISION: Self = Self {
        min_rpm: 800,
        max_rpm: 3600,
        rpm_to_pwm: &[
            (800, 20),
            (900, 23),
            (1000, 25),
            (1100, 27),
            (1200, 30),
            (1300, 31),
            (1400, 34),
            (1500, 36),
            (1600, 39),
            (1700, 41),
            (1800, 44),
            (1900, 47),
            (2000, 49),
            (2100, 52),
            (2200, 55),
            (2300, 58),
            (2400, 60),
            (2500, 64),
            (2600, 67),
            (2700, 70),
            (2800, 73),
            (2900, 77),
            (3000, 80),
            (3100, 83),
            (3200, 87),
            (3300, 90),
            (3400, 94),
            (3500, 98),
            (3600, 100),
        ],
    };

    /// HydroShift LCD RGB (PID 0x7399).
    /// ~18% lower max RPM than base — over-speeding risk if confused.
    pub const HYDROSHIFT_LCD_RGB: Self = Self {
        min_rpm: 2000,
        max_rpm: 3200,
        rpm_to_pwm: &[(2000, 1), (2300, 25), (2600, 50), (2900, 75), (3200, 100)],
    };

    /// HydroShift LCD TL (PID 0x739A). Same envelope as RGB.
    pub const HYDROSHIFT_LCD_TL: Self = Self::HYDROSHIFT_LCD_RGB;
}

#[cfg(test)]
mod pump_envelope_tests {
    use super::PumpEnvelope;

    #[test]
    fn clamps_above_max() {
        // Requesting 4000 RPM on RGB variant should clamp to 3200 → 100%
        let pwm = PumpEnvelope::HYDROSHIFT_LCD_RGB.rpm_to_pwm(4000);
        assert_eq!(pwm, 100);
    }

    #[test]
    fn clamps_below_min() {
        // Requesting 1000 RPM on base should clamp to 2200 → 50%
        let pwm = PumpEnvelope::HYDROSHIFT_LCD.rpm_to_pwm(1000);
        assert_eq!(pwm, 50);
    }

    #[test]
    fn interpolates_midpoint() {
        // Base: 2600 RPM → 60%, 3000 RPM → 70%. Midpoint 2800 → ~65%.
        let pwm = PumpEnvelope::HYDROSHIFT_LCD.rpm_to_pwm(2800);
        assert_eq!(pwm, 65);
    }

    #[test]
    fn galahad2_performance_accepts_4200() {
        let pwm = PumpEnvelope::GALAHAD2_PERFORMANCE.rpm_to_pwm(4500);
        assert_eq!(pwm, 100);
    }

    #[test]
    fn galahad2_regular_clamps_at_3200() {
        // Regular variant: 4500 RPM should clamp to 3200 → 100%
        let pwm = PumpEnvelope::GALAHAD2_REGULAR.rpm_to_pwm(4500);
        assert_eq!(pwm, 100);
    }

    #[test]
    fn rgb_lower_floor_than_base() {
        // RGB min PWM is 1%; base min PWM is 50%.
        assert_eq!(PumpEnvelope::HYDROSHIFT_LCD_RGB.min_pwm(), 1);
        assert_eq!(PumpEnvelope::HYDROSHIFT_LCD.min_pwm(), 50);
    }

    #[test]
    fn vision_envelope_anchors_match_vendor() {
        let env = PumpEnvelope::GALAHAD2_VISION;
        assert_eq!(env.min_rpm, 800);
        assert_eq!(env.max_rpm, 3600);
        assert_eq!(env.min_pwm(), 20);
        assert_eq!(env.rpm_to_pwm(800), 20);
        assert_eq!(env.rpm_to_pwm(3600), 100);
    }

    #[test]
    fn vision_envelope_clamps_outside_range() {
        let env = PumpEnvelope::GALAHAD2_VISION;
        assert_eq!(env.rpm_to_pwm(100), 20);
        assert_eq!(env.rpm_to_pwm(5000), 100);
    }
}

// ─── AioConfig ────────────────────────────────────────────────────────

fn default_brightness() -> u8 {
    80
}

fn default_loop_interval() -> u8 {
    3
}

fn default_pump_speed() -> FanSpeed {
    FanSpeed::Constant(128)
}

fn default_fan_speeds() -> [FanSpeed; 4] {
    [
        FanSpeed::Constant(128),
        FanSpeed::Constant(128),
        FanSpeed::Constant(128),
        FanSpeed::Constant(128),
    ]
}

fn rgba_white() -> [u8; 4] {
    [255, 255, 255, 255]
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AioConfig {
    #[serde(default = "default_pump_speed")]
    pub pump_target_rpm: FanSpeed,
    #[serde(default = "default_fan_speeds")]
    pub fan_speeds: [FanSpeed; 4],
    #[serde(default)]
    pub theme_index: u8,
    #[serde(default = "default_brightness")]
    pub brightness: u8,
    #[serde(default)]
    pub rotation: u8,
    #[serde(default = "default_loop_interval")]
    pub loop_interval: u8,
    #[serde(default)]
    pub cpu_temp_source: Option<SensorSourceConfig>,
    #[serde(default = "default_cpu_load_source")]
    pub cpu_load_source: Option<SensorSourceConfig>,
    #[serde(default)]
    pub gpu_temp_source: Option<SensorSourceConfig>,
    #[serde(default)]
    pub gpu_load_source: Option<SensorSourceConfig>,
    #[serde(default = "rgba_white")]
    pub str_color: [u8; 4],
    #[serde(default = "rgba_white")]
    pub val_color: [u8; 4],
    #[serde(default = "rgba_white")]
    pub unit_color: [u8; 4],
}

fn default_cpu_load_source() -> Option<SensorSourceConfig> {
    Some(SensorSourceConfig::CpuUsage)
}

impl Default for AioConfig {
    fn default() -> Self {
        Self {
            pump_target_rpm: default_pump_speed(),
            fan_speeds: default_fan_speeds(),
            theme_index: 0,
            brightness: default_brightness(),
            rotation: 0,
            loop_interval: default_loop_interval(),
            cpu_temp_source: None,
            cpu_load_source: default_cpu_load_source(),
            gpu_temp_source: None,
            gpu_load_source: None,
            str_color: rgba_white(),
            val_color: rgba_white(),
            unit_color: rgba_white(),
        }
    }
}

impl AioConfig {
    pub fn defaults_for_host() -> Self {
        use crate::sensors::{enumerate_sensors, pick_source_for_category, SensorCategory};
        let sensors = enumerate_sensors();
        let mut cfg = Self::default();
        cfg.cpu_temp_source = pick_source_for_category(SensorCategory::CpuTemp, &sensors);
        cfg.cpu_load_source =
            pick_source_for_category(SensorCategory::CpuUsage, &sensors).or(cfg.cpu_load_source);
        cfg.gpu_temp_source = pick_source_for_category(SensorCategory::GpuTemp, &sensors);
        cfg.gpu_load_source = pick_source_for_category(SensorCategory::GpuUsage, &sensors);
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_json() {
        let cfg = AioConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AioConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.brightness, cfg.brightness);
        assert_eq!(back.rotation, cfg.rotation);
        assert_eq!(back.loop_interval, cfg.loop_interval);
        assert_eq!(back.theme_index, cfg.theme_index);
        assert_eq!(back.str_color, cfg.str_color);
        assert_eq!(back.val_color, cfg.val_color);
        assert_eq!(back.unit_color, cfg.unit_color);
    }

    #[test]
    fn sparse_json_fills_defaults() {
        let cfg: AioConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.brightness, 80);
        assert_eq!(cfg.loop_interval, 3);
        assert_eq!(cfg.str_color, [255, 255, 255, 255]);
        assert!(matches!(
            cfg.cpu_load_source,
            Some(SensorSourceConfig::CpuUsage)
        ));
    }
}

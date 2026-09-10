use super::{RgbRenderFamily, RgbRenderProfile};

/// Per-PID parameters.
#[derive(Debug, Clone, Copy)]
pub struct ReceiverParams {
    pub leds_per_fan: u16,
    pub pwm_floor: u8,
    pub pwm_zero: u8,
    pub name: &'static str,
    pub compresses_rgb: bool,
}

impl ReceiverParams {
    pub(super) fn from_pid(pid: u16) -> Option<Self> {
        match pid {
            0x0101 => Some(Self {
                leds_per_fan: 26,
                pwm_floor: 11,
                pwm_zero: 5,
                name: "TL Flex Controller",
                compresses_rgb: true,
            }),
            0x0102 => Some(Self {
                leds_per_fan: 26,
                pwm_floor: 11,
                pwm_zero: 5,
                name: "TL Flex LCD Controller",
                compresses_rgb: true,
            }),
            0x0103 => Some(Self {
                leds_per_fan: 44,
                pwm_floor: 10,
                pwm_zero: 5,
                name: "SL-INF Flex Controller",
                compresses_rgb: true,
            }),
            0x0104 => Some(Self {
                leds_per_fan: 44,
                pwm_floor: 10,
                pwm_zero: 5,
                name: "SL-INF Flex LCD Controller",
                compresses_rgb: true,
            }),
            0x0105 => Some(Self {
                leds_per_fan: 9,
                pwm_floor: 8,
                pwm_zero: 1,
                name: "P28 V2 Controller",
                compresses_rgb: false,
            }),
            0x0106 => Some(Self {
                leds_per_fan: 52,
                pwm_floor: 14,
                pwm_zero: 5,
                name: "SL V4 Controller",
                compresses_rgb: true,
            }),
            0x0107 => Some(Self {
                leds_per_fan: 24,
                pwm_floor: 10,
                pwm_zero: 5,
                name: "CL V2 Controller",
                compresses_rgb: false,
            }),
            _ => None,
        }
    }

    pub(super) fn render_family(pid: u16) -> Option<RgbRenderFamily> {
        match pid {
            0x0101 | 0x0102 => Some(RgbRenderFamily::Tl),
            0x0103 | 0x0104 => Some(RgbRenderFamily::SlInfV3),
            0x0105 => Some(RgbRenderFamily::P28),
            0x0106 => Some(RgbRenderFamily::SlV4),
            0x0107 => Some(RgbRenderFamily::Cl),
            _ => None,
        }
    }

    pub(super) fn render_profile(
        self,
        family: RgbRenderFamily,
        fan_count: u8,
    ) -> Option<RgbRenderProfile> {
        (1..=4).contains(&fan_count).then_some(RgbRenderProfile {
            family,
            fan_count,
            led_count: u16::from(fan_count) * self.leds_per_fan,
            right_attach: false,
        })
    }
}

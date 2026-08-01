use super::protocol::{
    A_HEADER_LEN, A_PACKET_SIZE, CMD_SET_FAN_LIGHT, CMD_SET_PUMP_LIGHT, FAN_LED_COUNT, REPORT_ID_A,
};
use super::AioLcdVariant;
use crate::traits::RgbDevice;
use anyhow::{bail, Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope, RgbZoneInfo};
use lianli_transport::RusbHid;
use parking_lot::Mutex;
use std::sync::Arc;
use tracing::{debug, info};

pub struct AioLcdRgbController {
    device: Arc<Mutex<RusbHid>>,
    variant: AioLcdVariant,
}

impl AioLcdRgbController {
    pub fn new(device: Arc<Mutex<RusbHid>>, pid: u16) -> Result<Self> {
        let variant = AioLcdVariant::from_pid(pid)
            .ok_or_else(|| anyhow::anyhow!("Unknown AIO LCD PID: {pid:#06x}"))?;
        info!("Opened {} RGB controller", variant.name());
        Ok(Self { device, variant })
    }

    fn set_pump_light(&self, effect: &RgbEffect, source_mcu: bool) -> Result<()> {
        let scope = match effect.scope {
            RgbScope::Inner => 0u8,
            RgbScope::Outer => 1,
            _ => 2,
        };
        let mode_byte = effect.mode.to_hydroshift_lcd_mode_byte().unwrap_or(3);
        let mut payload = [0u8; 19];
        payload[0] = scope;
        payload[1] = mode_byte;
        payload[2] = lianli_shared::rgb::brightness_scale(effect.brightness);
        payload[3] = lianli_shared::rgb::brightness_scale(effect.speed);
        for (i, color) in effect.colors.iter().take(4).enumerate() {
            let offset = 4 + i * 3;
            payload[offset] = color[0];
            payload[offset + 1] = color[1];
            payload[offset + 2] = color[2];
        }
        payload[16] = effect.direction.to_tl_byte();
        payload[17] = (effect.disabled || effect.mode == RgbMode::Off) as u8;
        payload[18] = if source_mcu { 0 } else { 1 };
        self.send_rgb_command(CMD_SET_PUMP_LIGHT, &payload)?;
        debug!("Set pump light: mode={mode_byte} scope={scope}");
        Ok(())
    }

    fn set_fan_light(
        &self,
        effect: &RgbEffect,
        source_mcu: bool,
        sync_to_pump: bool,
    ) -> Result<()> {
        let mode_byte = effect.mode.to_hydroshift_lcd_mode_byte().unwrap_or(3);
        let mut payload = [0u8; 20];
        payload[0] = mode_byte;
        payload[1] = lianli_shared::rgb::brightness_scale(effect.brightness);
        payload[2] = lianli_shared::rgb::brightness_scale(effect.speed);
        for (i, color) in effect.colors.iter().take(4).enumerate() {
            let offset = 3 + i * 3;
            payload[offset] = color[0];
            payload[offset + 1] = color[1];
            payload[offset + 2] = color[2];
        }
        payload[15] = effect.direction.to_tl_byte();
        payload[16] = (effect.disabled || effect.mode == RgbMode::Off) as u8;
        payload[17] = if source_mcu { 0 } else { 1 };
        payload[18] = sync_to_pump as u8;
        payload[19] = FAN_LED_COUNT as u8;
        self.send_rgb_command(CMD_SET_FAN_LIGHT, &payload)?;
        debug!("Set fan light: mode={mode_byte} sync_to_pump={sync_to_pump}");
        Ok(())
    }

    fn send_rgb_command(&self, cmd: u8, data: &[u8]) -> Result<()> {
        let max_payload = A_PACKET_SIZE - A_HEADER_LEN;
        if data.len() > max_payload {
            bail!(
                "AIO LCD RGB: command {cmd:#04x} payload too large ({} > {max_payload})",
                data.len()
            );
        }
        let mut pkt = [0u8; A_PACKET_SIZE];
        pkt[0] = REPORT_ID_A;
        pkt[1] = cmd;
        pkt[5] = data.len() as u8;
        pkt[A_HEADER_LEN..A_HEADER_LEN + data.len()].copy_from_slice(data);

        let mut dev = self.device.lock();
        dev.write(&pkt).context("AIO LCD RGB: write")?;
        Ok(())
    }
}

impl RgbDevice for AioLcdRgbController {
    fn device_name(&self) -> String {
        format!("{} AIO", self.variant.name())
    }

    fn supported_modes(&self) -> Vec<RgbMode> {
        if matches!(self.variant, super::AioLcdVariant::Galahad2Vision) {
            vec![
                RgbMode::Off,
                RgbMode::Static,
                RgbMode::Rainbow,
                RgbMode::RainbowMorph,
                RgbMode::Breathing,
                RgbMode::Runway,
                RgbMode::Meteor,
            ]
        } else {
            vec![
                RgbMode::Off,
                RgbMode::Static,
                RgbMode::Rainbow,
                RgbMode::RainbowMorph,
                RgbMode::Breathing,
                RgbMode::Runway,
                RgbMode::Meteor,
                RgbMode::TickerTape,
                RgbMode::Fluctuation,
                RgbMode::Transmit,
                RgbMode::ColorfulStarryNight,
                RgbMode::StaticStarryNight,
                RgbMode::Voice,
                RgbMode::BigBang,
                RgbMode::Burst,
                RgbMode::ColorsMorph,
                RgbMode::Bounce,
            ]
        }
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        if self.variant.has_pump_rgb() {
            vec![
                RgbZoneInfo {
                    name: "Pump Head".to_string(),
                    led_count: 24,
                },
                RgbZoneInfo {
                    name: "Fans".to_string(),
                    led_count: FAN_LED_COUNT,
                },
            ]
        } else {
            vec![RgbZoneInfo {
                name: "Fans".to_string(),
                led_count: FAN_LED_COUNT,
            }]
        }
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        if self.variant.has_pump_rgb() {
            match zone {
                0 => self.set_pump_light(effect, true),
                1 => self.set_fan_light(effect, true, false),
                _ => bail!("{}: zone {zone} out of range (0-1)", self.variant.name()),
            }
        } else {
            match zone {
                0 => self.set_fan_light(effect, true, false),
                _ => bail!("{}: zone {zone} out of range (0)", self.variant.name()),
            }
        }
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        if self.variant.has_pump_rgb() {
            vec![
                vec![RgbScope::All, RgbScope::Inner, RgbScope::Outer], // Pump Head
                vec![],                                                // Fans
            ]
        } else {
            vec![]
        }
    }

    fn supports_mb_rgb_sync(&self) -> bool {
        true
    }

    fn set_mb_rgb_sync(&self, enabled: bool) -> Result<()> {
        let source_mcu = !enabled;
        let dummy = RgbEffect::default();
        if self.variant.has_pump_rgb() {
            self.set_pump_light(&dummy, source_mcu)?;
        }
        self.set_fan_light(&dummy, source_mcu, false)?;
        debug!("Set MB RGB sync: enabled={enabled}");
        Ok(())
    }
}

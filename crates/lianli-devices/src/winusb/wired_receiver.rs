//! Wired receiver driver for TL Flex / SL-INF Flex / P28 V2 / SLV4 / CLV2
//! controllers (VID `0x43A8`, PIDs `0x0101`–`0x0107`).
//!
//! All share the same WinUsbLed base protocol: 64-byte unencrypted packets,
//! TX+RX pattern. Per-PID differences: LED count, PWM floor, and whether
//! RGB is sent raw (0x11) or flash-saved (0x18 + 0x19).

use crate::traits::{FanDevice, RgbDevice};
use anyhow::{Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope, RgbZoneInfo};
use lianli_transport::usb::{RusbBulk, LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use parking_lot::Mutex;
use rusb::{Device, GlobalContext};
use std::sync::Arc;
use tracing::{debug, info, warn};

const PACKET_SIZE: usize = 64;

// LEDCmdType opcodes
const CMD_GET_VER: u8 = 0x10;
const CMD_STREAM_RGB: u8 = 0x11;
const CMD_GET_INFO: u8 = 0x12;
const CMD_SET_FANS_PWM: u8 = 0x13;
const CMD_SET_LIGHT_SYNC_MB: u8 = 0x14;
const CMD_SAVE_OR_CLEAR: u8 = 0x15;
const CMD_SELECTED_GROUP: u8 = 0x16;
const CMD_REBOOT_LCD: u8 = 0x17;
const CMD_SEND_LIGHT_PACKAGE: u8 = 0x18;
const CMD_APPLY_LIGHTING: u8 = 0x19;
const CMD_FAN_AND_FIXED_DATA: u8 = 0x26;
const CMD_FAN_THEME_COLOR: u8 = 0x27;
const CMD_WIRELESS_THEME_SWITCH: u8 = 0x29;

const RGB_LED_CHUNK: usize = 20;
const RGB_ACK_INTERVAL: u8 = 14;

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
    fn from_pid(pid: u16) -> Option<Self> {
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
}

/// Status response from GetInfo (0x12).
pub struct ReceiverStatus {
    pub mac: [u8; 6],
    pub master_mac: [u8; 6],
    pub channel: u8,
    pub rx_type: u8,
    pub dev_type: u8,
    pub fan_count: u8,
    pub is_inf_right_attach: bool,
    pub effect_index: [u8; 4],
    pub fans_type: [u8; 4],
    pub fan_rpm: [u16; 4],
    pub fan_pwm: [u8; 4],
    pub cmd_seq: u8,
    pub firmware: Option<String>,
}

pub struct WiredReceiverController {
    transport: Mutex<RusbBulk>,
    params: ReceiverParams,
    fan_count: Mutex<u8>,
    /// True when this receiver chains right-to-left (SL-INF daisy-chain with
    /// `fan_num >= 10`). Per-fan PWM/RGB ordering must be reversed on send.
    is_inf_right_attach: Mutex<bool>,
    #[allow(dead_code)]
    firmware: Mutex<Option<String>>,
    led_buffer: Mutex<Vec<[u8; 3]>>,
    /// Incrementing nonce for FanThemeColor (0x27) packets.
    color_nonce: Mutex<u8>,
    /// Frame-terminator counter for RGB streaming ack cadence (vendor: read
    /// ack every 14th frame terminator, not every frame).
    stream_ack_counter: Mutex<u8>,
}

impl WiredReceiverController {
    pub fn new(device: Device<GlobalContext>, pid: u16) -> Result<Self> {
        let params = ReceiverParams::from_pid(pid)
            .ok_or_else(|| anyhow::anyhow!("unknown wired receiver PID {pid:#06x}"))?;

        let mut transport =
            RusbBulk::open_device(device).context("opening wired receiver device")?;
        transport
            .detach_and_configure(params.name)
            .context("configuring wired receiver device")?;

        info!("{} opened", params.name);

        let ctrl = Self {
            transport: Mutex::new(transport),
            params,
            fan_count: Mutex::new(4),
            is_inf_right_attach: Mutex::new(false),
            firmware: Mutex::new(None),
            led_buffer: Mutex::new(Vec::new()),
            color_nonce: Mutex::new(1),
            stream_ack_counter: Mutex::new(0),
        };

        if let Ok(status) = ctrl.get_info() {
            *ctrl.fan_count.lock() = status.fan_count.max(1).min(4);
            *ctrl.is_inf_right_attach.lock() = status.is_inf_right_attach;
            info!(
                "{}: {} fans detected{}",
                params.name,
                status.fan_count,
                if status.is_inf_right_attach {
                    " (SL-INF right-attach)"
                } else {
                    ""
                }
            );
        }

        let total = ctrl.total_leds();
        *ctrl.led_buffer.lock() = vec![[0, 0, 0]; total];

        Ok(ctrl)
    }

    fn total_leds(&self) -> usize {
        *self.fan_count.lock() as usize * self.params.leds_per_fan as usize
    }

    pub fn params(&self) -> ReceiverParams {
        self.params
    }

    /// Send a command and read the response (TX+RX pattern).
    fn send_and_read(&self, tx: &[u8; PACKET_SIZE]) -> Result<[u8; PACKET_SIZE]> {
        let transport = self.transport.lock();
        transport
            .write(tx, LCD_WRITE_TIMEOUT)
            .context("wired receiver write")?;
        let mut rx = [0u8; PACKET_SIZE];
        transport
            .read(&mut rx, LCD_READ_TIMEOUT)
            .context("wired receiver read")?;
        Ok(rx)
    }

    /// GetInfo (0x12) — full status blob with MAC, fans, RPM, firmware.
    pub fn get_info(&self) -> Result<ReceiverStatus> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_GET_INFO;
        let rx = self.send_and_read(&tx)?;

        let mut mac = [0u8; 6];
        mac.copy_from_slice(&rx[1..7]);
        let mut master_mac = [0u8; 6];
        master_mac.copy_from_slice(&rx[7..13]);

        let raw_fan_count = rx[20];
        let (fan_count, is_inf_right_attach) = if raw_fan_count >= 10 {
            (raw_fan_count.saturating_sub(10).min(4), true)
        } else {
            (raw_fan_count.min(4), false)
        };

        let mut effect_index = [0u8; 4];
        effect_index.copy_from_slice(&rx[21..25]);

        let mut fans_type = [0u8; 4];
        fans_type.copy_from_slice(&rx[25..29]);

        let mut fan_rpm = [0u16; 4];
        for i in 0..4 {
            let off = 29 + i * 2;
            fan_rpm[i] = u16::from_be_bytes([rx[off] & 0x0F, rx[off + 1]]);
        }
        let fan_pwm = [rx[37], rx[38], rx[39], rx[40]];

        Ok(ReceiverStatus {
            mac,
            master_mac,
            channel: rx[13],
            rx_type: rx[14],
            dev_type: rx[19],
            fan_count,
            is_inf_right_attach,
            effect_index,
            fans_type,
            fan_rpm,
            fan_pwm,
            cmd_seq: rx[41],
            firmware: None,
        })
    }

    /// GetLedVer (0x10) — firmware version string.
    pub fn read_firmware(&self) -> Result<String> {
        let tx = [0u8; PACKET_SIZE];
        let mut tx = tx;
        tx[0] = CMD_GET_VER;
        let rx = self.send_and_read(&tx)?;
        let fw_bytes = &rx[3..19];
        let end = fw_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(fw_bytes.len());
        let fw = String::from_utf8_lossy(&fw_bytes[..end]).trim().to_string();
        Ok(fw)
    }

    /// SetFansPWM (0x13) with per-device PWM floor.
    pub fn set_fans_pwm(&self, duties: [u8; 4]) -> Result<()> {
        let fan_count = *self.fan_count.lock() as usize;
        let right_attach = *self.is_inf_right_attach.lock();
        let mut duties = duties;
        if right_attach {
            reverse_fan_slots(&mut duties, fan_count);
        }
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_SET_FANS_PWM;
        for i in 0..4 {
            let p = duties[i] as u16;
            // Per-device floor: P28 max(p,8)/zero→1; TL Flex max(p,11)/zero→5
            let mapped = if p == 0 {
                self.params.pwm_zero
            } else {
                ((p.max(self.params.pwm_floor as u16) as f64 / 100.0) * 255.0).round() as u8
            };
            // 0x06 → 0x00 rewrite (external-sync sentinel)
            tx[1 + i] = if mapped == 0x06 { 0 } else { mapped };
        }
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_SET_FANS_PWM || rx[1] != 0 {
            warn!("SetFansPWM unexpected response: [{}, {}]", rx[0], rx[1]);
        }
        debug!("{}: SetFansPWM {:?}", self.params.name, &tx[1..5]);
        Ok(())
    }

    /// Enable/disable motherboard PWM sync. Sentinels all four fan ports
    /// to value 6, which the firmware interprets as "follow MB PWM header."
    pub fn set_mb_sync(&self, enabled: bool) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_SET_FANS_PWM;
        if enabled {
            tx[1] = 6;
            tx[2] = 6;
            tx[3] = 6;
            tx[4] = 6;
        } else {
            return Ok(());
        }
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_SET_FANS_PWM || rx[1] != 0 {
            warn!("MB sync unexpected response: [{}, {}]", rx[0], rx[1]);
        }
        debug!("{}: MB PWM sync = {enabled}", self.params.name);
        Ok(())
    }

    /// SelectedGroup (0x16) — group selection / keepalive.
    pub fn selected_group(&self) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_SELECTED_GROUP;
        self.send_and_read(&tx)?;
        Ok(())
    }

    /// SetLightSyncMB (0x14) — enable/disable motherboard RGB sync.
    pub fn set_light_sync_mb(&self, enable: bool) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_SET_LIGHT_SYNC_MB;
        tx[1] = if enable { 1 } else { 0 };
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_SET_LIGHT_SYNC_MB {
            warn!("SetLightSyncMB unexpected response: 0x{:02x}", rx[0]);
        }
        Ok(())
    }

    /// SaveOrClearConfig (0x15) — save current RGB to NVRAM, or clear.
    /// `save=true` writes to NVRAM; `save=false` clears saved config.
    pub fn save_or_clear_config(&self, save: bool) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_SAVE_OR_CLEAR;
        tx[1] = if save { 1 } else { 0 };
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_SAVE_OR_CLEAR {
            warn!("SaveOrClearConfig unexpected response: 0x{:02x}", rx[0]);
        }
        Ok(())
    }

    /// RebootLcd (0x17) — reboot the wired LCD group.
    pub fn reboot_lcd(&self) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_REBOOT_LCD;
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_REBOOT_LCD && rx[0] != CMD_SAVE_OR_CLEAR {
            warn!("RebootLcd unexpected response: 0x{:02x}", rx[0]);
        }
        Ok(())
    }

    /// FanAndFixedData (0x26) — per-fan theme/data/brightness push.
    /// `fans_data` is up to 62 bytes of per-fan configuration.
    pub fn update_fans_theme_and_data(&self, fans_data: &[u8]) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_FAN_AND_FIXED_DATA;
        let len = fans_data.len().min(62);
        tx[2..2 + len].copy_from_slice(&fans_data[..len]);
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_FAN_THEME_COLOR && rx[0] != CMD_FAN_AND_FIXED_DATA {
            warn!("FanAndFixedData unexpected response: 0x{:02x}", rx[0]);
        }
        Ok(())
    }

    /// FanThemeColor (0x27) — per-fan color palette.
    /// `colors` is the RGB data; `fan_index` selects which fan (0-3).
    pub fn update_fans_color(&self, colors: &[[u8; 3]], fan_index: u8) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_FAN_THEME_COLOR;
        tx[1] = if fan_index > 1 { 1 } else { 0 };
        let start = 2 + (fan_index as usize % 2) * 19;
        let mut num = start;
        for c in colors.iter().take(6) {
            if num + 3 > PACKET_SIZE {
                break;
            }
            tx[num] = c[0];
            tx[num + 1] = c[1];
            tx[num + 2] = c[2];
            num += 3;
        }
        let mut nonce = self.color_nonce.lock();
        *nonce = nonce.wrapping_add(1).max(1);
        tx[num] = *nonce;
        tx[num + 1] = nonce.wrapping_add(1).max(1);
        drop(nonce);
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_FAN_THEME_COLOR {
            warn!("FanThemeColor unexpected response: 0x{:02x}", rx[0]);
        }
        Ok(())
    }

    /// WirelessThemeSwitch (0x29) — toggle embedded theme vs USB frames.
    /// Bit `i` (0..3) = fan `i` uses embedded/wireless theme.
    pub fn update_wireless_theme_switch(&self, mask: u8) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_WIRELESS_THEME_SWITCH;
        tx[1] = mask;
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_WIRELESS_THEME_SWITCH {
            warn!("WirelessThemeSwitch unexpected response: 0x{:02x}", rx[0]);
        }
        Ok(())
    }

    /// Send a full RGB frame (all fans' LEDs) using the per-PID transport.
    pub fn send_rgb_frame(&self, colors: &[[u8; 3]]) -> Result<()> {
        self.send_rgb_frame_ext(colors, 0, 1, 0, 20, false)
    }

    /// Send a full RGB frame with explicit flash-save parameters (TL Flex).
    pub fn send_rgb_frame_ext(
        &self,
        colors: &[[u8; 3]],
        effect_index: u32,
        total_frame: u16,
        total_sub_frame: u16,
        interval_ms: u16,
        is_outer_match_max: bool,
    ) -> Result<()> {
        let right_attach = *self.is_inf_right_attach.lock();
        let ordered = if right_attach {
            reverse_per_fan_chunks(colors, self.params.leds_per_fan as usize)
        } else {
            colors.to_vec()
        };
        let mut raw = Vec::with_capacity(ordered.len() * 3);
        for c in &ordered {
            raw.extend_from_slice(c);
        }
        if self.params.compresses_rgb {
            self.send_rgb_flash_save(
                &raw,
                ordered.len(),
                effect_index,
                total_frame,
                total_sub_frame,
                interval_ms,
                is_outer_match_max,
            )?;
        } else {
            self.send_rgb_stream(&raw)?;
        }
        Ok(())
    }

    /// P28 V2 / CL V2: stream raw RGB via 0x11 in 20-LED chunks.
    /// Ack is read every 14th frame-terminating chunk to avoid USB contention.
    fn send_rgb_stream(&self, raw: &[u8]) -> Result<()> {
        let total_leds = raw.len() / 3;
        let transport = self.transport.lock();
        let mut ack_counter = self.stream_ack_counter.lock();
        let mut offset = 0usize;
        while offset < total_leds {
            let count = (total_leds - offset).min(RGB_LED_CHUNK);
            let is_last = offset + count >= total_leds;

            let mut pkt = [0u8; PACKET_SIZE];
            pkt[0] = CMD_STREAM_RGB;
            pkt[1] = offset as u8;
            pkt[2] = count as u8;
            pkt[3] = if is_last { 2 } else { 0 };

            let boff = offset * 3;
            let blen = count * 3;
            pkt[4..4 + blen].copy_from_slice(&raw[boff..boff + blen]);

            transport.write(&pkt, LCD_WRITE_TIMEOUT)?;

            if is_last {
                *ack_counter += 1;
                if *ack_counter >= RGB_ACK_INTERVAL {
                    drop(ack_counter);
                    let mut rx = [0u8; PACKET_SIZE];
                    let _ = transport.read(&mut rx, LCD_READ_TIMEOUT);
                    ack_counter = self.stream_ack_counter.lock();
                    *ack_counter = 0;
                }
            }
            offset += count;
        }
        debug!("{}: streamed {total_leds} LEDs via 0x11", self.params.name);
        Ok(())
    }

    /// TL Flex: tinyuz-compress, upload via 0x18, commit via 0x19.
    fn send_rgb_flash_save(
        &self,
        raw: &[u8],
        led_total: usize,
        effect_index: u32,
        total_frame: u16,
        total_sub_frame: u16,
        interval_ms: u16,
        is_outer_match_max: bool,
    ) -> Result<()> {
        let compressed = crate::tinyuz::compress(raw).context("compressing RGB data")?;

        let mut hdr = [0u8; PACKET_SIZE];
        hdr[0] = CMD_SEND_LIGHT_PACKAGE;
        hdr[1] = 0x00;
        hdr[16..20].copy_from_slice(&effect_index.to_be_bytes());
        hdr[22..26].copy_from_slice(&(compressed.len() as u32).to_be_bytes());
        hdr[26] = 0;
        hdr[27..29].copy_from_slice(&total_frame.to_be_bytes());
        hdr[29] = led_total as u8;
        hdr[34..36].copy_from_slice(&interval_ms.to_be_bytes());
        hdr[36] = ((interval_ms as u32) * 100 % 100) as u8;
        hdr[39] = if is_outer_match_max { 1 } else { 0 };
        hdr[40..42].copy_from_slice(&total_sub_frame.to_be_bytes());

        let transport = self.transport.lock();
        transport.write(&hdr, LCD_WRITE_TIMEOUT)?;
        let mut rx = [0u8; PACKET_SIZE];
        let _ = transport.read(&mut rx, LCD_READ_TIMEOUT);

        let mut offset = 0usize;
        let mut idx = 1u8;
        while offset < compressed.len() {
            let mut pkt = [0u8; PACKET_SIZE];
            pkt[0] = CMD_SEND_LIGHT_PACKAGE;
            pkt[1] = idx;
            let chunk = (compressed.len() - offset).min(60);
            pkt[4..4 + chunk].copy_from_slice(&compressed[offset..offset + chunk]);
            transport.write(&pkt, LCD_WRITE_TIMEOUT)?;
            let mut rx = [0u8; PACKET_SIZE];
            let _ = transport.read(&mut rx, LCD_READ_TIMEOUT);
            offset += 60;
            idx += 1;
        }

        let mut apply = [0u8; PACKET_SIZE];
        apply[0] = CMD_APPLY_LIGHTING;
        transport.write(&apply, LCD_WRITE_TIMEOUT)?;
        let mut rx = [0u8; PACKET_SIZE];
        let _ = transport.read(&mut rx, LCD_READ_TIMEOUT);

        debug!(
            "{}: flash-saved {led_total} LEDs ({} compressed bytes, idx={:#010x}, frames={total_frame}, interval={interval_ms}ms) via 0x18+0x19",
            self.params.name,
            compressed.len(),
            effect_index,
        );
        Ok(())
    }
}

impl FanDevice for WiredReceiverController {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        let mut duties = [0u8; 4];
        duties[slot as usize % 4] = duty;
        self.set_fans_pwm(duties)
    }

    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        let mut pwm = [0u8; 4];
        for (i, &d) in duties.iter().enumerate().take(4) {
            pwm[i] = lianli_shared::fan::duty_to_percent(d);
        }
        self.set_fans_pwm(pwm)
    }

    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        let status = self.get_info()?;
        Ok(status.fan_rpm.to_vec())
    }

    fn fan_slot_count(&self) -> u8 {
        *self.fan_count.lock()
    }

    fn stop_pwm(&self) -> u8 {
        // zero duty maps to pwm_zero sentinel, not actual 0
        self.params.pwm_zero
    }

    fn supports_mb_sync(&self) -> bool {
        true
    }

    fn set_mb_rpm_sync(&self, _port: u8, sync: bool) -> Result<()> {
        self.set_mb_sync(sync)
    }
}

impl RgbDevice for WiredReceiverController {
    fn device_name(&self) -> String {
        self.params.name.to_string()
    }

    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Off, RgbMode::Static, RgbMode::Direct]
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        let count = *self.fan_count.lock() as u16;
        (0..count)
            .map(|fan| RgbZoneInfo {
                name: format!("Fan {}", fan + 1),
                led_count: self.params.leds_per_fan,
            })
            .collect()
    }

    fn supports_direct(&self) -> bool {
        true
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        if self.params.compresses_rgb {
            vec![vec![RgbScope::All, RgbScope::Top, RgbScope::Bottom]]
        } else {
            let count = *self.fan_count.lock() as usize;
            vec![vec![]; count]
        }
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        let color = if effect.mode == RgbMode::Off || effect.disabled {
            [0, 0, 0]
        } else {
            let base = effect.colors.first().copied().unwrap_or([255, 255, 255]);
            scale_brightness(base, effect.brightness)
        };

        let leds_per_fan = self.params.leds_per_fan as usize;
        let mut buf = self.led_buffer.lock();
        let start = (zone as usize).min(buf.len() / leds_per_fan.max(1)) * leds_per_fan;
        let end = (start + leds_per_fan).min(buf.len());
        for led in &mut buf[start..end] {
            *led = color;
        }
        let frame = buf.clone();
        drop(buf);
        self.send_rgb_frame(&frame)
    }

    fn set_direct_colors(&self, zone: u8, colors: &[[u8; 3]]) -> Result<()> {
        let leds_per_fan = self.params.leds_per_fan as usize;
        let mut buf = self.led_buffer.lock();
        let start = (zone as usize).min(buf.len() / leds_per_fan.max(1)) * leds_per_fan;
        for (i, &c) in colors.iter().enumerate().take(leds_per_fan) {
            if start + i < buf.len() {
                buf[start + i] = c;
            }
        }
        let frame = buf.clone();
        drop(buf);
        self.send_rgb_frame(&frame)
    }

    fn ping(&self, _zone: u8) -> Result<()> {
        self.selected_group()
    }
}

fn scale_brightness([r, g, b]: [u8; 3], brightness: u8) -> [u8; 3] {
    let scale = (lianli_shared::rgb::brightness_scale(brightness) as f32) / 4.0;
    [
        (r as f32 * scale).round() as u8,
        (g as f32 * scale).round() as u8,
        (b as f32 * scale).round() as u8,
    ]
}

/// Reverse per-fan slot ordering for SL-INF right-attach daisy-chains.
fn reverse_fan_slots<T: Copy>(slots: &mut [T; 4], fan_count: usize) {
    let n = fan_count.min(4);
    if n > 1 {
        slots[..n].reverse();
    }
}

/// Reverse the per-fan chunk order in a flat LED color buffer for SL-INF
/// right-attach daisy-chains (chain wires right-to-left).
fn reverse_per_fan_chunks(colors: &[[u8; 3]], leds_per_fan: usize) -> Vec<[u8; 3]> {
    if leds_per_fan == 0 {
        return colors.to_vec();
    }
    let total = colors.len();
    let fan_count = total.div_ceil(leds_per_fan);
    if fan_count <= 1 {
        return colors.to_vec();
    }
    let mut out = Vec::with_capacity(total);
    for fan_idx in (0..fan_count).rev() {
        let start = fan_idx * leds_per_fan;
        let end = (start + leds_per_fan).min(total);
        if start < end {
            out.extend_from_slice(&colors[start..end]);
        }
    }
    out
}

pub struct WiredReceiverDriver;

impl crate::registry::DeviceDriver for WiredReceiverDriver {
    fn family(&self) -> lianli_shared::device_id::DeviceFamily {
        lianli_shared::device_id::DeviceFamily::WiredReceiver
    }

    fn open(
        &self,
        ctx: &crate::registry::OpenContext,
    ) -> anyhow::Result<crate::registry::OpenedDevice> {
        let ctrl = std::sync::Arc::new(WiredReceiverController::new(
            rusb::Device::clone(&ctx.device),
            ctx.pid,
        )?);
        let name = ctrl.params().name.to_string();
        let firmware = ctrl.read_firmware().ok();

        Ok(crate::registry::OpenedDevice {
            id: ctx.device_id(),
            family: lianli_shared::device_id::DeviceFamily::WiredReceiver,
            capabilities: lianli_shared::device_id::DeviceFamily::WiredReceiver.capabilities(),
            transport_kind: lianli_shared::device_id::TransportKind::UsbBulk,
            model_name: name,
            firmware,
            fan: Some(Box::new(std::sync::Arc::clone(&ctrl))),
            lcd: None,
            rgb: vec![(
                String::new(),
                Arc::new(ctrl) as Arc<dyn crate::traits::RgbDevice>,
            )],
            aio: None,
            shared_hid: None,
        })
    }
}

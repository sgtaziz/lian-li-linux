//! Wired receiver driver for TL Flex / SL-INF Flex / P28 V2 / SLV4 / CLV2
//! controllers (VID `0x43A8`, PIDs `0x0101`–`0x0107`).
//!
//! All share the same WinUsbLed base protocol: 64-byte unencrypted packets,
//! TX+RX pattern. Per-PID differences: LED count, PWM floor, and whether
//! RGB is sent raw (0x11) or flash-saved (0x18 + 0x19).

mod control;
mod profile;
mod rgb;
mod rgb_packets;
mod sync;
pub use profile::ReceiverParams;
use rgb_packets::{checked_frame_count, read_rgb_ack, rgb_flash_header, rgb_timeout};
#[cfg(test)]
mod tests;

use crate::traits::{FanDevice, RgbDevice};
use anyhow::{Context, Result};
use lianli_shared::rgb::{
    RgbEffect, RgbMode, RgbPlaybackTiming, RgbRenderFamily, RgbRenderProfile, RgbScope, RgbZoneInfo,
};
use lianli_transport::usb::{RusbBulk, LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use lianli_transport::UsbContext;
use parking_lot::Mutex;
use rusb::Device;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
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
const RGB_PACKAGE_DEADLINE: Duration = Duration::from_secs(3);

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
    pid: u16,
    params: ReceiverParams,
    render_family: RgbRenderFamily,
    fan_count: Mutex<u8>,
    lcd_count: Option<u8>,
    fan_pwm: Mutex<Option<[u8; 4]>>,
    /// True when this receiver chains right-to-left (SL-INF daisy-chain with
    /// `fan_num >= 10`). Per-fan PWM/RGB ordering must be reversed on send.
    is_inf_right_attach: Mutex<bool>,
    #[allow(dead_code)]
    firmware: Mutex<Option<String>>,
    led_buffer: Mutex<Vec<[u8; 3]>>,
    /// Incrementing nonce for FanThemeColor (0x27) packets.
    color_nonce: Mutex<u8>,
    /// Receiver cache key for the currently uploaded lighting package.
    effect_nonce: AtomicU32,
    /// Frame-terminator counter for RGB streaming ack cadence (vendor: read
    /// ack every 14th frame terminator, not every frame).
    stream_ack_counter: Mutex<u8>,
    /// Group MAC from GetInfo, equals the wireless record MAC of the same fans.
    mac: Mutex<Option<[u8; 6]>>,
    /// Set while the fan group is bound over RF, suppresses wired writes and telemetry.
    is_wireless: AtomicBool,
}

impl WiredReceiverController {
    pub fn new(device: Device<UsbContext>, pid: u16) -> Result<Self> {
        let params = ReceiverParams::from_pid(pid)
            .ok_or_else(|| anyhow::anyhow!("unknown wired receiver PID {pid:#06x}"))?;
        let render_family = ReceiverParams::render_family(pid)
            .ok_or_else(|| anyhow::anyhow!("wired receiver PID {pid:#06x} has no renderer"))?;

        let mut transport =
            RusbBulk::open_device(device).context("opening wired receiver device")?;
        transport
            .detach_and_configure(params.name)
            .context("configuring wired receiver device")?;

        info!("{} opened", params.name);

        let mut ctrl = Self {
            transport: Mutex::new(transport),
            pid,
            params,
            render_family,
            fan_count: Mutex::new(4),
            lcd_count: None,
            fan_pwm: Mutex::new(None),
            is_inf_right_attach: Mutex::new(false),
            firmware: Mutex::new(None),
            led_buffer: Mutex::new(Vec::new()),
            color_nonce: Mutex::new(1),
            effect_nonce: AtomicU32::new(1),
            stream_ack_counter: Mutex::new(0),
            mac: Mutex::new(None),
            is_wireless: AtomicBool::new(false),
        };

        if let Ok(status) = ctrl.get_info() {
            if status.dev_type == 0 {
                ctrl.lcd_count = flex_lcd_count(pid, status.fan_count, &status.fans_type);
            }
            *ctrl.fan_count.lock() = status.fan_count.clamp(1, 4);
            *ctrl.is_inf_right_attach.lock() = status.is_inf_right_attach;
            if !status.mac.iter().all(|&b| b == 0) {
                *ctrl.mac.lock() = Some(status.mac);
            }
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
    fn send_and_read(&self, tx: &[u8; PACKET_SIZE]) -> Result<Vec<u8>> {
        let transport = self.transport.lock();
        transport
            .write_full(tx, LCD_WRITE_TIMEOUT)
            .context("wired receiver write")?;
        let mut rx = [0u8; PACKET_SIZE];
        let len = transport
            .read(&mut rx, LCD_READ_TIMEOUT)
            .context("wired receiver read")?;
        let response = &rx[..len];
        validate_response_length(response, tx[0])?;
        Ok(response.to_vec())
    }

    /// GetInfo (0x12) — full status blob with MAC, fans, RPM, firmware.
    pub fn get_info(&self) -> Result<ReceiverStatus> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_GET_INFO;
        let rx = self.send_and_read(&tx)?;

        anyhow::ensure!(rx[0] == CMD_GET_INFO, "unexpected receiver status opcode");

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
        for (i, rpm) in fan_rpm.iter_mut().enumerate() {
            let off = 29 + i * 2;
            *rpm = u16::from_be_bytes([rx[off] & 0x0F, rx[off + 1]]);
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
        anyhow::ensure!(rx[0] == CMD_GET_VER, "unexpected receiver firmware opcode");
        Ok(firmware_text(&rx))
    }
}

/// Only date-stamped firmware reports text. Older replies carry a binary
/// version in bytes 1 and 2, followed by unrelated data.
fn firmware_text(reply: &[u8]) -> String {
    let text = super::nul_terminated_text(&reply[3..19]);
    if text.starts_with("202") {
        text
    } else {
        u16::from_be_bytes([reply[1], reply[2]]).to_string()
    }
}

fn flex_lcd_count(pid: u16, fan_count: u8, fan_types: &[u8; 4]) -> Option<u8> {
    let (known, lcd_types) = match pid {
        0x0102 => (51..=58, [51, 52, 55, 56]),
        0x0104 => (43..=50, [43, 44, 47, 48]),
        _ => return None,
    };
    let types = fan_types.get(..usize::from(fan_count))?;
    if types.iter().any(|kind| !known.contains(kind)) {
        return None;
    }
    Some(types.iter().filter(|kind| lcd_types.contains(kind)).count() as u8)
}

fn validate_response_length(response: &[u8], command: u8) -> Result<()> {
    let required = match command {
        CMD_GET_INFO => 42,
        CMD_GET_VER => 19,
        CMD_SET_FANS_PWM => 2,
        _ => 1,
    };
    anyhow::ensure!(
        response.len() >= required,
        "receiver command {command:#04x}: short response ({} bytes, need {required})",
        response.len()
    );
    Ok(())
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
            sensors: None,
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
            shared_usb: None,
        })
    }
}

//! Generic WinUSB LED driver for vendor-class bulk USB LED rings.
//!
//! Per-LED RGB stream sent as fixed-size 64-byte packets over EP_OUT:
//!   - byte[0] = 0x11 (set LED chunk)
//!   - byte[1] = LED offset (0, 20, 40, …)
//!   - byte[4..63] = 60 bytes RGB (20 LEDs × 3 bytes)
//!
//! `LEDS_PER_CHUNK` is fixed at 20; total LED count is rounded up to the
//! nearest multiple.

use crate::traits::RgbDevice;
use anyhow::{bail, Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbZoneInfo};
use lianli_transport::usb::{RusbBulk, USB_TIMEOUT};
use lianli_transport::TransportError;
use parking_lot::Mutex;
use rusb::{Device, GlobalContext};
use std::sync::Arc;
use tracing::{info, warn};

const LEDS_PER_CHUNK: usize = 20;
const PACKET_SIZE: usize = 64;
const CMD_SET_LEDS: u8 = 0x11;

pub struct WinUsbLedDevice {
    transport: Arc<Mutex<RusbBulk>>,
    name: String,
    led_count: u16,
    vid: u16,
    pid: u16,
}

impl WinUsbLedDevice {
    pub fn new(device: Device<GlobalContext>, led_count: u16, name: &str) -> Result<Self> {
        let desc = device
            .device_descriptor()
            .context("reading device descriptor")?;
        let vid = desc.vendor_id();
        let pid = desc.product_id();
        let mut transport = RusbBulk::open_device(device).map_err(transport_to_anyhow)?;
        transport
            .detach_and_configure(name)
            .map_err(transport_to_anyhow)?;
        info!(
            "{name} opened: {} LEDs ({:04x}:{:04x})",
            led_count, vid, pid
        );
        Ok(Self {
            transport: Arc::new(Mutex::new(transport)),
            name: name.to_string(),
            led_count,
            vid,
            pid,
        })
    }

    /// Device display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn reopen(&self) -> Result<()> {
        let mut new_transport = RusbBulk::open(self.vid, self.pid)
            .map_err(transport_to_anyhow)
            .with_context(|| format!("reopening {}", self.name))?;
        new_transport
            .detach_and_configure(&self.name)
            .map_err(transport_to_anyhow)?;
        *self.transport.lock() = new_transport;
        Ok(())
    }

    fn write_with_recovery<F>(&self, label: &str, mut op: F) -> Result<()>
    where
        F: FnMut(&RusbBulk) -> Result<()>,
    {
        let first = {
            let h = self.transport.lock();
            op(&h)
        };
        match first {
            Ok(()) => Ok(()),
            Err(e) => {
                warn!("{label} failed ({e}); attempting reopen");
                self.reopen()?;
                info!("{label} transport reopened, retrying");
                let h = self.transport.lock();
                op(&h)
            }
        }
    }

    fn send_frame(&self, colors: &[[u8; 3]]) -> Result<()> {
        let total = self.led_count as usize;
        let chunks = total.div_ceil(LEDS_PER_CHUNK);
        let mut padded: Vec<[u8; 3]> = colors.iter().copied().take(total).collect();
        padded.resize(total, [0, 0, 0]);

        self.write_with_recovery("LED frame", |handle| {
            for chunk in 0..chunks {
                let mut packet = [0u8; PACKET_SIZE];
                packet[0] = CMD_SET_LEDS;
                packet[1] = (chunk * LEDS_PER_CHUNK) as u8;
                let start = chunk * LEDS_PER_CHUNK;
                let end = (start + LEDS_PER_CHUNK).min(total);
                for (i, c) in padded[start..end].iter().enumerate() {
                    let off = 4 + i * 3;
                    packet[off] = c[0];
                    packet[off + 1] = c[1];
                    packet[off + 2] = c[2];
                }
                handle
                    .write(&packet, USB_TIMEOUT)
                    .map_err(transport_to_anyhow)
                    .context("WinUSB LED: write chunk")?;
            }
            Ok(())
        })
    }
}

fn transport_to_anyhow(e: TransportError) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

impl RgbDevice for WinUsbLedDevice {
    fn device_name(&self) -> String {
        self.name.clone()
    }

    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Off, RgbMode::Static, RgbMode::Direct]
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![RgbZoneInfo {
            name: "Ring".to_string(),
            led_count: self.led_count,
        }]
    }

    fn supports_direct(&self) -> bool {
        true
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        if zone != 0 {
            bail!("{}: zone {zone} out of range (only zone 0)", self.name);
        }
        let color = match effect.mode {
            RgbMode::Off => [0, 0, 0],
            _ => effect.colors.first().copied().unwrap_or([255, 255, 255]),
        };
        let frame = vec![color; self.led_count as usize];
        self.send_frame(&frame)
    }

    fn set_direct_colors(&self, zone: u8, colors: &[[u8; 3]]) -> Result<()> {
        if zone != 0 {
            bail!("{}: zone {zone} out of range (only zone 0)", self.name);
        }
        self.send_frame(colors)
    }
}

/// Driver entry point for the WinUSB LED ring family (Universal Screen 8.8\"
/// LED ring at VID=0x0416 PID=0x8050).
pub struct WinUsbLedDriver;

impl crate::registry::DeviceDriver for WinUsbLedDriver {
    fn family(&self) -> lianli_shared::device_id::DeviceFamily {
        lianli_shared::device_id::DeviceFamily::UniversalScreenLighting
    }

    fn open(
        &self,
        ctx: &crate::registry::OpenContext,
    ) -> anyhow::Result<crate::registry::OpenedDevice> {
        use lianli_shared::device_id::{DeviceCapabilities, DeviceFamily, TransportKind};
        let dev = WinUsbLedDevice::new(ctx.device.clone(), 60, "Universal Screen 8.8\" LED Ring")?;
        Ok(crate::registry::OpenedDevice {
            id: ctx.device_id(),
            family: DeviceFamily::UniversalScreenLighting,
            capabilities: DeviceCapabilities::RGB,
            transport_kind: TransportKind::UsbBulk,
            model_name: dev.name().to_string(),
            firmware: None,
            fan: None,
            lcd: None,
            rgb: vec![(
                String::new(),
                Arc::new(dev) as Arc<dyn crate::traits::RgbDevice>,
            )],
            aio: None,
            shared_hid: None,
        })
    }
}

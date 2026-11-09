//! WinUSB LCD driver for VID=0x1CBE direct-connect LCD panels.
//!
//! The wire protocol (DES-CBC encrypted 512-byte header + JPEG/H264 payload)
//! is shared, but device families differ in timeouts, the StartPlay header,
//! and the init sequence. Each variant lives in its own module and shares the
//! common [`core::WinUsbLcdCore`] transport/crypto layer:
//!   - [`base`]      — Lancool 207 (lcd207 `WinUsb`)
//!   - [`h2_lcd`]    — HydroShift II LCD Circle/Square (lcd207 `WinUsbH2`)
//!   - [`hs2_oled`]  — HydroShift II OLED Curve (lcd207 `WinUsbHS2`)
//!   - [`slv3`]      — Universal Screen / Vision / Flex streaming parameters

use crate::traits::LcdDevice;
use anyhow::{bail, Result};
use lianli_shared::device_id::DeviceFamily;
use lianli_shared::screen::ScreenInfo;
use lianli_transport::UsbContext;
use rusb::Device;
use std::sync::Arc;

mod base;
mod core;
mod h2_lcd;
mod hs2_oled;
mod playback;
mod ring;
mod slv3;
mod trait_;

pub use self::core::{LcdLink, PendingCmd, SharedTransport};
pub use self::playback::LcdTransferInterrupted;
use self::trait_::BoxedWinUsbLcd;

pub struct WinUsbLcdDevice(BoxedWinUsbLcd, u16);

impl WinUsbLcdDevice {
    pub fn open(device: Device<UsbContext>, pid: u16) -> Result<Self> {
        Ok(Self(make_device(device, pid)?, pid))
    }

    pub fn from_shared_transport(transport: SharedTransport, pid: u16) -> Result<Self> {
        let (screen, _family, name) = screen_for_pid(pid)
            .ok_or_else(|| anyhow::anyhow!("unknown WinUSB LCD PID {:#06x}", pid))?;
        let boxed: BoxedWinUsbLcd = match pid {
            0xA068 => Box::new(hs2_oled::Hs2OledWinUsbLcd::from_shared(
                transport,
                screen,
                name.to_string(),
            )),
            0xA021 | 0xA034 => Box::new(h2_lcd::H2WinUsbLcd::from_shared(
                transport,
                screen,
                name.to_string(),
            )),
            0xA065 => Box::new(base::BaseWinUsbLcd::from_shared(
                transport,
                screen,
                name.to_string(),
            )),
            _ => Box::new(slv3::Slv3WinUsbLcd::from_shared(
                transport,
                screen,
                name.to_string(),
                supports_stop_play(pid),
                is_flex_panel(pid),
            )),
        };
        Ok(Self(boxed, pid))
    }

    pub fn shared_transport(&self) -> SharedTransport {
        self.0.shared_transport()
    }

    pub fn upload_startup_image(
        &mut self,
        jpeg: &[u8],
        stop: &std::sync::atomic::AtomicBool,
        transfer: &crate::startup_image::Transfer,
    ) -> Result<bool> {
        anyhow::ensure!(
            matches!(self.1, 0xa018 | 0xa019),
            "Startup image upload is not supported for this panel"
        );
        crate::startup_image::ensure_not_cancelled(stop)?;
        let link = self.shared_transport();
        self.stop_playback()?;
        let command = self.0.packet_builder().lcd_revision_header();
        let relative_path = link.probe_relative_startup_path(&command)?;
        let packet =
            crate::startup_image::packet(self.0.packet_builder(), jpeg, relative_path, true, true)?;
        link.upload_startup_image(&packet, stop, transfer)
    }

    pub fn firmware_str(&self) -> Option<&str> {
        self.0.firmware_str()
    }

    pub fn shares_cooling_transport(&self) -> bool {
        matches!(self.1, 0xa021 | 0xa034)
    }

    pub fn stop_playback(&mut self) -> Result<()> {
        self.shared_transport().ensure_storage_ready()?;
        self.0.stop_playback()
    }

    pub fn pause_for_wireless_image(&mut self) -> Result<()> {
        anyhow::ensure!(
            matches!(self.1, 0xa021 | 0xa034),
            "Wireless image preparation requires an H2 Circle or Square LCD"
        );
        self.stop_playback()?;
        self.shared_transport().set_needs_init(true);
        Ok(())
    }

    pub fn send_frame(&mut self, frame: &[u8]) -> Result<()> {
        self.shared_transport().ensure_storage_ready()?;
        self.0.send_frame(frame)
    }

    pub fn send_frame_verified(&mut self, frame: &[u8]) -> Result<()> {
        self.shared_transport().ensure_storage_ready()?;
        self.0.send_frame_verified(frame)
    }

    pub fn set_brightness_val(&mut self, brightness: u8) -> Result<()> {
        self.shared_transport().ensure_storage_ready()?;
        self.0.set_brightness_val(brightness)
    }

    pub fn switch_to_desktop_mode(&mut self) -> Result<()> {
        self.shared_transport().ensure_storage_ready()?;
        self.0.switch_to_desktop_mode()
    }

    pub fn stream_h264(
        &mut self,
        path: &std::path::Path,
        looping: bool,
        stop: &std::sync::atomic::AtomicBool,
        fps: f32,
    ) -> Result<()> {
        self.shared_transport().ensure_storage_ready()?;
        self.0.stream_h264(path, looping, stop, fps)
    }

    pub fn observe_h264_transfer(
        &mut self,
        transferred: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        self.0.observe_h264_transfer(transferred);
    }

    pub fn stream_h264_reader(
        &mut self,
        reader: &mut dyn std::io::Read,
        stop: &std::sync::atomic::AtomicBool,
        fps: f32,
    ) -> Result<()> {
        self.shared_transport().ensure_storage_ready()?;
        self.0.stream_h264_reader(reader, stop, fps)
    }
}

impl LcdDevice for WinUsbLcdDevice {
    fn upload_startup_image(
        &mut self,
        jpeg: &[u8],
        stop: &std::sync::atomic::AtomicBool,
        transfer: &crate::startup_image::Transfer,
    ) -> Result<bool> {
        WinUsbLcdDevice::upload_startup_image(self, jpeg, stop, transfer)
    }
    fn screen_info(&self) -> &ScreenInfo {
        self.0.screen_info()
    }

    fn send_jpeg_frame(&mut self, jpeg_data: &[u8]) -> Result<()> {
        self.send_frame(jpeg_data)
    }

    fn set_brightness(&self, _brightness: u8) -> Result<()> {
        Ok(())
    }

    fn set_rotation(&self, _degrees: u16) -> Result<()> {
        Ok(())
    }

    fn initialize(&mut self) -> Result<()> {
        self.shared_transport().ensure_storage_ready()?;
        self.0.initialize()
    }
}

fn make_device(device: Device<UsbContext>, pid: u16) -> Result<BoxedWinUsbLcd> {
    let (screen, _family, name) = screen_for_pid(pid)
        .ok_or_else(|| anyhow::anyhow!("unknown WinUSB LCD PID {:#06x}", pid))?;
    match pid {
        0xA068 => hs2_oled::boxed(device, screen, name),
        0xA021 | 0xA034 => h2_lcd::boxed(device, screen, name),
        0xA065 => base::boxed(device, screen, name),
        0xA088 | 0xA092 | 0xA018 | 0xA019 => slv3::boxed(
            device,
            screen,
            name,
            supports_stop_play(pid),
            is_flex_panel(pid),
        ),
        _ => bail!("no WinUSB LCD variant for PID {:#06x}", pid),
    }
}

fn is_flex_panel(pid: u16) -> bool {
    matches!(pid, 0xA018 | 0xA019)
}

fn supports_stop_play(pid: u16) -> bool {
    // Vendor lcd207 controllers use StopPlay; slv3 Flex only cancels its producer.
    matches!(pid, 0xA021 | 0xA034 | 0xA065 | 0xA068 | 0xA088 | 0xA092)
}

/// Driver entry point for the WinUSB LCD family. Dispatches to the correct
/// protocol variant by PID via [`make_device`].
pub struct WinUsbLcdDriver;

impl crate::registry::DeviceDriver for WinUsbLcdDriver {
    fn family(&self) -> DeviceFamily {
        DeviceFamily::HydroShift2Lcd
    }

    fn open(
        &self,
        ctx: &crate::registry::OpenContext,
    ) -> anyhow::Result<crate::registry::OpenedDevice> {
        let (_screen, family, name) = screen_for_pid(ctx.pid)
            .ok_or_else(|| anyhow::anyhow!("unknown WinUSB LCD PID {:#06x}", ctx.pid))?;
        let mut lcd = WinUsbLcdDevice::open(ctx.device.clone(), ctx.pid)?;
        LcdDevice::initialize(&mut lcd)?;
        let firmware = lcd.firmware_str().map(|s| s.to_string());

        let (fan, aio, rgb) = if matches!(ctx.pid, 0xA021 | 0xA034) {
            let shared = lcd.shared_transport();
            let ctrl = std::sync::Arc::new(super::h2_aio::H2AioController::new(shared, ctx.pid));
            (
                Some(Box::new(std::sync::Arc::clone(&ctrl)) as Box<dyn crate::traits::FanDevice>),
                Some(Box::new(std::sync::Arc::clone(&ctrl)) as Box<dyn crate::traits::AioDevice>),
                vec![(String::new(), ctrl as Arc<dyn crate::traits::RgbDevice>)],
            )
        } else {
            (None, None, Vec::new())
        };

        let shared_usb = if matches!(ctx.pid, 0xA021 | 0xA034) {
            Some(lcd.shared_transport())
        } else {
            None
        };

        Ok(crate::registry::OpenedDevice {
            sensors: None,
            id: ctx.device_id(),
            family,
            capabilities: family.capabilities(),
            transport_kind: lianli_shared::device_id::TransportKind::UsbBulk,
            model_name: name.to_string(),
            firmware,
            fan,
            lcd: Some(Box::new(lcd)),
            rgb,
            aio,
            shared_hid: None,
            shared_usb,
        })
    }
}

/// Map a WinUSB LCD PID to its `(ScreenInfo, DeviceFamily, display name)`.
pub(crate) fn screen_for_pid(pid: u16) -> Option<(ScreenInfo, DeviceFamily, &'static str)> {
    use lianli_shared::screen::ScreenInfo;
    match pid {
        0xA021 => Some((
            ScreenInfo::HYDROSHIFT2,
            DeviceFamily::HydroShift2Lcd,
            "HydroShift II LCD Circle",
        )),
        0xA034 => Some((
            ScreenInfo::HYDROSHIFT2,
            DeviceFamily::HydroShift2Lcd,
            "HydroShift II LCD Square",
        )),
        0xA065 => Some((
            ScreenInfo::LANCOOL_207,
            DeviceFamily::Lancool207,
            "Lancool 207 Digital",
        )),
        0xA088 => Some((
            ScreenInfo::UNIVERSAL_SCREEN,
            DeviceFamily::UniversalScreen,
            "Universal Screen 8.8\"",
        )),
        0xA092 => Some((
            ScreenInfo::VISION_9P2,
            DeviceFamily::Vision9p2,
            "Vision 9.2\"",
        )),
        0xA018 => Some((ScreenInfo::FLEX_LCD, DeviceFamily::TlFlexLcd, "TL Flex LCD")),
        0xA019 => Some((
            ScreenInfo::FLEX_LCD,
            DeviceFamily::SlInfFlexLcd,
            "SL Infinity Flex LCD",
        )),
        0xA068 => Some((
            ScreenInfo::HYDROSHIFT2_OLED_CURVE,
            DeviceFamily::HydroShift2OledCurveLcd,
            "HydroShift II OLED Curve",
        )),
        _ => None,
    }
}

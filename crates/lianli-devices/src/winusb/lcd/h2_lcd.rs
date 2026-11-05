use anyhow::Result;
use lianli_shared::screen::ScreenInfo;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::core::{SharedTransport, WinUsbLcdCore};
use super::trait_::{BoxedWinUsbLcd, WinUsbLcd};

const BLANK_CLIP: &[u8] = include_bytes!("../../../assets/blank_480.h264");

pub struct H2WinUsbLcd {
    core: WinUsbLcdCore,
    play_started: bool,
}

impl H2WinUsbLcd {
    pub(crate) fn new(
        device: rusb::Device<lianli_transport::UsbContext>,
        screen: ScreenInfo,
        name: &str,
    ) -> Result<Self> {
        let core = WinUsbLcdCore::open(device, screen, name, WRITE_TIMEOUT, READ_TIMEOUT, true)?
            .without_stalled_chunk_resend();
        Ok(Self {
            core,
            play_started: false,
        })
    }

    pub(crate) fn from_shared(
        transport: SharedTransport,
        screen: ScreenInfo,
        name: String,
    ) -> Self {
        Self {
            play_started: false,
            core: WinUsbLcdCore::from_shared(
                transport,
                screen,
                name,
                WRITE_TIMEOUT,
                READ_TIMEOUT,
                true,
            )
            .without_stalled_chunk_resend(),
        }
    }

    /// The firmware keeps its coolant and pump overlay on top of JPEG frames
    /// until an H.264 play session has run; the clip is 15 black 480x480
    /// frames encoded with the daemon's libx264 live settings.
    fn hide_firmware_overlay(&mut self) {
        if self.play_started {
            return;
        }
        let stop = AtomicBool::new(false);
        let mut clip: &[u8] = BLANK_CLIP;
        if let Err(error) = WinUsbLcd::stream_h264_reader(self, &mut clip, &stop, 30.0) {
            tracing::warn!("Hiding the firmware overlay failed: {error:#}");
        }
    }

    fn do_init(&mut self) -> Result<()> {
        self.core.init_logging();
        self.core.reset_failure_state();
        if let Err(error) = self.core.stop_playback() {
            tracing::warn!("Stopping previous LCD playback failed: {error:#}");
        }
        self.core.h2_control_init()?;
        self.core.initialized = true;
        Ok(())
    }
}

// Bulk OUT writes block on device NAK while the panel drains its h264
// buffer (usbmon shows 200 ms+ stalls mid-keyframe). A short timeout makes
// libusb cancel the URB mid-packet; rusb then reports the partial length and
// `write_full` resumes from it, which desyncs the firmware and wedges the MCU
// until power-cycled. This was the last LCD driver still on the pre-split
// 200 ms constant; 2 s matches base.rs, slv3.rs and hs2_oled.rs.
const WRITE_TIMEOUT: Duration = Duration::from_millis(2_000);
const READ_TIMEOUT: Duration = Duration::from_millis(2_000);

impl WinUsbLcd for H2WinUsbLcd {
    fn packet_builder(&mut self) -> &mut crate::crypto::PacketBuilder {
        self.core.builder_mut()
    }
    fn observe_h264_transfer(&mut self, transferred: std::sync::Arc<AtomicBool>) {
        self.core.h264_transferred = Some(transferred);
    }
    fn screen_info(&self) -> &ScreenInfo {
        self.core.screen()
    }
    fn firmware_str(&self) -> Option<&str> {
        self.core.firmware_str()
    }
    fn shared_transport(&self) -> SharedTransport {
        self.core.shared_transport()
    }
    fn stop_playback(&mut self) -> Result<()> {
        self.core.stop_playback()
    }
    fn initialize(&mut self) -> Result<()> {
        if !self.core.initialized || self.core.needs_init() {
            self.do_init()?;
        }
        Ok(())
    }
    fn send_frame(&mut self, frame: &[u8]) -> Result<()> {
        self.hide_firmware_overlay();
        if !self.core.initialized || self.core.needs_init() {
            self.do_init()?;
        }
        self.core.send_frame(frame)
    }
    fn send_frame_verified(&mut self, frame: &[u8]) -> Result<()> {
        self.hide_firmware_overlay();
        if !self.core.initialized || self.core.needs_init() {
            self.do_init()?;
        }
        self.core.send_frame_verified(frame)
    }
    fn set_brightness_val(&mut self, brightness: u8) -> Result<()> {
        self.core.set_brightness_val(brightness)
    }
    fn switch_to_desktop_mode(&mut self) -> Result<()> {
        self.core.switch_to_desktop_mode()
    }
    fn stream_h264(
        &mut self,
        path: &Path,
        looping: bool,
        stop: &AtomicBool,
        fps: f32,
    ) -> Result<()> {
        self.play_started = true;
        if !self.core.initialized || self.core.needs_init() {
            self.do_init()?;
        }
        self.core.apply_stream_fps(fps)?;
        self.core
            .stream_h264(path, looping, stop, fps, self.core.screen().play_count, 0)
    }
    fn stream_h264_reader(
        &mut self,
        reader: &mut dyn std::io::Read,
        stop: &AtomicBool,
        fps: f32,
    ) -> Result<()> {
        self.play_started = true;
        if !self.core.initialized || self.core.needs_init() {
            self.do_init()?;
        }
        self.core.apply_stream_fps(fps)?;
        self.core
            .stream_h264_reader(reader, stop, self.core.screen().play_count, 0)
    }
}

pub(crate) fn boxed(
    device: rusb::Device<lianli_transport::UsbContext>,
    screen: ScreenInfo,
    name: &str,
) -> Result<BoxedWinUsbLcd> {
    Ok(Box::new(H2WinUsbLcd::new(device, screen, name)?))
}

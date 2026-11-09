use anyhow::Result;
use lianli_shared::screen::ScreenInfo;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::core::{SharedTransport, WinUsbLcdCore};
use super::trait_::{BoxedWinUsbLcd, WinUsbLcd};

pub struct Slv3WinUsbLcd {
    core: WinUsbLcdCore,
    flex: bool,
}

impl Slv3WinUsbLcd {
    pub(crate) fn new(
        device: rusb::Device<lianli_transport::UsbContext>,
        screen: ScreenInfo,
        name: &str,
        stop_play_supported: bool,
        flex: bool,
    ) -> Result<Self> {
        let core = WinUsbLcdCore::open(
            device,
            screen,
            name,
            WRITE_TIMEOUT,
            READ_TIMEOUT,
            stop_play_supported,
        )?;
        Ok(Self { core, flex })
    }

    pub(crate) fn from_shared(
        transport: SharedTransport,
        screen: ScreenInfo,
        name: String,
        stop_play_supported: bool,
        flex: bool,
    ) -> Self {
        Self {
            core: WinUsbLcdCore::from_shared(
                transport,
                screen,
                name,
                WRITE_TIMEOUT,
                READ_TIMEOUT,
                stop_play_supported,
            ),
            flex,
        }
    }

    fn do_init(&mut self) -> Result<()> {
        self.core.init_logging();
        self.core.reset_failure_state();
        self.core.stop_playback()?;
        self.core.read_firmware();
        if self.flex {
            self.core.open_flex_layers()?;
            self.core.initialized = true;
            return Ok(());
        }
        self.core.set_frame_rate(SLV3_FRAME_RATE)?;
        self.core.query_h264_block();
        let sync = self.core.builder_mut().sync_clock_header_winusb(2);
        self.core.send_command(sync, "SyncClock");
        let stop_clock = self.core.builder_mut().stop_clock_header_winusb();
        self.core.send_command(stop_clock, "StopClock");
        self.core.clear_layers();
        self.core.initialized = true;
        Ok(())
    }

    fn prepare_stream(&mut self, fps: f32) -> Result<()> {
        if !self.core.initialized {
            self.do_init()?;
        }
        if self.flex {
            self.core.query_h264_block();
        }
        self.core.apply_stream_fps(fps)
    }
}

const WRITE_TIMEOUT: Duration = Duration::from_millis(2_000);
const READ_TIMEOUT: Duration = Duration::from_millis(2_000);
const SLV3_FRAME_RATE: u8 = 120;

impl WinUsbLcd for Slv3WinUsbLcd {
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
        if !self.core.initialized {
            self.do_init()?;
        }
        Ok(())
    }
    fn send_frame(&mut self, frame: &[u8]) -> Result<()> {
        if !self.core.initialized {
            self.do_init()?;
        }
        self.core.send_frame(frame)
    }
    fn send_frame_verified(&mut self, frame: &[u8]) -> Result<()> {
        if !self.core.initialized {
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
        self.prepare_stream(fps)?;
        self.core
            .stream_h264(path, looping, stop, fps, self.core.screen().play_count, 0)
    }
    fn stream_h264_reader(
        &mut self,
        reader: &mut dyn std::io::Read,
        stop: &AtomicBool,
        fps: f32,
    ) -> Result<()> {
        self.prepare_stream(fps)?;
        self.core
            .stream_h264_reader(reader, stop, self.core.screen().play_count, 0)
    }
}

pub(crate) fn boxed(
    device: rusb::Device<lianli_transport::UsbContext>,
    screen: ScreenInfo,
    name: &str,
    stop_play_supported: bool,
    flex: bool,
) -> Result<BoxedWinUsbLcd> {
    Ok(Box::new(Slv3WinUsbLcd::new(
        device,
        screen,
        name,
        stop_play_supported,
        flex,
    )?))
}

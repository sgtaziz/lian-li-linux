use std::sync::Arc;

use anyhow::Result;
use lianli_devices::traits::{RgbDevice, RgbFrameDelivery};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbPlaybackTiming, RgbZoneInfo};

pub(super) struct SyncDevice {
    pub inner: Arc<dyn RgbDevice>,
    pub led_count: u16,
}

impl RgbDevice for SyncDevice {
    fn device_name(&self) -> String {
        self.inner.device_name()
    }
    fn supported_modes(&self) -> Vec<RgbMode> {
        Vec::new()
    }
    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![RgbZoneInfo {
            name: "Sync".into(),
            led_count: self.led_count,
        }]
    }
    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        self.inner.set_zone_effect(zone, effect)
    }
    fn rf_owned(&self) -> bool {
        self.inner.rf_owned()
    }
    fn software_frame_delivery(&self) -> Option<RgbFrameDelivery> {
        self.inner.software_frame_delivery()
    }
    fn validate_software_animation(
        &self,
        frames: &[Vec<[u8; 3]>],
        timing: RgbPlaybackTiming,
    ) -> Result<()> {
        self.inner.validate_sync_animation(frames, timing)
    }
    fn set_software_animation(
        &self,
        frames: &[Vec<[u8; 3]>],
        timing: RgbPlaybackTiming,
    ) -> Result<()> {
        self.inner.set_sync_animation(frames, timing)
    }
}

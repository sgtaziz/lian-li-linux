//! Per-device trait adapters for wireless devices discovered via the RF dongle.
//!
//! A `WirelessController` is a USB-attached dongle that speaks to many
//! wireless devices over 2.4 GHz RF. Each discovered device is identified by
//! its MAC and presents as one of:
//!
//! - [`WirelessFanDevice`] — fan PWM control via `FanDevice`
//! - [`WirelessAioDevice`] — full AIO (pump + coolant temp) via `AioDevice`
//! - [`WirelessRgbDevice`] — RGB effects via `RgbDevice`
//!
//! With these adapters, wireless devices flow through `Box<dyn FanDevice>`
//! (etc.) exactly like wired USB devices — the daemon's controller loops no
//! longer need a separate "is this wireless?" branch.

use anyhow::Result;
use lianli_shared::id::MacAddress;
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope, RgbZoneInfo};
use parking_lot::Mutex;
use std::sync::Arc;

use crate::wireless::{DiscoveredDevice, WirelessController, WirelessFanType};

/// Fan controller backed by a wireless device.
///
/// Holds a reference to the dongle and the device's MAC. All `FanDevice`
/// methods translate to RF packet sends via the dongle.
pub struct WirelessFanDevice {
    pub(crate) ctrl: Arc<WirelessController>,
    pub(crate) mac: MacAddress,
    pub(crate) fan_count: u8,
    pub(crate) fan_type: WirelessFanType,
    pub(crate) last_known_rpms: Arc<Mutex<[u16; 4]>>,
}

impl WirelessFanDevice {
    pub fn new(
        ctrl: Arc<WirelessController>,
        mac: MacAddress,
        fan_count: u8,
        fan_type: WirelessFanType,
    ) -> Self {
        Self {
            ctrl,
            mac,
            fan_count,
            fan_type,
            last_known_rpms: Arc::new(Mutex::new([0; 4])),
        }
    }

    /// Update the cached RPM values from the latest discovery snapshot.
    pub fn update_rpms(&self, rpms: [u16; 4]) {
        *self.last_known_rpms.lock() = rpms;
    }

    pub fn mac(&self) -> MacAddress {
        self.mac
    }

    pub fn fan_type(&self) -> WirelessFanType {
        self.fan_type
    }
}

impl crate::traits::FanDevice for WirelessFanDevice {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        let mut duties = [0u8; 4];
        duties[slot as usize % 4] = duty;
        self.set_fan_speeds(&duties)
    }

    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        let mut fan_pwm = [0u8; 4];
        for (i, &d) in duties.iter().take(4).enumerate() {
            fan_pwm[i] = d;
        }
        self.ctrl.set_fan_speeds_by_mac(&self.mac.bytes(), &fan_pwm)
    }

    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        Ok(self.last_known_rpms.lock()[..self.fan_count as usize].to_vec())
    }

    fn fan_slot_count(&self) -> u8 {
        self.fan_count.max(1)
    }

    fn per_fan_control(&self) -> bool {
        false
    }

    fn has_pump_control(&self) -> bool {
        self.fan_type.is_aio()
    }

    fn set_pump_speed(&self, duty: u8) -> Result<()> {
        if !self.fan_type.is_aio() {
            anyhow::bail!("not an AIO");
        }
        // Pump PWM goes into slot 3 of the RF packet for AIOs.
        let mut fan_pwm = [0u8; 4];
        fan_pwm[3] = duty;
        self.ctrl.set_fan_speeds_by_mac(&self.mac.bytes(), &fan_pwm)
    }
}

/// AIO controller backed by a wireless device (WaterBlock / WaterBlock2).
///
/// Adds coolant-temp + pump-RPM reading on top of [`WirelessFanDevice`].
pub struct WirelessAioDevice {
    pub(crate) fan: WirelessFanDevice,
    pub(crate) last_coolant_temp: Arc<Mutex<Option<f32>>>,
}

impl WirelessAioDevice {
    pub fn new(fan: WirelessFanDevice) -> Self {
        Self {
            fan,
            last_coolant_temp: Arc::new(Mutex::new(None)),
        }
    }

    /// Update the cached coolant temperature from the latest discovery snapshot.
    pub fn update_coolant_temp(&self, temp_celsius: Option<f32>) {
        *self.last_coolant_temp.lock() = temp_celsius;
    }
}

impl crate::traits::FanDevice for WirelessAioDevice {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        self.fan.set_fan_speed(slot, duty)
    }
    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        self.fan.set_fan_speeds(duties)
    }
    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        self.fan.read_fan_rpm()
    }
    fn fan_slot_count(&self) -> u8 {
        self.fan.fan_slot_count()
    }
    fn per_fan_control(&self) -> bool {
        false
    }
    fn has_pump_control(&self) -> bool {
        true
    }
    fn set_pump_speed(&self, duty: u8) -> Result<()> {
        self.fan.set_pump_speed(duty)
    }
}

impl crate::traits::AioDevice for WirelessAioDevice {
    fn read_pump_rpm(&self) -> Result<u16> {
        let rpms = self.fan.last_known_rpms.lock();
        Ok(rpms.get(3).copied().unwrap_or(0))
    }

    fn read_coolant_temp(&self) -> Result<f32> {
        self.last_coolant_temp
            .lock()
            .ok_or_else(|| anyhow::anyhow!("coolant temp not available"))
    }
}

/// RGB controller backed by a wireless device.
///
/// Sends host-rendered RGB frames to the device via the dongle's RF channel.
/// Each call to `set_zone_effect` or `set_direct_colors` results in a tinyuz-
/// compressed payload chunked across multiple RF packets.
pub struct WirelessRgbDevice {
    pub(crate) ctrl: Arc<WirelessController>,
    pub(crate) mac: MacAddress,
    pub(crate) fan_type: WirelessFanType,
    pub(crate) led_state: Arc<Mutex<Vec<[u8; 3]>>>,
}

impl WirelessRgbDevice {
    pub fn new(
        ctrl: Arc<WirelessController>,
        mac: MacAddress,
        fan_type: WirelessFanType,
        led_count: usize,
    ) -> Self {
        Self {
            ctrl,
            mac,
            fan_type,
            led_state: Arc::new(Mutex::new(vec![[0, 0, 0]; led_count])),
        }
    }

    pub fn mac(&self) -> MacAddress {
        self.mac
    }

    pub fn led_state(&self) -> Arc<Mutex<Vec<[u8; 3]>>> {
        Arc::clone(&self.led_state)
    }
}

impl crate::traits::RgbDevice for WirelessRgbDevice {
    fn device_name(&self) -> String {
        format!("Wireless {}", self.fan_type.display_name())
    }

    fn supported_modes(&self) -> Vec<RgbMode> {
        // Wireless devices accept any mode; the host renders the effect and
        // streams frames. The GUI filters the visible list per fan type.
        vec![
            RgbMode::Off,
            RgbMode::Direct,
            RgbMode::Static,
            RgbMode::Rainbow,
            RgbMode::Breathing,
            RgbMode::ColorCycle,
            RgbMode::Meteor,
            RgbMode::Runway,
            RgbMode::Staggered,
            RgbMode::Tide,
            RgbMode::Mixing,
            RgbMode::Ripple,
            RgbMode::Wave,
            RgbMode::TailChasing,
            RgbMode::PingPong,
            RgbMode::Stack,
            RgbMode::CoverCycle,
            RgbMode::Racing,
            RgbMode::Lottery,
            RgbMode::Intertwine,
            RgbMode::MeteorShower,
            RgbMode::Collide,
            RgbMode::ElectricCurrent,
            RgbMode::Kaleidoscope,
        ]
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![RgbZoneInfo {
            name: String::new(),
            led_count: self.led_state.lock().len() as u16,
        }]
    }

    fn set_zone_effect(&self, _zone: u8, effect: &RgbEffect) -> Result<()> {
        // Wireless devices render the effect on the host and stream the result
        // as RGB frames. The RgbController invokes `send_rgb_direct` from its
        // wireless effect-rendering thread; this method just records the
        // current colour state.
        let mut state = self.led_state.lock();
        let color = effect.colors.first().copied().unwrap_or([0, 0, 0]);
        for led in state.iter_mut() {
            *led = color;
        }
        Ok(())
    }

    fn set_direct_colors(&self, _zone: u8, colors: &[[u8; 3]]) -> Result<()> {
        let mut state = self.led_state.lock();
        for (i, led) in state.iter_mut().enumerate() {
            *led = colors.get(i).copied().unwrap_or([0, 0, 0]);
        }
        let snapshot = state.clone();
        drop(state);
        self.ctrl
            .send_rgb_direct(&self.mac.bytes(), &snapshot, &[0, 0, 0, 1], 2)
    }

    fn supports_direct(&self) -> bool {
        true
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        vec![vec![RgbScope::All]]
    }
}

/// Build the appropriate trait objects for a wireless discovered device.
///
/// Returns `(Option<WirelessFanDevice>, Option<WirelessAioDevice>)` so the
/// caller can decide what to register. RGB is handled separately by the
/// RgbController because effect animation runs on its own thread.
pub fn adapters_for_discovered(
    ctrl: Arc<WirelessController>,
    dev: &DiscoveredDevice,
) -> (Option<WirelessFanDevice>, Option<WirelessAioDevice>) {
    let mac = MacAddress::new(dev.mac);
    let fan_count = dev.fan_count.max(1);
    let fan_type = dev.fan_type;

    let fan = WirelessFanDevice::new(ctrl.clone(), mac, fan_count, fan_type);
    fan.update_rpms(dev.fan_rpms);

    if fan_type.is_aio() {
        let aio = WirelessAioDevice::new(fan);
        aio.update_coolant_temp(dev.coolant_temp_c.map(|c| c as f32));
        (None, Some(aio))
    } else {
        (Some(fan), None)
    }
}

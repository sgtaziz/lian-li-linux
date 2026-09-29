use super::*;
use std::collections::HashMap;

pub(super) type States = HashMap<String, Arc<Mutex<State>>>;

pub(super) fn states(caps: &[RgbDeviceCapabilities], previous: &States) -> States {
    caps.iter()
        .map(|cap| {
            let state = previous
                .get(&cap.device_id)
                .filter(|state| state.lock().cap == *cap)
                .cloned()
                .unwrap_or_else(|| Arc::new(Mutex::new(State::new(cap.clone()))));
            (cap.device_id.clone(), state)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_devices::traits::RgbDevice;
    use lianli_shared::rgb::RgbZoneInfo;

    struct FailingZones {
        calls: Arc<Mutex<Vec<u8>>>,
        all_fail: bool,
        zone_count: usize,
    }
    impl RgbDevice for FailingZones {
        fn device_name(&self) -> String {
            "Test".into()
        }
        fn supported_modes(&self) -> Vec<RgbMode> {
            vec![RgbMode::Breathing]
        }
        fn zone_info(&self) -> Vec<RgbZoneInfo> {
            vec![
                RgbZoneInfo {
                    name: "Zone".into(),
                    led_count: 1
                };
                self.zone_count
            ]
        }
        fn set_zone_effect(&self, zone: u8, _: &RgbEffect) -> anyhow::Result<()> {
            self.calls.lock().push(zone);
            anyhow::ensure!(zone != 0 && !self.all_fail, "test write failure");
            Ok(())
        }
    }

    #[test]
    fn partial_mode_delivery_attempts_every_zone_and_preserves_concurrent_colors() {
        for all_fail in [false, true] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let device = Arc::new(FailingZones {
                calls: calls.clone(),
                all_fail,
                zone_count: 2,
            });
            let mut rgb = RgbController::new(
                HashMap::from([("test".into(), device as Arc<dyn RgbDevice>)]),
                None,
            );
            let mut state = State::new(rgb.exposed_capabilities().remove(0));
            let mut next = state.clone();
            state
                .apply_colors(
                    &Command::Colors(vec![[1, 2, 3]; 2]),
                    &Mutex::new(DirectColorBuffer::new()),
                )
                .unwrap();
            let mode = Command::Mode {
                index: 1,
                mode: next.modes[1].clone(),
            };
            let result = next.apply_mode(&mode, &mut rgb);
            assert_eq!(*calls.lock(), [0, 1]);
            assert_eq!(result.is_err(), all_fail);
            if result.is_ok() {
                state.commit_mode(next);
            }
            assert_eq!(state.active, usize::from(!all_fail));
            assert_eq!(state.colors, [[1, 2, 3]; 2]);
            assert_eq!(state.revision, if all_fail { 1 } else { 2 });
            rgb.stop();
        }
    }

    #[test]
    fn mode_on_device_without_zones_updates_state_without_hardware_writes() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let device = Arc::new(FailingZones {
            calls: calls.clone(),
            all_fail: true,
            zone_count: 0,
        });
        let mut rgb = RgbController::new(
            HashMap::from([("test".into(), device as Arc<dyn RgbDevice>)]),
            None,
        );
        let mut state = State::new(rgb.exposed_capabilities().remove(0));
        let mut next = state.clone();
        let command = Command::Mode {
            index: 1,
            mode: next.modes[1].clone(),
        };
        next.apply_mode(&command, &mut rgb).unwrap();
        state.commit_mode(next);
        assert_eq!(state.active, 1);
        assert_eq!(state.revision, 1);
        assert!(calls.lock().is_empty());
        rgb.stop();
    }
}

#[derive(Clone)]
pub(super) struct State {
    cap: RgbDeviceCapabilities,
    modes: Vec<ModeData>,
    active: usize,
    colors: Vec<[u8; 3]>,
    revision: u64,
}

impl State {
    pub fn new(cap: RgbDeviceCapabilities) -> Self {
        let modes = ControllerSerializer {
            protocol_version: 6,
        }
        .build_modes(&cap);
        let colors = vec![[255; 3]; cap.total_led_count as usize];
        Self {
            cap,
            modes,
            active: 0,
            colors,
            revision: 0,
        }
    }

    pub fn description(&self, version: u32) -> Vec<u8> {
        ControllerSerializer {
            protocol_version: version,
        }
        .build_controller_zones(
            &self.cap,
            &self.modes,
            self.active as u32,
            &self.colors,
            None,
        )
    }

    pub fn apply_mode(&mut self, command: &Command, rgb: &mut RgbController) -> anyhow::Result<()> {
        match command {
            Command::Custom => {
                rgb.set_openrgb_active(true);
                self.active = 0;
            }
            Command::Mode { index, mode } => {
                let selected = self
                    .modes
                    .get_mut(*index)
                    .ok_or_else(|| anyhow::anyhow!("Unknown device mode"))?;
                anyhow::ensure!(selected.name == mode.name, "Device mode identity mismatch");
                anyhow::ensure!(
                    mode.color_mode == selected.color_mode && mode.direction <= DIR_DOWN,
                    "Invalid device mode parameters"
                );
                anyhow::ensure!(
                    (selected.colors_min as usize..=selected.colors_max as usize)
                        .contains(&mode.colors.len()),
                    "Invalid device palette size"
                );
                selected.speed = mode.speed.min(MAX_EFFECT_VALUE);
                selected.brightness = mode.brightness.min(MAX_EFFECT_VALUE);
                selected.direction = mode.direction;
                selected.colors.clone_from(&mode.colors);
                let effect = RgbEffect {
                    mode: mode_from_openrgb_name(&selected.name, selected.value),
                    colors: selected.colors.clone(),
                    speed: selected.speed as u8,
                    brightness: selected.brightness as u8,
                    direction: match selected.direction {
                        DIR_LEFT => RgbDirection::CounterClockwise,
                        DIR_UP => RgbDirection::Up,
                        DIR_DOWN => RgbDirection::Down,
                        _ => RgbDirection::Clockwise,
                    },
                    ..Default::default()
                };
                if effect.mode != RgbMode::Direct {
                    let mut failed = 0;
                    for zone in 0..self.cap.zones.len() {
                        if let Err(error) = rgb.set_effect(&self.cap.device_id, zone as u8, &effect)
                        {
                            failed += 1;
                            warn!(device = %self.cap.device_id, zone, %error, "OpenRGB mode delivery failed");
                        }
                    }
                    anyhow::ensure!(
                        self.cap.zones.is_empty() || failed < self.cap.zones.len(),
                        "Mode delivery failed for every zone"
                    );
                    if failed > 0 {
                        warn!(device = %self.cap.device_id, failed, total = self.cap.zones.len(),
                            "OpenRGB mode only partially delivered; reporting the requested setting");
                    }
                }
                self.active = *index;
            }
            _ => anyhow::bail!("Unsupported device mode command"),
        }
        Ok(())
    }

    pub fn commit_mode(&mut self, next: Self) {
        // Colour updates can arrive while the mode write is waiting on the hardware.
        self.modes = next.modes;
        self.active = next.active;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn apply_colors(
        &mut self,
        command: &Command,
        buffer: &Mutex<DirectColorBuffer>,
    ) -> anyhow::Result<()> {
        match command {
            Command::Colors(colors) => {
                anyhow::ensure!(
                    colors.len() == self.colors.len(),
                    "Invalid device color count"
                );
                self.colors.clone_from(colors);
                self.queue_colors(buffer, None);
            }
            Command::ZoneColors { zone, colors } => {
                let range = self.zone_range(*zone)?;
                anyhow::ensure!(colors.len() == range.len(), "Invalid zone color count");
                self.colors[range].copy_from_slice(colors);
                self.queue_colors(buffer, Some(*zone));
            }
            Command::SingleColor { led, color } => {
                let target = self
                    .colors
                    .get_mut(*led)
                    .ok_or_else(|| anyhow::anyhow!("Invalid device LED"))?;
                *target = *color;
                let zone = (0..self.cap.zones.len())
                    .find(|zone| {
                        self.zone_range(*zone)
                            .is_ok_and(|range| range.contains(led))
                    })
                    .ok_or_else(|| anyhow::anyhow!("LED has no zone"))?;
                self.queue_colors(buffer, Some(zone));
            }
            _ => anyhow::bail!("Unsupported device command"),
        }
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    fn zone_range(&self, zone: usize) -> anyhow::Result<std::ops::Range<usize>> {
        let info = self
            .cap
            .zones
            .get(zone)
            .ok_or_else(|| anyhow::anyhow!("Invalid device zone"))?;
        let start: usize = self.cap.zones[..zone]
            .iter()
            .map(|zone| zone.led_count as usize)
            .sum();
        let end = start + info.led_count as usize;
        anyhow::ensure!(end <= self.colors.len(), "Invalid device layout");
        Ok(start..end)
    }

    fn queue_colors(&self, buffer: &Mutex<DirectColorBuffer>, selected: Option<usize>) {
        let mut buffer = buffer.lock();
        for zone in 0..self.cap.zones.len() {
            if selected.is_none_or(|selected| selected == zone) {
                if let Ok(range) = self.zone_range(zone) {
                    buffer.set(
                        self.cap.device_id.clone(),
                        zone as u8,
                        self.colors[range].to_vec(),
                    );
                }
            }
        }
    }

    pub fn update(&self, kind: u32) -> clients::Update {
        let description = if clients::is_color_update(kind) {
            Vec::new()
        } else {
            self.description(6)
        };
        clients::Update {
            revision: self.revision,
            data: clients::signal(kind, &self.colors, &description),
        }
    }
}

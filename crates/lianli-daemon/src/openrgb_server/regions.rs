use super::*;
use lianli_shared::rgb::{RgbEffectParameters, RgbScope, RgbZoneInfo};
use std::collections::HashMap;

pub(super) type Groups = HashMap<String, Arc<Mutex<RegionGroup>>>;

#[derive(Clone)]
pub(super) enum Target {
    Legacy(Arc<RgbDeviceCapabilities>),
    Region {
        group: Arc<Mutex<RegionGroup>>,
        region: Option<usize>,
    },
}

pub(super) fn groups(
    caps: &[RgbDeviceCapabilities],
    rgb: &RgbController,
    previous: &Groups,
    enabled: bool,
) -> Groups {
    if !enabled {
        return Groups::new();
    }
    caps.iter()
        .filter(|cap| cap.supports_openrgb_regions())
        .map(|cap| {
            let group = previous
                .get(&cap.device_id)
                .filter(|group| group.lock().cap == *cap)
                .cloned()
                .unwrap_or_else(|| {
                    let effects = match rgb.saved_group_effects(&cap.device_id) {
                        Ok(effects) => effects,
                        Err(error) => {
                            warn!(
                                device = %cap.device_id, %error,
                                "Cannot initialize OpenRGB regions from saved lighting"
                            );
                            Vec::new()
                        }
                    };
                    Arc::new(Mutex::new(RegionGroup::new(cap.clone(), &effects)))
                });
            (cap.device_id.clone(), group)
        })
        .collect()
}

pub(super) fn targets(
    caps: &[RgbDeviceCapabilities],
    groups: &Groups,
    version: u32,
) -> Vec<Target> {
    let mut targets = Vec::new();
    for cap in caps {
        if let Some(group) = groups.get(&cap.device_id) {
            if cap.zones.is_empty() {
                continue;
            }
            if version >= 6 {
                targets.push(Target::Region {
                    group: Arc::clone(group),
                    region: None,
                });
            } else {
                for region in 0..2 {
                    targets.push(Target::Region {
                        group: Arc::clone(group),
                        region: Some(region),
                    });
                }
            }
        } else {
            targets.push(Target::Legacy(Arc::new(cap.clone())));
        }
    }
    targets
}

#[derive(Clone, PartialEq, Eq)]
struct State {
    mode: usize,
    effect: RgbEffect,
    colors: Vec<[u8; 3]>,
    follow: bool,
}

struct Region {
    modes: Vec<RgbEffectParameters>,
    state: State,
}

pub(super) struct RegionGroup {
    pub(super) cap: RgbDeviceCapabilities,
    regions: [Region; 2],
    modes: Vec<RgbEffectParameters>,
    state: State,
    revision: u64,
}

fn direct_mode() -> RgbEffectParameters {
    RgbEffectParameters {
        mode: RgbMode::Direct,
        min_colors: 0,
        max_colors: 0,
        per_fan_colors: true,
        directions: Vec::new(),
        supports_speed: false,
    }
}

impl RegionGroup {
    fn new(cap: RgbDeviceCapabilities, saved: &[RgbEffect]) -> Self {
        let regions = [RgbScope::Inner, RgbScope::Outer].map(|scope| {
            let mut modes = vec![direct_mode()];
            modes.extend(
                cap.region_parameters
                    .iter()
                    .find(|region| region.scope == scope)
                    .unwrap()
                    .effects
                    .clone(),
            );
            let mut effect = saved
                .iter()
                .rev()
                .find(|effect| effect.scope == scope || effect.scope == RgbScope::All)
                .cloned()
                .unwrap_or_else(|| RgbEffect {
                    scope,
                    ..Default::default()
                });
            effect.scope = scope;
            if effect.disabled || lianli_shared::rgb::is_brightness_off(effect.brightness) {
                effect.mode = if modes.iter().any(|mode| mode.mode == RgbMode::Off) {
                    RgbMode::Off
                } else {
                    effect.colors = vec![[0; 3]];
                    RgbMode::Direct
                };
                effect.brightness = DEFAULT_BRIGHTNESS as u8;
                effect.disabled = false;
            }
            let mode = modes
                .iter()
                .position(|mode| mode.mode == effect.mode)
                .unwrap_or(0);
            effect.mode = modes[mode].mode;
            if !modes[mode].per_fan_colors {
                effect.colors.truncate(modes[mode].max_colors as usize);
                if effect.colors.len() < modes[mode].min_colors as usize {
                    effect
                        .colors
                        .resize(modes[mode].min_colors as usize, [255; 3]);
                }
            }
            let colors = (0..cap.zones.len())
                .map(|index| {
                    effect
                        .colors
                        .get(index)
                        .or(effect.colors.first())
                        .copied()
                        .unwrap_or([255; 3])
                })
                .collect();
            Region {
                modes,
                state: State {
                    mode,
                    effect,
                    colors,
                    follow: false,
                },
            }
        });
        let modes = regions[0]
            .modes
            .iter()
            .filter(|mode| regions[1].modes.contains(mode))
            .cloned()
            .collect();
        let state = State {
            mode: 0,
            effect: RgbEffect {
                mode: RgbMode::Direct,
                speed: 0,
                ..Default::default()
            },
            colors: Vec::new(),
            follow: false,
        };
        Self {
            cap,
            regions,
            modes,
            state,
            revision: 0,
        }
    }

    pub(super) fn count(&self) -> usize {
        self.cap.zones.len()
    }

    fn effects(&self, states: &[State; 2]) -> Vec<RgbEffect> {
        states
            .iter()
            .enumerate()
            .map(|(region, state)| {
                let mut effect = state.effect.clone();
                if self.regions[region].modes[state.mode].per_fan_colors {
                    effect.colors = state.colors.clone();
                }
                if effect.mode == RgbMode::Direct {
                    effect.mode = RgbMode::Static;
                }
                effect
            })
            .collect()
    }

    pub(super) fn apply(
        &mut self,
        command: &Command,
        region: Option<usize>,
        buffer: &Mutex<DirectColorBuffer>,
    ) -> anyhow::Result<()> {
        let mut next = [self.regions[0].state.clone(), self.regions[1].state.clone()];
        let mut device = self.state.clone();
        match command {
            Command::Custom => {
                if let Some(region) = region {
                    next[region].mode = 0;
                    next[region].effect.mode = RgbMode::Direct;
                    next[region].follow = false;
                } else {
                    device.mode = 0;
                    device.effect.mode = RgbMode::Direct;
                    for (index, state) in next.iter_mut().enumerate() {
                        self.follow(&device, index, state);
                    }
                }
            }
            Command::Mode { index, mode } => {
                if let Some(region) = region {
                    set_mode(&self.regions[region].modes, &mut next[region], *index, mode)?;
                } else {
                    set_mode(&self.modes, &mut device, *index, mode)?;
                    for (index, state) in next.iter_mut().enumerate() {
                        self.follow(&device, index, state);
                    }
                }
            }
            Command::ZoneMode { zone, selection } => {
                anyhow::ensure!(region.is_none() && *zone < 2, "Invalid region zone");
                if let Some((index, mode)) = selection {
                    set_mode(&self.regions[*zone].modes, &mut next[*zone], *index, mode)?;
                } else {
                    self.follow(&device, *zone, &mut next[*zone]);
                }
            }
            Command::Colors(colors) => {
                let count = self.count();
                anyhow::ensure!(
                    colors.len() == count * if region.is_some() { 1 } else { 2 },
                    "Invalid region color count"
                );
                if let Some(region) = region {
                    next[region].colors.clone_from(colors);
                } else {
                    next[0].colors = colors[..count].to_vec();
                    next[1].colors = colors[count..].to_vec();
                }
            }
            Command::ZoneColors { zone, colors } => {
                if let Some(region) = region {
                    anyhow::ensure!(
                        *zone < self.count() && colors.len() == 1,
                        "Invalid fan colors"
                    );
                    next[region].colors[*zone] = colors[0];
                } else {
                    anyhow::ensure!(
                        *zone < 2 && colors.len() == self.count(),
                        "Invalid region colors"
                    );
                    next[*zone].colors.clone_from(colors);
                }
            }
            Command::SingleColor { led, color } => {
                let count = self.count();
                anyhow::ensure!(
                    *led < count * if region.is_some() { 1 } else { 2 },
                    "Invalid region LED"
                );
                let index = region.unwrap_or(*led / count);
                next[index].colors[*led % count] = *color;
            }
            Command::Resize { .. } => anyhow::bail!("Resize is not a lighting update"),
        }
        // Hardware palettes remain independent of the per-fan colour cache replayed by profiles.
        let effects = self.effects(&next);
        buffer.lock().set_group(self.cap.device_id.clone(), effects);
        let [inner, outer] = next;
        self.regions[0].state = inner;
        self.regions[1].state = outer;
        self.state = device;
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub(super) fn queue_current(&self, buffer: &Mutex<DirectColorBuffer>) {
        buffer.lock().set_group(
            self.cap.device_id.clone(),
            self.effects(&[self.regions[0].state.clone(), self.regions[1].state.clone()]),
        );
    }

    pub(super) fn update(&self, kind: u32) -> clients::Update {
        let colors: Vec<_> = self
            .regions
            .iter()
            .flat_map(|region| region.state.colors.iter().copied())
            .collect();
        let description = if clients::is_color_update(kind) {
            Vec::new()
        } else {
            self.description(6, None)
        };
        clients::Update {
            revision: self.revision,
            data: clients::signal(kind, &colors, &description),
        }
    }

    fn follow(&self, device: &State, region: usize, state: &mut State) {
        state.mode = self.regions[region]
            .modes
            .iter()
            .position(|mode| mode.mode == device.effect.mode)
            .expect("Common mode exists in both regions");
        state.effect = device.effect.clone();
        state.effect.scope = if region == 0 {
            RgbScope::Inner
        } else {
            RgbScope::Outer
        };
        state.follow = true;
    }

    pub(super) fn description(&self, version: u32, region: Option<usize>) -> Vec<u8> {
        let mut cap = self.cap.clone();
        let (modes, active, colors, zones) = if let Some(index) = region {
            let label = if index == 0 { "Inner" } else { "Outer" };
            cap.device_name = format!("{} — {label}", cap.device_name);
            cap.device_id = format!("{}:{}", cap.device_id, label.to_lowercase());
            cap.total_led_count = self.count() as u16;
            cap.zones = (0..self.count())
                .map(|index| RgbZoneInfo {
                    name: format!("Fan {} {label}", index + 1),
                    led_count: 1,
                })
                .collect();
            let region = &self.regions[index];
            (
                mode_list(&region.modes, &region.state),
                region.state.mode,
                region.state.colors.clone(),
                None,
            )
        } else {
            cap.device_id = format!("{}:regions", cap.device_id);
            cap.total_led_count = (self.count() * 2) as u16;
            cap.zones = ["Inner", "Outer"]
                .map(|name| RgbZoneInfo {
                    name: name.into(),
                    led_count: self.count() as u16,
                })
                .to_vec();
            let zones = self
                .regions
                .iter()
                .map(|region| ZoneModeData {
                    active_mode: if region.state.follow {
                        -1
                    } else {
                        region.state.mode as i32
                    },
                    modes: mode_list(&region.modes, &region.state),
                })
                .collect::<Vec<_>>();
            let colors = self
                .regions
                .iter()
                .flat_map(|region| region.state.colors.iter().copied())
                .collect();
            (
                mode_list(&self.modes, &self.state),
                self.state.mode,
                colors,
                Some(zones),
            )
        };
        ControllerSerializer {
            protocol_version: version,
        }
        .build_controller_zones(&cap, &modes, active as u32, &colors, zones.as_deref())
    }

    #[cfg(test)]
    fn signal_update(&self, kind: u32) -> Vec<u8> {
        self.update(kind).data
    }
}

fn color_mode(spec: &RgbEffectParameters) -> u32 {
    if spec.per_fan_colors {
        COLOR_MODE_PER_LED
    } else if spec.max_colors > 0 {
        COLOR_MODE_MODE_SPECIFIC
    } else {
        COLOR_MODE_NONE
    }
}

fn mode_list(specs: &[RgbEffectParameters], state: &State) -> Vec<ModeData> {
    specs
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let active = index == state.mode;
            let directional = spec.directions.contains(&RgbDirection::Clockwise)
                && spec.directions.contains(&RgbDirection::CounterClockwise);
            let mut flags = MODE_FLAG_HAS_BRIGHTNESS;
            if spec.supports_speed {
                flags |= MODE_FLAG_HAS_SPEED;
            }
            if directional {
                flags |= MODE_FLAG_HAS_DIRECTION_LR;
            }
            let color_mode = color_mode(spec);
            flags |= match color_mode {
                COLOR_MODE_PER_LED => MODE_FLAG_HAS_PER_LED_COLOR,
                COLOR_MODE_MODE_SPECIFIC => MODE_FLAG_HAS_MODE_SPECIFIC_COLOR,
                _ => 0,
            };
            let (colors_min, colors_max) = if spec.per_fan_colors {
                (0, 0)
            } else {
                (spec.min_colors as u32, spec.max_colors as u32)
            };
            let mut colors: Vec<_> = if active {
                state
                    .effect
                    .colors
                    .iter()
                    .copied()
                    .take(colors_max as usize)
                    .collect()
            } else {
                Vec::new()
            };
            colors.resize(colors.len().max(colors_min as usize), [255; 3]);
            ModeData {
                name: spec.mode.display_name().into(),
                value: index as u32,
                flags,
                speed_min: 0,
                speed_max: if spec.supports_speed {
                    MAX_EFFECT_VALUE
                } else {
                    0
                },
                brightness_min: 0,
                brightness_max: MAX_EFFECT_VALUE,
                colors_min,
                colors_max,
                speed: if !spec.supports_speed {
                    0
                } else if active {
                    state.effect.speed as u32
                } else {
                    DEFAULT_SPEED
                },
                brightness: if active {
                    state.effect.brightness as u32
                } else {
                    DEFAULT_BRIGHTNESS
                },
                direction: if directional
                    && state.effect.direction != RgbDirection::CounterClockwise
                {
                    DIR_RIGHT
                } else {
                    DIR_LEFT
                },
                color_mode,
                colors,
            }
        })
        .collect()
}

fn set_mode(
    specs: &[RgbEffectParameters],
    state: &mut State,
    index: usize,
    mode: &ModeData,
) -> anyhow::Result<()> {
    let spec = specs
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("Invalid region mode"))?;
    anyhow::ensure!(
        mode.name == spec.mode.display_name(),
        "Region mode identity mismatch"
    );
    anyhow::ensure!(
        mode.direction <= DIR_RIGHT && mode.color_mode == color_mode(spec),
        "Invalid region mode parameters"
    );
    // SDK mode packets carry palettes; per-fan colours use separate LED packets.
    spec.validate_values(mode.speed, mode.brightness, mode.colors.len(), 0)
        .map_err(anyhow::Error::msg)?;
    state.mode = index;
    state.effect = RgbEffect {
        mode: spec.mode,
        scope: state.effect.scope,
        colors: mode.colors.clone(),
        speed: mode.speed as u8,
        brightness: mode.brightness as u8,
        direction: if !spec.directions.is_empty() && mode.direction == DIR_LEFT {
            RgbDirection::CounterClockwise
        } else {
            RgbDirection::Clockwise
        },
        disabled: false,
    };
    state.follow = false;
    Ok(())
}

#[cfg(test)]
mod tests;

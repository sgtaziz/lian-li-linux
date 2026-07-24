use crate::backend;
use crate::{MainWindow, Shared};
use lianli_shared::fan::FanSpeed;
use slint::ComponentHandle;

pub(crate) fn wire_aio_callbacks(
    window: &MainWindow,
    _backend: &backend::BackendHandle,
    shared: &Shared,
) {
    use lianli_shared::aio::AioConfig;

    fn with_aio<F>(shared: &Shared, device_id: &str, f: F) -> bool
    where
        F: FnOnce(&mut AioConfig),
    {
        let mut state = shared.lock().unwrap();
        let Some(cfg) = state.config.as_mut() else {
            return false;
        };
        let entry = cfg.aio.entry(device_id.to_string()).or_default();
        f(entry);
        true
    }

    fn mark_dirty(weak: &slint::Weak<MainWindow>) {
        let weak = weak.clone();
        slint::invoke_from_event_loop(move || {
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        })
        .ok();
    }

    // Pump speed mode (Off / curve name / Constant PWM)
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_aio_set_pump_speed_mode(move |dev_id, val| {
            let dev_id = dev_id.to_string();
            let val = val.to_string();
            let changed = with_aio(&shared, &dev_id, |aio| {
                aio.pump_target_rpm = match val.as_str() {
                    "Off" => FanSpeed::Constant(0),
                    "Constant PWM" => FanSpeed::Constant(128),
                    "MB Sync" => FanSpeed::Constant(128),
                    curve => FanSpeed::Curve(curve.to_string()),
                };
            });
            if changed {
                crate::refresh_aio_ui(&weak, &shared);
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_aio_set_pump_pwm(move |dev_id, percent| {
            let dev_id = dev_id.to_string();
            let percent = percent.clamp(0, 100) as u32;
            let byte = ((percent * 255) / 100).min(255) as u8;
            let changed = with_aio(&shared, &dev_id, |aio| {
                aio.pump_target_rpm = FanSpeed::Constant(byte);
            });
            if changed {
                mark_dirty(&weak);
            }
        });
    }

    // Fan speed mode
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_aio_set_fan_speed_mode(move |dev_id, slot, val| {
            let dev_id = dev_id.to_string();
            let slot = (slot as usize).min(3);
            let val = val.to_string();
            let changed = with_aio(&shared, &dev_id, |aio| {
                aio.fan_speeds[slot] = match val.as_str() {
                    "Off" => FanSpeed::Constant(0),
                    "Constant PWM" => FanSpeed::Constant(128),
                    "MB Sync" => FanSpeed::Constant(128),
                    curve => FanSpeed::Curve(curve.to_string()),
                };
            });
            if changed {
                crate::refresh_aio_ui(&weak, &shared);
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_aio_set_fan_pwm(move |dev_id, slot, percent| {
            let dev_id = dev_id.to_string();
            let slot = (slot as usize).min(3);
            let percent = percent.clamp(0, 100) as u32;
            let byte = ((percent * 255) / 100).min(255) as u8;
            let changed = with_aio(&shared, &dev_id, |aio| {
                aio.fan_speeds[slot] = FanSpeed::Constant(byte);
            });
            if changed {
                mark_dirty(&weak);
            }
        });
    }

    // Sensor picker
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_aio_set_sensor(move |dev_id, which, new_index| {
            let dev_id = dev_id.to_string();
            let which = which.to_string();
            let picked: Option<lianli_shared::media::SensorSourceConfig> = {
                let state = shared.lock().unwrap();
                if new_index <= 0 {
                    None
                } else {
                    state
                        .available_sensors
                        .get((new_index - 1) as usize)
                        .map(|s| source_to_config(s.source.clone()))
                }
            };
            let changed = with_aio(&shared, &dev_id, |aio| match which.as_str() {
                "cpu_temp" => aio.cpu_temp_source = picked.clone(),
                "cpu_load" => aio.cpu_load_source = picked.clone(),
                "gpu_temp" => aio.gpu_temp_source = picked.clone(),
                "gpu_load" => aio.gpu_load_source = picked.clone(),
                _ => {}
            });
            if changed {
                crate::refresh_aio_ui(&weak, &shared);
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_aio_set_color(move |dev_id, which, r, g, b, a| {
            let dev_id = dev_id.to_string();
            let which = which.to_string();
            let rgba = [
                r.clamp(0, 255) as u8,
                g.clamp(0, 255) as u8,
                b.clamp(0, 255) as u8,
                a.clamp(0, 255) as u8,
            ];
            let changed = with_aio(&shared, &dev_id, |aio| match which.as_str() {
                "str" => aio.str_color = rgba,
                "val" => aio.val_color = rgba,
                "unit" => aio.unit_color = rgba,
                _ => {}
            });
            if changed {
                mark_dirty(&weak);
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_aio_set_brightness(move |dev_id, v| {
            let dev_id = dev_id.to_string();
            let v = v.clamp(0, 100) as u8;
            let changed = with_aio(&shared, &dev_id, |aio| {
                aio.brightness = v;
            });
            if changed {
                mark_dirty(&weak);
            }
        });
    }

    // Rotation
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_aio_set_rotation(move |dev_id, v| {
            let dev_id = dev_id.to_string();
            let v = v.clamp(0, 3) as u8;
            let changed = with_aio(&shared, &dev_id, |aio| {
                aio.rotation = v;
            });
            if changed {
                crate::refresh_aio_ui(&weak, &shared);
            }
        });
    }

    // Theme index
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_aio_set_theme_index(move |dev_id, v| {
            let dev_id = dev_id.to_string();
            let v = v.clamp(0, 12) as u8;
            let changed = with_aio(&shared, &dev_id, |aio| {
                aio.theme_index = v;
            });
            if changed {
                crate::refresh_aio_ui(&weak, &shared);
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_aio_set_loop_interval(move |dev_id, v| {
            let dev_id = dev_id.to_string();
            let v = v.clamp(1, 30) as u8;
            let changed = with_aio(&shared, &dev_id, |aio| {
                aio.loop_interval = v;
            });
            if changed {
                mark_dirty(&weak);
            }
        });
    }
}

pub(super) fn source_to_config(
    s: lianli_shared::sensors::SensorSource,
) -> lianli_shared::media::SensorSourceConfig {
    use lianli_shared::media::SensorSourceConfig;
    use lianli_shared::sensors::SensorSource;
    match s {
        SensorSource::Hwmon {
            name,
            label,
            device_path,
        } => SensorSourceConfig::Hwmon {
            name,
            label,
            device_path,
        },
        SensorSource::NvidiaGpu { gpu_index, metric } => {
            SensorSourceConfig::NvidiaGpu { gpu_index, metric }
        }
        SensorSource::AmdGpuUsage { card_index } => SensorSourceConfig::AmdGpuUsage { card_index },
        SensorSource::Command { cmd } => SensorSourceConfig::Command { cmd },
        SensorSource::WirelessCoolant { device_id } => {
            SensorSourceConfig::WirelessCoolant { device_id }
        }
        SensorSource::CpuUsage => SensorSourceConfig::CpuUsage,
        SensorSource::MemUsage => SensorSourceConfig::MemUsage,
        SensorSource::MemUsed => SensorSourceConfig::MemUsed,
        SensorSource::MemFree => SensorSourceConfig::MemFree,
        SensorSource::NetworkRate { iface, direction } => match direction {
            lianli_shared::sensors::NetDirection::Rx => SensorSourceConfig::NetworkRx { iface },
            lianli_shared::sensors::NetDirection::Tx => SensorSourceConfig::NetworkTx { iface },
        },
        SensorSource::DiskRate { device, direction } => match direction {
            lianli_shared::sensors::DiskDirection::Read => SensorSourceConfig::DiskRead { device },
            lianli_shared::sensors::DiskDirection::Write => {
                SensorSourceConfig::DiskWrite { device }
            }
        },
    }
}

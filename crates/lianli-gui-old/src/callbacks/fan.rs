use crate::backend;
use crate::conversions;
use crate::{CurvePoint, MainWindow, Shared};
use lianli_shared::fan::{FanConfig, FanCurve, FanGroup, FanSpeed};
use lianli_shared::sensors::Unit;
use slint::{ComponentHandle, Model};

pub(crate) fn wire_fan_callbacks(
    window: &MainWindow,
    _backend: &backend::BackendHandle,
    shared: &Shared,
) {
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_add_curve(move || {
            {
                let mut state = shared.lock().unwrap();
                let default_source = state.available_sensors.first().map(|s| s.source.clone());
                if let Some(ref mut c) = state.config {
                    let n = c.fan_curves.len() + 1;
                    c.fan_curves.push(FanCurve {
                        name: format!("curve-{n}"),
                        temp_source: default_source,
                        temp_command: String::new(),
                        curve: vec![(30.0, 30.0), (50.0, 50.0), (70.0, 80.0), (85.0, 100.0)],
                    });
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_remove_curve(move |idx| {
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let idx = idx as usize;
                    if idx < c.fan_curves.len() {
                        c.fan_curves.remove(idx);
                    }
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_rename_curve(move |idx, name| {
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    if let Some(curve) = c.fan_curves.get_mut(idx as usize) {
                        curve.name = name.to_string();
                    }
                }
            }
            // Don't rebuild model — would destroy the focused LineEdit.
            // The typed text is already visible. Mark dirty only.
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_set_temp_source(move |idx, display_name| {
            let display = display_name.to_string();
            {
                let mut state = shared.lock().unwrap();
                let source = if display.ends_with("Custom command") {
                    None
                } else {
                    let sensor_idx: usize = display
                        .split('.')
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    sensor_idx.checked_sub(1).and_then(|i| {
                        state
                            .available_sensors
                            .iter()
                            .filter(|s| s.unit == Unit::C)
                            .nth(i)
                            .map(|s| s.source.clone())
                    })
                };
                if let Some(ref mut c) = state.config {
                    if let Some(curve) = c.fan_curves.get_mut(idx as usize) {
                        curve.temp_source = source;
                        if curve.temp_source.is_some() {
                            curve.temp_command.clear();
                        }
                    }
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        window.on_fan_set_temp_command(move |idx, cmd| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                if let Some(curve) = c.fan_curves.get_mut(idx as usize) {
                    curve.temp_command = cmd.to_string();
                }
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_point_moved(move |cidx, pidx, temp, speed| {
            let temp = temp.round().clamp(20.0, 100.0);
            let speed = speed.round().clamp(0.0, 100.0);
            let cidx_u = cidx as usize;
            let pidx_u = pidx as usize;

            // Update shared state, get sorted points for path rebuild
            let sorted = {
                let mut state = shared.lock().unwrap();
                let c = match state.config.as_mut() {
                    Some(c) => c,
                    None => return,
                };
                let curve = match c.fan_curves.get_mut(cidx_u) {
                    Some(curve) => curve,
                    None => return,
                };
                if let Some(pt) = curve.curve.get_mut(pidx_u) {
                    pt.0 = temp;
                    pt.1 = speed;
                }
                let mut sorted = curve.curve.clone();
                sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                sorted
            };

            // Synchronous in-place model update (we're on the UI thread).
            // This preserves the TouchArea so the drag continues.
            if let Some(w) = weak.upgrade() {
                let model = w.get_fan_curves();
                if let Some(mut curve_data) = model.row_data(cidx_u) {
                    // Update inner points model in-place
                    curve_data
                        .points
                        .set_row_data(pidx_u, CurvePoint { temp, speed });
                    // Update segment models
                    curve_data.curve_segments = slint::ModelRc::new(slint::VecModel::from(
                        conversions::build_curve_segments(&sorted),
                    ));
                    curve_data.clamp_segments = slint::ModelRc::new(slint::VecModel::from(
                        conversions::build_clamp_segments(&sorted),
                    ));
                    model.set_row_data(cidx_u, curve_data);
                    w.set_config_dirty(true);
                }
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_point_added(move |cidx, temp, speed| {
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    if let Some(curve) = c.fan_curves.get_mut(cidx as usize) {
                        curve.curve.push((
                            temp.round().clamp(20.0, 100.0),
                            speed.round().clamp(0.0, 100.0),
                        ));
                    }
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_point_removed(move |cidx, pidx| {
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    if let Some(curve) = c.fan_curves.get_mut(cidx as usize) {
                        let pidx = pidx as usize;
                        if pidx < curve.curve.len() {
                            curve.curve.remove(pidx);
                        }
                    }
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }

    // Fan speed assignment
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_set_slot_speed(move |dev_id, slot, val| {
            let dev_id = dev_id.to_string();
            let slot = slot as usize;
            let val = val.to_string();
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let fc = c.fans.get_or_insert_with(FanConfig::default);
                    let group = fc
                        .speeds
                        .iter_mut()
                        .find(|g| g.device_id.as_deref() == Some(&dev_id));
                    let group = if let Some(g) = group {
                        g
                    } else {
                        fc.speeds.push(FanGroup {
                            device_id: Some(dev_id.clone()),
                            speeds: [
                                FanSpeed::Constant(0),
                                FanSpeed::Constant(0),
                                FanSpeed::Constant(0),
                                FanSpeed::Constant(0),
                            ],
                        });
                        fc.speeds.last_mut().unwrap()
                    };

                    let speed: FanSpeed = match val.as_str() {
                        "Off" => FanSpeed::Constant(0),
                        "Constant PWM" => FanSpeed::Constant(128),
                        "MB Sync" => FanSpeed::Curve("__mb_sync__".to_string()),
                        curve_name => FanSpeed::Curve(curve_name.to_string()),
                    };
                    if slot < 4 {
                        group.speeds[slot] = speed;
                    }
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_set_slot_pwm(move |dev_id, slot, percent| {
            let dev_id = dev_id.to_string();
            let slot = slot as usize;
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let fc = c.fans.get_or_insert_with(FanConfig::default);
                    if let Some(group) = fc
                        .speeds
                        .iter_mut()
                        .find(|g| g.device_id.as_deref() == Some(&dev_id))
                    {
                        if slot < 4 {
                            group.speeds[slot] = FanSpeed::Constant(
                                ((percent as f32 / 100.0) * 255.0).round() as u8,
                            );
                        }
                    }
                }
            }
            // In-place update to avoid destroying the Slider during drag
            if let Some(w) = weak.upgrade() {
                let model = w.get_fan_groups();
                for i in 0..model.row_count() {
                    if let Some(mut group_data) = model.row_data(i) {
                        if group_data.device_id.as_str() == dev_id {
                            if let Some(mut slot_data) = group_data.slots.row_data(slot) {
                                slot_data.pwm_percent = percent;
                                group_data.slots.set_row_data(slot, slot_data);
                            }
                            if slot == 3 && group_data.has_pump_control {
                                group_data.pump_slot.pwm_percent = percent;
                                model.set_row_data(i, group_data);
                            }
                            break;
                        }
                    }
                }
                w.set_config_dirty(true);
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_set_pwm_header(move |dev_id, slot, label| {
            let dev_id = dev_id.to_string();
            let slot = slot as usize;
            let label = label.to_string();
            let pwm_headers = lianli_shared::sensors::enumerate_pwm_headers();
            // Match label prefix (before the " (XX%)" suffix)
            let header_id = pwm_headers
                .iter()
                .find(|h| label.starts_with(&h.label))
                .map(|h| h.id.clone())
                .unwrap_or_default();
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let fc = c.fans.get_or_insert_with(FanConfig::default);
                    if let Some(group) = fc
                        .speeds
                        .iter_mut()
                        .find(|g| g.device_id.as_deref() == Some(&dev_id))
                    {
                        if slot < 4 {
                            group.speeds[slot] = FanSpeed::Curve(format!(
                                "{}{}",
                                lianli_shared::fan::MB_SYNC_PREFIX,
                                header_id
                            ));
                        }
                    }
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }
}

fn refresh_fan_ui(weak: &slint::Weak<MainWindow>, shared: &Shared) {
    let (curves, fans, devices, sensors) = {
        let state = shared.lock().unwrap();
        let config = match state.config.as_ref() {
            Some(c) => c,
            None => return,
        };
        (
            config.fan_curves.clone(),
            config.fans.clone(),
            state.devices.clone(),
            state.available_sensors.clone(),
        )
    };

    let weak = weak.clone();
    slint::invoke_from_event_loop(move || {
        if let Some(w) = weak.upgrade() {
            w.set_fan_curves(conversions::fan_curves_to_model(&curves, &sensors));
            w.set_curve_names(conversions::curve_names_to_model(&curves));
            w.set_fan_speed_options(conversions::speed_options_model(&curves, true));
            w.set_config_dirty(true);
            let fc = fans.unwrap_or_default();
            let pwm_headers = lianli_shared::sensors::enumerate_pwm_headers();
            w.set_fan_groups(conversions::fan_groups_to_model(
                &fc,
                &devices,
                &pwm_headers,
            ));
        }
    })
    .ok();
}

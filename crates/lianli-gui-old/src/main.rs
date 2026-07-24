mod backend;
mod callbacks;
mod conversions;
mod editor;
mod ipc_client;
mod state;
mod template_browser;

use lianli_shared::fan::FanConfig;
use std::sync::{Arc, Mutex};

slint::include_modules!();

/// Shared mutable state: config + cached capabilities + devices.
/// Backend thread updates it on load; callbacks mutate config; save sends it.
pub type Shared = Arc<Mutex<state::SharedState>>;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("lianli_gui2=info".parse().unwrap()),
        )
        .init();

    let window = MainWindow::new().expect("Failed to create main window");
    if let Err(e) = slint::set_xdg_app_id("com.sgtaziz.lianlilinux") {
        tracing::warn!("set_xdg_app_id failed: {e}");
    }
    window.set_app_version(env!("CARGO_PKG_VERSION").into());

    // Shared state — backend will populate on first load
    let shared: Shared = Arc::new(Mutex::new(state::SharedState::default()));
    let backend = backend::start(window.as_weak(), shared.clone());

    // ── Refresh devices ──
    {
        let tx = backend.tx.clone();
        window.on_refresh_devices(move || {
            let _ = tx.send(backend::BackendCommand::RefreshDevices);
        });
    }

    // ── Switch display mode ──
    {
        let tx = backend.tx.clone();
        let shared_inner = shared.clone();
        window.on_switch_display_mode(move |device_id| {
            shared_inner
                .lock()
                .unwrap()
                .set_pending(device_id.to_string(), state::PendingAction::SwitchDisplay);
            let _ = tx.send(backend::BackendCommand::IpcRequest(
                lianli_shared::ipc::IpcRequest::SwitchDisplayMode {
                    device_id: device_id.to_string(),
                },
            ));
            let _ = tx.send(backend::BackendCommand::RefreshDevices);
        });
    }

    // ── Bind wireless device ──
    {
        let tx = backend.tx.clone();
        let shared_inner = shared.clone();
        window.on_bind_wireless_device(move |device_id| {
            shared_inner
                .lock()
                .unwrap()
                .set_pending(device_id.to_string(), state::PendingAction::Bind);
            let mac = device_id
                .to_string()
                .strip_prefix("wireless-unbound:")
                .unwrap_or(&device_id)
                .to_string();
            let _ = tx.send(backend::BackendCommand::IpcRequest(
                lianli_shared::ipc::IpcRequest::BindWirelessDevice { mac },
            ));
            let _ = tx.send(backend::BackendCommand::RefreshDevices);
        });
    }

    // ── Unbind wireless device ──
    {
        let tx = backend.tx.clone();
        let shared_inner = shared.clone();
        window.on_unbind_wireless_device(move |device_id| {
            shared_inner
                .lock()
                .unwrap()
                .set_pending(device_id.to_string(), state::PendingAction::Unbind);
            let mac = device_id
                .to_string()
                .strip_prefix("wireless:")
                .unwrap_or(&device_id)
                .to_string();
            let _ = tx.send(backend::BackendCommand::IpcRequest(
                lianli_shared::ipc::IpcRequest::UnbindWirelessDevice { mac },
            ));
            let _ = tx.send(backend::BackendCommand::RefreshDevices);
        });
    }

    // ── Set ENE 6K77 fan quantity (debounced) ──
    {
        use std::cell::RefCell;
        use std::collections::HashMap;
        use std::rc::Rc;

        let tx = backend.tx.clone();
        let shared_inner = shared.clone();
        let pending: Rc<RefCell<HashMap<String, i32>>> = Rc::new(RefCell::new(HashMap::new()));
        let timer = Rc::new(slint::Timer::default());

        window.on_set_fan_quantity(move |device_id, qty| {
            shared_inner
                .lock()
                .unwrap()
                .set_pending(device_id.to_string(), state::PendingAction::SetFanQuantity);
            pending.borrow_mut().insert(device_id.to_string(), qty);

            let pending_for_timer = pending.clone();
            let tx_for_timer = tx.clone();
            timer.start(
                slint::TimerMode::SingleShot,
                std::time::Duration::from_millis(400),
                move || {
                    let drained: Vec<_> = pending_for_timer.borrow_mut().drain().collect();
                    for (device_id, qty) in drained {
                        let _ = tx_for_timer.send(backend::BackendCommand::IpcRequest(
                            lianli_shared::ipc::IpcRequest::SetEne6k77FanQuantity {
                                device_id,
                                quantity: qty.clamp(0, 255) as u8,
                            },
                        ));
                    }
                    let _ = tx_for_timer.send(backend::BackendCommand::RefreshDevices);
                },
            );
        });
    }

    // ── Save config ──
    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        window.on_save_config(move || {
            let state = shared.lock().unwrap();
            if let Some(ref c) = state.config {
                let _ = tx.send(backend::BackendCommand::SaveConfig(c.clone()));
            }
        });
    }

    // ── Toggle OpenRGB ──
    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        window.on_toggle_openrgb(move |enabled| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                let rgb = c.rgb.get_or_insert_with(Default::default);
                rgb.openrgb_server = enabled;
                let _ = tx.send(backend::BackendCommand::SaveConfig(c.clone()));
            }
        });
    }

    // ── Set default FPS ──
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_set_default_fps(move |fps| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                c.default_fps = fps as f32;
            }
            drop(state);
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    // ── Set OpenRGB port ──
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_set_openrgb_port(move |port| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                let rgb = c.rgb.get_or_insert_with(Default::default);
                rgb.openrgb_port = port as u16;
            }
            drop(state);
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    // ── Set HID driver ──
    // (hidapi backend was dropped; only rusb is supported now. The Slint UI
    // still has the dropdown but it's a no-op until Tauri migration.)
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_set_hid_driver(move |_driver| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                c.hid_driver = None;
            }
            drop(state);
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    // ── Set fan update interval ──
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_set_update_interval(move |ms| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                let fc = c.fans.get_or_insert_with(FanConfig::default);
                fc.update_interval_ms = ms as u64;
            }
            drop(state);
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    // ── Set fan hysteresis (temp) ──
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_set_hysteresis_temp(move |v| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                let fc = c.fans.get_or_insert_with(FanConfig::default);
                fc.hysteresis_temp = v.max(0.0);
            }
            drop(state);
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    // ── Set fan hysteresis (PWM) ──
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_set_hysteresis_pwm(move |v| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                let fc = c.fans.get_or_insert_with(FanConfig::default);
                fc.hysteresis_pwm = v.clamp(0, 255) as u8;
            }
            drop(state);
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    // ── RGB callbacks ──
    callbacks::wire_rgb_callbacks(&window, &backend, &shared);

    // ── Fan callbacks ──
    callbacks::wire_fan_callbacks(&window, &backend, &shared);

    // ── AIO callbacks ──
    callbacks::wire_aio_callbacks(&window, &backend, &shared);

    let editor_handle = editor::install(&window, shared.clone());
    let browser_handle = template_browser::install(&window, shared.clone());

    callbacks::wire_lcd_callbacks(&window, &shared, &editor_handle, &browser_handle);

    window.run().expect("Failed to run Slint event loop");
    backend.send(backend::BackendCommand::Shutdown);
}

pub(crate) fn generate_template_id(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{:x}", nanos)
}

pub(crate) fn make_blank_template(
    existing: &[lianli_shared::template::LcdTemplate],
) -> lianli_shared::template::LcdTemplate {
    lianli_shared::template::LcdTemplate {
        id: generate_template_id("user"),
        name: unique_template_name("New Template", existing),
        base_width: 480,
        base_height: 480,
        background: lianli_shared::template::TemplateBackground::Color {
            rgb: [0, 0, 0, 255],
        },
        widgets: Vec::new(),
        rotated: false,
        target_device: None,
    }
}

pub(crate) fn unique_template_name(
    base: &str,
    existing: &[lianli_shared::template::LcdTemplate],
) -> String {
    let in_use = |candidate: &str| existing.iter().any(|t| t.name == candidate);
    if !in_use(base) {
        return base.to_string();
    }
    for n in 2..u32::MAX {
        let candidate = format!("{base} {n}");
        if !in_use(&candidate) {
            return candidate;
        }
    }
    base.to_string()
}

/// Generates a non-conflicting template name. If `base` is already a "(Copy N)"
/// form we strip the suffix before bumping, so duplicating "Foo (Copy 2)" yields
/// "Foo (Copy 3)" rather than "Foo (Copy 2) (Copy)".
pub(crate) fn next_unique_name(
    base: &str,
    existing: &[lianli_shared::template::LcdTemplate],
) -> String {
    let stem = callbacks::strip_copy_suffix(base);
    let names: std::collections::HashSet<&str> = existing.iter().map(|t| t.name.as_str()).collect();
    if !names.contains(stem) && stem != base {
        return stem.to_string();
    }
    let first = format!("{stem} (Copy)");
    if !names.contains(first.as_str()) {
        return first;
    }
    for i in 2..1000 {
        let candidate = format!("{stem} (Copy {i})");
        if !names.contains(candidate.as_str()) {
            return candidate;
        }
    }
    format!("{stem} (Copy {})", generate_template_id(""))
}

pub(crate) fn next_unique_downloaded_name(
    base: &str,
    existing: &[lianli_shared::template::LcdTemplate],
) -> String {
    let names: std::collections::HashSet<&str> = existing.iter().map(|t| t.name.as_str()).collect();
    let first = format!("{base} (Downloaded)");
    if !names.contains(first.as_str()) {
        return first;
    }
    for i in 2..1000 {
        let candidate = format!("{base} (Downloaded {i})");
        if !names.contains(candidate.as_str()) {
            return candidate;
        }
    }
    format!("{base} (Downloaded {})", generate_template_id(""))
}

pub(crate) fn user_templates_only(
    all: &[lianli_shared::template::LcdTemplate],
) -> Vec<lianli_shared::template::LcdTemplate> {
    all.to_vec()
}

pub(crate) fn send_set_templates(templates: Vec<lianli_shared::template::LcdTemplate>) {
    match ipc_client::send_request(&lianli_shared::ipc::IpcRequest::SetLcdTemplates { templates }) {
        Ok(lianli_shared::ipc::IpcResponse::Error { message }) => {
            tracing::warn!("SetLcdTemplates failed: {message}");
        }
        Err(e) => tracing::warn!("SetLcdTemplates IPC error: {e}"),
        _ => {}
    }
}

// ── Refresh helpers ──
// These read from SharedState (lock briefly), then push models to UI via invoke_from_event_loop.

pub(crate) fn refresh_aio_ui(weak: &slint::Weak<MainWindow>, shared: &Shared) {
    let (config, devices, sensors) = {
        let state = shared.lock().unwrap();
        let Some(cfg) = state.config.clone() else {
            return;
        };
        (cfg, state.devices.clone(), state.available_sensors.clone())
    };
    let weak = weak.clone();
    slint::invoke_from_event_loop(move || {
        if let Some(w) = weak.upgrade() {
            let pwm_headers = lianli_shared::sensors::enumerate_pwm_headers();
            let telemetry = lianli_shared::ipc::TelemetrySnapshot::default();
            w.set_aios(conversions::aios_to_model(
                &config,
                &devices,
                &telemetry,
                &sensors,
                &pwm_headers,
            ));
            w.set_aio_sensor_options(conversions::aio_sensor_options_model(&sensors));
            w.set_aio_speed_options(conversions::speed_options_model(&config.fan_curves, false));
            w.set_aio_theme_options(conversions::aio_theme_options_model());
            w.set_aio_rotation_options(conversions::aio_rotation_options_model());
            w.set_config_dirty(true);
        }
    })
    .ok();
}

pub(crate) fn refresh_lcd_ui(weak: &slint::Weak<MainWindow>, shared: &Shared) {
    let (lcds, devices, sensors, templates) = {
        let state = shared.lock().unwrap();
        match state.config.as_ref() {
            Some(c) => (
                c.lcds.clone(),
                state.devices.clone(),
                state.available_sensors.clone(),
                state.lcd_templates.clone(),
            ),
            None => return,
        }
    };

    let weak = weak.clone();
    slint::invoke_from_event_loop(move || {
        if let Some(w) = weak.upgrade() {
            w.set_lcd_entries(conversions::lcd_entries_to_model(
                &lcds, &devices, &sensors, &templates,
            ));
            w.set_lcd_template_labels(conversions::template_labels_model(&templates));
            w.set_config_dirty(true);
        }
    })
    .ok();
}

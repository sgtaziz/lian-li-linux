use super::parse_mac_str;
use super::{DaemonEvent, ServiceManager};
use crate::controllers::aio::AioController;
use crate::controllers::fan::FanController;
use crate::controllers::rgb::RgbController;
use crate::openrgb_server;
use crate::persistence;
use crate::template_store;
use lianli_devices::crypto::PacketBuilder;
use lianli_devices::detect::enumerate_devices;
use lianli_devices::registry;
use lianli_devices::traits::FanDevice;
use lianli_shared::config::AppConfig;
use lianli_shared::device_id::DeviceFamily;
use lianli_shared::ipc::DeviceInfo;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use tracing::{debug, info, warn};

impl ServiceManager {
    pub(super) fn start_fan_control(&mut self) {
        if let Some(controller) = self.controllers.fan.take() {
            info!("Stopping existing fan controller for reload...");
            controller.stop();
        }
        let Some(cfg) = &self.config else {
            return;
        };
        let fan_config = cfg.fans.clone().unwrap_or_default();
        let fan_curves = cfg.fan_curves.clone();

        // Reuse the already-opened wired fan device handles (populated at startup).
        let wired_devices = Arc::clone(&self.registry.fan_devices);

        let wireless = if self.wireless.has_discovered_devices() {
            Some(Arc::new(self.wireless.clone()))
        } else {
            None
        };

        info!(
            "Starting fan control: {} curve(s), {} group(s), wireless={}, wired={}",
            fan_curves.len(),
            fan_config.speeds.len(),
            wireless.is_some(),
            wired_devices.len()
        );

        let mut controller = FanController::new(
            fan_config,
            fan_curves,
            wireless,
            wired_devices,
            self.tx.clone(),
            cfg.rgb_drift_detection_enabled,
            std::time::Duration::from_millis(cfg.rgb_drift_detection_interval_ms.max(100)),
        );
        controller.start();
        self.controllers.fan = Some(controller);
    }

    pub(super) fn start_aio_control(&mut self) {
        if let Some(existing) = self.controllers.aio.take() {
            existing.stop();
        }
        let Some(cfg) = self.config.clone() else {
            return;
        };
        let wireless = Arc::new(self.wireless.clone());
        let mut controller = AioController::new(wireless, cfg);
        controller.start();
        self.controllers.aio = Some(controller);
    }

    /// For each discovered AIO, ensure an AioConfig exists in the user's config.
    /// Migrates any legacy FanGroup targeting that device, then inserts defaults.
    pub(super) fn ensure_aio_defaults(&mut self) {
        let Some(cfg) = self.config.as_mut() else {
            return;
        };
        let aio_device_ids: Vec<String> = self
            .wireless
            .devices()
            .iter()
            .filter(|d| d.is_aio())
            .map(|d| format!("wireless:{}", d.mac_str()))
            .collect();
        if aio_device_ids.is_empty() {
            return;
        }

        let mut changed = false;
        for device_id in aio_device_ids {
            if cfg.migrate_aio_fangroup(&device_id) {
                info!("Migrated legacy fan group for AIO {device_id} into aio config");
                changed = true;
            }
            if !cfg.aio.contains_key(&device_id) {
                cfg.aio.insert(
                    device_id.clone(),
                    lianli_shared::aio::AioConfig::defaults_for_host(),
                );
                info!("Created default AIO config for {device_id}");
                changed = true;
            }
        }

        if changed {
            let snapshot = cfg.clone();
            if let Err(e) = persistence::write_config(&self.config_path, &snapshot) {
                warn!("Failed to persist AIO config additions: {e}");
            } else {
                self.ipc.state.lock().config = Some(snapshot);
            }
        }
    }

    pub(super) fn enumerate_wired_controller_ids(&self) -> std::collections::HashSet<String> {
        use lianli_shared::device_id::DeviceFamily;
        fn is_wired_controller(family: DeviceFamily) -> bool {
            lianli_shared::device_id::uses_hid(family)
                || matches!(family, DeviceFamily::UniversalScreenLighting)
        }
        enumerate_devices()
            .ok()
            .into_iter()
            .flatten()
            .filter(|det| is_wired_controller(det.family))
            .map(|det| Self::rusb_device_id(&det))
            .collect()
    }

    pub(super) fn check_wired_hotplug(&mut self) {
        let current = self.enumerate_wired_controller_ids();
        if current == self.registry.last_wired_ids {
            return;
        }

        let added = current.difference(&self.registry.last_wired_ids).count();
        let removed = self.registry.last_wired_ids.difference(&current).count();
        info!("Wired device topology changed (+{added} -{removed}): re-initializing");

        self.registry
            .hid_backends
            .retain(|k, _| current.contains(k));
        self.init_wired_devices();
        self.start_fan_control();
        self.registry.last_wired_ids = current;
    }

    /// Initialize all wired USB devices (fan + RGB + LCD + AIO) via the
    /// [`registry`] dispatch table. Each device is opened on its own thread
    /// with a timeout so that one unresponsive controller cannot block the
    /// rest of the daemon. Devices that time out are skipped and will be
    /// retried by the hotplug poller.
    pub(super) fn init_wired_devices(&mut self) {
        let mut fan_devices: HashMap<String, Box<dyn FanDevice>> = HashMap::new();
        let mut wired_rgb: HashMap<String, std::sync::Arc<dyn lianli_devices::traits::RgbDevice>> =
            HashMap::new();
        self.registry.fan_device_info.clear();

        let usb_devs = match enumerate_devices() {
            Ok(devs) => devs,
            Err(err) => {
                warn!("Failed to enumerate USB devices: {err}");
                self.registry.fan_devices = Arc::new(fan_devices);
                self.init_rgb_controller_from(wired_rgb);
                return;
            }
        };

        const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

        // Spawn each device open on its own thread so a single hung
        // controller can't stall the rest of initialization.
        let mut pending: Vec<(
            String,
            &str,
            DeviceFamily,
            u16,
            u16,
            Option<String>,
            std::sync::mpsc::Receiver<anyhow::Result<registry::OpenedDevice>>,
        )> = Vec::new();

        for det in &usb_devs {
            if det.family == lianli_shared::device_id::DeviceFamily::TlLcd
                || det.family == lianli_shared::device_id::DeviceFamily::HydroShift2OledCurveLcd
            {
                continue;
            }
            let Some(driver) = registry::driver_for_family(det.family) else {
                continue;
            };
            let ctx = registry::OpenContext {
                device: det.device.clone(),
                family: det.family,
                vid: det.vid,
                pid: det.pid,
                bus: det.bus,
                address: det.address,
                serial: det.serial.clone(),
                hid_usage_page: det.hid_usage_page,
            };
            let base_id = Self::rusb_device_id(det);
            let name = det.name;
            let family = det.family;
            let vid = det.vid;
            let pid = det.pid;
            let serial = det.serial.clone();

            let (tx, rx) =
                std::sync::mpsc::sync_channel::<anyhow::Result<registry::OpenedDevice>>(1);
            let label = format!("{name} ({vid:04x}:{pid:04x})");
            std::thread::Builder::new()
                .name(format!("dev-open-{base_id}"))
                .spawn(move || {
                    let _ = tx.send(driver.open(&ctx));
                })
                .ok();

            pending.push((base_id, name, family, vid, pid, serial, rx));
            debug!("Spawned open thread for {label}");
        }

        // Collect results using a single global deadline so that N hung
        // devices waste at most OPEN_TIMEOUT total, not N × OPEN_TIMEOUT.
        let deadline = std::time::Instant::now() + OPEN_TIMEOUT;
        for (base_id, name, family, vid, pid, serial, rx) in pending {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                warn!("Skipped {name} ({vid:04x}:{pid:04x}) — global open deadline exceeded");
                continue;
            }
            match rx.recv_timeout(remaining) {
                Ok(Ok(mut opened)) => {
                    let shared_hid = opened.shared_hid.take();
                    if let Some(backend) = shared_hid {
                        self.registry.hid_backends.insert(base_id.clone(), backend);
                    }
                    self.register_opened_device(
                        base_id,
                        name,
                        family,
                        vid,
                        pid,
                        serial.as_deref(),
                        opened,
                        &mut fan_devices,
                        &mut wired_rgb,
                    );
                }
                Ok(Err(e)) => warn!("Failed to open {name} ({vid:04x}:{pid:04x}): {e}"),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => warn!(
                    "Timeout opening {name} ({vid:04x}:{pid:04x}) — skipping; will retry on hotplug"
                ),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    warn!("Open thread for {name} ({vid:04x}:{pid:04x}) panicked — skipping")
                }
            }
        }

        let arc = Arc::new(fan_devices);
        self.registry.fan_devices = Arc::clone(&arc);
        self.init_rgb_controller_from(wired_rgb);
        self.registry.last_wired_ids = self.enumerate_wired_controller_ids();
    }

    /// Dispatch an [`registry::OpenedDevice`] into the fan / RGB / AIO
    /// subsystems based on which slots are populated.
    fn register_opened_device(
        &mut self,
        base_id: String,
        name: &str,
        family: DeviceFamily,
        vid: u16,
        pid: u16,
        serial: Option<&str>,
        opened: registry::OpenedDevice,
        fan_devices: &mut HashMap<String, Box<dyn FanDevice>>,
        wired_rgb: &mut HashMap<String, std::sync::Arc<dyn lianli_devices::traits::RgbDevice>>,
    ) {
        // Register fan controller.
        if let Some(fan_ctrl) = opened.fan {
            info!("Opened {name} as fan device: {base_id}");
            let supports_quantity = fan_ctrl.supports_fan_quantity();
            let max_quantity = supports_quantity.then(|| fan_ctrl.max_fan_quantity_per_port());

            if supports_quantity {
                if let Some(serial_str) = serial {
                    if let Some(cfg) = self.config.as_ref() {
                        if let Some(dev_cfg) = cfg.ene6k77.get(serial_str) {
                            for (&port, &qty) in &dev_cfg.fan_quantities {
                                if let Err(e) = fan_ctrl.set_port_fan_quantity(port, qty) {
                                    warn!(
                                        "Failed to apply persisted fan quantity for {base_id} port {port}: {e}"
                                    );
                                }
                            }
                        }
                    }
                }
            }

            let ports = fan_ctrl.fan_port_info();
            let per_fan = fan_ctrl.per_fan_control();
            let mb_sync = fan_ctrl.supports_mb_sync();
            let pump_control = fan_ctrl.has_pump_control();
            for &(port, fan_count) in &ports {
                let device_id = if ports.len() > 1 {
                    format!("{base_id}:port{port}")
                } else {
                    base_id.clone()
                };
                let dev_name = if ports.len() > 1 {
                    format!("{name} Port {port}")
                } else {
                    name.to_string()
                };
                self.registry.fan_device_info.push(DeviceInfo {
                    device_id,
                    family,
                    name: dev_name,
                    serial: serial.map(|s| s.to_string()),
                    vid,
                    pid,
                    has_lcd: false,
                    has_fan: true,
                    has_pump: pump_control,
                    has_rgb: family.has_rgb(),
                    has_pump_control: pump_control,
                    fan_count: Some(fan_count),
                    per_fan_control: Some(per_fan),
                    mb_sync_support: mb_sync,
                    rgb_zone_count: None,
                    screen_width: None,
                    screen_height: None,
                    is_unbound_wireless: false,
                    pump_rpm_range: None,
                    fan_quantity: supports_quantity.then_some(fan_count),
                    max_fan_quantity: max_quantity,
                    firmware_version: opened.firmware.clone(),
                    supports_c_command: false,
                    port_index: None,
                    wireless_group_mac: None,
                });
            }
            fan_devices.insert(base_id.clone(), fan_ctrl);
        }

        // Register RGB devices (one per zone).
        for (suffix, rgb) in opened.rgb {
            let device_id = if suffix.is_empty() {
                base_id.clone()
            } else {
                format!("{base_id}:{suffix}")
            };
            wired_rgb.insert(device_id, rgb);
        }
    }

    pub(super) fn handle_set_ene6k77_fan_quantity(&mut self, device_id: &str, quantity: u8) {
        let (base_id, port) = match device_id.rsplit_once(":port") {
            Some((base, port_str)) => match port_str.parse::<u8>() {
                Ok(p) => (base.to_string(), p),
                Err(_) => {
                    warn!("Invalid port suffix in device_id: {device_id}");
                    return;
                }
            },
            None => (device_id.to_string(), 0),
        };

        let serial = self
            .registry
            .fan_device_info
            .iter()
            .find(|d| d.device_id == device_id)
            .and_then(|d| d.serial.clone());

        let Some(ctrl) = self.registry.fan_devices.get(&base_id) else {
            warn!("Fan device not found for quantity update: {base_id}");
            return;
        };
        if let Err(e) = ctrl.set_port_fan_quantity(port, quantity) {
            warn!("Failed to set fan quantity for {device_id}: {e}");
            return;
        }

        if let Some(serial) = serial {
            if let Some(cfg) = self.config.as_mut() {
                cfg.ene6k77
                    .entry(serial)
                    .or_default()
                    .fan_quantities
                    .insert(port, quantity);
                let snapshot = cfg.clone();
                if let Err(e) = persistence::write_config(&self.config_path, &snapshot) {
                    warn!("Failed to persist ENE 6K77 fan quantity: {e}");
                } else {
                    self.ipc.state.lock().config = Some(snapshot);
                }
            }
        }

        for info in self.registry.fan_device_info.iter_mut() {
            if info.device_id == device_id {
                info.fan_count = Some(quantity);
                info.fan_quantity = Some(quantity);
                break;
            }
        }

        info!("Set ENE 6K77 fan quantity: {device_id} → {quantity}");
        self.device_poll();
    }

    /// Create the RgbController from pre-opened wired RGB devices + wireless.
    fn init_rgb_controller_from(
        &mut self,
        wired_rgb: HashMap<String, std::sync::Arc<dyn lianli_devices::traits::RgbDevice>>,
    ) {
        let wireless = if self.wireless.has_discovered_devices() {
            Some(Arc::new(self.wireless.clone()))
        } else {
            None
        };

        let mut controller = RgbController::new(wired_rgb, wireless);

        // Start thermal alert monitor and share override state with RGB controller
        let thermal_settings = self
            .config
            .as_ref()
            .map(|c| c.thermal_alert.clone())
            .unwrap_or_default();
        let mut monitor = crate::thermal_alert::ThermalAlertMonitor::new(thermal_settings);
        controller.set_thermal_override(monitor.shared_override());
        monitor.start();
        self.controllers.thermal_alert = Some(monitor);

        if let Some(ref cfg) = self.config {
            if let Some(ref rgb_cfg) = cfg.rgb {
                let presets = self.ipc.state.lock().rgb_presets.clone();
                controller.apply_config(rgb_cfg, &presets);
            }
        }

        let rgb_arc = Arc::new(Mutex::new(controller));
        self.controllers.rgb = Some(Arc::clone(&rgb_arc));
        self.ipc.state.lock().rgb_controller = Some(rgb_arc);
    }

    /// Rebuild RGB controller to pick up newly discovered wireless devices.
    pub(super) fn rebuild_rgb_controller(&mut self) {
        let wireless = if self.wireless.has_discovered_devices() {
            Some(Arc::new(self.wireless.clone()))
        } else {
            None
        };
        if let Some(ref rgb) = self.controllers.rgb {
            let mut ctrl = rgb.lock();
            ctrl.set_wireless(wireless);
            ctrl.refresh_wireless_devices();
            if let Some(ref cfg) = self.config {
                if let Some(ref rgb_cfg) = cfg.rgb {
                    let presets = self.ipc.state.lock().rgb_presets.clone();
                    ctrl.apply_config(rgb_cfg, &presets);
                }
            }
        }
    }

    /// Restart the fan controller to pick up newly discovered wireless devices.
    pub(super) fn restart_fan_control(&mut self) {
        self.start_fan_control();
    }

    /// Apply RGB config from the current AppConfig to the RGB controller.
    pub(super) fn apply_rgb_config(&self) {
        if let (Some(ref rgb), Some(ref cfg)) = (&self.controllers.rgb, &self.config) {
            if let Some(ref rgb_cfg) = cfg.rgb {
                let presets = self.ipc.state.lock().rgb_presets.clone();
                rgb.lock().apply_config(rgb_cfg, &presets);
            }
        }
    }

    /// Start or restart the OpenRGB SDK server based on config.
    pub(super) fn start_openrgb_server(&mut self) {
        let (enabled, port) = self
            .config
            .as_ref()
            .and_then(|c| c.rgb.as_ref())
            .map(|rgb| (rgb.openrgb_server, rgb.openrgb_port))
            .unwrap_or((false, 6743));

        // Check if we need to restart (port changed or toggled)
        let current_state = self.openrgb.state.lock().clone();
        let needs_restart =
            self.openrgb.thread.is_some() && (current_state.port != Some(port) || !enabled);

        if needs_restart {
            info!("Stopping OpenRGB server for reconfiguration");
            self.openrgb.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.openrgb.thread.take() {
                let _ = thread.join();
            }
            if let Some(thread) = self.controllers.direct_color_writer.take() {
                let _ = thread.join();
            }
            let mut s = self.openrgb.state.lock();
            *s = openrgb_server::OpenRgbServerState::default();
        }

        if !enabled {
            return;
        }

        if self.openrgb.thread.is_some() {
            return; // Already running with correct port
        }

        if let Some(ref rgb) = self.controllers.rgb {
            self.openrgb.stop.store(false, Ordering::Relaxed);
            self.openrgb.thread = Some(openrgb_server::start_openrgb_server(
                Arc::clone(rgb),
                Arc::clone(&self.controllers.direct_color_buffer),
                port,
                Arc::clone(&self.openrgb.stop),
                Arc::clone(&self.openrgb.state),
            ));
            // Start the async writer thread that flushes buffered colors at 30fps
            if self.controllers.direct_color_writer.is_none() {
                self.controllers.direct_color_writer =
                    Some(crate::controllers::rgb::start_direct_color_writer(
                        Arc::clone(rgb),
                        Arc::clone(&self.controllers.direct_color_buffer),
                        Arc::clone(&self.openrgb.stop),
                    ));
            }
        }
    }

    fn auto_rebind_configured_wireless(&mut self) {
        let Some(cfg) = self.config.as_ref() else {
            return;
        };

        let mut configured_ids = std::collections::HashSet::new();

        if let Some(fans) = &cfg.fans {
            for group in &fans.speeds {
                if let Some(device_id) = &group.device_id {
                    configured_ids.insert(device_id.clone());
                }
            }
        }

        if let Some(rgb) = &cfg.rgb {
            for device in &rgb.devices {
                configured_ids.insert(device.device_id.clone());
            }
        }

        configured_ids.extend(cfg.aio.keys().cloned());

        for dev in self.wireless.unbound_devices() {
            let device_id = format!("wireless:{}", dev.mac_str());
            if !configured_ids.contains(&device_id) {
                continue;
            }

            let Some(mac_str) = device_id.strip_prefix("wireless:") else {
                continue;
            };
            let Some(mac) = parse_mac_str(mac_str) else {
                warn!("Invalid configured wireless MAC: {mac_str}");
                continue;
            };

            info!("Auto-rebinding configured wireless device {device_id}");
            if let Err(err) = self.wireless.bind_device(&mac) {
                warn!("Auto-rebind failed for {device_id}: {err}");
            }
        }
    }

    pub(super) fn try_wireless(&mut self) {
        if !lianli_devices::wireless::tx_dongle_present() {
            debug!("[wireless] no TX/RX devices found, skipping wireless");
            return;
        }
        match self.wireless.connect() {
            Ok(()) => match self.wireless.start_polling() {
                Ok(()) => {
                    let _ = self.wireless.send_rx_sequence();
                    self.auto_rebind_configured_wireless();
                    info!("Wireless links active");
                }
                Err(err) => warn!("[wireless] polling start failed: {err}"),
            },
            Err(_) => {
                debug!("[wireless] no TX/RX devices found, skipping wireless");
            }
        }
    }

    #[allow(dead_code)]
    pub(super) fn recover_wireless(&mut self) -> bool {
        if self.wireless.soft_reset() {
            return true;
        }
        warn!("Wireless soft-reset failed; reinitialising");
        self.wireless.stop();
        self.try_wireless();
        self.wireless.has_discovered_devices()
    }

    pub(super) fn load_config(&mut self, tx: Sender<DaemonEvent>) -> bool {
        let templates_path = template_store::templates_path_for(&self.config_path);
        let user_templates = template_store::load_user_templates(&templates_path);
        for t in &user_templates {
            if let Err(e) = t.validate() {
                warn!("Template: {e}");
            }
        }
        let sensors_for_preview = lianli_shared::sensors::enumerate_sensors();
        template_store::regenerate_template_previews(&user_templates, &sensors_for_preview);
        self.ipc.state.lock().user_templates = user_templates;

        match AppConfig::load(&self.config_path) {
            Ok((cfg, warnings)) => {
                for w in &warnings {
                    warn!("Config: {w}");
                }
                self.config = Some(cfg);
                self.packet_builder = PacketBuilder::new();
                self.prepare_media_assets(tx);
                true
            }
            Err(err) => {
                warn!("Failed to load config: {err}");
                false
            }
        }
    }
}

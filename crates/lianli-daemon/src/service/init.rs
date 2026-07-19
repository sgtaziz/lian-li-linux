use super::{DaemonEvent, ServiceManager};
use crate::aio_controller::AioController;
use crate::fan_controller::FanController;
use crate::ipc_server;
use crate::openrgb_server;
use crate::rgb_controller::RgbController;
use crate::template_store;
use lianli_devices::crypto::PacketBuilder;
use lianli_devices::detect::{create_wired_controllers, enumerate_devices};
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
        if let Some(controller) = self.fan_controller.take() {
            info!("Stopping existing fan controller for reload...");
            controller.stop();
        }
        let Some(cfg) = &self.config else {
            return;
        };
        let fan_config = cfg.fans.clone().unwrap_or_default();
        let fan_curves = cfg.fan_curves.clone();

        // Reuse the already-opened wired fan device handles (populated at startup).
        let wired_devices = Arc::clone(&self.wired_fan_devices);

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
        );
        controller.start();
        self.fan_controller = Some(controller);
    }

    pub(super) fn start_aio_control(&mut self) {
        if let Some(existing) = self.aio_controller.take() {
            existing.stop();
        }
        let Some(cfg) = self.config.clone() else {
            return;
        };
        let wireless = Arc::new(self.wireless.clone());
        let mut controller = AioController::new(wireless, cfg);
        controller.start();
        self.aio_controller = Some(controller);
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
            if let Err(e) = ipc_server::write_config(&self.config_path, &snapshot) {
                warn!("Failed to persist AIO config additions: {e}");
            } else {
                self.ipc_state.lock().config = Some(snapshot);
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
        if current == self.last_wired_hid_ids {
            return;
        }

        let added = current.difference(&self.last_wired_hid_ids).count();
        let removed = self.last_wired_hid_ids.difference(&current).count();
        info!("Wired device topology changed (+{added} -{removed}): re-initializing");

        self.hid_backends.retain(|k, _| current.contains(k));
        self.init_wired_devices();
        self.start_fan_control();
        self.last_wired_hid_ids = current;
    }

    /// Initialize all wired HID devices (fan + RGB) in a single pass.
    /// Shares one USB handle per physical device across fan and RGB controllers.
    pub(super) fn init_wired_devices(&mut self) {
        let mut fan_devices: HashMap<String, Box<dyn FanDevice>> = HashMap::new();
        let mut wired_rgb: HashMap<String, Box<dyn lianli_devices::traits::RgbDevice>> =
            HashMap::new();
        self.wired_fan_device_info.clear();

        let usb_devs = match enumerate_devices() {
            Ok(devs) => devs,
            Err(err) => {
                warn!("Failed to enumerate USB devices: {err}");
                self.wired_fan_devices = Arc::new(fan_devices);
                self.init_rgb_controller_from(wired_rgb);
                return;
            }
        };
        for det in usb_devs {
            if !lianli_shared::device_id::uses_hid(det.family) {
                continue;
            }
            if det.family == lianli_shared::device_id::DeviceFamily::TlLcd {
                continue;
            }
            let base_id = Self::rusb_device_id(&det);
            let backend = match self.get_or_open_backend_rusb(&det) {
                Ok(b) => b,
                Err(e) => {
                    warn!("Failed to open HID backend for {}: {e}", det.name);
                    continue;
                }
            };
            if let Some(result) = create_wired_controllers(det.family, det.pid, backend) {
                self.register_wired_controllers(
                    &base_id,
                    det.name,
                    det.family,
                    det.vid,
                    det.pid,
                    det.serial.as_deref(),
                    result,
                    &mut fan_devices,
                    &mut wired_rgb,
                );
            }
        }

        self.init_usb_bulk_rgb_devices(&mut wired_rgb);

        let arc = Arc::new(fan_devices);
        self.wired_fan_devices = Arc::clone(&arc);
        self.init_rgb_controller_from(wired_rgb);
        self.last_wired_hid_ids = self.enumerate_wired_controller_ids();
    }

    fn init_usb_bulk_rgb_devices(
        &mut self,
        wired_rgb: &mut HashMap<String, Box<dyn lianli_devices::traits::RgbDevice>>,
    ) {
        let usb_devs = match enumerate_devices() {
            Ok(devs) => devs,
            Err(err) => {
                warn!("Failed to enumerate USB devices for bulk RGB scan: {err}");
                return;
            }
        };
        for det in usb_devs {
            let opener: Option<
                fn(
                    rusb::Device<rusb::GlobalContext>,
                ) -> anyhow::Result<lianli_devices::winusb_led::WinUsbLedDevice>,
            > = match det.family {
                lianli_shared::device_id::DeviceFamily::UniversalScreenLighting => {
                    Some(lianli_devices::universal_screen_lighting::open)
                }
                _ => None,
            };
            let Some(opener) = opener else { continue };

            let device_id = Self::rusb_device_id(&det);
            let device = rusb::Device::clone(&det.device);
            match opener(device) {
                Ok(ctrl) => {
                    info!("Opened {} as RGB device: {device_id}", det.name);
                    wired_rgb.insert(
                        device_id,
                        Box::new(ctrl) as Box<dyn lianli_devices::traits::RgbDevice>,
                    );
                }
                Err(e) => warn!(
                    "Failed to open {} ({:04x}:{:04x}): {e}",
                    det.name, det.vid, det.pid
                ),
            }
        }
    }

    /// Register fan + RGB from a unified controller set.
    fn register_wired_controllers(
        &mut self,
        base_id: &str,
        name: &str,
        family: DeviceFamily,
        vid: u16,
        pid: u16,
        serial: Option<&str>,
        result: anyhow::Result<lianli_devices::detect::WiredControllerSet>,
        fan_devices: &mut HashMap<String, Box<dyn FanDevice>>,
        wired_rgb: &mut HashMap<String, Box<dyn lianli_devices::traits::RgbDevice>>,
    ) {
        match result {
            Ok(set) => {
                if let Some(fan_ctrl) = set.fan {
                    info!("Opened {name} as fan device: {base_id}");
                    let supports_quantity = fan_ctrl.supports_fan_quantity();
                    let max_quantity =
                        supports_quantity.then(|| fan_ctrl.max_fan_quantity_per_port());

                    if supports_quantity {
                        if let (Some(serial_str), Some(cfg)) = (serial, self.config.as_ref()) {
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

                    let ports = fan_ctrl.fan_port_info();
                    let per_fan = fan_ctrl.per_fan_control();
                    let mb_sync = fan_ctrl.supports_mb_sync();
                    let pump_control = fan_ctrl.has_pump_control();
                    for &(port, fan_count) in &ports {
                        let device_id = if ports.len() > 1 {
                            format!("{base_id}:port{port}")
                        } else {
                            base_id.to_string()
                        };
                        let dev_name = if ports.len() > 1 {
                            format!("{name} Port {port}")
                        } else {
                            name.to_string()
                        };
                        self.wired_fan_device_info.push(DeviceInfo {
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
                            firmware_version: None,
                            supports_c_command: false,
                            port_index: None,
                        });
                    }
                    fan_devices.insert(base_id.to_string(), fan_ctrl);
                }
                for (suffix, rgb_ctrl) in set.rgb {
                    let device_id = if suffix.is_empty() {
                        base_id.to_string()
                    } else {
                        format!("{base_id}:{suffix}")
                    };
                    info!("Opened {name} as RGB device: {device_id}");
                    wired_rgb.insert(device_id, rgb_ctrl);
                }
            }
            Err(err) => warn!("Failed to init {name}: {err}"),
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
            .wired_fan_device_info
            .iter()
            .find(|d| d.device_id == device_id)
            .and_then(|d| d.serial.clone());

        let Some(ctrl) = self.wired_fan_devices.get(&base_id) else {
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
                if let Err(e) = ipc_server::write_config(&self.config_path, &snapshot) {
                    warn!("Failed to persist ENE 6K77 fan quantity: {e}");
                } else {
                    self.ipc_state.lock().config = Some(snapshot);
                }
            }
        }

        for info in self.wired_fan_device_info.iter_mut() {
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
        wired_rgb: HashMap<String, Box<dyn lianli_devices::traits::RgbDevice>>,
    ) {
        let wireless = if self.wireless.has_discovered_devices() {
            Some(Arc::new(self.wireless.clone()))
        } else {
            None
        };

        let mut controller = RgbController::new(wired_rgb, wireless);

        if let Some(ref cfg) = self.config {
            if let Some(ref rgb_cfg) = cfg.rgb {
                let presets = self.ipc_state.lock().rgb_presets.clone();
                controller.apply_config(rgb_cfg, &presets);
            }
        }

        let rgb_arc = Arc::new(Mutex::new(controller));
        self.rgb_controller = Some(Arc::clone(&rgb_arc));
        self.ipc_state.lock().rgb_controller = Some(rgb_arc);
    }

    /// Rebuild RGB controller to pick up newly discovered wireless devices.
    pub(super) fn rebuild_rgb_controller(&mut self) {
        let wireless = if self.wireless.has_discovered_devices() {
            Some(Arc::new(self.wireless.clone()))
        } else {
            None
        };
        if let Some(ref rgb) = self.rgb_controller {
            let mut ctrl = rgb.lock();
            ctrl.set_wireless(wireless);
            ctrl.refresh_wireless_devices();
            if let Some(ref cfg) = self.config {
                if let Some(ref rgb_cfg) = cfg.rgb {
                    let presets = self.ipc_state.lock().rgb_presets.clone();
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
        if let (Some(ref rgb), Some(ref cfg)) = (&self.rgb_controller, &self.config) {
            if let Some(ref rgb_cfg) = cfg.rgb {
                let presets = self.ipc_state.lock().rgb_presets.clone();
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
        let current_state = self.openrgb_state.lock().clone();
        let needs_restart =
            self.openrgb_thread.is_some() && (current_state.port != Some(port) || !enabled);

        if needs_restart {
            info!("Stopping OpenRGB server for reconfiguration");
            self.openrgb_stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.openrgb_thread.take() {
                let _ = thread.join();
            }
            if let Some(thread) = self.direct_color_writer.take() {
                let _ = thread.join();
            }
            let mut s = self.openrgb_state.lock();
            *s = openrgb_server::OpenRgbServerState::default();
        }

        if !enabled {
            return;
        }

        if self.openrgb_thread.is_some() {
            return; // Already running with correct port
        }

        if let Some(ref rgb) = self.rgb_controller {
            self.openrgb_stop.store(false, Ordering::Relaxed);
            self.openrgb_thread = Some(openrgb_server::start_openrgb_server(
                Arc::clone(rgb),
                Arc::clone(&self.direct_color_buffer),
                port,
                Arc::clone(&self.openrgb_stop),
                Arc::clone(&self.openrgb_state),
            ));
            // Start the async writer thread that flushes buffered colors at 30fps
            if self.direct_color_writer.is_none() {
                self.direct_color_writer = Some(crate::rgb_controller::start_direct_color_writer(
                    Arc::clone(rgb),
                    Arc::clone(&self.direct_color_buffer),
                    Arc::clone(&self.openrgb_stop),
                ));
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
                    info!("Wireless links active");
                }
                Err(err) => warn!("[wireless] polling start failed: {err}"),
            },
            Err(_) => {
                debug!("[wireless] no TX/RX devices found, skipping wireless");
            }
        }
    }

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
        self.ipc_state.lock().user_templates = user_templates;

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

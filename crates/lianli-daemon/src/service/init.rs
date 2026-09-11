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
use lianli_shared::config::{AppConfig, HidBackend};
use lianli_shared::device_id::DeviceFamily;
use lianli_shared::ipc::DeviceInfo;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use tracing::{debug, info, warn};

const MAX_INIT_RETRIES: u32 = 18;

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

        let wireless = if self.wireless.is_connected() {
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
        let wired = Arc::clone(&self.registry.fan_devices);
        let mut controller = AioController::new(wireless, wired, cfg);
        controller.start();
        self.controllers.aio = Some(controller);
    }

    /// For each discovered AIO, ensure an AioConfig exists in the user's config.
    /// Migrates any legacy FanGroup targeting that device, then inserts defaults.
    /// Wireless devices get a default config (all-off / device-managed); wired
    /// AIOs only get legacy-group migration — no config means no PWM writes,
    /// leaving the device's firmware in control until the user configures it.
    pub(super) fn ensure_aio_defaults(&mut self) {
        let mut wired_aio_ids: Vec<String> = Vec::new();
        // Wired AIOs: fan devices with pump control (HydroShift LCD family).
        for info in &self.registry.fan_device_info {
            if info.has_pump_control {
                wired_aio_ids.push(info.device_id.clone());
            }
        }
        let any_wireless_aio = self.wireless.devices().iter().any(|d| d.is_aio());
        if wired_aio_ids.is_empty() && !any_wireless_aio {
            return;
        }

        // Keep the lock held across the write, IPC handlers update
        // state.config concurrently.
        let mut ipc_state = self.ipc.state.lock();
        let Some(mut cfg) = ipc_state.config.clone().or_else(|| self.config.clone()) else {
            return;
        };

        let mut changed = false;
        // Wired: migrate legacy fan groups only — absence of config leaves
        // the device firmware in control.
        for device_id in &wired_aio_ids {
            if cfg.migrate_aio_fangroup(device_id) {
                info!("Migrated legacy fan group for AIO {device_id} into aio config");
                changed = true;
            }
        }
        for device in self.wireless.devices() {
            if !device.is_aio() {
                continue;
            }
            let device_id = format!("wireless:{}", device.mac_str());
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
            if let Err(e) = persistence::write_config(&self.config_path, &cfg) {
                warn!("Failed to persist AIO config additions: {e}");
            } else {
                self.config = Some(cfg.clone());
                ipc_state.config = Some(cfg);
            }
        } else {
            self.config = Some(cfg);
        }
    }

    /// One enumeration snapshot with the derived id and topology sets.
    /// Callers must treat `Err` as "poll skipped", never as "no devices".
    fn snapshot_wired(&self) -> Result<(HashSet<String>, HashSet<String>), anyhow::Error> {
        use lianli_shared::device_id::DeviceFamily;
        fn is_wired_controller(family: DeviceFamily) -> bool {
            lianli_shared::device_id::uses_hid(family)
                || matches!(family, DeviceFamily::UniversalScreenLighting)
        }
        let usb_devs = enumerate_devices()?;
        let mut ids = HashSet::new();
        let mut topos = HashSet::new();
        for det in usb_devs
            .iter()
            .filter(|det| is_wired_controller(det.family))
        {
            ids.insert(Self::rusb_device_id(det));
            topos.insert(format!("{}:{}", det.bus, det.topology_key()));
        }
        Ok((ids, topos))
    }

    pub(super) fn check_wired_hotplug(&mut self) {
        let (current_ids, current_topos) = match self.snapshot_wired() {
            Ok(sets) => sets,
            Err(e) => {
                warn!("Wired enumeration failed, skipping hotplug poll: {e}");
                return;
            }
        };
        // Compare identities too, not just topology. A device replaced at
        // the same port keeps its topology entry but gets a new id from
        // its serial, and without this check the daemon would keep the
        // stale backends forever and never open the replacement.
        let topology_changed = current_topos != self.registry.last_wired_topos;
        let identities_changed = current_ids != self.registry.last_wired_ids;
        if topology_changed || identities_changed {
            let added = current_topos
                .difference(&self.registry.last_wired_topos)
                .count();
            let removed = self
                .registry
                .last_wired_topos
                .difference(&current_topos)
                .count();
            let id_added = current_ids
                .difference(&self.registry.last_wired_ids)
                .count();
            let id_removed = self
                .registry
                .last_wired_ids
                .difference(&current_ids)
                .count();
            let now = std::time::Instant::now();
            const MIN_REINIT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
            if let Some(last) = self.registry.last_reinit {
                if now.duration_since(last) < MIN_REINIT_INTERVAL {
                    debug!(
                        "Wired device set changed (topology +{added} -{removed}, identity +{id_added} -{id_removed}) \
                         but re-init rate-limited"
                    );
                    return;
                }
            }
            self.registry.last_reinit = Some(now);
            info!(
                "Wired device set changed (topology +{added} -{removed}, identity +{id_added} -{id_removed}): re-initializing"
            );

            // stop controllers first, they hold Arcs into fan_devices
            if let Some(controller) = self.controllers.fan.take() {
                controller.stop();
            }
            if let Some(controller) = self.controllers.aio.take() {
                controller.stop();
            }
            self.registry
                .hid_backends
                .retain(|k, _| current_ids.contains(k));
            self.registry
                .usb_backends
                .retain(|k, _| current_ids.contains(k));
            self.registry
                .aio_lcd_devices
                .retain(|k, _| current_ids.contains(k));
            self.registry.v2_hid_entries.clear();
            self.init_wired_devices();
            self.start_fan_control();
            self.start_aio_control();
            return;
        }

        let pending: HashSet<String> = self
            .registry
            .failed_open_ids
            .intersection(&current_ids)
            .cloned()
            .collect();
        if pending.is_empty() || self.registry.init_retry_count >= MAX_INIT_RETRIES {
            return;
        }
        self.registry.init_retry_count += 1;
        info!(
            "Retrying {} device(s) that failed to open (attempt {}/{})",
            pending.len(),
            self.registry.init_retry_count,
            MAX_INIT_RETRIES
        );
        if let Some(controller) = self.controllers.fan.take() {
            controller.stop();
        }
        if let Some(controller) = self.controllers.aio.take() {
            controller.stop();
        }
        self.init_wired_devices();
        self.start_fan_control();
        self.start_aio_control();
    }

    pub(super) fn hid_backend(&self) -> HidBackend {
        self.config
            .as_ref()
            .map(|c| c.hid_backend)
            .unwrap_or_default()
    }

    pub(super) fn reconcile_wired_wireless_binding(&self) {
        // Wired devices whose link MAC matches a bound wireless group are RF owned
        let bound_macs: HashSet<[u8; 6]> = self.wireless.devices().iter().map(|d| d.mac).collect();
        for dev in self.registry.fan_devices.values() {
            if let Some(mac) = dev.wireless_link_mac() {
                dev.set_wireless_bound(bound_macs.contains(&mac));
            }
        }
    }

    /// Initialize all wired USB devices (fan + RGB + LCD + AIO) via the
    /// [`registry`] dispatch table. Each device is opened on its own thread
    /// with a timeout so that one unresponsive controller cannot block the
    /// rest of the daemon. Devices that time out are skipped and will be
    /// retried by the hotplug poller.
    ///
    /// Returns `false` when the rebuild was deferred or enumeration failed,
    /// in which case the topology baseline is left untouched so a later
    /// poll retries.
    pub(super) fn init_wired_devices(&mut self) -> bool {
        let already_opened: HashSet<String> = self
            .registry
            .hid_backends
            .keys()
            .chain(self.registry.usb_backends.keys())
            .cloned()
            .collect();

        let mut fan_devices: HashMap<String, Box<dyn FanDevice>> =
            match Arc::try_unwrap(std::mem::take(&mut self.registry.fan_devices)) {
                Ok(map) => map,
                Err(arc) => {
                    warn!("fan_devices map still shared — deferring wired re-init");
                    self.registry.fan_devices = arc;
                    return false;
                }
            };
        let mut wired_rgb: HashMap<String, std::sync::Arc<dyn lianli_devices::traits::RgbDevice>> =
            HashMap::new();

        let usb_devs = match enumerate_devices() {
            Ok(devs) => devs,
            Err(err) => {
                warn!("Failed to enumerate USB devices: {err}");
                self.registry.fan_devices = Arc::new(fan_devices);
                self.init_rgb_controller_from(wired_rgb);
                return false;
            }
        };

        let present_ids: HashSet<String> = usb_devs.iter().map(Self::rusb_device_id).collect();
        let present_topos: HashSet<String> =
            usb_devs.iter().map(|det| det.topology_key()).collect();
        fan_devices.retain(|id, _| present_ids.contains(id));
        self.registry.fan_device_info.retain(|info| {
            info.topology_key
                .as_ref()
                .is_some_and(|t| present_topos.contains(t))
        });

        const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

        let mut pending: Vec<(
            String,
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
            let base_id = Self::rusb_device_id(det);

            if already_opened.contains(&base_id) {
                debug!("Skipping {base_id} — already opened, preserving handle");
                continue;
            }

            let ctx = registry::OpenContext {
                device: det.device.clone(),
                family: det.family,
                vid: det.vid,
                pid: det.pid,
                bus: det.bus,
                address: det.address,
                serial: det.serial.clone(),
                hid_usage_page: det.hid_usage_page,
                hid_backend: self.hid_backend(),
            };
            let name = det.name;
            let family = det.family;
            let vid = det.vid;
            let pid = det.pid;
            let serial = det.serial.clone();
            let topology_key = det.topology_key();

            let (tx, rx) =
                std::sync::mpsc::sync_channel::<anyhow::Result<registry::OpenedDevice>>(1);
            let label = format!("{name} ({vid:04x}:{pid:04x})");
            std::thread::Builder::new()
                .name(format!("dev-open-{base_id}"))
                .spawn(move || {
                    let _ = tx.send(driver.open(&ctx));
                })
                .ok();

            pending.push((base_id, topology_key, name, family, vid, pid, serial, rx));
            debug!("Spawned open thread for {label}");
        }

        // Collect results using a single global deadline so that N hung
        // devices waste at most OPEN_TIMEOUT total, not N × OPEN_TIMEOUT.
        let deadline = std::time::Instant::now() + OPEN_TIMEOUT;
        let mut failed_ids: HashSet<String> = HashSet::new();
        for (base_id, topology_key, name, family, vid, pid, serial, rx) in pending {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                warn!("Skipped {name} ({vid:04x}:{pid:04x}) — global open deadline exceeded");
                failed_ids.insert(base_id);
                continue;
            }
            match rx.recv_timeout(remaining) {
                Ok(Ok(mut opened)) => {
                    let shared_hid = opened.shared_hid.take();
                    if let Some(backend) = shared_hid {
                        self.registry.hid_backends.insert(base_id.clone(), backend);
                    }
                    let shared_usb = opened.shared_usb.take();
                    if let Some(transport) = shared_usb {
                        self.registry
                            .usb_backends
                            .insert(base_id.clone(), transport);
                    }
                    self.register_opened_device(
                        base_id,
                        topology_key,
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
                Ok(Err(e)) => {
                    warn!("Failed to open {name} ({vid:04x}:{pid:04x}): {e}");
                    failed_ids.insert(base_id);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    warn!(
                        "Timeout opening {name} ({vid:04x}:{pid:04x}) — skipping; will retry on hotplug"
                    );
                    failed_ids.insert(base_id);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    warn!("Open thread for {name} ({vid:04x}:{pid:04x}) panicked — skipping");
                    failed_ids.insert(base_id);
                }
            }
        }

        let arc = Arc::new(fan_devices);
        self.registry.fan_devices = Arc::clone(&arc);
        if let Some(rgb) = &self.controllers.rgb {
            rgb.lock().retain_wired(&present_ids);
        }
        self.init_rgb_controller_from(wired_rgb);
        match self.snapshot_wired() {
            Ok((ids, topos)) => {
                self.registry.last_wired_ids = ids;
                self.registry.last_wired_topos = topos;
            }
            Err(e) => warn!("Post-init enumeration failed, keeping previous baseline: {e}"),
        }

        if failed_ids.is_empty() {
            if self.registry.init_retry_count > 0 {
                info!(
                    "All wired devices opened successfully after {} retry/retries",
                    self.registry.init_retry_count
                );
            }
            self.registry.init_retry_count = 0;
        } else {
            warn!(
                "{} device(s) failed to open — will retry on next hotplug check",
                failed_ids.len()
            );
        }
        self.registry.failed_open_ids = failed_ids;
        true
    }

    /// Dispatch an [`registry::OpenedDevice`] into the fan / RGB / AIO
    /// subsystems based on which slots are populated.
    fn register_opened_device(
        &mut self,
        base_id: String,
        topology_key: String,
        name: &str,
        family: DeviceFamily,
        vid: u16,
        pid: u16,
        serial: Option<&str>,
        mut opened: registry::OpenedDevice,
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
                    has_lcd: opened.lcd.is_some(),
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
                    wireless_bind_status: None,
                    foreign_master_online: false,
                    pump_rpm_range: None,
                    fan_quantity: supports_quantity.then_some(fan_count),
                    max_fan_quantity: max_quantity,
                    firmware_version: opened.firmware.clone(),
                    supports_c_command: false,
                    port_index: None,
                    wireless_group_mac: None,
                    topology_key: Some(topology_key.clone()),
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

        if let Some(lcd) = opened.lcd.take() {
            self.registry.aio_lcd_devices.insert(base_id.clone(), lcd);
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
            let mut ipc_state = self.ipc.state.lock();
            if let Some(mut cfg) = ipc_state.config.clone().or_else(|| self.config.clone()) {
                cfg.ene6k77
                    .entry(serial)
                    .or_default()
                    .fan_quantities
                    .insert(port, quantity);
                if let Err(e) = persistence::write_config(&self.config_path, &cfg) {
                    warn!("Failed to persist ENE 6K77 fan quantity: {e}");
                } else {
                    self.config = Some(cfg.clone());
                    ipc_state.config = Some(cfg);
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
        let mut all_wired = if let Some(ref rgb) = self.controllers.rgb {
            rgb.lock().drain_wired()
        } else {
            HashMap::new()
        };
        all_wired.extend(wired_rgb);

        let wireless = if self.wireless.has_discovered_devices() {
            Some(Arc::new(self.wireless.clone()))
        } else {
            None
        };

        if let Some(rgb) = &self.controllers.rgb {
            {
                let mut controller = rgb.lock();
                controller.replace_wired(all_wired);
                controller.set_wireless(wireless);
                controller.refresh_wireless_devices();
            }
            self.apply_rgb_config();
            return;
        }

        let mut controller = RgbController::new(all_wired, wireless);

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
            // The state lock must be taken before the controller lock, the
            // IPC handlers use that order and the reverse can deadlock.
            let ipc_state = self.ipc.state.lock();
            let mut ctrl = rgb.lock();
            ctrl.set_wireless(wireless);
            ctrl.refresh_wireless_devices();
            if let Some(rgb_cfg) = ipc_state.config.as_ref().and_then(|c| c.rgb.clone()) {
                let presets = ipc_state.rgb_presets.clone();
                ctrl.apply_config(&rgb_cfg, &presets);
            }
        }
    }

    /// Restart the fan controller to pick up newly discovered wireless devices.
    pub(super) fn restart_fan_control(&mut self) {
        self.start_fan_control();
    }

    pub(super) fn apply_rgb_config(&self) {
        let (controller, config, presets) = {
            let state = self.ipc.state.lock();
            (
                state.rgb_controller.clone(),
                state
                    .config
                    .as_ref()
                    .and_then(|c| c.rgb.clone())
                    .unwrap_or_else(|| lianli_shared::rgb::RgbAppConfig {
                        enabled: false,
                        ..Default::default()
                    }),
                state.rgb_presets.clone(),
            )
        };
        if let Some(controller) = controller {
            controller.lock().apply_config(&config, &presets);
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

    pub(super) fn configured_wireless_device_ids(&self) -> std::collections::HashSet<String> {
        let mut configured_ids = std::collections::HashSet::new();

        if let Some(cfg) = self.config.as_ref() {
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
        }

        configured_ids
    }

    pub(super) fn try_wireless(&mut self) {
        if !lianli_devices::wireless::tx_dongle_present() {
            debug!("[wireless] no TX/RX devices found, skipping wireless");
            return;
        }
        let restart_controllers = self.controllers.fan.is_some() || self.controllers.aio.is_some();
        if let Some(rgb) = &self.controllers.rgb {
            rgb.lock().set_wireless(None);
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
        if restart_controllers {
            self.start_fan_control();
            self.start_aio_control();
            self.rebuild_rgb_controller();
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

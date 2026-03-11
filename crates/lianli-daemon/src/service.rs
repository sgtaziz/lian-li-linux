use crate::fan_controller::FanController;
use crate::ipc_server::{self, DaemonState};
use crate::openrgb_server;
use crate::rgb_controller::RgbController;
use anyhow::Result;
use lianli_devices::crypto::PacketBuilder;
use lianli_devices::detect::{
    enumerate_devices, enumerate_hid_devices, open_ene6k77_rgb_via_rusb, open_ene6k77_via_rusb,
    open_fan_device, open_rgb_devices,
};
use lianli_devices::hydroshift_lcd::HydroShiftLcdController;
use lianli_devices::slv3_lcd::Slv3LcdDevice;
use lianli_devices::traits::FanDevice;
use lianli_devices::winusb_lcd::WinUsbLcdDevice;
use lianli_devices::wireless::WirelessController;
use lianli_shared::device_id::DeviceFamily;
use lianli_media::{prepare_media_asset, MediaAsset, SensorAsset};
use lianli_shared::config::{config_identity, AppConfig, ConfigKey};
use lianli_shared::ipc::DeviceInfo;
use lianli_shared::media::MediaType;
use lianli_shared::screen::{screen_info_for, ScreenInfo};
use parking_lot::Mutex;
use rusb::Device;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

const DEVICE_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Full USB bus enumeration interval — only needed for hot-plug detection of
/// wired USB devices (LCD, AIO, etc.). Wireless discovery uses its own RX polling.
const USB_ENUM_INTERVAL: Duration = Duration::from_secs(10);
const ACTIVE_SLEEP: Duration = Duration::from_millis(1);
const IDLE_SLEEP: Duration = Duration::from_millis(200);

pub struct ServiceManager {
    config_path: PathBuf,
    config: Option<AppConfig>,
    media_assets: HashMap<usize, MediaAsset>,
    targets: HashMap<usize, ActiveTarget>,
    wireless: WirelessController,
    packet_builder: PacketBuilder,
    fan_controller: Option<FanController>,
    rgb_controller: Option<Arc<Mutex<RgbController>>>,
    /// Per-port DeviceInfo for wired fan devices (populated by open_wired_fan_devices).
    wired_fan_device_info: Vec<DeviceInfo>,
    /// Shared reference to wired fan device handles (for RPM reading).
    wired_fan_devices: Arc<HashMap<String, Box<dyn FanDevice>>>,
    last_device_scan: Instant,
    last_usb_enum: Instant,
    /// Cached USB device list from enumerate_devices() — refreshed every USB_ENUM_INTERVAL.
    cached_usb_devices: Vec<DeviceInfo>,
    running: bool,
    ipc_state: Arc<Mutex<DaemonState>>,
    ipc_stop: Arc<AtomicBool>,
    ipc_thread: Option<JoinHandle<()>>,
    openrgb_stop: Arc<AtomicBool>,
    openrgb_thread: Option<JoinHandle<()>>,
    openrgb_state: Arc<Mutex<openrgb_server::OpenRgbServerState>>,
    direct_color_buffer: Arc<Mutex<crate::rgb_controller::DirectColorBuffer>>,
    direct_color_writer: Option<JoinHandle<()>>,
}

impl ServiceManager {
    pub fn new(config_path: PathBuf) -> Result<Self> {
        let ipc_state = Arc::new(Mutex::new(DaemonState::new(config_path.clone())));

        Ok(Self {
            config_path,
            config: None,
            media_assets: HashMap::new(),
            targets: HashMap::new(),
            wireless: WirelessController::new(),
            packet_builder: PacketBuilder::new(),
            fan_controller: None,
            rgb_controller: None,
            wired_fan_device_info: Vec::new(),
            wired_fan_devices: Arc::new(HashMap::new()),
            last_device_scan: Instant::now() - DEVICE_POLL_INTERVAL,
            last_usb_enum: Instant::now() - USB_ENUM_INTERVAL,
            cached_usb_devices: Vec::new(),
            running: true,
            ipc_state,
            ipc_stop: Arc::new(AtomicBool::new(false)),
            ipc_thread: None,
            openrgb_stop: Arc::new(AtomicBool::new(false)),
            openrgb_thread: None,
            openrgb_state: Arc::new(Mutex::new(openrgb_server::OpenRgbServerState::default())),
            direct_color_buffer: Arc::new(Mutex::new(crate::rgb_controller::DirectColorBuffer::new())),
            direct_color_writer: None,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        info!("=====================================================================");
        info!("LIAN LI DAEMON");
        info!("=====================================================================");

        {
            let config_path = &self.config_path;
            if !config_path.exists() {
                info!(
                    "No config found at {}, creating default",
                    config_path.display()
                );
                if let Some(parent) = config_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let default_config = AppConfig::default();
                match serde_json::to_string_pretty(&default_config) {
                    Ok(json) => {
                        if let Err(e) = std::fs::write(config_path, json) {
                            warn!("Failed to write default config: {e}");
                        }
                    }
                    Err(e) => warn!("Failed to serialize default config: {e}"),
                }
            }
        }

        // Load config before IPC starts — prevents GUI from getting empty defaults
        self.load_config();
        self.sync_ipc_state();

        // Start IPC server
        self.ipc_thread = Some(ipc_server::start_ipc_server(
            Arc::clone(&self.ipc_state),
            Arc::clone(&self.ipc_stop),
        ));
        self.try_wireless();
        self.open_wired_fan_devices();
        self.init_rgb_controller();
        self.start_openrgb_server();
        self.start_fan_control();

        while self.running {
            let now = Instant::now();

            // Check for IPC-triggered config reload
            {
                let mut ipc_state = self.ipc_state.lock();
                if ipc_state.config_reload_pending {
                    ipc_state.config_reload_pending = false;
                    info!("Config reload triggered via IPC");
                    // Force the config watcher to pick up the new file
                    drop(ipc_state);
                    if self.load_config() {
                        self.last_device_scan = Instant::now() - DEVICE_POLL_INTERVAL;
                        self.start_fan_control();
                        self.apply_rgb_config();
                        self.start_openrgb_server();
                        self.sync_ipc_state();
                    }
                }
            }

            if now.duration_since(self.last_device_scan) >= DEVICE_POLL_INTERVAL {
                self.last_device_scan = Instant::now();
                self.refresh_targets();
                // Refresh USB device enumeration at a slower rate (hot-plug detection).
                // Wireless discovery is handled by its own RX polling thread.
                if now.duration_since(self.last_usb_enum) >= USB_ENUM_INTERVAL {
                    self.last_usb_enum = Instant::now();
                    self.refresh_usb_device_cache();
                }
                self.sync_ipc_telemetry();
            }

            self.stream_targets();

            thread::sleep(if self.targets.is_empty() {
                IDLE_SLEEP
            } else {
                ACTIVE_SLEEP
            });
        }

        self.shutdown();
        Ok(())
    }

    /// Sync current config to IPC shared state.
    fn sync_ipc_state(&self) {
        let mut ipc_state = self.ipc_state.lock();
        ipc_state.config = self.config.clone();
    }

    /// Refresh the cached USB device list (full bus enumeration).
    fn refresh_usb_device_cache(&mut self) {
        match enumerate_devices() {
            Ok(usb_devices) => {
                let mut cached = Vec::new();
                for det in usb_devices {
                    if matches!(
                        det.family,
                        lianli_shared::device_id::DeviceFamily::WirelessTx
                            | lianli_shared::device_id::DeviceFamily::WirelessRx
                            | lianli_shared::device_id::DeviceFamily::DisplaySwitcher
                            | lianli_shared::device_id::DeviceFamily::TlFan
                            | lianli_shared::device_id::DeviceFamily::Ene6k77
                    ) {
                        continue;
                    }
                    let screen = screen_info_for(det.family);
                    let device_id = if det.serial.is_none() && lianli_shared::device_id::uses_hid(det.family) {
                        format!("hid:{:04x}:{:04x}", det.vid, det.pid)
                    } else {
                        det.serial
                            .clone()
                            .unwrap_or_else(|| format!("usb:{}:{}", det.bus, det.address))
                    };

                    cached.push(DeviceInfo {
                        device_id: device_id.clone(),
                        family: det.family,
                        name: det.name.to_string(),
                        serial: Some(device_id),
                        has_lcd: det.family.has_lcd(),
                        has_fan: det.family.has_fan(),
                        has_pump: det.family.has_pump(),
                        has_rgb: det.family.has_rgb(),
                        fan_count: None,
                        per_fan_control: None,
                        mb_sync_support: false,
                        rgb_zone_count: None,
                        screen_width: screen.map(|s| s.width),
                        screen_height: screen.map(|s| s.height),
                    });
                }

                self.cached_usb_devices = cached;
            }
            Err(e) => {
                warn!("USB enumeration failed: {e}");
            }
        }
    }

    /// Update IPC telemetry and device list.
    fn sync_ipc_telemetry(&self) {
        let mut ipc_state = self.ipc_state.lock();
        ipc_state.telemetry.streaming_active = !self.targets.is_empty();

        // OpenRGB server status
        let (enabled, _) = self
            .config
            .as_ref()
            .and_then(|c| c.rgb.as_ref())
            .map(|rgb| (rgb.openrgb_server, rgb.openrgb_port))
            .unwrap_or((false, 6743));
        let orgb_state = self.openrgb_state.lock();
        ipc_state.telemetry.openrgb_status = lianli_shared::ipc::OpenRgbServerStatus {
            enabled,
            running: orgb_state.running,
            port: orgb_state.port,
            error: orgb_state.error.clone(),
        };

        // Build device list from wireless discovery
        let mut devices = Vec::new();
        for dev in self.wireless.devices() {
            let family = match dev.fan_type {
                lianli_devices::wireless::WirelessFanType::Slv3Led => {
                    lianli_shared::device_id::DeviceFamily::Slv3Led
                }
                lianli_devices::wireless::WirelessFanType::Slv3Lcd => {
                    lianli_shared::device_id::DeviceFamily::Slv3Lcd
                }
                lianli_devices::wireless::WirelessFanType::Tlv2Lcd => {
                    lianli_shared::device_id::DeviceFamily::Tlv2Lcd
                }
                lianli_devices::wireless::WirelessFanType::Tlv2Led => {
                    lianli_shared::device_id::DeviceFamily::Tlv2Led
                }
                lianli_devices::wireless::WirelessFanType::SlInf => {
                    lianli_shared::device_id::DeviceFamily::SlInf
                }
                lianli_devices::wireless::WirelessFanType::Clv1 => {
                    lianli_shared::device_id::DeviceFamily::Clv1
                }
                lianli_devices::wireless::WirelessFanType::Unknown => {
                    lianli_shared::device_id::DeviceFamily::Slv3Led
                }
            };
            devices.push(DeviceInfo {
                device_id: format!("wireless:{}", dev.mac_str()),
                family,
                name: dev.fan_type.display_name().to_string(),
                serial: Some(dev.mac_str()),
                has_lcd: false, // LCD streaming uses USB bulk, not wireless
                has_fan: dev.fan_count > 0,
                has_pump: false,
                has_rgb: true, // All wireless fans have RGB LEDs
                fan_count: Some(dev.fan_count),
                per_fan_control: Some(true),
                mb_sync_support: dev.fan_type.supports_hw_mobo_sync() || self.wireless.motherboard_pwm().is_some(),
                rgb_zone_count: Some(dev.fan_count), // One zone per fan
                screen_width: None,
                screen_height: None,
            });

            // Update RPM telemetry keyed by device_id
            let device_id = format!("wireless:{}", dev.mac_str());
            let rpms: Vec<u16> = dev.fan_rpms[..dev.fan_count as usize].to_vec();
            ipc_state.telemetry.fan_rpms.insert(device_id, rpms);
        }

        // Add wired USB/HID fan devices (per-port entries from open_wired_fan_devices)
        devices.extend(self.wired_fan_device_info.clone());

        // Read wired fan RPMs and split per port
        for (base_id, dev) in self.wired_fan_devices.iter() {
            if let Ok(all_rpms) = dev.read_fan_rpm() {
                let ports = dev.fan_port_info();
                let mut offset = 0;
                for &(port, count) in &ports {
                    let end = (offset + count as usize).min(all_rpms.len());
                    let port_rpms = all_rpms[offset..end].to_vec();
                    let device_id = if ports.len() > 1 {
                        format!("{base_id}:port{port}")
                    } else {
                        base_id.clone()
                    };
                    ipc_state.telemetry.fan_rpms.insert(device_id, port_rpms);
                    offset = end;
                }
            }
        }

        // Cache is refreshed every USB_ENUM_INTERVAL (30s) to avoid
        // USB bus contention from opening every device for serial reads.
        devices.extend(self.cached_usb_devices.clone());

        ipc_state.devices = devices;
    }

    fn shutdown(&mut self) {
        for target in self.targets.values_mut() {
            target.stop();
        }
        self.targets.clear();

        if let Some(fan_controller) = self.fan_controller.take() {
            info!("Stopping fan controller...");
            fan_controller.stop();
        }

        self.wireless.stop();

        // Stop OpenRGB server
        self.openrgb_stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.openrgb_thread.take() {
            let _ = thread.join();
        }

        // Stop IPC server
        self.ipc_stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.ipc_thread.take() {
            let _ = thread.join();
        }
    }

    fn start_fan_control(&mut self) {
        if let Some(controller) = self.fan_controller.take() {
            info!("Stopping existing fan controller for reload...");
            controller.stop();
        }

        let (fan_config, fan_curves) = if let Some(cfg) = &self.config {
            match (&cfg.fans, &cfg.fan_curves) {
                (Some(fans), curves) => (fans.clone(), curves.clone()),
                (None, _) => {
                    info!("No fan configuration found in config");
                    return;
                }
            }
        } else {
            return;
        };

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

        let mut controller = FanController::new(fan_config, fan_curves, wireless, wired_devices);
        controller.start();
        self.fan_controller = Some(controller);
    }

    /// Initialize the RGB controller with wired HID RGB devices and wireless controller.
    fn init_rgb_controller(&mut self) {
        let mut wired_rgb: HashMap<String, Box<dyn lianli_devices::traits::RgbDevice>> =
            HashMap::new();

        let api = match hidapi::HidApi::new() {
            Ok(api) => api,
            Err(err) => {
                warn!("Failed to initialize HID API for RGB devices: {err}");
                return;
            }
        };

        for det in enumerate_hid_devices(&api) {
            if let Some(result) = open_rgb_devices(&api, &det) {
                let base_id = det.device_id();
                match result {
                    Ok(devices) => {
                        for (suffix, ctrl) in devices {
                            let device_id = if suffix.is_empty() {
                                base_id.clone()
                            } else {
                                format!("{base_id}:{suffix}")
                            };
                            info!("Opened {} as RGB device: {device_id}", det.name);
                            wired_rgb.insert(device_id, ctrl);
                        }
                    }
                    Err(err) => warn!("Failed to init RGB for {}: {err}", det.name),
                }
            }
        }

        // ENE 6K77 RGB: fall back to rusb if hidapi found nothing (no hidraw node).
        if let Ok(usb_devices) = enumerate_devices() {
            for det in usb_devices {
                if det.family != DeviceFamily::Ene6k77 {
                    continue;
                }
                let base_id = det
                    .serial
                    .clone()
                    .unwrap_or_else(|| format!("usb:{}:{}", det.bus, det.address));
                if wired_rgb.contains_key(&base_id) {
                    continue; // Already opened via hidapi
                }
                if let Some(result) = open_ene6k77_rgb_via_rusb(&det) {
                    match result {
                        Ok(devices) => {
                            for (suffix, ctrl) in devices {
                                let device_id = if suffix.is_empty() {
                                    base_id.clone()
                                } else {
                                    format!("{base_id}:{suffix}")
                                };
                                info!("Opened {} (rusb) as RGB device: {device_id}", det.name);
                                wired_rgb.insert(device_id, ctrl);
                            }
                        }
                        Err(err) => warn!("Failed to init RGB for {} (rusb): {err}", det.name),
                    }
                }
            }
        }

        let wireless = if self.wireless.has_discovered_devices() {
            Some(Arc::new(self.wireless.clone()))
        } else {
            None
        };

        let mut controller = RgbController::new(wired_rgb, wireless);

        // Apply initial RGB config if available
        if let Some(ref cfg) = self.config {
            if let Some(ref rgb_cfg) = cfg.rgb {
                controller.apply_config(rgb_cfg);
            }
        }

        let rgb_arc = Arc::new(Mutex::new(controller));
        self.rgb_controller = Some(Arc::clone(&rgb_arc));

        // Share with IPC state
        self.ipc_state.lock().rgb_controller = Some(rgb_arc);
    }

    /// Apply RGB config from the current AppConfig to the RGB controller.
    fn apply_rgb_config(&self) {
        if let (Some(ref rgb), Some(ref cfg)) = (&self.rgb_controller, &self.config) {
            if let Some(ref rgb_cfg) = cfg.rgb {
                rgb.lock().apply_config(rgb_cfg);
            }
        }
    }

    /// Start or restart the OpenRGB SDK server based on config.
    fn start_openrgb_server(&mut self) {
        let (enabled, port) = self
            .config
            .as_ref()
            .and_then(|c| c.rgb.as_ref())
            .map(|rgb| (rgb.openrgb_server, rgb.openrgb_port))
            .unwrap_or((false, 6743));

        // Check if we need to restart (port changed or toggled)
        let current_state = self.openrgb_state.lock().clone();
        let needs_restart = self.openrgb_thread.is_some()
            && (current_state.port != Some(port) || !enabled);

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
                self.direct_color_writer =
                    Some(crate::rgb_controller::start_direct_color_writer(
                        Arc::clone(rgb),
                        Arc::clone(&self.direct_color_buffer),
                        Arc::clone(&self.openrgb_stop),
                    ));
            }
        }
    }

    /// Enumerate and open all wired HID fan devices on the system.
    /// Also populates `self.wired_fan_device_info` with per-port DeviceInfo entries
    /// and `self.wired_fan_devices` with shared device handles.
    fn open_wired_fan_devices(&mut self) -> Arc<HashMap<String, Box<dyn FanDevice>>> {
        let mut devices: HashMap<String, Box<dyn FanDevice>> = HashMap::new();
        self.wired_fan_device_info.clear();

        let api = match hidapi::HidApi::new() {
            Ok(api) => api,
            Err(err) => {
                warn!("Failed to initialize HID API for fan devices: {err}");
                return Arc::new(devices);
            }
        };

        for det in enumerate_hid_devices(&api) {
            if let Some(result) = open_fan_device(&api, &det) {
                let base_id = det.device_id();
                match result {
                    Ok(ctrl) => {
                        info!("Opened {} as fan device: {base_id}", det.name);
                        // Build per-port DeviceInfo entries
                        let ports = ctrl.fan_port_info();
                        let per_fan = ctrl.per_fan_control();
                        let mb_sync = ctrl.supports_mb_sync();
                        for &(port, fan_count) in &ports {
                            let device_id = if ports.len() > 1 {
                                format!("{base_id}:port{port}")
                            } else {
                                base_id.clone()
                            };
                            let name = if ports.len() > 1 {
                                format!("{} Port {port}", det.name)
                            } else {
                                det.name.to_string()
                            };
                            self.wired_fan_device_info.push(DeviceInfo {
                                device_id,
                                family: det.family,
                                name,
                                serial: det.serial.clone(),
                                has_lcd: false,
                                has_fan: true,
                                has_pump: false,
                                has_rgb: det.family.has_rgb(),
                                fan_count: Some(fan_count),
                                per_fan_control: Some(per_fan),
                                mb_sync_support: mb_sync,
                                rgb_zone_count: None, // Set by RGB controller later
                                screen_width: None,
                                screen_height: None,
                            });
                        }
                        devices.insert(base_id, ctrl);
                    }
                    Err(err) => warn!("Failed to init {}: {err}", det.name),
                }
            }
        }

        // ENE 6K77 devices: open via rusb (bypasses usbhid/hidraw requirement).
        // On some kernels the usbhid driver refuses to bind to the ENE HID descriptor,
        // leaving no hidraw node for hidapi to use. RusbHidTransport talks directly
        // to the USB device via interrupt + control transfers.
        match enumerate_devices() {
            Ok(usb_devices) => {
                for det in usb_devices {
                    if det.family != DeviceFamily::Ene6k77 {
                        continue;
                    }
                    let base_id = det
                        .serial
                        .clone()
                        .unwrap_or_else(|| format!("usb:{}:{}", det.bus, det.address));
                    if devices.contains_key(&base_id) {
                        continue; // Already opened via hidapi
                    }
                    if let Some(result) = open_ene6k77_via_rusb(&det) {
                        match result {
                            Ok(ctrl) => {
                                info!("Opened {} (rusb) as fan device: {base_id}", det.name);
                                let ports = ctrl.fan_port_info();
                                let per_fan = ctrl.per_fan_control();
                                let mb_sync = ctrl.supports_mb_sync();
                                for &(port, fan_count) in &ports {
                                    let device_id = if ports.len() > 1 {
                                        format!("{base_id}:port{port}")
                                    } else {
                                        base_id.clone()
                                    };
                                    let name = if ports.len() > 1 {
                                        format!("{} Port {port}", det.name)
                                    } else {
                                        det.name.to_string()
                                    };
                                    self.wired_fan_device_info.push(DeviceInfo {
                                        device_id,
                                        family: det.family,
                                        name,
                                        serial: det.serial.clone(),
                                        has_lcd: false,
                                        has_fan: true,
                                        has_pump: false,
                                        has_rgb: det.family.has_rgb(),
                                        fan_count: Some(fan_count),
                                        per_fan_control: Some(per_fan),
                                        mb_sync_support: mb_sync,
                                        rgb_zone_count: None,
                                        screen_width: None,
                                        screen_height: None,
                                    });
                                }
                                devices.insert(base_id, ctrl);
                            }
                            Err(err) => warn!("Failed to init {} (rusb): {err}", det.name),
                        }
                    }
                }
            }
            Err(e) => warn!("USB enumeration for ENE 6K77 failed: {e}"),
        }

        let arc = Arc::new(devices);
        self.wired_fan_devices = Arc::clone(&arc);
        arc
    }

    /// Try to connect wireless TX/RX once. Non-blocking — if no dongles found, skip gracefully.
    fn try_wireless(&mut self) {
        match self.wireless.connect() {
            Ok(()) => {
                match self.wireless.start_polling() {
                    Ok(()) => {
                        let _ = self.wireless.send_rx_sequence();
                        info!("Wireless links active");
                    }
                    Err(err) => warn!("[wireless] polling start failed: {err}"),
                }
            }
            Err(_) => {
                debug!("[wireless] no TX/RX devices found, skipping wireless");
            }
        }
    }

    fn recover_wireless(&mut self) -> bool {
        if self.wireless.soft_reset() {
            return true;
        }
        warn!("Wireless soft-reset failed; reinitialising");
        self.wireless.stop();
        self.try_wireless();
        self.wireless.has_discovered_devices()
    }

    fn load_config(&mut self) -> bool {
        match AppConfig::load(&self.config_path) {
            Ok((cfg, warnings)) => {
                for w in &warnings {
                    warn!("Config: {w}");
                }
                self.config = Some(cfg);
                self.packet_builder = PacketBuilder::new();
                self.prepare_media_assets();
                true
            }
            Err(err) => {
                warn!("Failed to load config: {err}");
                false
            }
        }
    }

    fn prepare_media_assets(&mut self) {
        self.media_assets.clear();

        // Build a serial to ScreenInfo map from currently connected devices so each
        // LCD gets its correct native resolution (e.g., H2 = 480×480, not 400×400).
        let screen_map: HashMap<String, ScreenInfo> = enumerate_devices()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|det| {
                let screen = screen_info_for(det.family)?;
                let id = if det.serial.is_none() && lianli_shared::device_id::uses_hid(det.family) {
                    format!("hid:{:04x}:{:04x}", det.vid, det.pid)
                } else {
                    det.serial?
                };
                Some((id, screen))
            })
            .collect();

        if let Some(cfg) = &self.config {
            for (idx, device) in cfg.lcds.iter().enumerate() {
                // Look up screen info by serial; fall back to WIRELESS_LCD (400×400) for
                // devices not currently connected or without a matching serial.
                let screen = device
                    .serial
                    .as_ref()
                    .and_then(|s| screen_map.get(s).copied())
                    .unwrap_or(ScreenInfo::WIRELESS_LCD);
                let cfg_key = config_identity(device);
                match prepare_media_asset(device, cfg.default_fps, &screen) {
                    Ok(asset) => {
                        self.media_assets.insert(idx, asset);
                        let device_id = device.device_id();
                        match device.media_type {
                            MediaType::Image => info!("Prepared image for LCD[{device_id}]"),
                            MediaType::Video => info!("Prepared video for LCD[{device_id}]"),
                            MediaType::Gif => info!("Prepared GIF for LCD[{device_id}]"),
                            MediaType::Color => info!("Prepared color frame for LCD[{device_id}]"),
                            MediaType::Sensor => info!(
                                "Prepared sensor for LCD[{device_id}]: {}",
                                device.sensor.as_ref().map(|s| s.label.as_str()).unwrap_or("<unknown>")
                            ),
                        }
                    }
                    Err(err) => warn!("Skipping LCD[{cfg_key}] media: {err}"),
                }
            }
        }
    }

    fn refresh_targets(&mut self) {
        if self.media_assets.is_empty() {
            return;
        }

        const LCD_FAMILIES: &[DeviceFamily] = &[
            DeviceFamily::Slv3Lcd,
            DeviceFamily::Tlv2Lcd,
            DeviceFamily::HydroShift2Lcd,
            DeviceFamily::Lancool207,
            DeviceFamily::UniversalScreen,
            DeviceFamily::HydroShiftLcd,
            DeviceFamily::Galahad2Lcd,
        ];

        struct LcdCandidate {
            family: DeviceFamily,
            serial: String,
            usb_device: Option<Device<rusb::GlobalContext>>,
            vid: u16,
            pid: u16,
        }

        let mut candidates: Vec<LcdCandidate> = Vec::new();

        if let Ok(usb_devs) = enumerate_devices() {
            for det in usb_devs {
                if !LCD_FAMILIES.contains(&det.family) {
                    continue;
                }
                let is_hid = lianli_shared::device_id::uses_hid(det.family);
                let serial = if det.serial.is_none() && is_hid {
                    format!("hid:{:04x}:{:04x}", det.vid, det.pid)
                } else {
                    det.serial
                        .clone()
                        .unwrap_or_else(|| format!("bus{}-addr{}", det.bus, det.address))
                };
                let transport = if is_hid { "HID" } else { "USB bulk" };
                debug!(
                    "LCD candidate: {} ({:04x}:{:04x}) serial={} ({transport})",
                    det.name, det.vid, det.pid, serial
                );
                candidates.push(LcdCandidate {
                    family: det.family,
                    serial,
                    usb_device: Some(det.device),
                    vid: det.vid,
                    pid: det.pid,
                });
            }
        }

        let mut new_targets = HashMap::new();

        if let Some(cfg) = &self.config {
            for (cfg_idx, device_cfg) in cfg.lcds.iter().enumerate() {
                let asset = match self.media_assets.get(&cfg_idx) {
                    Some(asset) => asset,
                    None => {
                        if let Some(mut existing) = self.targets.remove(&cfg_idx) {
                            existing.stop();
                        }
                        continue;
                    }
                };

                let matched = if let Some(serial) = &device_cfg.serial {
                    candidates.iter().find(|c| &c.serial == serial)
                } else if let Some(index) = device_cfg.index {
                    candidates.get(index)
                } else {
                    None
                };

                let candidate = match matched {
                    Some(c) => c,
                    None => {
                        if let Some(mut existing) = self.targets.remove(&cfg_idx) {
                            info!("[devices] LCD[{}] detached", device_cfg.device_id());
                            existing.stop();
                        }
                        continue;
                    }
                };

                let cfg_key = config_identity(device_cfg);
                if let Some(mut existing) = self.targets.remove(&cfg_idx) {
                    if existing.matches(&candidate.serial, &cfg_key) {
                        new_targets.insert(cfg_idx, existing);
                        continue;
                    } else {
                        existing.stop();
                    }
                }

                let backend_result: anyhow::Result<LcdBackend> = match candidate.family {
                    DeviceFamily::Slv3Lcd | DeviceFamily::Tlv2Lcd => {
                        let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                        Slv3LcdDevice::new(device).map(LcdBackend::Slv3)
                    }
                    DeviceFamily::HydroShift2Lcd => {
                        let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                        lianli_devices::hydroshift2_lcd::open(device).map(LcdBackend::WinUsb)
                    }
                    DeviceFamily::Lancool207 => {
                        let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                        lianli_devices::lancool207::open(device).map(LcdBackend::WinUsb)
                    }
                    DeviceFamily::UniversalScreen => {
                        let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                        lianli_devices::universal_screen::open(device).map(LcdBackend::WinUsb)
                    }
                    DeviceFamily::HydroShiftLcd | DeviceFamily::Galahad2Lcd => {
                        let usb_dev = Device::clone(candidate.usb_device.as_ref().unwrap());
                        let iface = lianli_transport::RusbHidTransport::find_hid_interface(&usb_dev)
                            .unwrap_or(0);
                        lianli_transport::RusbHidTransport::open(usb_dev, iface)
                            .map_err(|e| anyhow::anyhow!("RusbHid open {:04x}:{:04x}: {e}", candidate.vid, candidate.pid))
                            .and_then(|transport| {
                                let backend = lianli_devices::hydroshift_lcd::HidBackend::Rusb(transport);
                                HydroShiftLcdController::new(backend, candidate.pid)
                            })
                            .map(LcdBackend::HidLcd)
                    }
                    _ => unreachable!(),
                };

                match backend_result {
                    Ok(lcd) => {
                        info!(
                            "[devices] LCD[{}] attached (serial: {}, orientation: {:.0}°)",
                            device_cfg.device_id(),
                            candidate.serial,
                            device_cfg.orientation
                        );
                        let target = ActiveTarget::new(cfg_idx, cfg_key, candidate.serial.clone(), lcd, asset);
                        new_targets.insert(cfg_idx, target);
                    }
                    Err(err) => {
                        warn!(
                            "[devices] LCD[{}] unavailable during attach: {err}",
                            device_cfg.device_id()
                        );
                    }
                }
            }
        }

        for (_, mut target) in self.targets.drain() {
            target.stop();
        }

        self.targets = new_targets;
    }

    fn stream_targets(&mut self) {
        if self.targets.is_empty() {
            return;
        }

        let now = Instant::now();
        let ids: Vec<usize> = self.targets.keys().cloned().collect();
        for idx in ids {
            if let Some(target) = self.targets.get_mut(&idx) {
                if !target.should_send(now) {
                    continue;
                }

                match target.send_frame(&self.wireless, &mut self.packet_builder) {
                    Ok(true) => {
                        if target.frame_counter % 30 == 0 {
                            debug!(
                                "LCD[{}] streamed {} frames",
                                target.index, target.frame_counter
                            );
                        }
                    }
                    Ok(false) => {}
                    Err(SendError::Usb(err)) => {
                        self.handle_usb_error(idx, err);
                        break;
                    }
                    Err(SendError::Other(err)) => {
                        warn!("LCD[{}] media error: {err}", target.index);
                        let mut removed = self.targets.remove(&idx).unwrap();
                        removed.stop();
                        break;
                    }
                }
            }
        }
    }

    fn handle_usb_error(&mut self, index: usize, err: lianli_transport::TransportError) {
        if let Some(mut target) = self.targets.remove(&index) {
            warn!("LCD[{index}] USB error: {err}");
            target.stop();
        }
        if matches!(err, lianli_transport::TransportError::Timeout)
            && self.recover_wireless()
        {
            info!("Wireless link recovered");
        }
    }
}

enum LcdBackend {
    Slv3(Slv3LcdDevice),
    WinUsb(WinUsbLcdDevice),
    HidLcd(HydroShiftLcdController),
}

impl LcdBackend {
    fn send_frame(
        &mut self,
        wireless: &WirelessController,
        builder: &mut PacketBuilder,
        frame: &[u8],
    ) -> anyhow::Result<()> {
        match self {
            Self::Slv3(d) => {
                wireless.ensure_video_mode()?;
                d.send_frame(builder, frame)
            }
            Self::WinUsb(d) => d.send_frame(frame),
            Self::HidLcd(d) => d.send_frame(frame),
        }
    }
}

struct ActiveTarget {
    index: usize,
    key: ConfigKey,
    device_identity: String,
    lcd: LcdBackend,
    media: MediaRuntime,
    next_due: Option<Instant>,
    frame_counter: u64,
}

impl ActiveTarget {
    fn new(index: usize, key: ConfigKey, device_identity: String, lcd: LcdBackend, asset: &MediaAsset) -> Self {
        Self {
            index,
            key,
            device_identity,
            lcd,
            media: MediaRuntime::from_asset(asset),
            next_due: None,
            frame_counter: 0,
        }
    }

    fn matches(&self, identity: &str, key: &ConfigKey) -> bool {
        self.device_identity == identity && key == &self.key
    }

    fn should_send(&self, now: Instant) -> bool {
        match &self.media {
            MediaRuntime::Static { sent, .. } => !*sent,
            MediaRuntime::Video { .. } | MediaRuntime::Sensor { .. } => match self.next_due {
                Some(due) => now >= due,
                None => true,
            },
        }
    }

    fn send_frame(
        &mut self,
        wireless: &WirelessController,
        builder: &mut PacketBuilder,
    ) -> Result<bool, SendError> {
        let frame = match self.media.next_frame_bytes() {
            Some(bytes) => bytes,
            None => return Ok(false),
        };

        self.lcd
            .send_frame(wireless, builder, frame)
            .map_err(|err| match err.downcast::<lianli_transport::TransportError>() {
                Ok(usb) => SendError::Usb(usb),
                Err(other) => SendError::Other(other),
            })?;

        self.media.advance_schedule(&mut self.next_due);
        self.frame_counter += 1;
        Ok(true)
    }

    fn stop(&mut self) {}
}

enum MediaRuntime {
    Static {
        frame: Arc<Vec<u8>>,
        sent: bool,
    },
    Video {
        frames: Arc<Vec<Vec<u8>>>,
        durations: Arc<Vec<Duration>>,
        cursor: usize,
        start: Option<Instant>,
        elapsed: Duration,
        last_duration: Duration,
    },
    Sensor {
        renderer: Arc<AsyncSensorRenderer>,
        cached_frame: Vec<u8>,
        next_frame_time: Instant,
    },
}

struct AsyncSensorRenderer {
    asset: Arc<SensorAsset>,
    current_frame: Arc<Mutex<Vec<u8>>>,
    stop_flag: Arc<AtomicBool>,
    _thread: Option<JoinHandle<()>>,
}

impl AsyncSensorRenderer {
    fn new(asset: Arc<SensorAsset>) -> Self {
        let initial = match asset.render_frame() {
            Ok(frame) => frame,
            Err(err) => {
                warn!("sensor initial render failed: {err}");
                asset.blank_frame()
            }
        };

        let current_frame = Arc::new(Mutex::new(initial));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let update_interval = asset.update_interval();

        let asset_clone = Arc::clone(&asset);
        let frame_clone = Arc::clone(&current_frame);
        let stop_clone = Arc::clone(&stop_flag);

        let thread = thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                thread::sleep(update_interval);
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }
                match asset_clone.render_frame() {
                    Ok(new_frame) => {
                        *frame_clone.lock() = new_frame;
                    }
                    Err(err) => {
                        warn!("sensor background render failed: {err}");
                    }
                }
            }
        });

        Self {
            asset,
            current_frame,
            stop_flag,
            _thread: Some(thread),
        }
    }

    fn get_frame(&self) -> Vec<u8> {
        self.current_frame.lock().clone()
    }
}

impl Drop for AsyncSensorRenderer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

impl MediaRuntime {
    fn from_asset(asset: &MediaAsset) -> Self {
        match asset {
            MediaAsset::Static { frame } => Self::Static {
                frame: Arc::clone(frame),
                sent: false,
            },
            MediaAsset::Video {
                frames,
                frame_durations,
            } => Self::Video {
                frames: Arc::clone(frames),
                durations: Arc::clone(frame_durations),
                cursor: 0,
                start: None,
                elapsed: Duration::default(),
                last_duration: Duration::default(),
            },
            MediaAsset::Sensor { asset } => {
                let renderer = Arc::new(AsyncSensorRenderer::new(Arc::clone(asset)));
                let update_interval = asset.update_interval();
                let cached_frame = renderer.get_frame();
                Self::Sensor {
                    renderer,
                    cached_frame,
                    next_frame_time: Instant::now() + update_interval,
                }
            }
        }
    }

    fn next_frame_bytes(&mut self) -> Option<&[u8]> {
        match self {
            MediaRuntime::Static { frame, sent } => {
                if *sent {
                    None
                } else {
                    *sent = true;
                    Some(frame.as_slice())
                }
            }
            MediaRuntime::Video {
                frames,
                durations,
                cursor,
                last_duration,
                ..
            } => {
                if frames.is_empty() {
                    None
                } else {
                    let idx = *cursor % frames.len();
                    *cursor += 1;
                    let duration = durations
                        .get(idx)
                        .copied()
                        .unwrap_or_else(|| Duration::from_millis(33));
                    *last_duration = duration;
                    Some(frames[idx].as_slice())
                }
            }
            MediaRuntime::Sensor {
                renderer,
                cached_frame,
                next_frame_time,
                ..
            } => {
                let now = Instant::now();
                if now >= *next_frame_time {
                    *cached_frame = renderer.get_frame();
                    *next_frame_time = now + renderer.asset.update_interval();
                }
                Some(cached_frame.as_slice())
            }
        }
    }

    fn advance_schedule(&mut self, next_due: &mut Option<Instant>) {
        match self {
            MediaRuntime::Static { .. } => {
                *next_due = None;
            }
            MediaRuntime::Video {
                durations,
                cursor,
                start,
                elapsed,
                last_duration,
                ..
            } => {
                let base = start.get_or_insert_with(Instant::now);
                let frame_delay = (*last_duration).max(Duration::from_millis(10));
                *elapsed += frame_delay;
                *next_due = Some(*base + *elapsed);
                if !durations.is_empty() && *cursor % durations.len() == 0 {
                    *start = Some(Instant::now());
                    *elapsed = Duration::default();
                }
            }
            MediaRuntime::Sensor {
                next_frame_time, ..
            } => {
                *next_due = Some(*next_frame_time);
            }
        }
    }
}

enum SendError {
    Usb(lianli_transport::TransportError),
    Other(anyhow::Error),
}

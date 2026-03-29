use crate::fan_controller::FanController;
use crate::ipc_server::{self, DaemonState};
use crate::openrgb_server;
use crate::rgb_controller::RgbController;
use anyhow::Result;
use lianli_devices::crypto::PacketBuilder;
use lianli_devices::detect::{
    create_hid_lcd_device, create_wired_controllers,
    ensure_hid_devices_bound, enumerate_devices, enumerate_hid_devices,
    open_hid_backend_hidapi, open_hid_backend_rusb, open_hid_lcd_by_vid_pid,
    open_hid_lcd_device_rusb,
};
use lianli_media::sensor::FrameInfo;
use lianli_shared::config::HidDriver;
use lianli_transport::HidBackend;
use lianli_devices::hydroshift_lcd::HydroShiftLcdController;
use lianli_devices::slv3_lcd::Slv3LcdDevice;
use lianli_devices::traits::FanDevice;
use lianli_devices::winusb_lcd::WinUsbLcdDevice;
use lianli_devices::wireless::WirelessController;
use lianli_shared::device_id::DeviceFamily;
use lianli_media::{prepare_media_asset, MediaAsset, MediaAssetKind, SensorAsset};
use lianli_shared::config::{config_identity, AppConfig, ConfigKey};
use lianli_shared::ipc::DeviceInfo;
use lianli_shared::media::MediaType;
use lianli_shared::screen::{screen_info_for, ScreenInfo};
use parking_lot::Mutex;
use rusb::Device;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{debug, info, trace, warn};

const DEVICE_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Full USB bus enumeration interval — only needed for hot-plug detection of
/// wired USB devices (LCD, AIO, etc.). Wireless discovery uses its own RX polling.
const USB_ENUM_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum DaemonEvent {
    IpcUpdate, // Somebody changed the DaemonState in the mutex
    USBCheck,
    DevicePoll,
    DisplaySwitch{device_id: String}, // Device ID pending display mode switch (LCD→Desktop). Handled by main event loop.
    Bind{mac_address: String}, // MAC address pending wireless device bind. Handled by main event loop.
    FrameFinished { asset: Arc<MediaAsset> }, // A device has calculated a new frame, let's update the display
}

pub struct ServiceManager {
    config_path: PathBuf,
    config: Option<AppConfig>,
    media_assets: HashMap<usize, Arc<MediaAsset>>,
    targets: HashMap<usize, ActiveTarget>,
    wireless: WirelessController,
    packet_builder: PacketBuilder,
    fan_controller: Option<FanController>,
    rgb_controller: Option<Arc<Mutex<RgbController>>>,
    /// Per-port DeviceInfo for wired fan devices (populated by open_wired_fan_devices).
    wired_fan_device_info: Vec<DeviceInfo>,
    /// Shared reference to wired fan device handles (for RPM reading).
    wired_fan_devices: Arc<HashMap<String, Box<dyn FanDevice>>>,
    /// Shared HID backends keyed by device ID — allows fan, RGB, and LCD
    /// controllers for the same physical device to share one USB handle.
    hid_backends: HashMap<String, Arc<Mutex<HidBackend>>>,
    /// Cached USB device list from enumerate_devices() — refreshed every USB_ENUM_INTERVAL.
    cached_usb_devices: Vec<DeviceInfo>,
    restart_requested: bool,
    ipc_state: Arc<Mutex<DaemonState>>, // the (shared) state of the deamon. Shared between daemon itself and IPC thread.
    ipc_stop: Arc<AtomicBool>, // Flag which allows the deamon thread (on shutdown) to tell the IPC thread to stop.
    ipc_thread: Option<JoinHandle<()>>, // Here the deamon thread stores the handle to the IPC thread.
    openrgb_stop: Arc<AtomicBool>,
    openrgb_thread: Option<JoinHandle<()>>,
    openrgb_state: Arc<Mutex<openrgb_server::OpenRgbServerState>>,
    direct_color_buffer: Arc<Mutex<crate::rgb_controller::DirectColorBuffer>>,
    direct_color_writer: Option<JoinHandle<()>>,
    tx: Option<Sender<DaemonEvent>>,
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
            hid_backends: HashMap::new(),
            cached_usb_devices: Vec::new(),
            restart_requested: false,
            ipc_state,
            ipc_stop: Arc::new(AtomicBool::new(false)),
            ipc_thread: None,
            openrgb_stop: Arc::new(AtomicBool::new(false)),
            openrgb_thread: None,
            openrgb_state: Arc::new(Mutex::new(openrgb_server::OpenRgbServerState::default())),
            direct_color_buffer: Arc::new(Mutex::new(crate::rgb_controller::DirectColorBuffer::new())),
            direct_color_writer: None,
            tx: None,
        })
    }


    /// Check if the configured HID driver is rusb.
    fn use_rusb(&self) -> bool {
        self.config
            .as_ref()
            .map(|c| c.hid_driver == HidDriver::Rusb)
            .unwrap_or(false)
    }

    /// Stable device ID for a rusb device — uses serial or USB port path.
    fn rusb_device_id(det: &lianli_devices::detect::DetectedDevice) -> String {
        det.device_id()
    }

    /// Get a cached HID backend or open a new one via rusb.
    fn get_or_open_backend_rusb(
        &mut self,
        det: &lianli_devices::detect::DetectedDevice,
    ) -> anyhow::Result<Arc<Mutex<HidBackend>>> {
        let key = Self::rusb_device_id(det);
        if let Some(backend) = self.hid_backends.get(&key) {
            return Ok(Arc::clone(backend));
        }
        let backend = open_hid_backend_rusb(det)?;
        self.hid_backends.insert(key, Arc::clone(&backend));
        Ok(backend)
    }

    /// Get a cached HID backend or open a new one via hidapi.
    fn get_or_open_backend_hidapi(
        &mut self,
        api: &hidapi::HidApi,
        key: &str,
        det: &lianli_devices::detect::DetectedHidDevice,
    ) -> anyhow::Result<Arc<Mutex<HidBackend>>> {
        if let Some(backend) = self.hid_backends.get(key) {
            return Ok(Arc::clone(backend));
        }
        let backend = open_hid_backend_hidapi(api, det)?;
        self.hid_backends.insert(key.to_string(), Arc::clone(&backend));
        Ok(backend)
    }

    pub fn device_poll(&mut self) {
        self.refresh_targets();
        self.sync_ipc_telemetry();
    }

    /// Run the daemon main loop. Returns `true` if the daemon should restart.
    pub fn run(&mut self) -> Result<bool> {
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

        let (tx, rx) = std::sync::mpsc::channel::<DaemonEvent>();

        self.tx = Some(tx.clone());

        // We need to send these two events to ourselves before load_config, as load_config sets up the assets and already sends FrameFinished-Events
        tx.send(DaemonEvent::USBCheck).ok();
        tx.send(DaemonEvent::DevicePoll).ok();

        // Load config before IPC starts — prevents GUI from getting empty defaults
        self.load_config(tx.clone());
        self.sync_ipc_state();

        // Start IPC server
        let tx_cloned = tx.clone();
        self.ipc_thread = Some(ipc_server::start_ipc_server(
            Arc::clone(&self.ipc_state),
            Arc::clone(&self.ipc_stop),
            tx_cloned,
        ));
        self.try_wireless();
        if !self.use_rusb() {
            ensure_hid_devices_bound();
        }
        self.init_wired_devices();
        self.start_openrgb_server();
        self.start_fan_control();

        // Spawn a thread to regularily check for new USB devices.
        let usb_tx = tx.clone();
        thread::spawn(move || loop {
            thread::sleep(USB_ENUM_INTERVAL);
            if usb_tx.send(DaemonEvent::USBCheck).is_err() {
                break; // Daemon thread has ended. Time for us to die as well
            }
        });

        // Spawn a thread to regularily check for new known devices.
        let device_tx = tx.clone();
        thread::spawn(move || loop {
            thread::sleep(DEVICE_POLL_INTERVAL);
            if device_tx.send(DaemonEvent::DevicePoll).is_err() {
                break; // Daemon thread has ended. Time for us to die as well
            }
        });

        for event in rx {
            match event {
                DaemonEvent::USBCheck => {
                    // Refresh USB device enumeration
                    // Wireless discovery is handled by its own RX polling thread.
                    self.refresh_usb_device_cache();
                }
                DaemonEvent::DevicePoll => {
                    // Refresh USB device enumeration
                    // Wireless discovery is handled by its own RX polling thread.
                    self.device_poll();
                }
                DaemonEvent::DisplaySwitch{ device_id } => {
                    self.handle_display_switch_to_desktop(&device_id);
                }
                DaemonEvent::Bind { mac_address: mac_str } => {
                    if let Some(mac) = parse_mac_str(&mac_str) {
                        if let Err(e) = self.wireless.bind_device(&mac) {
                            warn!("Failed to bind wireless device {mac_str}: {e}");
                        }
                    } else {
                        warn!("Invalid MAC address for bind: {mac_str}");
                    }
                }
                DaemonEvent::IpcUpdate => {
                    // Check for IPC-triggered config reload
                    let ipc_state = self.ipc_state.lock();
                    info!("Config reload triggered via IPC");
                    let old_hid_driver = self.config.as_ref().map(|c| c.hid_driver);
                    // Force the config watcher to pick up the new file
                    drop(ipc_state);
                    if self.load_config(tx.clone()) {
                        let new_hid_driver = self.config.as_ref().map(|c| c.hid_driver);
                        if old_hid_driver != new_hid_driver {
                            info!("HID driver changed ({old_hid_driver:?} -> {new_hid_driver:?}), restarting daemon...");
                            self.restart_requested = true;
                            break;
                        }
                        self.start_fan_control();
                        self.apply_rgb_config();
                        self.start_openrgb_server();
                        self.sync_ipc_state();

                        self.device_poll();
                    }
                }
                DaemonEvent::FrameFinished { asset } => {
                    // which worker has a new image to send?
                    self.stream_target(asset);
                }
            }
        }

        self.shutdown();
        Ok(self.restart_requested)
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
                            | lianli_shared::device_id::DeviceFamily::TlFan
                            | lianli_shared::device_id::DeviceFamily::Ene6k77
                    ) {
                        continue;
                    }
                    let screen = screen_info_for(det.family);
                    let device_id = det.device_id();

                    cached.push(DeviceInfo {
                        device_id: device_id.clone(),
                        family: det.family,
                        name: det.name.to_string(),
                        serial: Some(device_id),
                        vid: det.vid,
                        pid: det.pid,
                        has_lcd: det.family.has_lcd(),
                        has_fan: det.family.has_fan(),
                        has_pump: det.family.has_pump(),
                        has_rgb: det.family.has_rgb(),
                        has_pump_control: false,
                        fan_count: None,
                        per_fan_control: None,
                        mb_sync_support: false,
                        rgb_zone_count: None,
                        screen_width: screen.map(|s| s.width),
                        screen_height: screen.map(|s| s.height),
                        is_unbound_wireless: false,
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
            use lianli_devices::wireless::WirelessFanType;
            use lianli_shared::device_id::DeviceFamily;

            let family = match dev.fan_type {
                WirelessFanType::Slv3Led => DeviceFamily::Slv3Led,
                WirelessFanType::Slv3Lcd => DeviceFamily::Slv3Lcd,
                WirelessFanType::Tlv2Lcd => DeviceFamily::Tlv2Lcd,
                WirelessFanType::Tlv2Led => DeviceFamily::Tlv2Led,
                WirelessFanType::SlInf => DeviceFamily::SlInf,
                WirelessFanType::Clv1 => DeviceFamily::Clv1,
                WirelessFanType::WaterBlock | WirelessFanType::WaterBlock2 => {
                    DeviceFamily::WirelessAio
                }
                WirelessFanType::Strimer(_) => DeviceFamily::WirelessStrimer,
                WirelessFanType::Lc217 => DeviceFamily::WirelessLc217,
                WirelessFanType::Led88 => DeviceFamily::WirelessLed88,
                WirelessFanType::V150 => DeviceFamily::WirelessV150,
                WirelessFanType::Unknown => DeviceFamily::Slv3Led,
            };

            let is_aio = dev.fan_type.is_aio();
            let is_rgb_only = dev.fan_type.is_rgb_only();

            // Fan count is the actual number of fans (excluding pump).
            // Pump speed control is handled separately via has_pump_control.
            let fan_count = dev.fan_count;

            // RGB zones: fans + pump head for AIO, or 1 zone for RGB-only devices
            let rgb_zone_count = if is_aio {
                dev.fan_count + 1 // fans + pump head
            } else if is_rgb_only {
                1
            } else {
                dev.fan_count
            };

            devices.push(DeviceInfo {
                device_id: format!("wireless:{}", dev.mac_str()),
                family,
                name: dev.fan_type.display_name().to_string(),
                serial: Some(dev.mac_str()),
                vid: 0,
                pid: 0,
                has_lcd: false,
                has_fan: dev.fan_count > 0,
                has_pump: is_aio,
                has_rgb: true,
                has_pump_control: is_aio,
                fan_count: Some(fan_count),
                per_fan_control: Some(!is_rgb_only),
                mb_sync_support: dev.fan_type.supports_hw_mobo_sync() || self.wireless.motherboard_pwm().is_some(),
                rgb_zone_count: Some(rgb_zone_count),
                screen_width: None,
                screen_height: None,
                is_unbound_wireless: false,
            });

            // Update RPM telemetry keyed by device_id
            let device_id = format!("wireless:{}", dev.mac_str());
            let mut rpms: Vec<u16> = dev.fan_rpms[..dev.fan_count as usize].to_vec();
            if is_aio {
                rpms.push(dev.fan_rpms[3]); // pump RPM
            }
            ipc_state.telemetry.fan_rpms.insert(device_id, rpms);
        }

        // Add unbound wireless devices (visible but not controllable until bound)
        for dev in self.wireless.unbound_devices() {
            use lianli_devices::wireless::WirelessFanType;
            use lianli_shared::device_id::DeviceFamily;

            let family = match dev.fan_type {
                WirelessFanType::Slv3Led => DeviceFamily::Slv3Led,
                WirelessFanType::Slv3Lcd => DeviceFamily::Slv3Lcd,
                WirelessFanType::Tlv2Lcd => DeviceFamily::Tlv2Lcd,
                WirelessFanType::Tlv2Led => DeviceFamily::Tlv2Led,
                WirelessFanType::SlInf => DeviceFamily::SlInf,
                WirelessFanType::Clv1 => DeviceFamily::Clv1,
                WirelessFanType::WaterBlock | WirelessFanType::WaterBlock2 => {
                    DeviceFamily::WirelessAio
                }
                WirelessFanType::Strimer(_) => DeviceFamily::WirelessStrimer,
                WirelessFanType::Lc217 => DeviceFamily::WirelessLc217,
                WirelessFanType::Led88 => DeviceFamily::WirelessLed88,
                WirelessFanType::V150 => DeviceFamily::WirelessV150,
                WirelessFanType::Unknown => DeviceFamily::Slv3Led,
            };

            devices.push(DeviceInfo {
                device_id: format!("wireless-unbound:{}", dev.mac_str()),
                family,
                name: dev.fan_type.display_name().to_string(),
                serial: Some(dev.mac_str()),
                vid: 0,
                pid: 0,
                has_lcd: false,
                has_fan: false,
                has_pump: false,
                has_rgb: false,
                has_pump_control: false,
                fan_count: Some(dev.fan_count),
                per_fan_control: None,
                mb_sync_support: false,
                rgb_zone_count: None,
                screen_width: None,
                screen_height: None,
                is_unbound_wireless: true,
            });
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

        // Drop RGB controller before HID backends so device handles are released cleanly
        self.rgb_controller = None;
        self.ipc_state.lock().rgb_controller = None;
        self.wired_fan_devices = Arc::new(HashMap::new());
        self.hid_backends.clear();

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

        let mut controller = FanController::new(fan_config, fan_curves, wireless, wired_devices);
        controller.start();
        self.fan_controller = Some(controller);
    }

    /// Initialize all wired HID devices (fan + RGB) in a single pass.
    /// Shares one USB handle per physical device across fan and RGB controllers.
    fn init_wired_devices(&mut self) {
        let mut fan_devices: HashMap<String, Box<dyn FanDevice>> = HashMap::new();
        let mut wired_rgb: HashMap<String, Box<dyn lianli_devices::traits::RgbDevice>> =
            HashMap::new();
        self.wired_fan_device_info.clear();

        if self.use_rusb() {
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
                        &base_id, det.name, det.family, det.vid, det.pid,
                        det.serial.as_deref(),
                        result, &mut fan_devices, &mut wired_rgb,
                    );
                }
            }
        } else {
            let api = match hidapi::HidApi::new() {
                Ok(api) => api,
                Err(err) => {
                    warn!("Failed to initialize HID API: {err}");
                    self.wired_fan_devices = Arc::new(fan_devices);
                    self.init_rgb_controller_from(wired_rgb);
                    return;
                }
            };
            for det in enumerate_hid_devices(&api) {
                let base_id = det.device_id();
                let backend = match self.get_or_open_backend_hidapi(&api, &base_id, &det) {
                    Ok(b) => b,
                    Err(e) => {
                        warn!("Failed to open HID backend for {}: {e}", det.name);
                        continue;
                    }
                };
                if let Some(result) = create_wired_controllers(det.family, det.pid, backend) {
                    self.register_wired_controllers(
                        &base_id, det.name, det.family, det.vid, det.pid,
                        det.serial.as_deref(),
                        result, &mut fan_devices, &mut wired_rgb,
                    );
                }
            }
        }

        let arc = Arc::new(fan_devices);
        self.wired_fan_devices = Arc::clone(&arc);
        self.init_rgb_controller_from(wired_rgb);
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
                controller.apply_config(rgb_cfg);
            }
        }

        let rgb_arc = Arc::new(Mutex::new(controller));
        self.rgb_controller = Some(Arc::clone(&rgb_arc));
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

    fn load_config(&mut self, tx: Sender<DaemonEvent>) -> bool {
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

    fn prepare_media_assets(&mut self, tx: Sender<DaemonEvent>) {
        self.media_assets.clear();

        // Build a serial to ScreenInfo map from currently connected devices so each
        // LCD gets its correct native resolution (e.g., H2 = 480×480, not 400×400).
        let screen_map: HashMap<String, ScreenInfo> = enumerate_devices()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|det| {
                let screen = screen_info_for(det.family)?;
                let id = det.device_id();
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
                    Ok(asset_kind) => {
                        let device_id = device.device_id();
                        let asset = MediaAsset{kind: asset_kind, config_key: cfg_key};
                        let asset_arc = Arc::new(asset);
                        self.media_assets.insert(idx, Arc::clone(&asset_arc));

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
                        tx.send(DaemonEvent::FrameFinished { asset: asset_arc })
                            .ok();
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
            device_id: String,
            usb_device: Option<Device<rusb::GlobalContext>>,
            vid: u16,
            pid: u16,
            bus: u8,
            address: u8,
        }

        let mut candidates: Vec<LcdCandidate> = Vec::new();

        if let Ok(usb_devs) = enumerate_devices() {
            for det in usb_devs {
                if !LCD_FAMILIES.contains(&det.family) {
                    continue;
                }
                let device_id = det.device_id();
                let transport = if lianli_shared::device_id::uses_hid(det.family) { "HID" } else { "USB bulk" };
                debug!(
                    "LCD candidate: {} ({:04x}:{:04x}) id={device_id} ({transport})",
                    det.name, det.vid, det.pid
                );
                candidates.push(LcdCandidate {
                    family: det.family,
                    device_id,
                    usb_device: Some(det.device),
                    vid: det.vid,
                    pid: det.pid,
                    bus: det.bus,
                    address: det.address,
                });
            }
        }

        let mut new_targets = HashMap::new();

        if let Some(cfg) = &self.config {
            for (cfg_idx, device_cfg) in cfg.lcds.iter().enumerate() {
                let asset = match self.media_assets.get(&cfg_idx) {
                    Some(asset_arc) => Arc::clone(asset_arc),
                    None => {
                        if let Some(mut existing) = self.targets.remove(&cfg_idx) {
                            existing.stop();
                        }
                        continue;
                    }
                };

                let matched = if let Some(serial) = &device_cfg.serial {
                    candidates.iter().find(|c| &c.device_id == serial)
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
                    if existing.matches(&candidate.device_id, &cfg_key) {
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
                        // Try to reuse a shared HID backend (opened by init_rgb_controller).
                        if let Some(backend) = self.hid_backends.get(&candidate.device_id) {
                            match create_hid_lcd_device(candidate.family, candidate.pid, Arc::clone(backend)) {
                                Some(result) => result.map(LcdBackend::HidLcd),
                                None => Err(anyhow::anyhow!("Not an LCD device")),
                            }
                        } else if self.use_rusb() {
                            let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                            let det = lianli_devices::detect::DetectedDevice {
                                device,
                                family: candidate.family,
                                name: "HydroShift/Galahad LCD",
                                vid: candidate.vid,
                                pid: candidate.pid,
                                bus: candidate.bus,
                                address: candidate.address,
                                serial: Some(candidate.device_id.clone()),
                                hid_usage_page: None,
                            };
                            match open_hid_lcd_device_rusb(&det) {
                                Some(result) => result.map(LcdBackend::HidLcd),
                                None => Err(anyhow::anyhow!("Not an LCD device")),
                            }
                        } else {
                            open_hid_lcd_by_vid_pid(
                                candidate.vid,
                                candidate.pid,
                                candidate.family,
                            )
                            .map(LcdBackend::HidLcd)
                        }
                    }
                    _ => unreachable!(),
                };

                match backend_result {
                    Ok(lcd) => {
                        info!(
                            "[devices] LCD[{}] attached (serial: {}, orientation: {:.0}°)",
                            device_cfg.device_id(),
                            candidate.device_id,
                            device_cfg.orientation
                        );
                        let target = ActiveTarget::new(
                            cfg_idx,
                            cfg_key,
                            candidate.device_id.clone(),
                            lcd,
                            asset,
                            self.tx.clone(),
                        );
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

    fn handle_display_switch_to_desktop(&mut self, device_id: &str) {
        // Find and remove the active LCD target for this device
        let target_idx = self.targets.iter().find_map(|(&idx, t)| {
            if t.device_identity == *device_id {
                Some(idx)
            } else {
                None
            }
        });

        if let Some(idx) = target_idx {
            if let Some(mut target) = self.targets.remove(&idx) {
                target.stop();
                if let LcdBackend::WinUsb(ref mut lcd) = target.lcd {
                    match lcd.switch_to_desktop_mode() {
                        Ok(()) => info!("Switched {device_id} to desktop mode"),
                        Err(e) => warn!("Failed to switch {device_id} to desktop mode: {e}"),
                    }
                } else {
                    warn!("Device {device_id} is not a WinUSB LCD, cannot switch to desktop mode");
                }
            }
        } else {
            // No active target — try opening a temporary connection
            info!("No active LCD target for {device_id}, opening temporary connection");
            let det = self.cached_usb_devices.iter().find(|d| d.device_id == *device_id);
            if let Some(det) = det {
                let family = det.family;
                if let Ok(usb_devs) = lianli_devices::detect::enumerate_devices() {
                    for usb_det in usb_devs {
                        if usb_det.family == family && usb_det.device_id() == *device_id {
                            let screen = lianli_shared::screen::screen_info_for(family)
                                .unwrap_or(lianli_shared::screen::ScreenInfo::AIO_LCD_480);
                            match WinUsbLcdDevice::new(usb_det.device, screen, det.name.as_str()) {
                                Ok(mut lcd) => {
                                    match lcd.switch_to_desktop_mode() {
                                        Ok(()) => info!("Switched {device_id} to desktop mode"),
                                        Err(e) => warn!("Switch to desktop failed: {e}"),
                                    }
                                }
                                Err(e) => warn!("Failed to open {device_id} for mode switch: {e}"),
                            }
                            break;
                        }
                    }
                }
            } else {
                warn!("Device {device_id} not found in cached devices");
            }
        }
    }


    fn stream_target(&mut self, this_asset: Arc<MediaAsset>) {
        // Find ID of matching target
        let target_id = self
            .targets
            .iter()
            .find(|(_, t)| t.asset.config_key == this_asset.config_key)
            .map(|(id, _)| *id);

        if let Some(id) = target_id {
            // 2. retrieve target mutable
            if let Some(target) = self.targets.get_mut(&id) {
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
                        self.handle_usb_error(id, err);
                    }
                    Err(SendError::Other(err)) => {
                        warn!("LCD[{}] media error: {err}", target.index);
                        let mut removed = self.targets.remove(&id).unwrap();
                        removed.stop();
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
        if matches!(err, lianli_transport::TransportError::Timeout) && self.recover_wireless() {
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

    fn send_frame_verified(
        &mut self,
        wireless: &WirelessController,
        builder: &mut PacketBuilder,
        frame: &[u8],
    ) -> anyhow::Result<()> {
        match self {
            Self::WinUsb(d) => d.send_frame_verified(frame),
            _ => self.send_frame(wireless, builder, frame),
        }
    }
}

struct ActiveTarget {
    index: usize,
    key: ConfigKey,
    device_identity: String,
    lcd: LcdBackend,
    media: MediaRuntime,
    asset: Arc<MediaAsset>,
    frame_counter: u64,
}

impl ActiveTarget {
    fn new(
        index: usize,
        key: ConfigKey,
        device_identity: String,
        lcd: LcdBackend,
        asset: Arc<MediaAsset>,
        tx: Option<Sender<DaemonEvent>>,
    ) -> Self {
        Self {
            index,
            key,
            device_identity,
            lcd,
            media: MediaRuntime::from_asset(Arc::clone(&asset), tx),
            asset: asset,
            frame_counter: 0,
        }
    }

    fn matches(&self, identity: &str, key: &ConfigKey) -> bool {
        self.device_identity == identity && key == &self.key
    }

    fn send_frame(
        &mut self,
        wireless: &WirelessController,
        builder: &mut PacketBuilder,
    ) -> Result<bool, SendError> {
        let is_static = matches!(self.media, MediaRuntime::Static { .. });
        let frame = match self.media.next_frame_bytes() {
            Some(bytes) => bytes,
            None => return Ok(false),
        };

        let result = if is_static {
            self.lcd.send_frame_verified(wireless, builder, frame)
        } else {
            self.lcd.send_frame(wireless, builder, frame)
        };
        result.map_err(
            |err| match err.downcast::<lianli_transport::TransportError>() {
                Ok(usb) => SendError::Usb(usb),
                Err(other) => SendError::Other(other),
            },
        )?;

        self.frame_counter += 1;
        Ok(true)
    }

    fn stop(&mut self) {}
}

enum MediaRuntime {
    Static {
        frame: Arc<Vec<u8>>,
    },
    Video {
        #[allow(dead_code)] // We do not read the player, we only store it
        player: Arc<AsyncVideoPlayer>,
        frames: Arc<Vec<Vec<u8>>>,
        sent_frame_index: usize,
    },
    Sensor {
        renderer: Arc<AsyncSensorRenderer>,
        cached_frame: Vec<u8>,
        sent_frame_index: usize,
    },
}

struct AsyncSensorRenderer {
    #[allow(dead_code)] // We'd like to keep the SensorAsset, who knows if we'll need it
    asset: Arc<SensorAsset>,
    current_frame: Arc<Mutex<FrameInfo>>,
    stop_flag: Arc<AtomicBool>,
    _thread: Option<JoinHandle<()>>,
}

impl AsyncSensorRenderer {
    fn new(
        tx: Option<Sender<DaemonEvent>>,
        asset: Arc<SensorAsset>,
        baseasset: Arc<MediaAsset>,
    ) -> Self {
        let initial = match asset.render_frame(true) {
            Ok(frame) => frame.unwrap(),
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

        let asset_for_thread = Arc::clone(&baseasset);
        let tx_for_thread = tx.clone();

        let thread = thread::spawn(move || {

            while !stop_clone.load(Ordering::Relaxed) {
                thread::sleep(update_interval);
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }
                match asset_clone.render_frame(false) {
                    Ok(new_frame) => {
                        if new_frame.is_some() {
                            if let Some(ref tx) = tx_for_thread {
                                let event = DaemonEvent::FrameFinished {
                                    asset: Arc::clone(&asset_for_thread),
                                };

                                tx.send(event).ok();
                            }
                            *frame_clone.lock() = new_frame.unwrap();
                        }
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

    fn get_frame_index(&self) -> usize {
        self.current_frame.lock().frame_index
    }

    fn get_current_frame(&self) -> Vec<u8> {
        self.current_frame.lock().data.clone()
    }

}

impl Drop for AsyncSensorRenderer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

struct AsyncVideoPlayer {
    stop_flag: Arc<AtomicBool>,
    _thread: Option<JoinHandle<()>>,
    frame_index: Arc<AtomicUsize>,
}

impl AsyncVideoPlayer {
    fn new(tx: Option<Sender<DaemonEvent>>, asset: Arc<MediaAsset>) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_flag);

        let tx_for_thread = tx.clone();

        let asset_for_thread = Arc::clone(&asset);

        let min_dur = Duration::from_millis(10);
        let std_dur = Duration::from_millis(100);

        let frame_durations: Vec<Duration> = if let MediaAssetKind::Video {
            frame_durations, ..
        } = &asset.kind
        {
            frame_durations.iter().map(|&d| d.max(min_dur)).collect()

        } else {
            vec![min_dur; 1]
        };

        let frame_index: Arc<AtomicUsize> = Arc::new(0.into());
        let frame_index_cloned= frame_index.clone();

        let thread = thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                let mut frame_cnt=0;
                if let Some(ref tx) = tx_for_thread {
                    frame_cnt = frame_index.fetch_add(1, Ordering::SeqCst);
                    let event = DaemonEvent::FrameFinished {
                        asset: Arc::clone(&asset_for_thread),
                    };
                    
                    tx.send(event).ok();
                }

                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }

                let millis = frame_durations.get(frame_cnt%frame_durations.len());
                thread::sleep(*millis.unwrap_or(&std_dur));
            }
        });

        Self {
            stop_flag,
            _thread: Some(thread),
            frame_index: frame_index_cloned,
        }
    }

    fn get_frame_index(&self) -> usize {
        self.frame_index.load(Ordering::SeqCst)
    }

}

impl Drop for AsyncVideoPlayer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

impl MediaRuntime {
    fn from_asset(asset: Arc<MediaAsset>, tx: Option<Sender<DaemonEvent>>) -> Self {
        match &asset.kind {
            MediaAssetKind::Static { frame } => Self::Static {
                frame: Arc::clone(frame),
            },
            MediaAssetKind::Video { frames, .. } => {
                let player = Arc::new(AsyncVideoPlayer::new(tx, Arc::clone(&asset)));

                Self::Video {
                    player,
                    frames: Arc::clone(frames),
                    sent_frame_index: 0,
                }
            }

            MediaAssetKind::Sensor {
                asset: sensor_asset,
            } => {
                let renderer = Arc::new(AsyncSensorRenderer::new(
                    tx,
                    Arc::clone(sensor_asset),
                    Arc::clone(&asset),
                ));
                let cached_frame = renderer.get_current_frame();
                Self::Sensor {
                    renderer,
                    cached_frame,
                    sent_frame_index: 0,
                }
            }
        }
    }

    fn next_frame_bytes(&mut self) -> Option<&[u8]> {
        match self {
            MediaRuntime::Static { frame } => Some(frame.as_slice()),
            MediaRuntime::Video { player,  frames, sent_frame_index, .. } => {
                let rendered_frame_index = player.get_frame_index();
                trace!("Last sent frame index: {}, most recent rendered frame index : {}", *sent_frame_index,rendered_frame_index);
                if rendered_frame_index<=*sent_frame_index {
                    trace!("==> nothing new, most recent rendered frame already sent to LCD");
                    return None;
                } else if frames.is_empty() {
                    return None
                } else {
                    trace!("==> Ok, a new frame has been rendered, so we sent out this one.");
                    let ret = Some(frames[rendered_frame_index % frames.len()].as_slice());
                    *sent_frame_index=rendered_frame_index;
                    return ret;
                }
            }
            MediaRuntime::Sensor {
                renderer,
                cached_frame,
                sent_frame_index,
                ..
            } => {
                let rendered_frame_index = renderer.get_frame_index();
                trace!("Last sent frame index: {}, most recent rendered frame index : {}", *sent_frame_index,rendered_frame_index);
                if rendered_frame_index<=*sent_frame_index {
                    trace!("==> nothing new, most recent rendered frame already sent to LCD");
                    return None;
                }
                trace!("==> Ok, a new frame has been rendered, so we sent out this one.");
                *cached_frame = renderer.get_current_frame();
                *sent_frame_index=rendered_frame_index;
                Some(cached_frame.as_slice())
            }
        }
    }
}

enum SendError {
    Usb(lianli_transport::TransportError),
    Other(anyhow::Error),
}

fn parse_mac_str(s: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, part) in parts.iter().enumerate() {
        mac[i] = u8::from_str_radix(part, 16).ok()?;
    }
    Some(mac)
}

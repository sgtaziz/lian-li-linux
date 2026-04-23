use crate::aio_controller::AioController;
use crate::fan_controller::FanController;
use crate::ipc_server::{self, DaemonState};
use crate::openrgb_server;
use crate::rgb_controller::RgbController;
use crate::template_store;
use anyhow::Result;
use lianli_devices::crypto::PacketBuilder;
use lianli_devices::detect::{
    create_hid_lcd_device, create_wired_controllers, ensure_hid_devices_bound, enumerate_devices,
    enumerate_hid_devices, open_hid_backend_hidapi, open_hid_backend_rusb, open_hid_lcd_by_vid_pid,
    open_hid_lcd_device_rusb,
};
use lianli_devices::slv3_lcd::Slv3LcdDevice;
use lianli_devices::traits::{FanDevice, LcdDevice};
use lianli_devices::winusb_lcd::WinUsbLcdDevice;
use lianli_devices::wireless::WirelessController;
use lianli_media::sensor::FrameInfo;
use lianli_media::{prepare_media_asset, CustomAsset, MediaAsset, MediaAssetKind, SensorAsset};
use lianli_shared::config::HidDriver;
use lianli_shared::config::{config_identity, AppConfig, ConfigKey, LcdConfig};
use lianli_shared::sensors::SensorInfo;
use lianli_shared::template::LcdTemplate;

fn asset_cache_key(
    device: &LcdConfig,
    user_templates: &[LcdTemplate],
    _sensors: &[SensorInfo],
) -> ConfigKey {
    let base = config_identity(device);
    if device.media_type != MediaType::Custom {
        return base;
    }
    let Some(id) = device.template_id.as_deref() else {
        return base;
    };
    let Some(tpl) = user_templates.iter().find(|t| t.id == id).cloned() else {
        return base;
    };
    let body = serde_json::to_string(&tpl).unwrap_or_default();
    format!("{base}|tpl:{body}")
}
use lianli_shared::device_id::DeviceFamily;
use lianli_shared::ipc::DeviceInfo;
use lianli_shared::media::MediaType;
use lianli_shared::screen::{screen_info_for, ScreenInfo};
use lianli_shared::systeminfo::SysSensor;
use lianli_transport::HidBackend;
use parking_lot::Mutex;
use rusb::Device;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

const DEVICE_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Full USB bus enumeration interval — only needed for hot-plug detection of
/// wired USB devices (LCD, AIO, etc.). Wireless discovery uses its own RX polling.
const USB_ENUM_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum DaemonEvent {
    IpcUpdate, // Somebody changed the DaemonState in the mutex
    USBCheck,
    DevicePoll,
    DisplaySwitch { device_id: String }, // LCD→Desktop. Handled by main event loop.
    DisplaySwitchToLcd { device_id: String, pid: u16 }, // Desktop→LCD. Handled by main event loop.
    Bind { mac_address: String }, // MAC address pending wireless device bind. Handled by main event loop.
    FrameFinished { asset: Arc<MediaAsset> }, // A device has calculated a new frame, let's update the display
    Shutdown, // SIGINT/SIGTERM received, exit the event loop cleanly
}

pub struct ServiceManager {
    config_path: PathBuf,
    config: Option<AppConfig>,
    media_assets: HashMap<usize, Arc<MediaAsset>>,
    targets: HashMap<usize, ActiveTarget>,
    wireless: WirelessController,
    packet_builder: PacketBuilder,
    fan_controller: Option<FanController>,
    aio_controller: Option<AioController>,
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
    last_wireless_count: usize,
    poll_tick: u32,
    restart_requested: bool,
    ipc_state: Arc<Mutex<DaemonState>>, // the (shared) state of the deamon. Shared between daemon itself and IPC thread.
    ipc_stop: Arc<AtomicBool>, // Flag which allows the deamon thread (on shutdown) to tell the IPC thread to stop.
    ipc_thread: Option<JoinHandle<()>>, // Here the deamon thread stores the handle to the IPC thread.
    openrgb_stop: Arc<AtomicBool>,
    openrgb_thread: Option<JoinHandle<()>>,
    openrgb_state: Arc<Mutex<openrgb_server::OpenRgbServerState>>,
    direct_color_buffer: Arc<Mutex<crate::rgb_controller::DirectColorBuffer>>,
    direct_color_writer: Option<JoinHandle<()>>,
    desktop_displays: crate::desktop_display::DesktopDisplayRegistry,
    tx: Option<Sender<DaemonEvent>>,
    mode_switch_suppression: HashMap<String, Instant>,
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
            aio_controller: None,
            rgb_controller: None,
            wired_fan_device_info: Vec::new(),
            wired_fan_devices: Arc::new(HashMap::new()),
            hid_backends: HashMap::new(),
            cached_usb_devices: Vec::new(),
            last_wireless_count: 0,
            poll_tick: 0,
            restart_requested: false,
            ipc_state,
            ipc_stop: Arc::new(AtomicBool::new(false)),
            ipc_thread: None,
            openrgb_stop: Arc::new(AtomicBool::new(false)),
            openrgb_thread: None,
            openrgb_state: Arc::new(Mutex::new(openrgb_server::OpenRgbServerState::default())),
            direct_color_buffer: Arc::new(Mutex::new(
                crate::rgb_controller::DirectColorBuffer::new(),
            )),
            direct_color_writer: None,
            desktop_displays: crate::desktop_display::DesktopDisplayRegistry::new(),
            tx: None,
            mode_switch_suppression: HashMap::new(),
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
        self.hid_backends
            .insert(key.to_string(), Arc::clone(&backend));
        Ok(backend)
    }

    pub fn device_poll(&mut self) {
        // Check for late wireless device discovery
        let current_wireless = self.wireless.devices().len();
        if current_wireless != self.last_wireless_count {
            if current_wireless > self.last_wireless_count {
                info!(
                    "Wireless device count changed ({} -> {}), rebuilding RGB controller",
                    self.last_wireless_count, current_wireless
                );
                self.rebuild_rgb_controller();
                self.ensure_aio_defaults();
                self.restart_fan_control();
                self.start_aio_control();
            }
            self.last_wireless_count = current_wireless;
        }

        self.refresh_targets();
        self.sync_ipc_telemetry();

        // Check HID LCD health every other tick (~2s)
        self.poll_tick = self.poll_tick.wrapping_add(1);
        if self.poll_tick % 2 == 0 {
            for target in self.targets.values_mut() {
                if let LcdBackend::HidLcd(d) = &mut target.lcd {
                    if let Err(e) = d.check_and_recover_lcd() {
                        debug!("LCD[{}] health check error: {e:#}", target.index);
                    }
                }
            }
        }
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
        self.last_wireless_count = self.wireless.devices().len();
        if !self.use_rusb() {
            ensure_hid_devices_bound();
        }
        self.init_wired_devices();
        self.start_openrgb_server();
        self.ensure_aio_defaults();
        self.start_fan_control();
        self.start_aio_control();

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
        SysSensor::init();

        let shutdown_tx = tx.clone();
        thread::spawn(move || {
            use signal_hook::consts::{SIGINT, SIGTERM};
            if let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGINT, SIGTERM]) {
                if let Some(sig) = signals.forever().next() {
                    info!("received signal {sig}, shutting down");
                    let _ = shutdown_tx.send(DaemonEvent::Shutdown);
                }
            }
        });

        for event in rx {
            match event {
                DaemonEvent::Shutdown => {
                    break;
                }
                DaemonEvent::USBCheck => {
                    // Refresh USB device enumeration
                    // Wireless discovery is handled by its own RX polling thread.
                    self.refresh_usb_device_cache();
                }
                DaemonEvent::DevicePoll => {
                    self.device_poll();
                }
                DaemonEvent::DisplaySwitch { device_id } => {
                    self.handle_display_switch_to_desktop(&device_id);
                }
                DaemonEvent::DisplaySwitchToLcd { device_id, pid } => {
                    self.handle_display_switch_to_lcd(&device_id, pid);
                }
                DaemonEvent::Bind {
                    mac_address: mac_str,
                } => {
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
                        if let (Some(aio), Some(cfg)) =
                            (self.aio_controller.as_ref(), self.config.as_ref())
                        {
                            aio.set_config(cfg.clone());
                        } else {
                            self.start_aio_control();
                        }
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

                    // For LCD families whose HID control interface is also registered
                    // by register_wired_controllers (fan/pump/RGB), this entry represents
                    // only the LCD facet — don't duplicate the control-side tags.
                    let lcd_only = matches!(
                        det.family,
                        lianli_shared::device_id::DeviceFamily::HydroShiftLcd
                            | lianli_shared::device_id::DeviceFamily::Galahad2Lcd
                    );

                    cached.push(DeviceInfo {
                        device_id: device_id.clone(),
                        family: det.family,
                        name: det.name.to_string(),
                        serial: Some(device_id),
                        vid: det.vid,
                        pid: det.pid,
                        has_lcd: det.family.has_lcd(),
                        has_fan: det.family.has_fan() && !lcd_only,
                        has_pump: det.family.has_pump() && !lcd_only,
                        has_rgb: det.family.has_rgb() && !lcd_only,
                        has_pump_control: false,
                        fan_count: None,
                        per_fan_control: None,
                        mb_sync_support: false,
                        rgb_zone_count: None,
                        screen_width: screen.map(|s| s.width),
                        screen_height: screen.map(|s| s.height),
                        is_unbound_wireless: false,
                        pump_rpm_range: None,
                    });
                }

                self.cached_usb_devices = cached;
            }
            Err(e) => {
                warn!("USB enumeration failed: {e}");
            }
        }

        match crate::desktop_display::enumerate_turzx() {
            Ok(present) => self.desktop_displays.sync(&present),
            Err(e) => warn!("TURZX enumeration failed: {e:#}"),
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
                mb_sync_support: dev.fan_type.supports_hw_mobo_sync()
                    || self.wireless.motherboard_pwm().is_some(),
                rgb_zone_count: Some(rgb_zone_count),
                screen_width: None,
                screen_height: None,
                is_unbound_wireless: false,
                pump_rpm_range: dev.fan_type.pump_rpm_range(),
            });

            // Update RPM telemetry keyed by device_id
            let device_id = format!("wireless:{}", dev.mac_str());
            let mut rpms: Vec<u16> = dev.fan_rpms[..dev.fan_count as usize].to_vec();
            if is_aio {
                rpms.push(dev.fan_rpms[3]); // pump RPM
            }
            ipc_state.telemetry.fan_rpms.insert(device_id.clone(), rpms);

            if let Some(temp) = dev.coolant_temp_c {
                ipc_state
                    .telemetry
                    .coolant_temps
                    .insert(device_id.clone(), temp as f32);
                lianli_shared::sensors::write_coolant_temp(&device_id, temp as f32);
            }
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
                pump_rpm_range: dev.fan_type.pump_rpm_range(),
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
        self.desktop_displays.shutdown();

        for target in self.targets.values_mut() {
            target.stop();
        }
        self.targets.clear();

        if let Some(fan_controller) = self.fan_controller.take() {
            info!("Stopping fan controller...");
            fan_controller.stop();
        }

        if let Some(aio) = self.aio_controller.take() {
            info!("Stopping AIO controller...");
            aio.stop();
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

    fn start_aio_control(&mut self) {
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
    fn ensure_aio_defaults(&mut self) {
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
        }

        self.init_usb_bulk_rgb_devices(&mut wired_rgb);

        let arc = Arc::new(fan_devices);
        self.wired_fan_devices = Arc::clone(&arc);
        self.init_rgb_controller_from(wired_rgb);
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
                let presets = self.ipc_state.lock().rgb_presets.clone();
                controller.apply_config(rgb_cfg, &presets);
            }
        }

        let rgb_arc = Arc::new(Mutex::new(controller));
        self.rgb_controller = Some(Arc::clone(&rgb_arc));
        self.ipc_state.lock().rgb_controller = Some(rgb_arc);
    }

    /// Rebuild RGB controller to pick up newly discovered wireless devices.
    fn rebuild_rgb_controller(&mut self) {
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
    fn restart_fan_control(&mut self) {
        self.start_fan_control();
    }

    /// Apply RGB config from the current AppConfig to the RGB controller.
    fn apply_rgb_config(&self) {
        if let (Some(ref rgb), Some(ref cfg)) = (&self.rgb_controller, &self.config) {
            if let Some(ref rgb_cfg) = cfg.rgb {
                let presets = self.ipc_state.lock().rgb_presets.clone();
                rgb.lock().apply_config(rgb_cfg, &presets);
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

    /// Try to connect wireless TX/RX once. Non-blocking — if no dongles found, skip gracefully.
    fn try_wireless(&mut self) {
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

    fn prepare_media_assets(&mut self, tx: Sender<DaemonEvent>) {
        let screen_map: HashMap<String, ScreenInfo> = enumerate_devices()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|det| {
                let screen = screen_info_for(det.family)?;
                let id = det.device_id();
                Some((id, screen))
            })
            .collect();

        let all_sensors = lianli_shared::sensors::enumerate_sensors();
        let user_templates = self.ipc_state.lock().user_templates.clone();

        self.media_assets.clear();

        if let Some(cfg) = &self.config {
            for (idx, device) in cfg.lcds.iter().enumerate() {
                let screen = device
                    .serial
                    .as_ref()
                    .and_then(|s| screen_map.get(s).copied())
                    .unwrap_or(ScreenInfo::WIRELESS_LCD);
                let cfg_key = asset_cache_key(device, &user_templates, &all_sensors);
                let device_id = device.device_id();

                match prepare_media_asset(
                    device,
                    cfg.default_fps,
                    &screen,
                    screen.h264,
                    &all_sensors,
                    &user_templates,
                ) {
                    Ok(asset_kind) => {
                        let asset = MediaAsset {
                            kind: asset_kind,
                            config_key: cfg_key,
                        };
                        let asset_arc = Arc::new(asset);
                        self.media_assets.insert(idx, Arc::clone(&asset_arc));

                        match device.media_type {
                            MediaType::Image => info!("Prepared image for LCD[{device_id}]"),
                            MediaType::Video => info!("Prepared video for LCD[{device_id}]"),
                            MediaType::Gif => info!("Prepared GIF for LCD[{device_id}]"),
                            MediaType::Color => info!("Prepared color frame for LCD[{device_id}]"),
                            MediaType::Sensor => info!(
                                "Prepared sensor for LCD[{device_id}]: {}",
                                device
                                    .sensor
                                    .as_ref()
                                    .map(|s| s.label.as_str())
                                    .unwrap_or("<unknown>")
                            ),
                            MediaType::Custom => info!(
                                "Prepared custom template for LCD[{device_id}]: {}",
                                device.template_id.as_deref().unwrap_or("<none>")
                            ),
                            MediaType::Doublegauge | MediaType::Cooler => {}
                        }
                        tx.send(DaemonEvent::FrameFinished { asset: asset_arc })
                            .ok();
                    }
                    Err(err) => warn!("Skipping LCD[{device_id}] media: {err}"),
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
            DeviceFamily::TlLcd,
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

        self.mode_switch_suppression
            .retain(|_, until| Instant::now() < *until);

        if let Ok(usb_devs) = enumerate_devices() {
            for det in usb_devs {
                if !LCD_FAMILIES.contains(&det.family) {
                    continue;
                }
                let device_id = det.device_id();
                if self.mode_switch_suppressed(&device_id) {
                    debug!("LCD candidate skipped (recent mode switch): {device_id}");
                    continue;
                }
                let transport = if lianli_shared::device_id::uses_hid(det.family) {
                    "HID"
                } else {
                    "USB bulk"
                };
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

                let cfg_key = asset.config_key.clone();
                if let Some(mut existing) = self.targets.remove(&cfg_idx) {
                    if existing.matches(&candidate.device_id, &cfg_key) {
                        new_targets.insert(cfg_idx, existing);
                        continue;
                    } else if existing.device_identity == candidate.device_id {
                        // Same device, different config — reuse the USB transport,
                        // just swap the media asset. Reopening the device can leave
                        // some firmware in a bad state.
                        existing.swap_media(Arc::clone(&asset), self.tx.clone());
                        existing.key = cfg_key;
                        new_targets.insert(cfg_idx, existing);
                        if let Some(ref tx) = self.tx {
                            tx.send(DaemonEvent::FrameFinished { asset }).ok();
                        }
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
                        lianli_devices::hydroshift2_lcd::open(device)
                            .map(|d| LcdBackend::WinUsb(ThreadedWinUsbSender::new(d, cfg_idx)))
                    }
                    DeviceFamily::Lancool207 => {
                        let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                        lianli_devices::lancool207::open(device)
                            .map(|d| LcdBackend::WinUsb(ThreadedWinUsbSender::new(d, cfg_idx)))
                    }
                    DeviceFamily::UniversalScreen => {
                        let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                        lianli_devices::universal_screen::open(device)
                            .map(|d| LcdBackend::WinUsb(ThreadedWinUsbSender::new(d, cfg_idx)))
                    }
                    DeviceFamily::HydroShiftLcd
                    | DeviceFamily::Galahad2Lcd
                    | DeviceFamily::TlLcd => {
                        // Try to reuse a shared HID backend (opened by init_rgb_controller).
                        if let Some(backend) = self.hid_backends.get(&candidate.device_id) {
                            match create_hid_lcd_device(
                                candidate.family,
                                candidate.pid,
                                Arc::clone(backend),
                            ) {
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
                            open_hid_lcd_by_vid_pid(candidate.vid, candidate.pid, candidate.family)
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
                            Arc::clone(&asset),
                            self.tx.clone(),
                        );
                        new_targets.insert(cfg_idx, target);
                        if let Some(ref tx) = self.tx {
                            tx.send(DaemonEvent::FrameFinished { asset }).ok();
                        }
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
                        Ok(()) => {
                            info!("Switched {device_id} to desktop mode");
                            self.mark_mode_switch(device_id);
                        }
                        Err(e) => warn!("Failed to switch {device_id} to desktop mode: {e}"),
                    }
                } else {
                    warn!("Device {device_id} is not a WinUSB LCD, cannot switch to desktop mode");
                }
            }
        } else {
            info!("No active LCD target for {device_id}, opening temporary connection");
            let det = self
                .cached_usb_devices
                .iter()
                .find(|d| d.device_id == *device_id);
            if let Some(det) = det {
                let family = det.family;
                if let Ok(usb_devs) = lianli_devices::detect::enumerate_devices() {
                    for usb_det in usb_devs {
                        if usb_det.family == family && usb_det.device_id() == *device_id {
                            let screen = lianli_shared::screen::screen_info_for(family)
                                .unwrap_or(lianli_shared::screen::ScreenInfo::AIO_LCD_480);
                            match WinUsbLcdDevice::new(usb_det.device, screen, det.name.as_str()) {
                                Ok(mut lcd) => match lcd.switch_to_desktop_mode() {
                                    Ok(()) => {
                                        info!("Switched {device_id} to desktop mode");
                                        self.mark_mode_switch(device_id);
                                    }
                                    Err(e) => warn!("Switch to desktop failed: {e}"),
                                },
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

        self.schedule_post_switch_refresh();
    }

    fn handle_display_switch_to_lcd(&mut self, device_id: &str, pid: u16) {
        self.desktop_displays.stop_for_pid(pid);
        self.mark_mode_switch(device_id);

        match hidapi::HidApi::new() {
            Ok(api) => match lianli_devices::display_switcher::switch_to_lcd_mode(&api, pid) {
                Ok(()) => info!("Switched {device_id} to LCD mode"),
                Err(e) => warn!("Failed to switch {device_id} to LCD mode: {e:#}"),
            },
            Err(e) => warn!("Failed to open HID for switch-to-LCD: {e:#}"),
        }

        self.schedule_post_switch_refresh();
    }

    /// Wake the USB cache + device poll a few times in the seconds following a
    /// mode switch, so the rebooted device shows up without waiting for the
    /// next 10-second enumeration tick.
    fn schedule_post_switch_refresh(&self) {
        let Some(tx) = self.tx.clone() else { return };
        thread::spawn(move || {
            for delay_secs in [3u64, 3, 3] {
                thread::sleep(Duration::from_secs(delay_secs));
                if tx.send(DaemonEvent::USBCheck).is_err() {
                    return;
                }
                let _ = tx.send(DaemonEvent::DevicePoll);
            }
        });
    }

    fn mark_mode_switch(&mut self, device_id: &str) {
        self.mode_switch_suppression.insert(
            device_id.to_string(),
            Instant::now() + Duration::from_secs(8),
        );
    }

    fn mode_switch_suppressed(&self, device_id: &str) -> bool {
        self.mode_switch_suppression
            .get(device_id)
            .is_some_and(|until| Instant::now() < *until)
    }

    fn stream_target(&mut self, this_asset: Arc<MediaAsset>) {
        // Find ID of matching target
        let target_id = self
            .targets
            .iter()
            .find(|(_, t)| t.asset.config_key == this_asset.config_key)
            .map(|(id, _)| *id);

        if let Some(id) = target_id {
            if let Some(target) = self.targets.get_mut(&id) {
                match target.send_frame(&self.wireless, &mut self.packet_builder) {
                    Ok(true) => {
                        target.consecutive_errors = 0;
                        if target.frame_counter % 30 == 0 {
                            debug!(
                                "LCD[{}] streamed {} frames",
                                target.index, target.frame_counter
                            );
                        }
                    }
                    Ok(false) => {}
                    Err(SendError::Usb(err)) => {
                        if let Some(target) = self.targets.get_mut(&id) {
                            target.consecutive_errors += 1;
                            if target.consecutive_errors < 3 {
                                warn!(
                                    "LCD[{}] USB error ({}/3): {err}",
                                    target.index, target.consecutive_errors
                                );
                                return;
                            }
                        }
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
    WinUsb(ThreadedWinUsbSender),
    HidLcd(Box<dyn LcdDevice>),
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
            Self::HidLcd(d) => d.send_jpeg_frame(frame),
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
            Self::HidLcd(d) => d.send_static_frame(frame),
            _ => self.send_frame(wireless, builder, frame),
        }
    }
}

enum LcdThreadMsg {
    Frame(Vec<u8>),
    FrameVerified(Vec<u8>, std::sync::mpsc::SyncSender<anyhow::Result<()>>),
    StreamH264 { path: PathBuf, looping: bool },
    SwitchDesktop(std::sync::mpsc::SyncSender<anyhow::Result<()>>),
    Stop,
}

struct ThreadedWinUsbSender {
    tx: std::sync::mpsc::SyncSender<LcdThreadMsg>,
    h264_stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ThreadedWinUsbSender {
    fn new(mut device: WinUsbLcdDevice, index: usize) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<LcdThreadMsg>(2);
        let h264_stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&h264_stop);
        let thread = thread::spawn(move || {
            for msg in rx {
                match msg {
                    LcdThreadMsg::Frame(data) => {
                        if let Err(e) = device.send_frame(&data) {
                            warn!("LCD[{index}] sender thread frame error: {e}");
                        }
                    }
                    LcdThreadMsg::FrameVerified(data, reply) => {
                        let result = device.send_frame_verified(&data);
                        let _ = reply.send(result);
                    }
                    LcdThreadMsg::StreamH264 { path, looping } => {
                        stop_clone.store(false, Ordering::Relaxed);
                        if let Err(e) = device.stream_h264(&path, looping, &stop_clone) {
                            warn!("LCD[{index}] h264 stream error: {e}");
                        }
                    }
                    LcdThreadMsg::SwitchDesktop(reply) => {
                        let result = device.switch_to_desktop_mode();
                        let _ = reply.send(result);
                        break;
                    }
                    LcdThreadMsg::Stop => break,
                }
            }
            device.transport_release();
        });
        Self {
            tx,
            h264_stop,
            thread: Some(thread),
        }
    }

    fn stream_h264(&self, path: PathBuf, looping: bool) -> anyhow::Result<()> {
        self.h264_stop.store(true, Ordering::Relaxed);
        self.tx
            .send(LcdThreadMsg::StreamH264 { path, looping })
            .map_err(|_| anyhow::anyhow!("LCD sender thread exited"))?;
        Ok(())
    }

    fn send_frame(&self, frame: &[u8]) -> anyhow::Result<()> {
        self.h264_stop.store(true, Ordering::Relaxed);
        match self.tx.try_send(LcdThreadMsg::Frame(frame.to_vec())) {
            Ok(()) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                debug!("LCD sender busy, dropping frame");
                Ok(())
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                anyhow::bail!("LCD sender thread exited")
            }
        }
    }

    fn switch_to_desktop_mode(&mut self) -> anyhow::Result<()> {
        self.h264_stop.store(true, Ordering::Relaxed);
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(LcdThreadMsg::SwitchDesktop(reply_tx))
            .map_err(|_| anyhow::anyhow!("LCD sender thread exited"))?;
        let result = reply_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| anyhow::anyhow!("LCD sender thread timeout"))?;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        result
    }

    fn send_frame_verified(&self, frame: &[u8]) -> anyhow::Result<()> {
        self.h264_stop.store(true, Ordering::Relaxed);
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(LcdThreadMsg::FrameVerified(frame.to_vec(), reply_tx))
            .map_err(|_| anyhow::anyhow!("LCD sender thread exited"))?;
        reply_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| anyhow::anyhow!("LCD sender thread timeout"))?
    }

    fn stop(&mut self) {
        self.h264_stop.store(true, Ordering::Relaxed);
        let _ = self.tx.send(LcdThreadMsg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ThreadedWinUsbSender {
    fn drop(&mut self) {
        self.stop();
    }
}

struct ActiveTarget {
    index: usize,
    key: ConfigKey,
    device_identity: String,
    lcd: LcdBackend,
    media: MediaRuntime,
    asset: Arc<MediaAsset>,
    // This variable contains the last seen frame version. Each renderer holds a frame version counter which gets increased each time it actually writes into the frame. The first time it writes into the frame sets the frame version to 1
    // By using this mechanism we are able to detect whether we actually need to send the frame via USB bus to the LCD, and thus we can save quite a lot of time by not sending frames which are already displayed.
    frame_counter: u64,
    consecutive_errors: u32,
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
            asset,
            frame_counter: 0,
            consecutive_errors: 0,
        }
    }

    fn matches(&self, identity: &str, key: &ConfigKey) -> bool {
        self.device_identity == identity && key == &self.key
    }

    /// Replace the media asset without reopening the LCD transport.
    fn swap_media(&mut self, asset: Arc<MediaAsset>, tx: Option<Sender<DaemonEvent>>) {
        self.asset = Arc::clone(&asset);
        self.media = MediaRuntime::from_asset(asset, tx);
        self.frame_counter = 0;
        info!(
            "[devices] LCD[{}] media swapped (keeping transport)",
            self.index
        );
    }

    fn send_frame(
        &mut self,
        wireless: &WirelessController,
        builder: &mut PacketBuilder,
    ) -> Result<bool, SendError> {
        // H264: start the stream on the LCD thread, then it runs autonomously
        if let MediaRuntime::H264 {
            path,
            looping,
            started,
        } = &mut self.media
        {
            if !*started {
                if let LcdBackend::WinUsb(ref sender) = self.lcd {
                    sender
                        .stream_h264(path.clone(), *looping)
                        .map_err(|e| SendError::Other(e))?;
                    *started = true;
                }
            }
            return Ok(true);
        }

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
        #[allow(dead_code)]
        player: Arc<AsyncVideoPlayer>,
        frames: Arc<Vec<Vec<u8>>>,
        sent_frame_index: usize,
    },
    Sensor {
        renderer: Arc<AsyncSensorRenderer>,
        cached_frame: Vec<u8>,
        sent_frame_index: usize,
    },
    H264 {
        path: PathBuf,
        looping: bool,
        started: bool,
    },
    Custom {
        renderer: Arc<AsyncCustomRenderer>,
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
            Ok(Some(frame)) => frame,
            Ok(None) => asset.blank_frame(),
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
                    Ok(Some(new_frame)) => {
                        *frame_clone.lock() = new_frame;
                        if let Some(ref tx) = tx_for_thread {
                            let event = DaemonEvent::FrameFinished {
                                asset: Arc::clone(&asset_for_thread),
                            };
                            if tx.send(event).is_err() {
                                break;
                            }
                        }
                    }
                    Ok(None) => {}
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
        let frame_index_cloned = frame_index.clone();

        let thread = thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                let mut frame_cnt = 0;
                if let Some(ref tx) = tx_for_thread {
                    frame_cnt = frame_index.fetch_add(1, Ordering::SeqCst);
                    let event = DaemonEvent::FrameFinished {
                        asset: Arc::clone(&asset_for_thread),
                    };
                    if tx.send(event).is_err() {
                        break;
                    }
                }

                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }

                let millis = frame_durations.get(frame_cnt % frame_durations.len());
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

struct AsyncCustomRenderer {
    current_frame: Arc<Mutex<FrameInfo>>,
    stop_flag: Arc<AtomicBool>,
    _thread: Option<JoinHandle<()>>,
}

impl AsyncCustomRenderer {
    fn new(
        tx: Option<Sender<DaemonEvent>>,
        asset: Arc<CustomAsset>,
        baseasset: Arc<MediaAsset>,
    ) -> Self {
        let initial = match asset.render_frame(true) {
            Ok(Some(frame)) => frame,
            Ok(None) => asset.blank_frame(),
            Err(err) => {
                warn!("Custom initial render failed: {err}");
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
            let mut next_deadline = Instant::now() + update_interval;
            while !stop_clone.load(Ordering::Relaxed) {
                let now = Instant::now();
                if now < next_deadline {
                    thread::sleep(next_deadline - now);
                }
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }
                next_deadline += update_interval;
                if next_deadline < Instant::now() {
                    next_deadline = Instant::now() + update_interval;
                }
                match asset_clone.render_frame(false) {
                    Ok(Some(new_frame)) => {
                        *frame_clone.lock() = new_frame;
                        if let Some(ref tx) = tx_for_thread {
                            let event = DaemonEvent::FrameFinished {
                                asset: Arc::clone(&asset_for_thread),
                            };
                            if tx.send(event).is_err() {
                                break;
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        warn!("Custom background render failed: {err}");
                    }
                }
            }
        });

        Self {
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

impl Drop for AsyncCustomRenderer {
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
            MediaAssetKind::H264Stream { path, looping, .. } => Self::H264 {
                path: path.clone(),
                looping: *looping,
                started: false,
            },
            MediaAssetKind::Custom {
                asset: custom_asset,
            } => {
                let renderer = Arc::new(AsyncCustomRenderer::new(
                    tx,
                    Arc::clone(custom_asset),
                    Arc::clone(&asset),
                ));

                let cached_frame = renderer.get_current_frame();
                Self::Custom {
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
            MediaRuntime::Video {
                player,
                frames,
                sent_frame_index,
                ..
            } => {
                let rendered_frame_index = player.get_frame_index();
                if rendered_frame_index <= *sent_frame_index || frames.is_empty() {
                    return None;
                }
                let ret = Some(frames[rendered_frame_index % frames.len()].as_slice());
                *sent_frame_index = rendered_frame_index;
                ret
            }
            MediaRuntime::Sensor {
                renderer,
                cached_frame,
                sent_frame_index,
                ..
            } => {
                let rendered_frame_index = renderer.get_frame_index();
                if rendered_frame_index <= *sent_frame_index {
                    return None;
                }
                *cached_frame = renderer.get_current_frame();
                *sent_frame_index = rendered_frame_index;
                Some(cached_frame.as_slice())
            }
            MediaRuntime::Custom {
                renderer,
                cached_frame,
                sent_frame_index,
                ..
            } => {
                let rendered_frame_index = renderer.get_frame_index();
                if rendered_frame_index <= *sent_frame_index {
                    return None;
                }
                *cached_frame = renderer.get_current_frame();
                *sent_frame_index = rendered_frame_index;
                Some(cached_frame.as_slice())
            }
            MediaRuntime::H264 { .. } => None,
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

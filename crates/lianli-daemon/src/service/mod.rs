use crate::aio_controller::AioController;
use crate::fan_controller::FanController;
use crate::ipc_server::{self, DaemonState};
use crate::openrgb_server;
use crate::rgb_controller::RgbController;
use anyhow::Result;
use lianli_devices::crypto::PacketBuilder;
use lianli_devices::detect::ensure_hid_devices_bound;
use lianli_devices::traits::FanDevice;
use lianli_devices::wireless::WirelessController;
use lianli_shared::config::AppConfig;
use lianli_shared::config::HidDriver;
use lianli_shared::ipc::DeviceInfo;
use lianli_shared::systeminfo::SysSensor;
use lianli_transport::HidBackend;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{info, warn};

mod display_mode;
mod init;
mod media;
mod runtime;
mod shutdown;
mod streaming;
mod sync;

use runtime::{parse_mac_str, ActiveTarget, LcdBackend};

const DEVICE_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Full USB bus enumeration interval — only needed for hot-plug detection of
/// wired USB devices (LCD, AIO, etc.). Wireless discovery uses its own RX polling.
const USB_ENUM_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum DaemonEvent {
    IpcUpdate, // Somebody changed the DaemonState in the mutex
    USBCheck,
    DevicePoll,
    DisplaySwitch {
        device_id: String,
    }, // LCD→Desktop. Handled by main event loop.
    DisplaySwitchToLcd {
        device_id: String,
        pid: u16,
    }, // Desktop→LCD. Handled by main event loop.
    Bind {
        mac_address: String,
    }, // MAC address pending wireless device bind. Handled by main event loop.
    Unbind {
        mac_address: String,
    }, // MAC address pending wireless device unbind. Handled by main event loop.
    SetEne6k77FanQuantity {
        device_id: String,
        quantity: u8,
    },
    FrameFinished {
        asset: Arc<lianli_media::MediaAsset>,
    }, // A device has calculated a new frame, let's update the display
    Shutdown, // SIGINT/SIGTERM received, exit the event loop cleanly
}

pub struct ServiceManager {
    config_path: PathBuf,
    config: Option<AppConfig>,
    media_assets: HashMap<usize, Arc<lianli_media::MediaAsset>>,
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
    /// Firmware string + C-command capability per AIO LCD device_id, populated
    /// when the controller attaches and surfaced through DeviceInfo.
    aio_lcd_info: HashMap<String, (Option<String>, bool)>,
    last_wireless_count: usize,
    poll_tick: u32,
    last_poll_mono: Instant,
    last_poll_wall: std::time::SystemTime,
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
            aio_lcd_info: HashMap::new(),
            last_wireless_count: 0,
            poll_tick: 0,
            last_poll_mono: Instant::now(),
            last_poll_wall: std::time::SystemTime::now(),
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
        let backend = lianli_devices::detect::open_hid_backend_rusb(det)?;
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
        let backend = lianli_devices::detect::open_hid_backend_hidapi(api, det)?;
        self.hid_backends
            .insert(key.to_string(), Arc::clone(&backend));
        Ok(backend)
    }

    pub fn device_poll(&mut self) {
        let now_mono = Instant::now();
        let now_wall = std::time::SystemTime::now();
        let mono_elapsed = now_mono.duration_since(self.last_poll_mono);
        let wall_elapsed = now_wall
            .duration_since(self.last_poll_wall)
            .unwrap_or(mono_elapsed);
        self.last_poll_mono = now_mono;
        self.last_poll_wall = now_wall;
        if wall_elapsed > mono_elapsed + Duration::from_secs(5) {
            info!(
                "System resume detected (~{:.0}s sleep), restarting daemon",
                (wall_elapsed - mono_elapsed).as_secs_f32()
            );
            self.restart_requested = true;
            return;
        }

        // Check for late wireless device discovery
        let current_wireless = self.wireless.devices().len();
        if current_wireless != self.last_wireless_count {
            if current_wireless > self.last_wireless_count {
                info!(
                    "Wireless device count changed ({} -> {}), rebuilding RGB controller",
                    self.last_wireless_count, current_wireless
                );
                std::thread::sleep(std::time::Duration::from_millis(500));
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
                if let LcdBackend::HidLcd(d) = &target.lcd {
                    if let Err(e) = d.lock().check_and_recover_lcd() {
                        tracing::debug!("LCD[{}] health check error: {e:#}", target.index);
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
        if self.wireless.has_discovered_devices() {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
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
                    // Force exit if graceful shutdown stalls (e.g. blocking USB
                    // call in a worker thread).
                    thread::sleep(Duration::from_secs(5));
                    warn!("shutdown exceeded 5s grace period, forcing exit");
                    std::process::exit(0);
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
                    if self.restart_requested {
                        break;
                    }
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
                        self.device_poll();
                    } else {
                        warn!("Invalid MAC address for bind: {mac_str}");
                    }
                }
                DaemonEvent::Unbind {
                    mac_address: mac_str,
                } => {
                    if let Some(mac) = parse_mac_str(&mac_str) {
                        if let Err(e) = self.wireless.unbind_device(&mac) {
                            warn!("Failed to unbind wireless device {mac_str}: {e}");
                        }
                        self.device_poll();
                    } else {
                        warn!("Invalid MAC address for unbind: {mac_str}");
                    }
                }
                DaemonEvent::SetEne6k77FanQuantity {
                    device_id,
                    quantity,
                } => {
                    self.handle_set_ene6k77_fan_quantity(&device_id, quantity);
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
}

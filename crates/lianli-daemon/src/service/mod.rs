use crate::ipc::{self, DaemonState};
use anyhow::Result;
use lianli_devices::crypto::PacketBuilder;
use lianli_devices::detect::ensure_hid_devices_bound;
use lianli_devices::wireless::WirelessController;
use lianli_shared::config::AppConfig;
use lianli_shared::systeminfo::SysSensor;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use runtime::LcdBackend;

mod aio_lcd_firmware;
mod display_mode;
mod init;
mod media;
mod renderers;
mod runtime;
mod shutdown;
mod streaming;
mod subsystems;
mod suspend;
mod sync;

use aio_lcd_firmware::AioLcdFirmwareTracker;
use subsystems::{Controllers, DeviceRegistry, IpcSubsystem, OpenRgbSubsystem};

use runtime::ActiveTarget;

/// Parse a colon-separated MAC address string (e.g. `"01:23:45:67:89:AB"`)
/// into a 6-byte array. Returns `None` on malformed input.
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
    Unbind { mac_address: String }, // MAC address pending wireless device unbind. Handled by main event loop.
    SetEne6k77FanQuantity { device_id: String, quantity: u8 },
    FrameFinished,
    RecreateMedia { target_index: usize },
    ResyncWirelessRgb,
    SystemResumed,
    RebootWirelessLcd { mac: [u8; 6] },
    DisableLc217Wifi { mac: [u8; 6], disable: bool },
    SetLcdBrightness { device_id: String, brightness: u8 },
    BindAll,
    UnbindAll,
    Shutdown, // SIGINT/SIGTERM received, exit the event loop cleanly
}

pub struct ServiceManager {
    config_path: PathBuf,
    config: Option<AppConfig>,
    media_assets: HashMap<usize, Arc<lianli_media::MediaAsset>>,
    targets: Arc<Mutex<HashMap<usize, ActiveTarget>>>,
    wireless: WirelessController,
    packet_builder: PacketBuilder,
    /// Wired USB device registry (fan handles, HID backends, hot-plug caches).
    registry: DeviceRegistry,
    /// AIO LCD device IDs with pending deferred firmware reads, plus the
    /// devices whose reads previously failed and should be skipped.
    aio_lcd_firmware: AioLcdFirmwareTracker,
    last_wireless_count: usize,
    last_poll_mono: Instant,
    last_poll_wall: std::time::SystemTime,
    restart_requested: bool,
    /// Background controllers (fan/AIO/RGB) and direct-color flush thread.
    controllers: Controllers,
    /// IPC server thread + shared state.
    ipc: IpcSubsystem,
    /// OpenRGB SDK server thread + shared state.
    openrgb: OpenRgbSubsystem,
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
            targets: Arc::new(Mutex::new(HashMap::new())),
            wireless: WirelessController::new(),
            packet_builder: PacketBuilder::new(),
            registry: DeviceRegistry::new(),
            aio_lcd_firmware: AioLcdFirmwareTracker::new(),
            last_wireless_count: 0,
            last_poll_mono: Instant::now(),
            last_poll_wall: std::time::SystemTime::now(),
            restart_requested: false,
            controllers: Controllers::new(),
            ipc: IpcSubsystem::new(ipc_state),
            openrgb: OpenRgbSubsystem::new(),
            desktop_displays: crate::desktop_display::DesktopDisplayRegistry::new(),
            tx: None,
            mode_switch_suppression: HashMap::new(),
        })
    }

    /// Check if the configured HID driver is rusb.
    ///
    /// Always `true` after the hidapi backend was dropped. Kept as a thin
    /// helper so legacy call sites can stay readable while they're being
    /// migrated off the `use_rusb()` branch pattern during the daemon rewrite.
    fn use_rusb(&self) -> bool {
        true
    }

    /// Stable device ID for a rusb device — uses serial or USB port path.
    fn rusb_device_id(det: &lianli_devices::detect::DetectedDevice) -> String {
        det.device_id()
    }

    /// Process deferred firmware reads for AIO LCD devices.
    /// Called every DevicePoll tick.
    fn process_pending_lcd_firmware(&mut self) {
        let ready = self.aio_lcd_firmware.drain_due();

        for (device_id, enable_512) in ready {
            let mut firmware_result: Option<Result<(Option<String>, bool), anyhow::Error>> = None;
            {
                let mut targets = self.targets.lock();
                if let Some(target) = targets
                    .values_mut()
                    .find(|t| t.device_identity == device_id)
                {
                    if let LcdBackend::HidLcd(ref hid) = target.lcd {
                        let mut guard = hid.lock();
                        match guard.try_read_firmware() {
                            Ok(()) => {
                                let fw = guard.firmware_version_str().map(|s| s.to_string());
                                let supports = guard.supports_c_command();
                                guard.set_use_c_command(enable_512);
                                firmware_result = Some(Ok((fw, supports)));
                            }
                            Err(e) => {
                                firmware_result = Some(Err(e));
                            }
                        }
                    }
                }
            }
            match firmware_result {
                Some(Ok((fw, supports))) => {
                    self.aio_lcd_firmware.record(&device_id, fw, supports);
                    info!("AIO LCD firmware read succeeded for {device_id}");
                }
                Some(Err(e)) => {
                    warn!(
                        "AIO LCD firmware read failed for {device_id}: {e:#}. \
                         Skipping firmware reads for 30 minutes."
                    );
                    self.aio_lcd_firmware.mark_failed(&device_id);
                }
                None => {}
            }
        }
    }

    pub fn device_poll(&mut self) {
        let now_mono = Instant::now();
        let now_wall = std::time::SystemTime::now();
        let _mono_elapsed = now_mono.duration_since(self.last_poll_mono);
        self.last_poll_mono = now_mono;
        self.last_poll_wall = now_wall;

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

        self.check_wired_hotplug();
        self.refresh_targets();
        self.process_pending_lcd_firmware();
        self.check_thermal_alert();
        self.sync_ipc_telemetry();
    }

    /// Check thermal alert state and trigger RGB override/restore if changed.
    fn check_thermal_alert(&self) {
        if let Some(ref rgb) = self.controllers.rgb {
            rgb.lock().check_thermal_override();
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

        suspend::spawn(tx.clone());

        // We need to send these two events to ourselves before load_config, as load_config sets up the assets and already sends FrameFinished-Events
        tx.send(DaemonEvent::USBCheck).ok();
        tx.send(DaemonEvent::DevicePoll).ok();

        // Load config before IPC starts — prevents GUI from getting empty defaults
        self.load_config(tx.clone());
        self.sync_ipc_state();

        // Start IPC server
        let tx_cloned = tx.clone();
        self.ipc.thread = Some(ipc::start_ipc_server(
            Arc::clone(&self.ipc.state),
            Arc::clone(&self.ipc.stop),
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

        // Spawn a thread to regularly check for new known devices.
        let device_tx = tx.clone();
        thread::spawn(move || loop {
            thread::sleep(DEVICE_POLL_INTERVAL);
            if device_tx.send(DaemonEvent::DevicePoll).is_err() {
                break;
            }
        });
        // Spawn the dedicated LCD streaming thread.
        // Polls all targets for new frames so DevicePoll / USB enumeration
        // on the main loop can never block video playback.
        let stream_targets = Arc::clone(&self.targets);
        let stream_main_tx = tx.clone();
        thread::spawn(move || {
            let mut builder = PacketBuilder::new();
            loop {
                let mut to_recreate = Vec::new();
                {
                    let mut targets = stream_targets.lock();
                    for (&id, target) in targets.iter_mut() {
                        match target.send_frame(None, &mut builder) {
                            Ok(true) => {
                                target.consecutive_errors = 0;
                            }
                            Ok(false) => {}
                            Err(runtime::SendError::Usb(err)) => {
                                target.consecutive_errors += 1;
                                if target.consecutive_errors >= 3 {
                                    warn!("LCD[{id}] USB error (3/3): {err}");
                                    to_recreate.push(id);
                                }
                            }
                            Err(runtime::SendError::Other(err)) => {
                                warn!("LCD[{id}] media error: {err}");
                                to_recreate.push(id);
                            }
                        }
                    }
                    for id in &to_recreate {
                        targets.remove(id);
                    }
                }
                for id in to_recreate {
                    stream_main_tx
                        .send(DaemonEvent::RecreateMedia { target_index: id })
                        .ok();
                }
                thread::sleep(Duration::from_millis(1));
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
                    if !self.wireless.is_connected() {
                        self.try_wireless();
                    }
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
                    let ipc_state = self.ipc.state.lock();
                    info!("Config reload triggered via IPC");
                    // Force the config watcher to pick up the new file
                    drop(ipc_state);
                    if self.load_config(tx.clone()) {
                        self.start_fan_control();
                        if let (Some(aio), Some(cfg)) =
                            (self.controllers.aio.as_ref(), self.config.as_ref())
                        {
                            aio.set_config(cfg.clone());
                        } else {
                            self.start_aio_control();
                        }
                        self.start_openrgb_server();
                        if let Some(ref ta) = self.controllers.thermal_alert {
                            if let Some(ref cfg) = self.config {
                                ta.update_settings(cfg.thermal_alert.clone());
                            }
                        }
                        self.sync_ipc_state();

                        self.device_poll();
                    }
                }
                DaemonEvent::FrameFinished => {
                    // Handled by the polling streaming thread — no action needed.
                }
                DaemonEvent::ResyncWirelessRgb => {
                    if let Some(ref rgb) = self.controllers.rgb {
                        let rgb = rgb.lock();
                        if rgb.is_openrgb_controlled() {
                            debug!("OpenRGB server active, skipping wireless drift resync");
                        } else {
                            drop(rgb);
                            self.apply_rgb_config();
                        }
                    }
                }
                DaemonEvent::RecreateMedia { target_index } => {
                    if let Some(asset) = self.media_assets.get(&target_index).cloned() {
                        if let Some(target) = self.targets.lock().get_mut(&target_index) {
                            info!(
                                "[devices] LCD[{}] recreating media after recovery",
                                target.device_identity
                            );
                            target.swap_media(asset, target.custom_h264, self.tx.clone());
                        }
                    }
                }
                DaemonEvent::RebootWirelessLcd { mac } => {
                    if let Err(e) = self.wireless.reboot_lcd_group(&mac) {
                        warn!("Failed to reboot wireless LCD: {e}");
                    }
                }
                DaemonEvent::DisableLc217Wifi { mac, disable } => {
                    if let Err(e) = self.wireless.close_217_wifi(&mac, disable) {
                        warn!("Failed to toggle LC217 wifi: {e}");
                    }
                }
                DaemonEvent::BindAll => {
                    for dev in self.wireless.unbound_devices() {
                        if let Err(e) = self.wireless.bind_device(&dev.mac) {
                            warn!("Failed to bind {}: {e}", dev.mac_str());
                        }
                    }
                    self.device_poll();
                }
                DaemonEvent::UnbindAll => {
                    for dev in self.wireless.devices() {
                        if let Err(e) = self.wireless.unbind_device(&dev.mac) {
                            warn!("Failed to unbind {}: {e}", dev.mac_str());
                        }
                    }
                    self.device_poll();
                }
                DaemonEvent::SetLcdBrightness {
                    device_id,
                    brightness,
                } => {
                    let mut targets = self.targets.lock();
                    if let Some((_, target)) = targets
                        .iter_mut()
                        .find(|(_, t)| t.device_identity == device_id)
                    {
                        if let Err(e) = target.lcd.set_brightness(
                            Some(&self.wireless),
                            &mut self.packet_builder,
                            brightness,
                        ) {
                            warn!("Failed to set LCD brightness for {device_id}: {e}");
                        }
                    }
                }
                DaemonEvent::SystemResumed => {
                    info!("System resumed — waiting for USB re-enumeration");
                    thread::sleep(Duration::from_secs(2));
                    self.rebuild_rgb_controller();
                    self.restart_fan_control();
                    self.start_aio_control();
                    self.sync_ipc_state();
                    info!("Device state re-applied after resume");
                }
            }
        }

        self.shutdown();
        Ok(self.restart_requested)
    }
}

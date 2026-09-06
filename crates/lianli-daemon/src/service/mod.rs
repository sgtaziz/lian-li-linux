use crate::ipc::{self, DaemonState};
use anyhow::Result;
use lianli_devices::crypto::PacketBuilder;
use lianli_devices::wireless::WirelessController;
use lianli_shared::config::AppConfig;
use lianli_shared::systeminfo::SysSensor;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

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

fn event_label(event: &DaemonEvent) -> &'static str {
    match event {
        DaemonEvent::IpcUpdate => "IpcUpdate",
        DaemonEvent::USBCheck => "USBCheck",
        DaemonEvent::DevicePoll => "DevicePoll",
        DaemonEvent::DisplaySwitch { .. } => "DisplaySwitch",
        DaemonEvent::DisplaySwitchToLcd { .. } => "DisplaySwitchToLcd",
        DaemonEvent::Bind { .. } => "Bind",
        DaemonEvent::Unbind { .. } => "Unbind",
        DaemonEvent::SetEne6k77FanQuantity { .. } => "SetEne6k77FanQuantity",
        DaemonEvent::FrameFinished => "FrameFinished",
        DaemonEvent::RecreateMedia { .. } => "RecreateMedia",
        DaemonEvent::ResyncWirelessRgb => "ResyncWirelessRgb",
        DaemonEvent::LcdInitComplete { .. } => "LcdInitComplete",
        DaemonEvent::SystemResumed => "SystemResumed",
        DaemonEvent::RebootWirelessLcd { .. } => "RebootWirelessLcd",
        DaemonEvent::DisableLc217Wifi { .. } => "DisableLc217Wifi",
        DaemonEvent::SetLcdBrightness { .. } => "SetLcdBrightness",
        DaemonEvent::BindAll => "BindAll",
        DaemonEvent::UnbindAll => "UnbindAll",
        DaemonEvent::Shutdown => "Shutdown",
    }
}

struct WatchdogClearGuard(Arc<Mutex<(&'static str, Instant)>>);
impl Drop for WatchdogClearGuard {
    fn drop(&mut self) {
        let mut op = self.0.lock();
        op.0 = "idle";
        op.1 = Instant::now();
    }
}

/// How long graceful shutdown gets before the process is forced down.
/// Must exceed the longest blocking USB call, or the device is left mid-transfer.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(20);

/// Set once ServiceManager::shutdown() returns, so the signal-handler watchdog
/// stands down instead of forcing an exit over a shutdown that already worked.
pub(crate) static SHUTDOWN_DONE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

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
    FrameFinished,
    RecreateMedia {
        target_index: usize,
        device_id: String,
    },
    ResyncWirelessRgb,
    /// Background LCD init finished; the target is rebuilt so the recovery
    /// thread starts with firmware state now known.
    LcdInitComplete {
        device_id: String,
    },
    SystemResumed,
    RebootWirelessLcd {
        mac: [u8; 6],
    },
    DisableLc217Wifi {
        mac: [u8; 6],
        disable: bool,
    },
    SetLcdBrightness {
        device_id: String,
        brightness: u8,
    },
    BindAll,
    UnbindAll,
    Shutdown, // SIGINT/SIGTERM received, exit the event loop cleanly
}

pub struct ServiceManager {
    config_path: PathBuf,
    socket_path: PathBuf,
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
    wireless_stable_count: usize,
    wireless_pending_count: Option<usize>,
    wireless_pending_streak: u32,
    wireless_rebind_in_flight: Arc<AtomicBool>,
    wireless_rebind_last: HashMap<[u8; 6], Instant>,
    wireless_channel_streak: Option<(u8, u32)>,
    wireless_channel_in_flight: Arc<AtomicBool>,
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
    serial_rewrite_backoff: Option<Instant>,
}

impl ServiceManager {
    pub fn new(config_path: PathBuf, socket_path: PathBuf) -> Result<Self> {
        let ipc_state = Arc::new(Mutex::new(DaemonState::new(config_path.clone())));

        Ok(Self {
            config_path,
            socket_path,
            config: None,
            media_assets: HashMap::new(),
            targets: Arc::new(Mutex::new(HashMap::new())),
            wireless: WirelessController::new(),
            packet_builder: PacketBuilder::new(),
            registry: DeviceRegistry::new(),
            aio_lcd_firmware: AioLcdFirmwareTracker::new(),
            wireless_stable_count: 0,
            wireless_pending_count: None,
            wireless_pending_streak: 0,
            wireless_rebind_in_flight: Arc::new(AtomicBool::new(false)),
            wireless_rebind_last: HashMap::new(),
            wireless_channel_streak: None,
            wireless_channel_in_flight: Arc::new(AtomicBool::new(false)),
            last_poll_mono: Instant::now(),
            last_poll_wall: std::time::SystemTime::now(),
            restart_requested: false,
            controllers: Controllers::new(),
            ipc: IpcSubsystem::new(ipc_state),
            openrgb: OpenRgbSubsystem::new(),
            desktop_displays: crate::desktop_display::DesktopDisplayRegistry::new(),
            tx: None,
            mode_switch_suppression: HashMap::new(),
            serial_rewrite_backoff: None,
        })
    }

    fn rusb_device_id(det: &lianli_devices::detect::DetectedDevice) -> String {
        det.device_id()
    }

    /// Process deferred firmware reads for AIO LCD devices.
    /// Called every DevicePoll tick.
    fn process_pending_lcd_firmware(&mut self) {
        let ready = self.aio_lcd_firmware.drain_due();

        for (device_id, enable_512) in ready {
            let lcd: Option<runtime::SharedHidLcd> = {
                let targets = self.targets.lock();
                targets
                    .values()
                    .find(|t| t.device_identity == device_id)
                    .and_then(|t| match &t.lcd {
                        LcdBackend::HidLcd(hid) => Some(Arc::clone(hid)),
                        _ => None,
                    })
            };

            let Some(lcd) = lcd else {
                continue;
            };
            // Defer the read while an H.264 stream runs, reads interrupt playback
            let Some(_idle) = lcd.recovery_idle() else {
                debug!("AIO LCD {device_id}: stream active, deferring firmware read by 10s");
                self.aio_lcd_firmware
                    .schedule(&device_id, Duration::from_secs(10), enable_512);
                continue;
            };
            let Some(mut guard) = lcd.try_lock_for(Duration::from_millis(500)) else {
                debug!("AIO LCD {device_id}: busy, deferring firmware read by 10s");
                self.aio_lcd_firmware
                    .schedule(&device_id, Duration::from_secs(10), enable_512);
                continue;
            };
            match guard.try_read_firmware() {
                Ok(()) => {
                    let fw = guard.firmware_version_str().map(|s| s.to_string());
                    let supports = guard.supports_c_command();
                    guard.set_use_c_command(enable_512);
                    self.aio_lcd_firmware.record(&device_id, fw, supports);
                    info!("AIO LCD firmware read succeeded for {device_id}");
                }
                Err(e) => {
                    warn!(
                        "AIO LCD firmware read failed for {device_id}: {e:#}. \
                         Skipping firmware reads for 30 minutes."
                    );
                    self.aio_lcd_firmware.mark_failed(&device_id);
                }
            }
        }
    }

    pub fn device_poll(&mut self) {
        let now_mono = Instant::now();
        let now_wall = std::time::SystemTime::now();
        let _mono_elapsed = now_mono.duration_since(self.last_poll_mono);
        self.last_poll_mono = now_mono;
        self.last_poll_wall = now_wall;

        // Rebuild wireless-dependent controllers only after the bound-device
        // count holds stable for 3 consecutive polls.
        let current_wireless = self.wireless.devices().len();
        if current_wireless != self.wireless_stable_count {
            match self.wireless_pending_count {
                Some(c) if c == current_wireless => self.wireless_pending_streak += 1,
                _ => {
                    self.wireless_pending_count = Some(current_wireless);
                    self.wireless_pending_streak = 1;
                }
            }
            if self.wireless_pending_streak >= 3 {
                info!(
                    "Wireless device count changed ({} -> {}), rebuilding RGB controller",
                    self.wireless_stable_count, current_wireless
                );
                self.rebuild_rgb_controller();
                self.ensure_aio_defaults();
                self.start_aio_control();
                // The fan controller caches its wireless handle at startup.
                // Without this restart it never sees late discovered
                // wireless devices and every group errors out.
                self.restart_fan_control();
                self.wireless_stable_count = current_wireless;
                self.wireless_pending_count = None;
                self.wireless_pending_streak = 0;
            }
        } else if self.wireless_pending_count.is_some() {
            self.wireless_pending_count = None;
            self.wireless_pending_streak = 0;
        }

        self.run_wireless_rebind_supervisor();
        self.run_wireless_channel_supervisor();

        self.check_wired_hotplug();
        self.reconcile_wired_wireless_binding();
        self.refresh_targets();

        // Retry starting LCD recovery threads that were skipped because the
        // LCD mutex was busy at creation or init completion. The zero wait
        // keeps this cheap for targets that have nothing to do.
        {
            let tx = self.tx.clone();
            let mut targets = self.targets.lock();
            for target in targets.values_mut() {
                target.maybe_start_recovery(tx.clone(), Duration::ZERO);
            }
        }

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

    fn run_wireless_rebind_supervisor(&mut self) {
        if self.wireless_rebind_in_flight.load(Ordering::Relaxed) {
            return;
        }
        let configured = self.configured_wireless_device_ids();
        let now = Instant::now();

        let Some(mac) = self.wireless.rebind_candidates().into_iter().find(|m| {
            let id = format!(
                "wireless:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                m[0], m[1], m[2], m[3], m[4], m[5]
            );
            configured.contains(&id)
                && self
                    .wireless_rebind_last
                    .get(m)
                    .is_none_or(|t| now.duration_since(*t) >= Duration::from_secs(30))
        }) else {
            return;
        };

        info!("Auto-rebinding configured wireless device {:02x?}", mac);
        self.wireless_rebind_last.insert(mac, now);
        self.wireless_rebind_in_flight
            .store(true, Ordering::Relaxed);
        let wireless = self.wireless.clone();
        let in_flight = Arc::clone(&self.wireless_rebind_in_flight);
        thread::spawn(move || {
            if let Err(e) = wireless.bind_device(&mac) {
                warn!("Auto-rebind failed for {:02x?}: {e:#}", mac);
            }
            in_flight.store(false, Ordering::Relaxed);
        });
    }

    /// Move our dongle off a channel shared with another master. The
    /// conflict must persist three consecutive polls before switching so a
    /// briefly powered neighbour does not reshuffle anything.
    fn run_wireless_channel_supervisor(&mut self) {
        if self.wireless_channel_in_flight.load(Ordering::Relaxed) {
            return;
        }
        match self.wireless.arbitration_target() {
            Some(target) => {
                let streak = match self.wireless_channel_streak {
                    Some((t, n)) if t == target => (t, n + 1),
                    _ => (target, 1),
                };
                let ready = streak.1 >= 3;
                self.wireless_channel_streak = Some(streak);
                if !ready {
                    return;
                }
                self.wireless_channel_streak = None;
                self.wireless_channel_in_flight
                    .store(true, Ordering::Relaxed);
                let wireless = self.wireless.clone();
                let in_flight = Arc::clone(&self.wireless_channel_in_flight);
                thread::spawn(move || {
                    if let Err(e) = wireless.switch_channel(target) {
                        warn!("channel arbitration failed: {e:#}");
                    }
                    in_flight.store(false, Ordering::Relaxed);
                });
            }
            None => self.wireless_channel_streak = None,
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
            self.socket_path.clone(),
        ));
        self.try_wireless();
        self.wireless_stable_count = self.wireless.devices().len();
        if self.wireless.has_discovered_devices() {
            std::thread::sleep(std::time::Duration::from_millis(500));
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
                                    to_recreate.push((id, target.device_identity.clone()));
                                }
                            }
                            Err(runtime::SendError::Other(err)) => {
                                warn!("LCD[{id}] media error: {err}");
                                to_recreate.push((id, target.device_identity.clone()));
                            }
                        }
                    }
                    for &(id, _) in &to_recreate {
                        targets.remove(&id);
                    }
                }
                for (id, device_id) in to_recreate {
                    stream_main_tx
                        .send(DaemonEvent::RecreateMedia {
                            target_index: id,
                            device_id,
                        })
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
                    // Raise this first: worker threads sitting in multi-second
                    // USB retry loops poll it and bail out, so shutdown()'s
                    // join() can actually return instead of stalling until the
                    // grace period forces a mid-transfer exit.
                    lianli_transport::usb::SHUTTING_DOWN
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    let _ = shutdown_tx.send(DaemonEvent::Shutdown);
                    // Force exit if graceful shutdown stalls (e.g. blocking USB
                    // call in a worker thread).
                    //
                    // FIX: 5s was not enough and the process died with a bulk
                    // transfer still in flight, which leaves the HydroShift II
                    // MCU waiting for the rest of a transaction it never gets.
                    // It then stops servicing its USB stack entirely — no
                    // descriptors, no bulk — and only a power cycle brings it
                    // back; a libusb reset does not. Worst case here is the
                    // interface-claim retry loop (20 x 250ms) plus a 2s read
                    // timeout, so 5s expired right in the middle of it. Give
                    // graceful shutdown room to actually finish.
                    // Only force the process down if graceful shutdown has not
                    // finished by then; otherwise this watchdog would race the
                    // re-exec path in main() and kill a daemon that is already
                    // on its way out cleanly.
                    let deadline = std::time::Instant::now() + SHUTDOWN_GRACE;
                    while std::time::Instant::now() < deadline {
                        if SHUTDOWN_DONE.load(std::sync::atomic::Ordering::Relaxed) {
                            return;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                    // The final sleep can end just after the deadline even
                    // when shutdown finished during it, so check the flag
                    // once more before forcing the process down.
                    if SHUTDOWN_DONE.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    warn!(
                        "shutdown exceeded {}s grace period, forcing exit — \
                         a USB transfer may be left in flight",
                        SHUTDOWN_GRACE.as_secs()
                    );
                    std::process::exit(0);
                }
            }
        });

        let watchdog_op: Arc<Mutex<(&'static str, Instant)>> =
            Arc::new(Mutex::new(("idle", Instant::now())));
        {
            let wd = Arc::clone(&watchdog_op);
            thread::spawn(move || {
                let mut warned_5 = false;
                let mut warned_30 = false;
                let mut warned_120 = false;
                loop {
                    thread::sleep(Duration::from_secs(1));
                    let (label, since) = *wd.lock();
                    let elapsed = since.elapsed();
                    if elapsed >= Duration::from_secs(120) {
                        if !warned_120 {
                            error!("WATCHDOG: main loop stuck on '{label}' for 2min+");
                            warned_120 = true;
                        }
                    } else if elapsed >= Duration::from_secs(30) {
                        if !warned_30 {
                            warn!("WATCHDOG: main loop stuck on '{label}' for 30s+");
                            warned_30 = true;
                        }
                    } else if elapsed >= Duration::from_secs(5) {
                        if !warned_5 {
                            warn!("WATCHDOG: main loop slow: '{label}' taking 5s+");
                            warned_5 = true;
                        }
                    } else {
                        warned_5 = false;
                        warned_30 = false;
                        warned_120 = false;
                    }
                }
            });
        }

        for event in rx {
            {
                let mut op = watchdog_op.lock();
                op.0 = event_label(&event);
                op.1 = Instant::now();
            }
            let _watchdog_guard = WatchdogClearGuard(Arc::clone(&watchdog_op));
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
                    let ipc_state = self.ipc.state.lock();
                    info!("Config reload triggered via IPC");
                    drop(ipc_state);
                    let old_backend = self.config.as_ref().map(|c| c.hid_backend);
                    if self.load_config(tx.clone()) {
                        if old_backend != self.config.as_ref().map(|c| c.hid_backend) {
                            info!("HID backend changed — requesting daemon restart");
                            self.restart_requested = true;
                            break;
                        }
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
                        let mut rgb = rgb.lock();
                        if rgb.is_openrgb_controlled() {
                            debug!(
                                "OpenRGB server active — resyncing last direct-color frame instead of native effect"
                            );
                            rgb.resync_wireless_direct_colors();
                        } else if rgb.thermal_override_active() {
                            // The drift checker sees the thermal override as
                            // drift from the configured effect. Do not let
                            // the resync fight the alert coloring.
                            debug!("Thermal override active — skipping RGB resync");
                        } else {
                            drop(rgb);
                            self.apply_rgb_config();
                        }
                    }
                }
                DaemonEvent::RecreateMedia {
                    target_index,
                    device_id,
                } => {
                    // ignore stale events from a detached recovery thread
                    // whose target slot has since been reused
                    let matches_current = self
                        .targets
                        .lock()
                        .get(&target_index)
                        .is_some_and(|t| t.device_identity == device_id);
                    if !matches_current {
                        debug!("Ignoring stale RecreateMedia for LCD[{device_id}]");
                    } else if let Some(asset) = self.media_assets.get(&target_index).cloned() {
                        if let Some(target) = self.targets.lock().get_mut(&target_index) {
                            info!(
                                "[devices] LCD[{}] recreating media after recovery",
                                target.device_identity
                            );
                            target.swap_media(asset, target.custom_h264, self.tx.clone());
                        }
                    }
                }
                DaemonEvent::LcdInitComplete { device_id } => {
                    // firmware state is now recorded by the init worker;
                    // start the recovery thread if the target was created
                    // before init finished. Idempotent, no teardown.
                    // Answers from the device are definitive now, and any
                    // brightness deferred while the init worker held the
                    // LCD can be applied.
                    let tx = self.tx.clone();
                    let mut targets = self.targets.lock();
                    if let Some((_, target)) = targets
                        .iter_mut()
                        .find(|(_, t)| t.device_identity == device_id)
                    {
                        target.mark_init_complete();
                        target.maybe_start_recovery(tx, Duration::from_millis(200));
                        target.flush_pending_brightness(
                            Some(&self.wireless),
                            &mut self.packet_builder,
                        );
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
        SHUTDOWN_DONE.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(self.restart_requested)
    }
}

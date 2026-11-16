use crate::ipc::DaemonState;
use anyhow::Result;
use lianli_devices::crypto::PacketBuilder;
use lianli_devices::wireless::WirelessController;
use lianli_shared::config::AppConfig;
use lianli_shared::systeminfo::SysSensor;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use runtime::LcdBackend;

mod aio_lcd_firmware;
mod display_mode;
mod init;
mod lcd_group_recovery;
mod lifecycle_monitor;
mod media;
mod media_preparation;
mod open_workers;
mod pixel_cleaner;
mod renderers;
mod runtime;
mod shutdown;
mod source_preparation;
mod source_retirement;
mod startup_image;
mod streaming;
mod subsystems;
pub(crate) mod suspend;
mod sync;
mod usb_rescan;
mod wired_identity;
mod wireless_rebind;
mod wireless_recovery;

use aio_lcd_firmware::AioLcdFirmwareTracker;
use lifecycle_monitor::OperationMonitor;
pub use lifecycle_monitor::SignalMonitor;
use subsystems::{Controllers, DeviceRegistry, IpcSubsystem, OpenRgbSubsystem};

use runtime::ActiveTarget;

fn event_label(event: &DaemonEvent) -> &'static str {
    match event {
        DaemonEvent::Coordinated { .. } => "IpcMutation",
        DaemonEvent::IpcUpdate => "IpcUpdate",
        DaemonEvent::RetryOpenRgb => "RetryOpenRgb",
        DaemonEvent::RetryMedia => "RetryMedia",
        DaemonEvent::ClearStartupImageRecovery { .. } => "ClearStartupImageRecovery",
        DaemonEvent::USBCheck => "USBCheck",
        DaemonEvent::DevicePoll => "DevicePoll",
        DaemonEvent::DisplaySwitch { .. } => "DisplaySwitch",
        DaemonEvent::DisplaySwitchToLcd { .. } => "DisplaySwitchToLcd",
        DaemonEvent::Bind { .. } => "Bind",
        DaemonEvent::Unbind { .. } => "Unbind",
        DaemonEvent::SetEne6k77FanQuantity { .. } => "SetEne6k77FanQuantity",
        DaemonEvent::FrameFinished => "FrameFinished",
        DaemonEvent::MediaPrepared => "MediaPrepared",
        DaemonEvent::RecreateMedia { .. } => "RecreateMedia",
        DaemonEvent::RemoveFailedLcd { .. } => "RemoveFailedLcd",
        DaemonEvent::RetryDesktopDisplay { .. } => "RetryDesktopDisplay",
        DaemonEvent::MediaPlaybackStopped { .. } => "MediaPlaybackStopped",
        DaemonEvent::ResyncWirelessRgb => "ResyncWirelessRgb",
        DaemonEvent::LcdInitComplete { .. } => "LcdInitComplete",
        DaemonEvent::SystemResumed => "SystemResumed",
        DaemonEvent::RebootWirelessLcd { .. } => "RebootWirelessLcd",
        DaemonEvent::DisableLc217Wifi { .. } => "DisableLc217Wifi",
        DaemonEvent::SetLcdBrightness { .. } => "SetLcdBrightness",
        DaemonEvent::StartPixelClean { .. } => "StartPixelClean",
        DaemonEvent::UploadStartupImage { .. } => "UploadStartupImage",
        DaemonEvent::StopPixelClean { .. } => "StopPixelClean",
        DaemonEvent::BindAll => "BindAll",
        DaemonEvent::UnbindAll => "UnbindAll",
        DaemonEvent::Shutdown => "Shutdown",
    }
}

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

/// Coalesces slow polls so they cannot build a backlog ahead of IPC events.
struct PollGate(AtomicBool);

impl PollGate {
    const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    fn try_queue(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }

    fn dequeued(&self) {
        self.0.store(false, Ordering::Release);
    }
}

static DEVICE_POLL_GATE: PollGate = PollGate::new();

fn queue_startup_events(
    tx: &std::sync::mpsc::Sender<DaemonEvent>,
    gate: &PollGate,
) -> Result<(), std::sync::mpsc::SendError<DaemonEvent>> {
    tx.send(DaemonEvent::USBCheck)?;
    if gate.try_queue() {
        tx.send(DaemonEvent::DevicePoll)?;
    }
    Ok(())
}

/// Full USB bus enumeration interval — only needed for hot-plug detection of
/// wired USB devices (LCD, AIO, etc.). Wireless discovery uses its own RX polling.
const USB_ENUM_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum DaemonEvent {
    UploadStartupImage {
        id: u64,
        device_id: String,
        jpeg: Vec<u8>,
        capabilities: lianli_shared::startup_image::StartupImageCapabilities,
        cancel: Arc<AtomicBool>,
    },
    Coordinated {
        event: Box<DaemonEvent>,
        permit: Arc<lianli_control::write_gate::ServiceWritePermit>,
    },
    IpcUpdate, // Somebody changed the DaemonState in the mutex
    RetryOpenRgb,
    RetryMedia,
    ClearStartupImageRecovery {
        device_id: String,
    },
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
        operation_id: String,
    },
    Unbind {
        mac_address: String,
        operation_id: String,
    },
    SetEne6k77FanQuantity {
        device_id: String,
        quantity: u8,
        reply: std::sync::mpsc::SyncSender<anyhow::Result<()>>,
    },
    FrameFinished,
    MediaPrepared,
    RetryDesktopDisplay {
        bus: u8,
        address: u8,
        product_id: u16,
    },
    MediaPlaybackStopped {
        target_index: usize,
        key: String,
        error: lianli_shared::ui_message::UiText,
    },
    RecreateMedia {
        target_index: usize,
        device_id: String,
    },
    RemoveFailedLcd {
        target_index: usize,
        removal: std::sync::Weak<()>,
        key: String,
        error: lianli_shared::ui_message::UiText,
    },
    ResyncWirelessRgb,
    LcdInitComplete {
        device_id: String,
        attachment: std::sync::Weak<()>,
        error: Option<String>,
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
        request_id: Option<String>,
        deadline: Instant,
        reply: std::sync::mpsc::SyncSender<Result<bool, String>>,
    },
    StartPixelClean {
        device_id: Option<String>,
        duration_minutes: u16,
        preparation_id: Option<u64>,
        deadline: Instant,
        reply: std::sync::mpsc::SyncSender<Result<(u64, bool), lianli_shared::ui_message::UiError>>,
    },
    StopPixelClean {
        device_id: Option<String>,
        session_id: Option<u64>,
        reply: Option<std::sync::mpsc::SyncSender<bool>>,
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
    media_settings: HashMap<usize, lianli_shared::config::LcdConfig>,
    media_requested_keys: Vec<lianli_shared::config::ConfigKey>,
    media_targets: HashMap<usize, media_preparation::MediaTarget>,
    media_asset_targets: HashMap<usize, media_preparation::MediaTarget>,
    media_reload_pending: bool,
    media_preparation: media_preparation::MediaPreparation,
    targets: Arc<Mutex<HashMap<usize, ActiveTarget>>>,
    wireless: WirelessController,
    wireless_recovery: wireless_recovery::WirelessRecovery,
    wireless_rx_init_pending: bool,
    packet_builder: PacketBuilder,
    /// Wired USB device registry (fan handles, HID backends, hot-plug caches).
    registry: DeviceRegistry,
    /// AIO LCD device IDs with pending deferred firmware reads, plus the
    /// devices whose reads previously failed and should be skipped.
    aio_lcd_firmware: AioLcdFirmwareTracker,
    wireless_stable_count: usize,
    wireless_pending_count: Option<usize>,
    wireless_pending_streak: u32,
    wireless_topology_mismatch_streak: u32,
    wireless_rebind_job: Option<wireless_rebind::RebindJob>,
    wireless_rebind: wireless_rebind::RebindSchedule,
    wireless_channel_streak: Option<(u8, u32)>,
    wireless_channel_in_flight: Arc<AtomicBool>,
    resume_detector: suspend::ResumeDetector,
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
    lcd_transfer_recovery: HashMap<String, runtime::TransferRecovery>,
    post_switch_refresh: display_mode::PostSwitchRefresh,
    usb_rescan: usb_rescan::UsbRescanPolicy,
    display_switch: Option<display_mode::DisplaySwitch>,
    serial_rewrite_backoff: Option<Instant>,
    pixel_clean_sessions: Vec<crate::pixel_cleaner::PixelCleanSession>,
    pixel_clean_preparation: Option<pixel_cleaner::PixelCleanPreparation>,
    startup_image_job: Option<startup_image::StartupImageJob>,
    startup_config_pending: bool,
    startup_image_quarantine: HashSet<String>,
    startup_absent_since: HashMap<String, Instant>,
    cleaner_reload_pending: bool,
    lcd_group_recovery: lcd_group_recovery::LcdGroupRecovery,
}

impl ServiceManager {
    pub fn set_service_invocation(&mut self, invocation: Option<String>) {
        self.ipc.state.lock().info.service_invocation = invocation;
    }

    pub fn set_ownership_lock(&mut self, identity: lianli_shared::daemon::FileIdentity) {
        let mut state = self.ipc.state.lock();
        state.info.ownership_lock = Some(identity);
        state.info.capabilities.push("ownership_lock".into());
    }

    pub fn new(
        config_path: PathBuf,
        socket_path: PathBuf,
        mode: lianli_shared::daemon::DaemonMode,
    ) -> Result<Self> {
        let mut state = DaemonState::new(config_path.clone());
        state.info.mode = mode;
        let ipc_state = Arc::new(Mutex::new(state));
        startup_image::clear_previous_recovery(&config_path)?;
        let startup_image_quarantine = HashSet::new();

        Ok(Self {
            config_path,
            socket_path,
            config: None,
            media_assets: HashMap::new(),
            media_settings: HashMap::new(),
            media_requested_keys: Vec::new(),
            media_targets: HashMap::new(),
            media_asset_targets: HashMap::new(),
            media_reload_pending: false,
            media_preparation: Default::default(),
            targets: Arc::new(Mutex::new(HashMap::new())),
            wireless: WirelessController::new(),
            wireless_recovery: Default::default(),
            wireless_rx_init_pending: false,
            packet_builder: PacketBuilder::new(),
            registry: DeviceRegistry::new(),
            aio_lcd_firmware: AioLcdFirmwareTracker::new(),
            wireless_stable_count: 0,
            wireless_pending_count: None,
            wireless_pending_streak: 0,
            wireless_topology_mismatch_streak: 0,
            wireless_rebind_job: None,
            wireless_rebind: Default::default(),
            wireless_channel_streak: None,
            wireless_channel_in_flight: Arc::new(AtomicBool::new(false)),
            resume_detector: suspend::ResumeDetector::new(),
            restart_requested: false,
            controllers: Controllers::new(),
            ipc: IpcSubsystem::new(ipc_state),
            openrgb: OpenRgbSubsystem::new(),
            desktop_displays: crate::desktop_display::DesktopDisplayRegistry::new(),
            tx: None,
            mode_switch_suppression: HashMap::new(),
            lcd_transfer_recovery: HashMap::new(),
            post_switch_refresh: display_mode::PostSwitchRefresh::default(),
            usb_rescan: usb_rescan::UsbRescanPolicy::default(),
            display_switch: None,
            serial_rewrite_backoff: None,
            pixel_clean_sessions: Vec::new(),
            pixel_clean_preparation: None,
            startup_image_job: None,
            startup_config_pending: false,
            startup_image_quarantine,
            startup_absent_since: HashMap::new(),
            cleaner_reload_pending: false,
            lcd_group_recovery: Default::default(),
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
            let (lcd, initializing) = {
                let targets = self.targets.lock();
                let target = targets.values().find(|t| t.device_identity == device_id);
                let lcd = target
                    .filter(|t| t.is_initialized())
                    .and_then(|t| match &t.lcd {
                        LcdBackend::HidLcd(hid) => Some(Arc::clone(hid)),
                        _ => None,
                    });
                (lcd, target.is_some_and(|t| t.is_initializing()))
            };

            let Some(lcd) = lcd else {
                if initializing {
                    self.aio_lcd_firmware
                        .schedule(&device_id, Duration::from_secs(10), enable_512);
                }
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
        self.poll_wireless_recovery();
        self.lcd_group_recovery.poll();
        self.poll_prepared_media();
        if self.cleaner_reload_pending {
            if let Some(tx) = &self.tx {
                let _ = tx.send(DaemonEvent::IpcUpdate);
                self.cleaner_reload_pending = false;
            }
        }
        if self.resume_detector.poll() {
            if let Some(tx) = &self.tx {
                let _ = tx.send(DaemonEvent::SystemResumed);
            }
        }

        // Rebuild wireless-dependent controllers only after the bound-device
        // count holds stable for 3 consecutive polls.
        let wireless_devices = self.wireless.devices();
        let current_wireless = wireless_devices.len();
        if current_wireless != self.wireless_stable_count {
            // A device-count change is a different kind of event than a
            // topology mismatch on a stable count; let it interrupt a
            // topology-mismatch streak in progress rather than leave stale
            // progress that a later, unrelated topology mismatch could
            // complete after too few genuinely consecutive polls.
            self.wireless_topology_mismatch_streak = 0;
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
            self.wireless_topology_mismatch_streak = 0;
        } else if self
            .controllers
            .rgb
            .as_ref()
            .is_some_and(|rgb| !rgb.lock().wireless_topology_matches(&wireless_devices))
        {
            // A single glitched discovery read must not immediately tear
            // down and repaint RGB state - require the mismatch to persist
            // for 3 consecutive polls, mirroring the debounce already used
            // for device-count changes above.
            self.wireless_topology_mismatch_streak += 1;
            if self.wireless_topology_mismatch_streak >= 3 {
                self.rebuild_rgb_controller();
                self.wireless_topology_mismatch_streak = 0;
            }
        } else {
            self.wireless_topology_mismatch_streak = 0;
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
                target.flush_pending_brightness(Some(&self.wireless), &mut self.packet_builder);
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
        if let Some((mac, job)) = self
            .wireless_rebind_job
            .take_if(|(_, job)| job.is_finished())
        {
            let now = Instant::now();
            let (transmitted, error) = job
                .join()
                .unwrap_or_else(|_| (false, Some("rebind thread panicked".into())));
            if transmitted {
                if let Some(error) = &error {
                    warn!("Auto-rebind failed for {:02x?}: {error}", mac);
                }
                let attempt = self.wireless_rebind.record_attempt(mac, now);
                if attempt == wireless_rebind::MAX_ATTEMPTS {
                    warn!(
                        "Auto-rebind attempts exhausted for {:02x?}; rebind it manually if it stays unreachable",
                        mac
                    );
                }
            } else {
                let error = error.unwrap_or_default();
                if self.wireless_rebind.defer(mac, now, &error) {
                    warn!("Auto-rebind for {:02x?} deferred: {error}", mac);
                } else {
                    debug!("Auto-rebind for {:02x?} deferred again: {error}", mac);
                }
            }
        }
        if self.wireless_rebind_job.is_some() || !self.wireless.is_connected() {
            return;
        }
        let wireless = &self.wireless;
        self.wireless_rebind
            .retain(|mac| !wireless.reports_valid_address(mac));
        let configured = self.configured_wireless_device_ids();
        let now = Instant::now();

        let mut candidates = self.wireless.rebind_candidates().into_iter();

        let Some(mac) = candidates.find(|m| {
            let id = format!(
                "wireless:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                m[0], m[1], m[2], m[3], m[4], m[5]
            );
            configured.contains(&id) && self.wireless_rebind.due(m, now)
        }) else {
            return;
        };

        let attempt = self.wireless_rebind.attempts(&mac) + 1;
        if self.wireless_rebind.deferred(&mac) {
            debug!("Retrying deferred auto-rebind for {:02x?}", mac);
        } else {
            info!(
                "Auto-rebinding configured wireless device {:02x?} ({attempt}/{})",
                mac,
                wireless_rebind::MAX_ATTEMPTS
            );
        }
        let wireless = self.wireless.clone();
        let job = thread::spawn(move || {
            let result = wireless.bind_device(&mac);
            (
                lianli_devices::wireless::bind_transmitted(&result),
                result.err().map(|error| format!("{error:#}")),
            )
        });
        self.wireless_rebind_job = Some((mac, job));
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
    pub fn run(&mut self, signals: &SignalMonitor) -> Result<bool> {
        let mut monitor = OperationMonitor::new()?;
        let startup = monitor.enter("startup");
        info!("=====================================================================");
        info!("LIAN LI LINUX DAEMON");
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
        signals.attach(tx.clone());

        self.tx = Some(tx.clone());

        queue_startup_events(&tx, &DEVICE_POLL_GATE)?;

        self.initialize_runtime(tx.clone(), signals);
        drop(startup);
        if signals.requested() {
            let _shutdown = monitor.enter("shutdown");
            self.shutdown();
            return Ok(false);
        }

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
            if !DEVICE_POLL_GATE.try_queue() {
                continue;
            }
            if device_tx.send(DaemonEvent::DevicePoll).is_err() {
                break;
            }
        });
        let stream_targets = Arc::clone(&self.targets);
        let stream_main_tx = tx.clone();
        let mut stream_worker = streaming::StreamingWorker::spawn(move |stop| {
            let mut builder = PacketBuilder::new();
            let mut source_preparation = source_preparation::SourcePreparation::default();
            let source_retirement = source_retirement::SourceRetirement::new();
            let mut source_start_error = None;
            while !stop.load(Ordering::Acquire) {
                let mut prepared = source_preparation
                    .poll()
                    .or_else(|| source_start_error.take());
                let mut source_request = None;
                let mut to_recreate = Vec::new();
                let mut stopped = Vec::new();
                let idle = {
                    let mut targets = stream_targets.lock();
                    for target in targets.values_mut() {
                        if let Some(source) = target.take_finished_retirement() {
                            if let Err(source) = source_retirement.try_retire(source) {
                                target.return_retirement(source);
                            }
                        }
                    }
                    if !stop.load(Ordering::Acquire) {
                        if let Some(result) = prepared.as_ref() {
                            if let Some(target) = targets.get_mut(&result.index) {
                                if target.accepts_source(result) {
                                    target.install_source(prepared.take().unwrap());
                                }
                            }
                        }
                    }
                    let now = Instant::now();
                    for (&id, target) in targets.iter_mut() {
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        if let Some(event) = target.due_transfer_recovery(now) {
                            to_recreate.push(event);
                            continue;
                        }
                        if !source_preparation.is_busy() && source_request.is_none() {
                            source_request = target.source_request();
                        }
                        match target.send_frame(None, &mut builder) {
                            Ok(true) => {
                                target.consecutive_errors = 0;
                            }
                            Ok(false) => {}
                            Err(
                                runtime::SendError::Stopped(error)
                                | runtime::SendError::Interrupted(error),
                            ) => {
                                if let Some(key) = target.playback_failure_key() {
                                    stopped.push((id, key, error));
                                }
                            }
                            Err(runtime::SendError::Usb(err)) => {
                                target.consecutive_errors += 1;
                                if target.consecutive_errors >= 3 {
                                    warn!("LCD[{id}] USB error (3/3): {err}");
                                    if let Some(event) =
                                        target.removal_event(runtime::failure_text(
                                            "status.media.usbTransferFailed",
                                            "USB transfer failed after three attempts",
                                            &err,
                                        ))
                                    {
                                        to_recreate.push(event);
                                    }
                                }
                            }
                            Err(runtime::SendError::Other(err)) => {
                                warn!("LCD[{id}] media error: {err}");
                                if let Some(event) = target.removal_event(runtime::failure_text(
                                    "status.media.playbackFailed",
                                    "Media playback failed",
                                    &err,
                                )) {
                                    to_recreate.push(event);
                                }
                            }
                        }
                    }
                    targets.values().all(|target| !target.is_initialized())
                };
                drop(prepared);
                if !stop.load(Ordering::Acquire) {
                    if let Some(request) = source_request {
                        let index = request.index;
                        let selection = request.selection.clone();
                        if let Err(error) = source_preparation.start(request) {
                            source_start_error = Some(source_preparation::SourceResult {
                                index,
                                selection,
                                result: Err(format!("Cannot start media preparation: {error}"))
                                    .into(),
                            });
                        }
                    }
                }
                for (target_index, key, error) in stopped {
                    let _ = stream_main_tx.send(DaemonEvent::MediaPlaybackStopped {
                        target_index,
                        key,
                        error,
                    });
                }
                for event in to_recreate {
                    let _ = stream_main_tx.send(event);
                }
                thread::park_timeout(Duration::from_millis(if idle { 100 } else { 1 }));
            }
        });

        SysSensor::init();

        for event in rx {
            let (event, _write_permit) = match event {
                DaemonEvent::Coordinated { event, permit } => (*event, Some(permit)),
                event => (event, None),
            };
            if signals.requested() {
                break;
            }
            let _operation = monitor.enter(event_label(&event));
            match event {
                DaemonEvent::Coordinated { .. } => {
                    warn!("Nested IPC mutation was rejected");
                }
                DaemonEvent::Shutdown => {
                    break;
                }
                DaemonEvent::USBCheck => {
                    self.rescan_stale_usb_devices();
                    self.refresh_usb_device_cache();
                    self.supervise_wireless();
                }
                DaemonEvent::DevicePoll => {
                    DEVICE_POLL_GATE.dequeued();
                    self.poll_startup_image();
                    self.refresh_after_mode_switch();
                    self.check_pixel_clean_sessions();
                    self.device_poll();
                    if self.restart_requested {
                        break;
                    }
                }
                DaemonEvent::UploadStartupImage {
                    id,
                    device_id,
                    jpeg,
                    capabilities,
                    cancel,
                } => {
                    if let Err(error) = self.start_startup_image(
                        id,
                        device_id,
                        jpeg,
                        capabilities,
                        cancel,
                        _write_permit,
                    ) {
                        self.finish_startup_status(
                            id,
                            lianli_shared::startup_image::StartupImageState::Failed {
                                message: error.to_string(),
                                message_ui: lianli_shared::ui_message::ui_message_of(&error),
                            },
                        );
                    }
                }
                DaemonEvent::DisplaySwitch { device_id } => {
                    self.handle_display_switch_to_desktop(&device_id);
                }
                DaemonEvent::DisplaySwitchToLcd { device_id, pid } => {
                    self.handle_display_switch_to_lcd(&device_id, pid);
                }
                DaemonEvent::Bind {
                    mac_address,
                    operation_id,
                } => {
                    let result = parse_mac_str(&mac_address)
                        .ok_or_else(|| crate::ipc::wireless::invalid_mac().into())
                        .and_then(|mac| self.wireless.bind_device(&mac));
                    if let Err(error) = &result {
                        warn!("Failed to bind wireless device {mac_address}: {error:#}");
                    }
                    self.ipc
                        .state
                        .lock()
                        .wireless_operations
                        .complete(&operation_id, result);
                    self.device_poll();
                }
                DaemonEvent::Unbind {
                    mac_address,
                    operation_id,
                } => {
                    let result = parse_mac_str(&mac_address)
                        .ok_or_else(|| crate::ipc::wireless::invalid_mac().into())
                        .and_then(|mac| self.wireless.unbind_device(&mac));
                    if let Err(error) = &result {
                        warn!("Failed to unbind wireless device {mac_address}: {error:#}");
                    }
                    self.ipc
                        .state
                        .lock()
                        .wireless_operations
                        .complete(&operation_id, result);
                    self.device_poll();
                }
                DaemonEvent::SetEne6k77FanQuantity {
                    device_id,
                    quantity,
                    reply,
                } => {
                    let result = self.handle_set_ene6k77_fan_quantity(&device_id, quantity);
                    let _ = reply.send(result);
                }
                DaemonEvent::IpcUpdate => {
                    if self.startup_image_job.is_some() {
                        // Preserve cooling ownership until the storage transaction releases USB.
                        self.startup_config_pending = true;
                        continue;
                    }
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
                        if !self
                            .controllers
                            .fan
                            .as_ref()
                            .zip(self.config.as_ref())
                            .is_some_and(|(controller, config)| controller.matches_config(config))
                        {
                            self.start_fan_control();
                        }
                        if let (Some(aio), Some(cfg)) =
                            (self.controllers.aio.as_ref(), self.config.as_ref())
                        {
                            aio.set_config(cfg.clone());
                        } else {
                            self.start_aio_control();
                        }
                        self.start_openrgb_server();
                        self.apply_ene6k77_quantities();
                        self.apply_rgb_config();
                        if let Some(ref ta) = self.controllers.thermal_alert {
                            if let Some(ref cfg) = self.config {
                                ta.update_settings(cfg.thermal_alert.clone());
                            }
                        }
                        self.sync_ipc_state();

                        self.device_poll();
                    }
                }
                DaemonEvent::RetryOpenRgb => {
                    let failed = self.openrgb.state.lock().error.is_some()
                        || self
                            .openrgb
                            .thread
                            .as_ref()
                            .is_some_and(|thread| thread.is_finished());
                    if failed {
                        self.start_openrgb_server();
                    }
                    self.ipc.state.lock().openrgb_retry_pending = false;
                }
                DaemonEvent::RetryMedia => {
                    self.poll_prepared_media();
                }
                DaemonEvent::ClearStartupImageRecovery { device_id } => {
                    if let Err(error) = self.clear_startup_recovery(&device_id) {
                        warn!(%device_id, %error, "Could not clear startup image recovery");
                    }
                }
                DaemonEvent::FrameFinished => {
                    stream_worker.wake();
                }
                DaemonEvent::MediaPrepared => {
                    self.poll_prepared_media();
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
                            rgb.resync_wireless_effects();
                        }
                    }
                }
                DaemonEvent::MediaPlaybackStopped {
                    target_index,
                    key,
                    error,
                } => {
                    self.record_playback_failure(target_index, &key, error);
                }
                DaemonEvent::RetryDesktopDisplay {
                    bus,
                    address,
                    product_id,
                } => {
                    self.desktop_displays.retry((bus, address), product_id);
                }
                DaemonEvent::RemoveFailedLcd {
                    target_index,
                    removal,
                    key,
                    error,
                } => {
                    let removed = {
                        let mut targets = self.targets.lock();
                        if targets
                            .get(&target_index)
                            .is_some_and(|target| target.matches_removal(&removal))
                        {
                            targets.remove(&target_index)
                        } else {
                            None
                        }
                    };
                    if let Some(mut target) = removed {
                        if let Some(recovery) = target.take_transfer_recovery() {
                            self.lcd_transfer_recovery
                                .insert(target.device_identity.clone(), recovery);
                        }
                        self.record_playback_failure(target_index, &key, error);
                        drop(target);
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
                DaemonEvent::LcdInitComplete {
                    device_id,
                    attachment,
                    error,
                } => {
                    let tx = self.tx.clone();
                    let mut targets = self.targets.lock();
                    if let Some((_, target)) = targets.iter_mut().find(|(_, t)| {
                        t.device_identity == device_id
                            && matches!(
                                &t.lcd,
                                LcdBackend::HidLcd(lcd)
                                    if lcd.matches_attachment(&attachment)
                            )
                    }) {
                        target.finish_initialization(error.as_deref());
                        target.maybe_start_recovery(tx, Duration::from_millis(200));
                        target.flush_pending_brightness(
                            Some(&self.wireless),
                            &mut self.packet_builder,
                        );
                        drop(targets);
                        self.ipc
                            .state
                            .lock()
                            .state_health
                            .lcd_initialization(&device_id, error.as_deref());
                        stream_worker.wake();
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
                    request_id,
                    deadline,
                    reply,
                } => {
                    let mut targets = self.targets.lock();
                    let result = if Instant::now() >= deadline {
                        Err("LCD brightness request expired".into())
                    } else if let Some((_, target)) = targets
                        .iter_mut()
                        .find(|(_, t)| t.device_identity == device_id)
                    {
                        target.request_brightness(
                            Some(&self.wireless),
                            &mut self.packet_builder,
                            brightness,
                            request_id,
                        )
                    } else {
                        Err(format!("LCD not found: {device_id}"))
                    };
                    let _ = reply.try_send(result);
                }
                DaemonEvent::StartPixelClean {
                    device_id,
                    duration_minutes,
                    preparation_id,
                    deadline,
                    reply,
                } => {
                    let res = if Instant::now() >= deadline {
                        Err(lianli_shared::ui_message::UiError::new(
                            lianli_shared::ui_message::UiMessage::new(
                                "daemon.pixelClean.requestExpired",
                            ),
                            "Pixel cleaner request expired",
                        ))
                    } else if let Some(id) = preparation_id {
                        self.activate_pixel_cleaning(id).map(|id| (id, true))
                    } else {
                        self.start_pixel_cleaning(device_id, duration_minutes)
                            .map(|id| (id, false))
                    };
                    if let Err(std::sync::mpsc::SendError(Ok((id, _)))) = reply.send(res) {
                        self.stop_pixel_cleaning(None, Some(id));
                    }
                }
                DaemonEvent::StopPixelClean {
                    device_id,
                    session_id,
                    reply,
                } => {
                    let stopped = self.stop_pixel_cleaning(device_id, session_id);
                    if let Some(r) = reply {
                        let _ = r.send(stopped);
                    }
                }
                DaemonEvent::SystemResumed => {
                    for sensor in self.registry.sensor_devices.values() {
                        sensor.invalidate();
                    }
                    if let Some(rgb) = &self.controllers.rgb {
                        rgb.lock().invalidate_hardware_state();
                    }
                    self.rebuild_rgb_controller();
                    self.restart_fan_control();
                    self.start_aio_control();
                    self.sync_ipc_state();
                    info!("Device state re-applied after resume");
                }
            }
        }

        let _shutdown = monitor.enter("shutdown");
        stream_worker.stop();
        self.shutdown();
        Ok(self.restart_requested && !signals.requested())
    }
}

#[cfg(test)]
mod poll_gate_tests {
    use super::{queue_startup_events, DaemonEvent, PollGate};

    #[test]
    fn slow_startup_keeps_one_poll_ahead_of_ipc() {
        let gate = PollGate::new();
        let (tx, rx) = std::sync::mpsc::channel();
        queue_startup_events(&tx, &gate).unwrap();
        assert!(matches!(rx.try_recv(), Ok(DaemonEvent::USBCheck)));

        for _ in 0..3 {
            if gate.try_queue() {
                tx.send(DaemonEvent::DevicePoll).unwrap();
            }
        }
        assert!(matches!(rx.try_recv(), Ok(DaemonEvent::DevicePoll)));
        gate.dequeued();
        tx.send(DaemonEvent::IpcUpdate).unwrap();

        for _ in 0..3 {
            if gate.try_queue() {
                tx.send(DaemonEvent::DevicePoll).unwrap();
            }
        }
        assert!(matches!(rx.try_recv(), Ok(DaemonEvent::IpcUpdate)));
        assert!(matches!(rx.try_recv(), Ok(DaemonEvent::DevicePoll)));
        assert!(matches!(
            rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn slow_polls_never_queue_more_than_one_event() {
        // The producer ticks every interval; each poll takes two intervals.
        let gate = PollGate::new();
        let mut queued = 0usize;
        let mut peak = 0usize;
        for tick in 0..100 {
            if gate.try_queue() {
                queued += 1;
            }
            peak = peak.max(queued);
            if tick % 2 == 1 && queued > 0 {
                queued -= 1;
                gate.dequeued();
            }
        }
        assert_eq!(peak, 1);
    }
}

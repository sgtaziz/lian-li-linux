use super::runtime::{ActiveTarget, LcdBackend, ThreadedWinUsbSender};
use super::{DaemonEvent, ServiceManager};
use lianli_devices::detect::{create_hid_lcd_device, enumerate_devices, open_hid_lcd_device};
use lianli_devices::slv3_lcd::Slv3LcdDevice;
use lianli_media::{prepare_media_asset, MediaAsset};
use lianli_shared::config::{config_identity, ConfigKey, LcdConfig};
use lianli_shared::device_id::DeviceFamily;
use lianli_shared::media::MediaType;
use lianli_shared::screen::{screen_info_for, ScreenInfo};
use lianli_shared::sensors::SensorInfo;
use lianli_shared::template::LcdTemplate;
use rusb::Device;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

const SERIAL_REWRITE_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

fn asset_cache_key(
    device: &LcdConfig,
    user_templates: &[LcdTemplate],
    _sensors: &[SensorInfo],
    default_fps: f32,
) -> ConfigKey {
    let base = format!("{}|fps:{default_fps}", config_identity(device));
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

impl ServiceManager {
    pub(super) fn prepare_media_assets(&mut self, tx: Sender<DaemonEvent>) {
        let screen_map: HashMap<String, ScreenInfo> = enumerate_devices()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|det| {
                let screen = screen_info_for(det.family)?;
                let id = hid_id_norm(&det.device_id()).to_string();
                Some((id, screen))
            })
            .collect();

        let all_sensors = lianli_shared::sensors::enumerate_sensors();
        let user_templates = self.ipc.state.lock().user_templates.clone();

        self.media_assets.clear();

        if let Some(cfg) = &self.config {
            for (idx, device) in cfg.lcds.iter().enumerate() {
                let screen = device
                    .serial
                    .as_ref()
                    .and_then(|s| screen_map.get(hid_id_norm(s)).copied())
                    .unwrap_or(ScreenInfo::WIRELESS_LCD);
                let cfg_key =
                    asset_cache_key(device, &user_templates, &all_sensors, cfg.default_fps);
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
                        let stream_fps = match &asset_kind {
                            lianli_media::MediaAssetKind::Custom { asset } => asset.render_fps(),
                            _ => device
                                .fps
                                .unwrap_or(cfg.default_fps)
                                .min(cfg.default_fps)
                                .min(screen.max_fps as f32)
                                .max(1.0),
                        };
                        let asset = MediaAsset {
                            kind: asset_kind,
                            config_key: cfg_key,
                            stream_fps,
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
                        tx.send(DaemonEvent::FrameFinished).ok();
                    }
                    Err(err) => warn!("Skipping LCD[{device_id}] media: {err}"),
                }
            }
        }
    }

    pub(super) fn refresh_targets(&mut self) {
        if self.media_assets.is_empty() {
            return;
        }

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
                if !is_streamable_lcd(det.family) {
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
        let mut canonicalize: Vec<(String, String)> = Vec::new();

        if let Some(cfg) = &self.config {
            let mut claimed: HashSet<usize> = HashSet::new();
            for (cfg_idx, device_cfg) in cfg.lcds.iter().enumerate() {
                let asset = match self.media_assets.get(&cfg_idx) {
                    Some(asset_arc) => Arc::clone(asset_arc),
                    None => {
                        if let Some(mut existing) = self.targets.lock().remove(&cfg_idx) {
                            existing.stop();
                        }
                        continue;
                    }
                };

                let matched = if let Some(serial) = &device_cfg.serial {
                    // Exact match first
                    let exact = candidates.iter().enumerate().find(|(idx, c)| {
                        !claimed.contains(idx) && lcd_id_matches(serial, &c.device_id)
                    });
                    exact.or_else(|| {
                        // Alias fallback for cold-boot serial changes: only when
                        // exactly one LCD config and one LCD candidate exist.
                        if cfg.lcds.len() == 1
                            && candidates.len() == 1
                            && serial.starts_with("hid:")
                        {
                            if let Some((idx, c)) = candidates.iter().enumerate()
                                .find(|(idx, c)| !claimed.contains(idx) && is_wired_aio_lcd(c.family))
                            {
                                warn!(
                                    "[devices] configured AIO LCD id '{}' unavailable; using compatible alias '{}'",
                                    serial, c.device_id
                                );
                                return Some((idx, c));
                            }
                        }
                        None
                    }).map(|(idx, c)| { claimed.insert(idx); c })
                } else if let Some(index) = device_cfg.index {
                    candidates
                        .get(index)
                        .filter(|_| !claimed.contains(&index))
                        .inspect(|_c| {
                            claimed.insert(index);
                        })
                } else {
                    None
                };

                let candidate = match matched {
                    Some(c) => c,
                    None => {
                        if let Some(mut existing) = self.targets.lock().remove(&cfg_idx) {
                            info!("[devices] LCD[{}] detached", device_cfg.device_id());
                            existing.stop();
                        }
                        continue;
                    }
                };

                // Form rewrites only: the alias fallback may have matched a
                // different physical device, which must not be persisted.
                if let Some(serial) = &device_cfg.serial {
                    if serial != &candidate.device_id
                        && hid_id_norm(serial) == hid_id_norm(&candidate.device_id)
                    {
                        canonicalize.push((serial.clone(), candidate.device_id.clone()));
                    }
                }

                let cfg_key = asset.config_key.clone();
                if let Some(mut existing) = self.targets.lock().remove(&cfg_idx) {
                    if existing.matches(&candidate.device_id, &cfg_key) {
                        // Media is unchanged, but the custom_h264 toggle may have
                        // flipped — rebuild the frame source so the H.264 pipeline
                        // engages/disengages without a daemon restart.
                        existing.update_custom_h264(device_cfg.custom_h264(), self.tx.clone());
                        new_targets.insert(cfg_idx, existing);
                        continue;
                    } else if existing.device_identity == candidate.device_id {
                        // Same device, different config — reuse the USB transport,
                        // just swap the media asset. Reopening the device can leave
                        // some firmware in a bad state.
                        existing.swap_media(
                            Arc::clone(&asset),
                            device_cfg.custom_h264(),
                            self.tx.clone(),
                        );
                        existing.key = cfg_key;
                        new_targets.insert(cfg_idx, existing);
                        if let Some(ref tx) = self.tx {
                            tx.send(DaemonEvent::FrameFinished).ok();
                        }
                        continue;
                    } else {
                        existing.stop();
                    }
                }

                let backend_result: anyhow::Result<LcdBackend> =
                    match lcd_backend_kind(candidate.family) {
                        Some(LcdBackendKind::Slv3) => {
                            let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                            Slv3LcdDevice::new(device).map(LcdBackend::Slv3)
                        }
                        Some(LcdBackendKind::WinUsbShared) => {
                            if let Some(transport) =
                                self.registry.usb_backends.get(&candidate.device_id)
                            {
                                lianli_devices::winusb::lcd::WinUsbLcdDevice::from_shared_transport(
                                    Arc::clone(transport),
                                    candidate.pid,
                                )
                                .map(|d| LcdBackend::WinUsb(ThreadedWinUsbSender::new(d, cfg_idx)))
                            } else {
                                let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                                lianli_devices::winusb::lcd::WinUsbLcdDevice::open(
                                    device,
                                    candidate.pid,
                                )
                                .map(|d| LcdBackend::WinUsb(ThreadedWinUsbSender::new(d, cfg_idx)))
                            }
                        }
                        Some(LcdBackendKind::WinUsb) => {
                            let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                            lianli_devices::winusb::lcd::WinUsbLcdDevice::open(
                                device,
                                candidate.pid,
                            )
                            .map(|d| LcdBackend::WinUsb(ThreadedWinUsbSender::new(d, cfg_idx)))
                        }
                        Some(LcdBackendKind::HidAio) => {
                            if let Some(d) =
                                self.registry.aio_lcd_devices.remove(&candidate.device_id)
                            {
                                Ok(LcdBackend::HidLcd(Arc::new(super::runtime::HidLcd::new(d))))
                            } else if let Some(backend) =
                                self.registry.hid_backends.get(&candidate.device_id)
                            {
                                match create_hid_lcd_device(
                                    candidate.family,
                                    candidate.pid,
                                    Arc::clone(backend),
                                ) {
                                    Some(result) => result.map(|d| {
                                        LcdBackend::HidLcd(Arc::new(super::runtime::HidLcd::new(d)))
                                    }),
                                    None => Err(anyhow::anyhow!("Not an LCD device")),
                                }
                            } else {
                                Err(anyhow::anyhow!(
                                    "AIO LCD '{}' not opened yet; deferring attach",
                                    candidate.device_id
                                ))
                            }
                        }
                        Some(LcdBackendKind::HidTl) => {
                            if let Some(backend) =
                                self.registry.hid_backends.get(&candidate.device_id)
                            {
                                match create_hid_lcd_device(
                                    candidate.family,
                                    candidate.pid,
                                    Arc::clone(backend),
                                ) {
                                    Some(result) => result.map(|d| {
                                        LcdBackend::HidLcd(Arc::new(super::runtime::HidLcd::new(d)))
                                    }),
                                    None => Err(anyhow::anyhow!("Not an LCD device")),
                                }
                            } else {
                                let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                                let det = lianli_devices::detect::DetectedDevice {
                                    device,
                                    family: candidate.family,
                                    name: "TL LCD",
                                    vid: candidate.vid,
                                    pid: candidate.pid,
                                    bus: candidate.bus,
                                    address: candidate.address,
                                    serial: Some(candidate.device_id.clone()),
                                    hid_usage_page: None,
                                };
                                match open_hid_lcd_device(&det, self.hid_backend()) {
                                    Some(result) => result.map(|d| {
                                        LcdBackend::HidLcd(Arc::new(super::runtime::HidLcd::new(d)))
                                    }),
                                    None => Err(anyhow::anyhow!("Not an LCD device")),
                                }
                            }
                        }
                        None => Err(anyhow::anyhow!(
                            "no LCD backend for family {:?}",
                            candidate.family
                        )),
                    };

                match backend_result {
                    Ok(lcd) => {
                        info!(
                            "[devices] LCD[{}] attached (serial: {}, orientation: {:.0}°)",
                            device_cfg.device_id(),
                            candidate.device_id,
                            device_cfg.orientation
                        );
                        if let LcdBackend::HidLcd(ref hid) = lcd {
                            // hydroshift init sleeps 10s, keep it off the main loop
                            if is_wired_aio_lcd(candidate.family) {
                                let hid_init = std::sync::Arc::clone(hid);
                                let enable_512 = device_cfg.aio_512_frame_for(candidate.family);
                                let device_id = candidate.device_id.clone();
                                let init_tx = self.tx.clone();
                                let spawn_err_id = device_id.clone();
                                if let Err(e) = std::thread::Builder::new()
                                    .name(format!("lcd-init-{device_id}"))
                                    .spawn(move || {
                                        let mut guard = hid_init.lock();
                                        if let Err(e) = guard.initialize() {
                                            warn!("AIO LCD init failed for {device_id}: {e:#}");
                                        }
                                        guard.set_use_c_command(enable_512);
                                        drop(guard);
                                        if let Some(tx) = init_tx {
                                            tx.send(DaemonEvent::LcdInitComplete { device_id })
                                                .ok();
                                        }
                                    })
                                {
                                    // Without the worker LcdInitComplete never
                                    // fires, so say why instead of failing
                                    // silently and leaving the LCD
                                    // uninitialized with no trace in the log.
                                    warn!(
                                        "Failed to spawn AIO LCD init thread for {spawn_err_id}: {e}"
                                    );
                                }
                                self.aio_lcd_firmware
                                    .record(&candidate.device_id, None, false);
                                if !self.aio_lcd_firmware.should_skip(&candidate.device_id) {
                                    self.aio_lcd_firmware.schedule(
                                        &candidate.device_id,
                                        std::time::Duration::from_secs(10),
                                        device_cfg.aio_512_frame_for(candidate.family),
                                    );
                                }
                            } else {
                                let mut guard = hid.lock();
                                if let Err(e) = guard.initialize() {
                                    warn!(
                                        "AIO LCD basic init failed for {}: {e:#}",
                                        candidate.device_id
                                    );
                                }
                            }
                        }
                        let screen =
                            screen_info_for(candidate.family).unwrap_or(ScreenInfo::WIRELESS_LCD);
                        let target = ActiveTarget::new(
                            cfg_idx,
                            cfg_key,
                            candidate.device_id.clone(),
                            lcd,
                            Arc::clone(&asset),
                            screen,
                            device_cfg.custom_h264(),
                            self.tx.clone(),
                        );
                        new_targets.insert(cfg_idx, target);
                        if let Some(brightness) = device_cfg.brightness {
                            if let Some(t) = new_targets.get_mut(&cfg_idx) {
                                // Bounded so the main loop cannot stall behind
                                // the init worker holding the LCD mutex. When
                                // the LCD is busy the value is deferred and
                                // applied once init completes.
                                t.apply_brightness(
                                    Some(&self.wireless),
                                    &mut self.packet_builder,
                                    brightness,
                                );
                            }
                        }
                        if let Some(ref tx) = self.tx {
                            tx.send(DaemonEvent::FrameFinished).ok();
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

        // This runs every device poll. Without backoff a failing write would
        // be retried at 1 Hz while holding the IPC state lock.
        let backoff_expired = self
            .serial_rewrite_backoff
            .is_none_or(|t| t.elapsed() >= SERIAL_REWRITE_RETRY_INTERVAL);
        if !canonicalize.is_empty() && backoff_expired {
            let mut ipc_state = self.ipc.state.lock();
            if let Some(mut cfg) = ipc_state.config.clone().or_else(|| self.config.clone()) {
                let mut changed = false;
                for (old, canonical) in &canonicalize {
                    for lcd in &mut cfg.lcds {
                        if lcd.serial.as_deref() == Some(old.as_str()) {
                            lcd.serial = Some(canonical.clone());
                            changed = true;
                        }
                    }
                }
                if changed {
                    if let Err(e) = crate::persistence::write_config(&self.config_path, &cfg) {
                        self.serial_rewrite_backoff = Some(Instant::now());
                        warn!("Failed to persist canonicalized LCD serials: {e}");
                    } else {
                        self.serial_rewrite_backoff = None;
                        self.config = Some(cfg.clone());
                        ipc_state.config = Some(cfg);
                    }
                }
            }
        }

        let mut targets = self.targets.lock();
        for (_, mut target) in targets.drain() {
            target.stop();
        }

        targets.extend(new_targets);
    }
}

/// Whether the daemon can stream media to this family over USB.
fn is_streamable_lcd(family: DeviceFamily) -> bool {
    family.has_lcd() && !family.is_desktop_mode()
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum LcdBackendKind {
    Slv3,
    WinUsbShared,
    WinUsb,
    HidAio,
    HidTl,
}

/// Single source of truth: maps an LCD family to its backend type.
fn lcd_backend_kind(family: DeviceFamily) -> Option<LcdBackendKind> {
    use DeviceFamily::*;
    Some(match family {
        Slv3Lcd | Tlv2Lcd => LcdBackendKind::Slv3,
        HydroShift2Lcd => LcdBackendKind::WinUsbShared,
        HydroShift2OledCurveLcd
        | Lancool207
        | UniversalScreen
        | Vision9p2
        | TlFlexLcd
        | SlInfFlexLcd => LcdBackendKind::WinUsb,
        HydroShiftLcd | Galahad2Lcd => LcdBackendKind::HidAio,
        TlLcd => LcdBackendKind::HidTl,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use lianli_shared::device_id::KNOWN_DEVICES;

    #[test]
    fn all_streamable_lcds_have_backends() {
        let mut seen = std::collections::HashSet::new();
        for entry in KNOWN_DEVICES {
            if !seen.insert(entry.family) {
                continue;
            }
            if super::is_streamable_lcd(entry.family) {
                assert!(
                    super::lcd_backend_kind(entry.family).is_some(),
                    "{:?} is streamable but lcd_backend_kind returns None",
                    entry.family
                );
            }
        }
    }
}

fn hid_id_norm(s: &str) -> &str {
    s.strip_prefix("hid:").unwrap_or(s)
}

fn lcd_id_matches(serial: &str, device_id: &str) -> bool {
    hid_id_norm(serial) == hid_id_norm(device_id)
}

/// Whether a device family is a wired AIO LCD that may benefit from alias matching.
fn is_wired_aio_lcd(family: DeviceFamily) -> bool {
    matches!(
        family,
        DeviceFamily::HydroShiftLcd
            | DeviceFamily::Galahad2Lcd
            | DeviceFamily::HydroShift2Lcd
            | DeviceFamily::HydroShift2OledCurveLcd
    )
}

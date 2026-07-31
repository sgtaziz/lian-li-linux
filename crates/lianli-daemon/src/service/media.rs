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

impl ServiceManager {
    pub(super) fn prepare_media_assets(&mut self, tx: Sender<DaemonEvent>) {
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
        let user_templates = self.ipc.state.lock().user_templates.clone();

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

        const LCD_FAMILIES: &[DeviceFamily] = &[
            DeviceFamily::Slv3Lcd,
            DeviceFamily::Tlv2Lcd,
            DeviceFamily::HydroShift2Lcd,
            DeviceFamily::HydroShift2OledCurveLcd,
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
                    let exact = candidates
                        .iter()
                        .enumerate()
                        .find(|(idx, c)| !claimed.contains(idx) && &c.device_id == serial);
                    exact.or_else(|| {
                        // Alias fallback: wired AIO firmwares may alternate between
                        // hardware serial and USB-topology ID across cold boot.
                        // Only apply when there's one LCD config and one compatible AIO.
                        if cfg.lcds.len() == 1 && serial.starts_with("hid:") {
                            let mut compatible = candidates.iter().enumerate()
                                .filter(|(idx, c)| !claimed.contains(idx) && is_wired_aio_lcd(c.family));
                            let first = compatible.next();
                            if first.is_some() && compatible.next().is_none() {
                                let (idx, c) = first.unwrap();
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
                        .map(|c| {
                            claimed.insert(index);
                            c
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

                let backend_result: anyhow::Result<LcdBackend> = match candidate.family {
                    DeviceFamily::Slv3Lcd | DeviceFamily::Tlv2Lcd => {
                        let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                        Slv3LcdDevice::new(device).map(LcdBackend::Slv3)
                    }
                    DeviceFamily::HydroShift2Lcd
                    | DeviceFamily::HydroShift2OledCurveLcd
                    | DeviceFamily::Lancool207
                    | DeviceFamily::UniversalScreen => {
                        let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                        lianli_devices::winusb::lcd::open_for_pid(candidate.pid, device)
                            .map(|d| LcdBackend::WinUsb(ThreadedWinUsbSender::new(d, cfg_idx)))
                    }
                    DeviceFamily::HydroShiftLcd
                    | DeviceFamily::Galahad2Lcd
                    | DeviceFamily::TlLcd => {
                        // Try to reuse a shared HID backend (opened by init_rgb_controller).
                        if let Some(backend) = self.registry.hid_backends.get(&candidate.device_id)
                        {
                            match create_hid_lcd_device(
                                candidate.family,
                                candidate.pid,
                                Arc::clone(backend),
                            ) {
                                Some(result) => result.map(|d| {
                                    LcdBackend::HidLcd(Arc::new(parking_lot::Mutex::new(d)))
                                }),
                                None => Err(anyhow::anyhow!("Not an LCD device")),
                            }
                        } else {
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
                            match open_hid_lcd_device(&det) {
                                Some(result) => result.map(|d| {
                                    LcdBackend::HidLcd(Arc::new(parking_lot::Mutex::new(d)))
                                }),
                                None => Err(anyhow::anyhow!("Not an LCD device")),
                            }
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
                        if let LcdBackend::HidLcd(ref hid) = lcd {
                            let mut guard = hid.lock();
                            if let Err(e) = guard.initialize() {
                                warn!(
                                    "AIO LCD basic init failed for {}: {e:#}",
                                    candidate.device_id
                                );
                            }
                            guard.set_use_c_command(device_cfg.aio_512_frame());
                            self.aio_lcd_firmware
                                .record(&candidate.device_id, None, false);
                            if !self.aio_lcd_firmware.should_skip(&candidate.device_id) {
                                self.aio_lcd_firmware.schedule(
                                    &candidate.device_id,
                                    std::time::Duration::from_secs(10),
                                    device_cfg.aio_512_frame(),
                                );
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
                                if let Err(e) = t.lcd.set_brightness(
                                    Some(&self.wireless),
                                    &mut self.packet_builder,
                                    brightness,
                                ) {
                                    warn!("Failed to apply LCD brightness for LCD[{cfg_idx}]: {e}");
                                }
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

        let mut targets = self.targets.lock();
        for (_, mut target) in targets.drain() {
            target.stop();
        }

        targets.extend(new_targets);
    }
}

/// Whether a device family is a wired AIO LCD that may benefit from alias matching.
fn is_wired_aio_lcd(family: DeviceFamily) -> bool {
    matches!(
        family,
        DeviceFamily::HydroShiftLcd
            | DeviceFamily::Galahad2Lcd
            | DeviceFamily::HydroShift2OledCurveLcd
    )
}

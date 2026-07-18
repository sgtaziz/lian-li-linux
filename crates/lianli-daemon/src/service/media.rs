use super::runtime::{ActiveTarget, LcdBackend, ThreadedWinUsbSender};
use super::{DaemonEvent, ServiceManager};
use lianli_devices::detect::{
    create_hid_lcd_device, enumerate_devices, open_hid_lcd_by_topology, open_hid_lcd_by_vid_pid,
    open_hid_lcd_device_rusb,
};
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct LcdIdentity {
    family: DeviceFamily,
    device_id: String,
}

fn is_wired_aio_lcd(family: DeviceFamily) -> bool {
    matches!(
        family,
        DeviceFamily::HydroShiftLcd | DeviceFamily::Galahad2Lcd
    )
}

/// Select an LCD without allowing two config entries to claim one device.
///
/// The alias fallback is intentionally narrow: some wired AIO firmwares switch
/// between a hardware serial and a USB-topology ID after a cold boot. We only
/// accept a sole compatible AIO when there is also a sole LCD configuration.
fn select_lcd_index(
    device: &LcdConfig,
    identities: &[LcdIdentity],
    claimed: &HashSet<usize>,
    config_count: usize,
) -> Option<usize> {
    if let Some(serial) = device.serial.as_deref() {
        if let Some((idx, _)) = identities
            .iter()
            .enumerate()
            .find(|(idx, candidate)| !claimed.contains(idx) && candidate.device_id == serial)
        {
            return Some(idx);
        }

        if config_count == 1 && serial.starts_with("hid:") {
            let mut compatible = identities.iter().enumerate().filter(|(idx, candidate)| {
                !claimed.contains(idx) && is_wired_aio_lcd(candidate.family)
            });
            let first = compatible.next().map(|(idx, _)| idx);
            if first.is_some() && compatible.next().is_none() {
                return first;
            }
        }
        return None;
    }

    device
        .index
        .filter(|idx| *idx < identities.len() && !claimed.contains(idx))
}

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

    pub(super) fn refresh_targets(&mut self) {
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
            let identities: Vec<LcdIdentity> = candidates
                .iter()
                .map(|candidate| LcdIdentity {
                    family: candidate.family,
                    device_id: candidate.device_id.clone(),
                })
                .collect();
            let mut claimed = HashSet::new();
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

                let selected = select_lcd_index(device_cfg, &identities, &claimed, cfg.lcds.len());
                let matched =
                    selected.and_then(|idx| candidates.get(idx).map(|candidate| (idx, candidate)));

                let candidate = match matched {
                    Some((selected, c)) => {
                        claimed.insert(selected);
                        if device_cfg
                            .serial
                            .as_deref()
                            .is_some_and(|configured| configured != c.device_id)
                        {
                            warn!(
                                "[devices] configured AIO LCD id '{}' is unavailable; using compatible alias '{}'",
                                device_cfg.serial.as_deref().unwrap_or("<index>"),
                                c.device_id
                            );
                        }
                        c
                    }
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
                                Some(result) => result.map(|d| {
                                    LcdBackend::HidLcd(Arc::new(parking_lot::Mutex::new(d)))
                                }),
                                None => Err(anyhow::anyhow!("Not an LCD device")),
                            }
                        } else if self.use_rusb() || candidate.family == DeviceFamily::TlLcd {
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
                                Some(result) => result.map(|d| {
                                    LcdBackend::HidLcd(Arc::new(parking_lot::Mutex::new(d)))
                                }),
                                None => Err(anyhow::anyhow!("Not an LCD device")),
                            }
                        } else {
                            let port_numbers = candidate
                                .usb_device
                                .as_ref()
                                .and_then(|d| d.port_numbers().ok())
                                .unwrap_or_default();
                            if !port_numbers.is_empty() {
                                open_hid_lcd_by_topology(
                                    candidate.vid,
                                    candidate.pid,
                                    candidate.family,
                                    candidate.bus,
                                    &port_numbers,
                                )
                                .map(|d| LcdBackend::HidLcd(Arc::new(parking_lot::Mutex::new(d))))
                            } else {
                                open_hid_lcd_by_vid_pid(
                                    candidate.vid,
                                    candidate.pid,
                                    candidate.family,
                                )
                                .map(|d| LcdBackend::HidLcd(Arc::new(parking_lot::Mutex::new(d))))
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
                            self.aio_lcd_info
                                .insert(candidate.device_id.clone(), (None, false));
                            let skip = self
                                .aio_lcd_skip_firmware
                                .get(&candidate.device_id)
                                .map(|t| t.elapsed() < std::time::Duration::from_secs(1800))
                                .unwrap_or(false);
                            if !skip {
                                self.aio_lcd_pending_firmware.insert(
                                    candidate.device_id.clone(),
                                    (
                                        std::time::Instant::now()
                                            + std::time::Duration::from_secs(10),
                                        device_cfg.aio_512_frame(),
                                    ),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serial_config(serial: &str) -> LcdConfig {
        serde_json::from_value(serde_json::json!({
            "serial": serial,
            "type": "color",
            "rgb": [0, 0, 0]
        }))
        .unwrap()
    }

    fn index_config(index: usize) -> LcdConfig {
        serde_json::from_value(serde_json::json!({
            "index": index,
            "serial": null,
            "type": "color",
            "rgb": [0, 0, 0]
        }))
        .unwrap()
    }

    fn identity(family: DeviceFamily, device_id: &str) -> LcdIdentity {
        LcdIdentity {
            family,
            device_id: device_id.into(),
        }
    }

    #[test]
    fn exact_id_wins_with_multiple_candidates() {
        let identities = vec![
            identity(DeviceFamily::Galahad2Lcd, "hid:topology"),
            identity(DeviceFamily::TlLcd, "hid:exact"),
        ];
        assert_eq!(
            select_lcd_index(&serial_config("hid:exact"), &identities, &HashSet::new(), 1),
            Some(1)
        );
    }

    #[test]
    fn sole_wired_aio_is_accepted_as_id_alias() {
        let identities = vec![identity(
            DeviceFamily::Galahad2Lcd,
            "hid:0416:7395:topology",
        )];
        assert_eq!(
            select_lcd_index(
                &serial_config("hid:hardware-serial"),
                &identities,
                &HashSet::new(),
                1
            ),
            Some(0)
        );
    }

    #[test]
    fn alias_fallback_rejects_incompatible_or_ambiguous_displays() {
        let incompatible = vec![identity(DeviceFamily::TlLcd, "hid:tl")];
        assert_eq!(
            select_lcd_index(
                &serial_config("hid:missing"),
                &incompatible,
                &HashSet::new(),
                1
            ),
            None
        );

        let ambiguous = vec![
            identity(DeviceFamily::Galahad2Lcd, "hid:a"),
            identity(DeviceFamily::HydroShiftLcd, "hid:b"),
        ];
        assert_eq!(
            select_lcd_index(
                &serial_config("hid:missing"),
                &ambiguous,
                &HashSet::new(),
                1
            ),
            None
        );
    }

    #[test]
    fn alias_fallback_is_disabled_for_multiple_configs() {
        let identities = vec![identity(DeviceFamily::Galahad2Lcd, "hid:topology")];
        assert_eq!(
            select_lcd_index(
                &serial_config("hid:serial"),
                &identities,
                &HashSet::new(),
                2
            ),
            None
        );
    }

    #[test]
    fn claimed_candidate_cannot_be_selected_twice() {
        let identities = vec![identity(DeviceFamily::Galahad2Lcd, "hid:exact")];
        assert_eq!(
            select_lcd_index(
                &serial_config("hid:exact"),
                &identities,
                &HashSet::from([0]),
                1
            ),
            None
        );
    }

    #[test]
    fn index_selection_preserves_order_and_honors_claims() {
        let identities = vec![
            identity(DeviceFamily::Slv3Lcd, "usb:first"),
            identity(DeviceFamily::Tlv2Lcd, "usb:second"),
        ];
        assert_eq!(
            select_lcd_index(&index_config(1), &identities, &HashSet::new(), 2),
            Some(1)
        );
        assert_eq!(
            select_lcd_index(&index_config(1), &identities, &HashSet::from([1]), 2),
            None
        );
    }
}

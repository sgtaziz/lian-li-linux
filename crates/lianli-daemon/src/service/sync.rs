use super::ServiceManager;
use lianli_devices::detect::{enumerate_devices, probe_tl_lcd_port_indices};
use lianli_shared::device_id::{panel_key, DeviceFamily};
use lianli_shared::ipc::{CurveTemperature, DeviceInfo, H264Setting};
use lianli_shared::screen::screen_info_for;
use lianli_shared::ui_message::{UiMessage, UiText};
use std::collections::{HashMap, HashSet};
use tracing::{debug, warn};

impl ServiceManager {
    /// Sync current config to IPC shared state.
    pub(super) fn sync_ipc_state(&self) {
        if self.startup_image_job.is_some() || self.startup_config_pending {
            return;
        }
        let mut ipc_state = self.ipc.state.lock();
        ipc_state.config = self.config.clone();
    }

    /// Refresh the cached USB device list (full bus enumeration).
    pub(super) fn refresh_usb_device_cache(&mut self) {
        match enumerate_devices() {
            Ok(usb_devices) => {
                let present: HashSet<_> = usb_devices
                    .iter()
                    .map(|device| device.device_id())
                    .collect();
                let previous: HashSet<_> = self.registry.usb_locations.keys().cloned().collect();
                if present != previous && self.migrate_wired_config(&usb_devices) {
                    if let Some(tx) = self.tx.clone() {
                        self.prepare_media_assets(tx);
                    }
                }
                self.reconcile_startup_quarantine(&present);
                self.refresh_tl_lcd_port_index_cache(&usb_devices);
                self.build_usb_device_cache(usb_devices);
                self.recover_lcd_groups();
            }
            Err(e) => {
                warn!("USB enumeration failed: {e}");
            }
        }
    }

    fn refresh_tl_lcd_port_index_cache(
        &mut self,
        usb_devices: &[lianli_devices::detect::DetectedDevice],
    ) {
        if self.startup_image_job.is_some() || !self.startup_image_quarantine.is_empty() {
            return;
        }
        let current_ids: HashSet<String> = usb_devices
            .iter()
            .filter(|d| d.family == DeviceFamily::TlLcd)
            .map(|d| d.device_id())
            .collect();
        let cached_ids: HashSet<String> = self.registry.tl_lcd_port_index.keys().cloned().collect();
        if current_ids == cached_ids {
            return;
        }
        let probed = probe_tl_lcd_port_indices(usb_devices, self.hid_backend());
        self.registry.tl_lcd_port_index.clear();

        let mut entries: Vec<(String, Vec<u8>, (u8, u8))> = Vec::new();
        for det in usb_devices
            .iter()
            .filter(|d| d.family == DeviceFamily::TlLcd)
        {
            let Ok(ports) = det.device.port_numbers() else {
                continue;
            };
            let device_id = det.device_id();
            if let Some(&pi) = probed.get(&device_id) {
                entries.push((device_id, ports, pi));
            }
        }

        // Firmware can report duplicate (port, index) for daisy-chained TL LCDs.
        // Within each port group, keep firmware values where unique; reassign
        // duplicates to the next free index, shallowest-first so the firmware
        // values closest to the controller win.
        let mut by_port: std::collections::HashMap<u8, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            by_port.entry(e.2 .0).or_default().push(i);
        }
        for indices in by_port.values_mut() {
            indices.sort_by(|&a, &b| entries[a].1.cmp(&entries[b].1));
            let mut used: HashSet<u8> = HashSet::new();
            let mut pending: Vec<usize> = Vec::new();
            for &i in indices.iter() {
                if !used.insert(entries[i].2 .1) {
                    pending.push(i);
                }
            }
            let mut next: u8 = 0;
            for i in pending {
                while !used.insert(next) {
                    next = next.saturating_add(1);
                }
                entries[i].2 .1 = next;
            }
        }

        for (device_id, _, pi) in entries {
            debug!("TL LCD port_index cached: {device_id} -> {pi:?}");
            self.registry.tl_lcd_port_index.insert(device_id, pi);
        }
    }

    fn build_usb_device_cache(&mut self, usb_devices: Vec<lianli_devices::detect::DetectedDevice>) {
        self.registry.usb_locations = usb_devices
            .iter()
            .filter_map(|device| {
                let ports = device.device.port_numbers().ok()?;
                (!ports.is_empty()).then(|| (device.device_id(), (device.bus, ports)))
            })
            .collect();
        if self.registry.v2_hid_entries.is_empty() {
            self.registry.v2_hid_entries =
                lianli_devices::wireless::query_v2_hid_macs(self.hid_backend());
        }
        let v2_hid_entries = self.registry.v2_hid_entries.clone();
        let known_wireless_macs: HashSet<[u8; 6]> =
            self.wireless.devices().iter().map(|d| d.mac).collect();
        if !v2_hid_entries.is_empty() {
            debug!("V2 HID MAC map: {} entr(y/ies)", v2_hid_entries.len());
        }
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

            let lcd_only = matches!(
                det.family,
                lianli_shared::device_id::DeviceFamily::HydroShiftLcd
                    | lianli_shared::device_id::DeviceFamily::Galahad2Lcd
                    | lianli_shared::device_id::DeviceFamily::HydroShift2Lcd
                    | lianli_shared::device_id::DeviceFamily::HydroShift2OledCurveLcd
                    | lianli_shared::device_id::DeviceFamily::Slv3Lcd
                    | lianli_shared::device_id::DeviceFamily::Tlv2Lcd
            );

            let (firmware_version, supports_c_command) = self
                .aio_lcd_firmware
                .get(&device_id)
                .unwrap_or((None, false));
            let port_index = if det.family == DeviceFamily::TlLcd {
                self.registry.tl_lcd_port_index.get(&device_id).copied()
            } else {
                None
            };

            let wireless_group_mac =
                if matches!(det.family, DeviceFamily::Slv3Lcd | DeviceFamily::Tlv2Lcd) {
                    match det.device.port_numbers() {
                        Ok(ports) => find_wireless_group_mac(
                            &v2_hid_entries,
                            &known_wireless_macs,
                            det.bus,
                            &ports,
                        ),
                        Err(_) => None,
                    }
                } else {
                    None
                };

            cached.push(DeviceInfo {
                startup_image: lianli_shared::startup_image::capabilities(det.family),
                telemetry: None,
                device_id: device_id.clone(),
                family: det.family,
                name: det.name.to_string(),
                serial: det.serial.clone(),
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
                pump_mb_sync_support: false,
                rgb_zone_count: None,
                screen_width: screen.map(|s| s.width),
                screen_height: screen.map(|s| s.height),
                is_unbound_wireless: false,
                wireless_bind_status: None,
                foreign_master_online: false,
                pump_rpm_range: None,
                fan_quantity: None,
                max_fan_quantity: None,
                firmware_version,
                supports_c_command,
                port_index,
                wireless_group_mac,
                topology_key: Some(det.topology_key()),
                h264: None,
            });
        }

        self.registry.cached_usb_devices = cached;

        match crate::desktop_display::enumerate_turzx() {
            Ok(present) => self.desktop_displays.sync(&present),
            Err(e) => warn!("TURZX enumeration failed: {e:#}"),
        }
    }

    /// Update IPC telemetry and device list.
    pub(super) fn sync_ipc_telemetry(&self) {
        let (streaming_active, media_runtime, lcd_brightness) = {
            let targets = self.targets.lock();
            let runtime: HashMap<_, _> = targets
                .iter()
                .filter_map(|(index, target)| {
                    self.media_requested_keys
                        .get(*index)
                        .filter(|key| **key == target.asset.config_key)
                        .map(|_| (*index, target.media_status()))
                })
                .collect();
            let brightness = targets
                .values()
                .filter_map(|target| {
                    target
                        .brightness_status()
                        .map(|status| (target.device_identity.clone(), status.clone()))
                })
                .collect();
            (!targets.is_empty(), runtime, brightness)
        };

        // OpenRGB server status
        let (enabled, _) = self
            .config
            .as_ref()
            .and_then(|c| c.rgb.as_ref())
            .map(|rgb| (rgb.openrgb_server, rgb.openrgb_port))
            .unwrap_or((false, 6743));
        let openrgb_status = {
            let orgb_state = self.openrgb.state.lock();
            let exited = enabled
                && !self.openrgb.stop.load(std::sync::atomic::Ordering::Relaxed)
                && self
                    .openrgb
                    .thread
                    .as_ref()
                    .is_some_and(|thread| thread.is_finished());
            let (error, error_ui) = UiText::split(orgb_state.error.clone().or_else(|| {
                exited.then(|| {
                    UiText::new(
                        UiMessage::new("status.openRgb.stopped"),
                        "OpenRGB stopped. Check daemon logs, then Retry OpenRGB.",
                    )
                })
            }));
            lianli_shared::ipc::OpenRgbServerStatus {
                enabled,
                running: orgb_state.running && !exited,
                port: orgb_state.port,
                error,
                error_ui,
            }
        };

        // Build device list from wireless discovery
        let mut devices = Vec::new();

        let mut fan_rpms: std::collections::HashMap<String, Vec<u16>> =
            std::collections::HashMap::new();
        let mut coolant_temps: std::collections::HashMap<String, f32> =
            std::collections::HashMap::new();
        let mut pump_rpms: std::collections::HashMap<String, u16> =
            std::collections::HashMap::new();

        for dev in self.wireless.devices() {
            use lianli_devices::wireless::WirelessFanType;
            use lianli_shared::device_id::DeviceFamily;

            let family = match dev.fan_type {
                WirelessFanType::Slv3Led | WirelessFanType::SlV4 => DeviceFamily::Slv3Led,
                WirelessFanType::Slv3Lcd => DeviceFamily::Slv3Lcd,
                WirelessFanType::Tlv2Lcd => DeviceFamily::Tlv2Lcd,
                WirelessFanType::Tlv2Led | WirelessFanType::TlV3 { .. } => DeviceFamily::Tlv2Led,
                WirelessFanType::SlInf | WirelessFanType::SlInfV3 { .. } => DeviceFamily::SlInf,
                WirelessFanType::Clv1 | WirelessFanType::ClV2 { .. } | WirelessFanType::P28V2 => {
                    DeviceFamily::Clv1
                }
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
                startup_image: is_aio
                    .then(|| lianli_shared::startup_image::capabilities(family))
                    .flatten(),
                telemetry: None,
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
                mb_sync_support: dev.fan_type.supports_hw_mobo_sync(),
                pump_mb_sync_support: false,
                rgb_zone_count: Some(rgb_zone_count),
                screen_width: None,
                screen_height: None,
                is_unbound_wireless: false,
                wireless_bind_status: Some(if self.wireless.needs_rebind(&dev.mac) {
                    "needs_rebind".to_string()
                } else {
                    "bind_link".to_string()
                }),
                foreign_master_online: false,
                pump_rpm_range: dev.fan_type.pump_rpm_range(),
                fan_quantity: None,
                max_fan_quantity: None,
                firmware_version: None,
                supports_c_command: false,
                port_index: None,
                wireless_group_mac: None,
                topology_key: None,
                h264: None,
            });

            // Update RPM telemetry keyed by device_id
            let device_id = format!("wireless:{}", dev.mac_str());
            let mut rpms: Vec<u16> = dev.fan_rpms[..dev.fan_count as usize].to_vec();
            if is_aio {
                rpms.push(dev.fan_rpms[3]); // pump RPM
                publish_pump_rpm(&mut pump_rpms, &device_id, dev.fan_rpms[3]);
            }
            fan_rpms.insert(device_id.clone(), rpms);

            if let Some(reading) = self
                .wireless
                .coolant_reading(&dev.mac)
                .filter(|reading| reading.observed_at.elapsed() < std::time::Duration::from_secs(5))
            {
                coolant_temps.insert(device_id.clone(), reading.value);
                if let Err(error) =
                    lianli_shared::sensors::write_coolant_reading(&device_id, reading)
                {
                    tracing::debug!(%device_id, %error, "Could not publish coolant reading");
                }
            }
        }

        // Add unbound wireless devices (visible but not controllable until bound)
        for dev in self.wireless.unbound_devices() {
            use lianli_devices::wireless::WirelessFanType;
            use lianli_shared::device_id::DeviceFamily;

            let family = match dev.fan_type {
                WirelessFanType::Slv3Led | WirelessFanType::SlV4 => DeviceFamily::Slv3Led,
                WirelessFanType::Slv3Lcd => DeviceFamily::Slv3Lcd,
                WirelessFanType::Tlv2Lcd => DeviceFamily::Tlv2Lcd,
                WirelessFanType::Tlv2Led | WirelessFanType::TlV3 { .. } => DeviceFamily::Tlv2Led,
                WirelessFanType::SlInf | WirelessFanType::SlInfV3 { .. } => DeviceFamily::SlInf,
                WirelessFanType::Clv1 | WirelessFanType::ClV2 { .. } | WirelessFanType::P28V2 => {
                    DeviceFamily::Clv1
                }
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
                startup_image: None,
                telemetry: None,
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
                pump_mb_sync_support: false,
                rgb_zone_count: None,
                screen_width: None,
                screen_height: None,
                is_unbound_wireless: true,
                wireless_bind_status: Some(if dev.master_mac == [0u8; 6] {
                    "ready_to_bind".to_string()
                } else {
                    "bind_other".to_string()
                }),
                foreign_master_online: self.wireless.foreign_master_online(&dev.master_mac),
                pump_rpm_range: dev.fan_type.pump_rpm_range(),
                fan_quantity: None,
                max_fan_quantity: None,
                firmware_version: None,
                supports_c_command: false,
                port_index: None,
                wireless_group_mac: None,
                topology_key: None,
                h264: None,
            });
        }

        // Tag wired fan entries whose link MAC is bound wireless so the GUI
        // can hide them, they stay targetable for LCD and brightness
        let bound_macs: HashSet<[u8; 6]> = self.wireless.devices().iter().map(|d| d.mac).collect();
        let link_mac_of_base: HashMap<&str, [u8; 6]> = self
            .registry
            .fan_devices
            .iter()
            .filter_map(|(id, d)| {
                let mac = d.wireless_link_mac()?;
                bound_macs.contains(&mac).then_some((id.as_str(), mac))
            })
            .collect();

        let mut fan_info: Vec<DeviceInfo> = self.registry.fan_device_info.clone();
        tag_fan_entries(&mut fan_info, &link_mac_of_base);
        devices.extend(fan_info);

        for device in &mut devices {
            if let Some(sensors) = self.registry.sensor_devices.get(&device.device_id) {
                let telemetry = sensors.telemetry();
                device.firmware_version.clone_from(&telemetry.firmware);
                device.telemetry = Some(telemetry);
            }
        }

        // Read wired fan RPMs and split per port.
        for (base_id, dev) in self.registry.fan_devices.iter() {
            // Coolant telemetry for wired AIOs (HydroShift LCD family),
            // mirroring the wireless path: publish via IPC and register as a
            // fan-curve sensor source.
            if let Some(reading) = dev
                .poll_coolant_reading()
                .filter(|reading| reading.observed_at.elapsed() < std::time::Duration::from_secs(5))
            {
                coolant_temps.insert(base_id.clone(), reading.value);
                if let Err(error) = lianli_shared::sensors::write_coolant_reading(base_id, reading)
                {
                    tracing::debug!(%base_id, %error, "Could not publish coolant reading");
                }
            }
            let pump_rpm = dev.read_pump_rpm();
            if let Some(rpm) = pump_rpm {
                publish_pump_rpm(&mut pump_rpms, base_id, rpm);
            }
            if let Ok(all_rpms) = dev.read_fan_rpm() {
                let ports = dev.fan_port_info();
                let per_fan = dev.per_fan_control();
                let mut offset = 0;
                for &(port, count) in &ports {
                    let port_rpms = if per_fan {
                        let end = (offset + count as usize).min(all_rpms.len());
                        let v = all_rpms[offset..end].to_vec();
                        offset = end;
                        // AIO pump RPM rides in the last telemetry slot
                        // (GUI reads rpms[fan_count]).
                        with_pump_rpm(v, count, pump_rpm)
                    } else {
                        all_rpms
                            .get(port as usize)
                            .map(|&r| vec![r])
                            .unwrap_or_default()
                    };
                    let device_id = if ports.len() > 1 {
                        format!("{base_id}:port{port}")
                    } else {
                        base_id.clone()
                    };
                    fan_rpms.insert(device_id, port_rpms);
                }
            }
        }

        // Cache is refreshed every USB_ENUM_INTERVAL (10s) to avoid
        // USB bus contention from opening every device for serial reads.
        // Drop entries already emitted from fan_device_info above so each
        // physical endpoint surfaces exactly once.
        let opened_topos: HashSet<&str> = self
            .registry
            .fan_device_info
            .iter()
            .filter_map(|d| d.topology_key.as_deref())
            .collect();
        let bound_mac_strs: HashSet<String> = self
            .wireless
            .devices()
            .iter()
            .map(|d| d.mac_str())
            .collect();
        devices.extend(
            self.registry
                .cached_usb_devices
                .iter()
                .cloned()
                .filter_map(|d| retained_cache_entry(d, &opened_topos, &bound_mac_strs)),
        );
        for device in &mut devices {
            device.h264 = h264_setting(device, |key, pid| {
                self.desktop_displays.h264_setting(key, pid)
            });
        }

        let desktop_streams = self.desktop_displays.statuses();
        let curve_temperatures = merge_curve_temperatures(
            self.controllers
                .fan
                .as_ref()
                .map(|fan| fan.curve_temperatures())
                .unwrap_or_default(),
            self.controllers
                .aio
                .as_ref()
                .map(|aio| aio.curve_temperatures())
                .unwrap_or_default(),
        );
        {
            let mut ipc_state = self.ipc.state.lock();
            for (index, status) in &mut ipc_state.telemetry.media_preparation {
                status.runtime = media_runtime.get(index).cloned();
            }
            ipc_state.telemetry.desktop_streams = desktop_streams;
            ipc_state.telemetry.streaming_active = streaming_active;
            ipc_state.telemetry.lcd_brightness = lcd_brightness;
            ipc_state.telemetry.openrgb_status = openrgb_status;
            ipc_state.telemetry.fan_rpms = fan_rpms;
            ipc_state.telemetry.coolant_temps = coolant_temps;
            ipc_state.telemetry.pump_rpms = pump_rpms;
            ipc_state.telemetry.curve_temperatures = curve_temperatures;
            ipc_state.telemetry.pixel_clean_statuses = ipc_state.pixel_clean_statuses();
            ipc_state.telemetry.unresponsive_dongle = self.wireless_recovery.unresponsive_dongle();
            ipc_state.devices = devices;
        }
    }
}

impl ServiceManager {
    pub(super) fn respawn_pending_desktop_workers(&mut self) {
        if !self.desktop_displays.restart_pending() {
            return;
        }
        match crate::desktop_display::enumerate_turzx() {
            Ok(present) => self.desktop_displays.sync(&present),
            Err(error) => debug!("TURZX enumeration failed: {error:#}"),
        }
    }
}

fn h264_setting(
    device: &DeviceInfo,
    desktop: impl FnOnce(&str, u16) -> Option<bool>,
) -> Option<H264Setting> {
    if !device.has_lcd {
        return None;
    }
    let key = device.topology_key.as_deref().and_then(panel_key)?;
    let jpeg_available = if device.family.is_desktop_mode() {
        desktop(&key, device.pid)?
    } else {
        screen_info_for(device.family)?.h264.then_some(true)?
    };
    Some(H264Setting {
        key,
        jpeg_available,
    })
}

fn merge_curve_temperatures(
    mut fans: HashMap<String, CurveTemperature>,
    aio: HashMap<String, CurveTemperature>,
) -> HashMap<String, CurveTemperature> {
    for (name, reading) in aio {
        fans.entry(name)
            .and_modify(|merged| {
                merged.full_speed_fallback |= reading.full_speed_fallback;
                merged.celsius = merged.celsius.or(reading.celsius);
            })
            .or_insert(reading);
    }
    fans
}

fn base_of_port_suffix(device_id: &str) -> &str {
    device_id
        .rsplit_once(":port")
        .map(|(base, _)| base)
        .unwrap_or(device_id)
}

/// Check if a wired device at `bus`:`ports` shares a USB parent hub with
/// any V2 dongle HID entry. If so, and the MAC corresponds to a discovered
/// wireless device, return the associated wireless group MAC.
fn find_wireless_group_mac(
    v2_hid_entries: &[lianli_devices::wireless::V2HidEntry],
    known_wireless_macs: &HashSet<[u8; 6]>,
    bus: u8,
    ports: &[u8],
) -> Option<String> {
    for entry in v2_hid_entries {
        if lianli_devices::wireless::share_parent(entry.bus, &entry.port_numbers, bus, ports)
            && known_wireless_macs.contains(&entry.mac)
        {
            return Some(mac_str(&entry.mac));
        }
    }
    None
}

fn mac_str(mac: &[u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

fn tag_fan_entries(fan_info: &mut [DeviceInfo], link_mac_of_base: &HashMap<&str, [u8; 6]>) {
    for d in fan_info.iter_mut() {
        if let Some(mac) = link_mac_of_base.get(base_of_port_suffix(&d.device_id)) {
            d.wireless_group_mac = Some(mac_str(mac));
        }
    }
}

fn retained_cache_entry(
    mut d: DeviceInfo,
    opened_topos: &HashSet<&str>,
    bound_mac_strs: &HashSet<String>,
) -> Option<DeviceInfo> {
    if d.topology_key
        .as_deref()
        .is_some_and(|t| opened_topos.contains(t))
    {
        return None;
    }
    if !d
        .wireless_group_mac
        .as_deref()
        .is_some_and(|m| bound_mac_strs.contains(m))
    {
        d.wireless_group_mac = None;
    }
    Some(d)
}

fn publish_pump_rpm(pump_rpms: &mut HashMap<String, u16>, device_id: &str, rpm: u16) {
    pump_rpms.insert(device_id.to_owned(), rpm);
    let reading = lianli_shared::sensors::SensorReading {
        value: f32::from(rpm),
        observed_at: std::time::Instant::now(),
    };
    if let Err(error) = lianli_shared::sensors::write_pump_reading(device_id, reading) {
        tracing::debug!(%device_id, %error, "Could not publish pump RPM");
    }
}

fn with_pump_rpm(mut rpms: Vec<u16>, fan_count: u8, pump: Option<u16>) -> Vec<u16> {
    if let Some(pump) = pump {
        rpms.resize(usize::from(fan_count), 0);
        rpms.push(pump);
    }
    rpms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aio_only_and_aio_lost_curves_reach_the_gui() {
        let reading = |celsius, full_speed_fallback| CurveTemperature {
            celsius,
            full_speed_fallback,
            source: None,
        };
        let fans = HashMap::from([("shared".to_string(), reading(Some(55.0), false))]);
        let aio = HashMap::from([
            ("shared".to_string(), reading(None, true)),
            ("pump".to_string(), reading(Some(40.0), false)),
        ]);
        let merged = merge_curve_temperatures(fans, aio);
        assert_eq!(merged["shared"], reading(Some(55.0), true));
        assert_eq!(merged["pump"], reading(Some(40.0), false));
    }

    #[test]
    fn pump_telemetry_occupies_its_own_slot_even_without_fans() {
        assert_eq!(with_pump_rpm(vec![], 0, Some(2500)), vec![2500]);
        assert_eq!(
            with_pump_rpm(vec![1000], 3, Some(2500)),
            vec![1000, 0, 0, 2500]
        );
        assert_eq!(with_pump_rpm(vec![1000], 1, None), vec![1000]);
    }

    #[test]
    fn older_device_info_defaults_pump_sync_to_unavailable() {
        let mut value = serde_json::to_value(dev("old", None)).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("pump_mb_sync_support");
        let decoded: DeviceInfo = serde_json::from_value(value).unwrap();
        assert!(!decoded.pump_mb_sync_support);
    }

    #[test]
    fn h264_toggle_is_offered_only_on_h264_panels_under_one_key_per_port() {
        let panel = |family, pid, topology: &str| DeviceInfo {
            family,
            pid,
            has_lcd: true,
            ..dev("panel", Some(topology))
        };
        let lcd = panel(DeviceFamily::UniversalScreen, 0xA088, "1cbe:a088:1-3.2");
        let expected = Some(H264Setting {
            key: "usb:1-3.2".into(),
            jpeg_available: true,
        });
        assert_eq!(h264_setting(&lcd, |_, _| unreachable!()), expected);
        let desktop = panel(
            DeviceFamily::UniversalScreenDesktop,
            0xAD21,
            "1a86:ad21:1-3.2",
        );
        assert_eq!(h264_setting(&desktop, |_, _| Some(true)), expected);
        assert_eq!(
            h264_setting(&desktop, |key, pid| {
                assert_eq!((key, pid), ("usb:1-3.2", 0xAD21));
                Some(false)
            })
            .map(|setting| setting.jpeg_available),
            Some(false)
        );
        assert_eq!(h264_setting(&desktop, |_, _| None), None);
        let jpeg_only = panel(DeviceFamily::Lancool207, 0xA065, "1cbe:a065:1-4");
        assert_eq!(h264_setting(&jpeg_only, |_, _| unreachable!()), None);
        assert_eq!(
            h264_setting(&dev("fan", Some("0cf2:a102:1-5")), |_, _| None),
            None
        );
    }

    fn dev(device_id: &str, topology_key: Option<&str>) -> DeviceInfo {
        DeviceInfo {
            startup_image: None,
            telemetry: None,
            device_id: device_id.to_string(),
            family: DeviceFamily::WiredReceiver,
            name: "Test".to_string(),
            serial: Some(device_id.to_string()),
            vid: 0x43A8,
            pid: 0x0101,
            has_lcd: false,
            has_fan: true,
            has_pump: false,
            has_rgb: true,
            has_pump_control: false,
            fan_count: Some(3),
            per_fan_control: None,
            mb_sync_support: false,
            pump_mb_sync_support: false,
            rgb_zone_count: None,
            screen_width: None,
            screen_height: None,
            is_unbound_wireless: false,
            wireless_bind_status: None,
            foreign_master_online: false,
            pump_rpm_range: None,
            fan_quantity: None,
            max_fan_quantity: None,
            firmware_version: None,
            supports_c_command: false,
            port_index: None,
            wireless_group_mac: None,
            topology_key: topology_key.map(str::to_string),
            h264: None,
        }
    }

    #[test]
    fn tag_fan_entries_matches_base_and_port_ids() {
        let mac = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
        let mut links = HashMap::new();
        links.insert("hid:abc", mac);
        let mut fan_info = vec![
            dev("hid:abc", None),
            dev("hid:abc:port2", None),
            dev("hid:other", None),
        ];
        tag_fan_entries(&mut fan_info, &links);
        assert_eq!(
            fan_info[0].wireless_group_mac.as_deref(),
            Some("11:22:33:44:55:66")
        );
        assert_eq!(
            fan_info[1].wireless_group_mac.as_deref(),
            Some("11:22:33:44:55:66")
        );
        assert_eq!(fan_info[2].wireless_group_mac, None);
    }

    #[test]
    fn cache_entry_dropped_when_topology_opened() {
        let mut topos = HashSet::new();
        topos.insert("1-2.3");
        let d = retained_cache_entry(dev("hid:abc", Some("1-2.3")), &topos, &HashSet::new());
        assert!(d.is_none());
    }

    #[test]
    fn cache_entry_stale_tag_cleared() {
        let mut bound = HashSet::new();
        bound.insert("aa:bb:cc:dd:ee:ff".to_string());
        let mut entry = dev("hid:abc", Some("1-2.3"));
        entry.wireless_group_mac = Some("11:22:33:44:55:66".to_string());
        let kept = retained_cache_entry(entry, &HashSet::new(), &bound).unwrap();
        assert_eq!(kept.wireless_group_mac, None);
    }

    #[test]
    fn cache_entry_bound_tag_kept() {
        let mut bound = HashSet::new();
        bound.insert("aa:bb:cc:dd:ee:ff".to_string());
        let mut entry = dev("hid:abc", None);
        entry.wireless_group_mac = Some("aa:bb:cc:dd:ee:ff".to_string());
        let kept = retained_cache_entry(entry, &HashSet::new(), &bound).unwrap();
        assert_eq!(
            kept.wireless_group_mac.as_deref(),
            Some("aa:bb:cc:dd:ee:ff")
        );
    }
}

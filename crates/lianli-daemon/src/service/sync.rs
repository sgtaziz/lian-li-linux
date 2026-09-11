use super::ServiceManager;
use lianli_devices::detect::{enumerate_devices, probe_tl_lcd_port_indices};
use lianli_shared::device_id::DeviceFamily;
use lianli_shared::ipc::DeviceInfo;
use lianli_shared::screen::screen_info_for;
use std::collections::{HashMap, HashSet};
use tracing::{debug, warn};

impl ServiceManager {
    /// Sync current config to IPC shared state.
    pub(super) fn sync_ipc_state(&self) {
        let mut ipc_state = self.ipc.state.lock();
        ipc_state.config = self.config.clone();
    }

    /// Refresh the cached USB device list (full bus enumeration).
    pub(super) fn refresh_usb_device_cache(&mut self) {
        match enumerate_devices() {
            Ok(usb_devices) => {
                self.refresh_tl_lcd_port_index_cache(&usb_devices);
                self.build_usb_device_cache(usb_devices);
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
        let streaming_active = !self.targets.lock().is_empty();

        // OpenRGB server status
        let (enabled, _) = self
            .config
            .as_ref()
            .and_then(|c| c.rgb.as_ref())
            .map(|rgb| (rgb.openrgb_server, rgb.openrgb_port))
            .unwrap_or((false, 6743));
        let openrgb_status = {
            let orgb_state = self.openrgb.state.lock();
            lianli_shared::ipc::OpenRgbServerStatus {
                enabled,
                running: orgb_state.running,
                port: orgb_state.port,
                error: orgb_state.error.clone(),
            }
        };

        // Build device list from wireless discovery
        let mut devices = Vec::new();

        let mut fan_rpms: std::collections::HashMap<String, Vec<u16>> =
            std::collections::HashMap::new();
        let mut coolant_temps: std::collections::HashMap<String, f32> =
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
                rgb_zone_count: Some(rgb_zone_count),
                screen_width: None,
                screen_height: None,
                is_unbound_wireless: false,
                wireless_bind_status: Some("bind_link".to_string()),
                foreign_master_online: false,
                pump_rpm_range: dev.fan_type.pump_rpm_range(),
                fan_quantity: None,
                max_fan_quantity: None,
                firmware_version: None,
                supports_c_command: false,
                port_index: None,
                wireless_group_mac: None,
                topology_key: None,
            });

            // Update RPM telemetry keyed by device_id
            let device_id = format!("wireless:{}", dev.mac_str());
            let mut rpms: Vec<u16> = dev.fan_rpms[..dev.fan_count as usize].to_vec();
            if is_aio {
                rpms.push(dev.fan_rpms[3]); // pump RPM
            }
            fan_rpms.insert(device_id.clone(), rpms);

            if let Some(temp) = dev.coolant_temp_c {
                coolant_temps.insert(device_id.clone(), temp as f32);
                lianli_shared::sensors::write_coolant_temp(&device_id, temp as f32);
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

        // Read wired fan RPMs and split per port.
        for (base_id, dev) in self.registry.fan_devices.iter() {
            // Coolant telemetry for wired AIOs (HydroShift LCD family),
            // mirroring the wireless path: publish via IPC and register as a
            // fan-curve sensor source.
            if let Some(temp) = dev.poll_coolant_temp() {
                coolant_temps.insert(base_id.clone(), temp);
                lianli_shared::sensors::write_coolant_temp(base_id, temp);
            }
            if let Ok(all_rpms) = dev.read_fan_rpm() {
                let ports = dev.fan_port_info();
                let per_fan = dev.per_fan_control();
                let mut offset = 0;
                for &(port, count) in &ports {
                    let port_rpms = if per_fan {
                        let end = (offset + count as usize).min(all_rpms.len());
                        let mut v = all_rpms[offset..end].to_vec();
                        offset = end;
                        // AIO pump RPM rides in the last telemetry slot
                        // (GUI reads rpms[fan_count]).
                        if count > 0 {
                            if let Some(pump) = dev.read_pump_rpm() {
                                v.push(pump);
                            }
                        }
                        v
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

        {
            let mut ipc_state = self.ipc.state.lock();
            ipc_state.telemetry.streaming_active = streaming_active;
            ipc_state.telemetry.openrgb_status = openrgb_status;
            ipc_state.telemetry.fan_rpms = fan_rpms;
            ipc_state.telemetry.coolant_temps = coolant_temps;
            ipc_state.telemetry.pixel_clean_statuses = ipc_state.pixel_clean_statuses();
            ipc_state.devices = devices;
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(device_id: &str, topology_key: Option<&str>) -> DeviceInfo {
        DeviceInfo {
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

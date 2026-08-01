use super::ServiceManager;
use lianli_devices::detect::{enumerate_devices, probe_tl_lcd_port_indices_rusb};
use lianli_shared::device_id::DeviceFamily;
use lianli_shared::ipc::DeviceInfo;
use lianli_shared::screen::screen_info_for;
use std::collections::HashSet;
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
        let probed = probe_tl_lcd_port_indices_rusb(usb_devices);
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
        let v2_hid_entries = lianli_devices::wireless::query_v2_hid_macs();
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
                pump_rpm_range: None,
                fan_quantity: None,
                max_fan_quantity: None,
                firmware_version,
                supports_c_command,
                port_index,
                wireless_group_mac,
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
        let mut ipc_state = self.ipc.state.lock();
        ipc_state.telemetry.streaming_active = !self.targets.lock().is_empty();

        // OpenRGB server status
        let (enabled, _) = self
            .config
            .as_ref()
            .and_then(|c| c.rgb.as_ref())
            .map(|rgb| (rgb.openrgb_server, rgb.openrgb_port))
            .unwrap_or((false, 6743));
        let orgb_state = self.openrgb.state.lock();
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
                pump_rpm_range: dev.fan_type.pump_rpm_range(),
                fan_quantity: None,
                max_fan_quantity: None,
                firmware_version: None,
                supports_c_command: false,
                port_index: None,
                wireless_group_mac: None,
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
                pump_rpm_range: dev.fan_type.pump_rpm_range(),
                fan_quantity: None,
                max_fan_quantity: None,
                firmware_version: None,
                supports_c_command: false,
                port_index: None,
                wireless_group_mac: None,
            });
        }

        // Add wired USB/HID fan devices (per-port entries from open_wired_fan_devices)
        devices.extend(self.registry.fan_device_info.clone());

        // Read wired fan RPMs and split per port.
        for (base_id, dev) in self.registry.fan_devices.iter() {
            if let Ok(all_rpms) = dev.read_fan_rpm() {
                let ports = dev.fan_port_info();
                let per_fan = dev.per_fan_control();
                let mut offset = 0;
                for &(port, count) in &ports {
                    let port_rpms = if per_fan {
                        let end = (offset + count as usize).min(all_rpms.len());
                        let v = all_rpms[offset..end].to_vec();
                        offset = end;
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
                    ipc_state.telemetry.fan_rpms.insert(device_id, port_rpms);
                }
            }
        }

        // Cache is refreshed every USB_ENUM_INTERVAL (30s) to avoid
        // USB bus contention from opening every device for serial reads.
        // Drop entries already emitted from fan_device_info above so each
        // physical endpoint surfaces exactly once.
        let opened_ids: std::collections::HashSet<&str> = self
            .registry
            .fan_device_info
            .iter()
            .map(|d| d.device_id.as_str())
            .collect();
        devices.extend(
            self.registry
                .cached_usb_devices
                .iter()
                .filter(|d| !opened_ids.contains(d.device_id.as_str()))
                .cloned(),
        );

        ipc_state.devices = devices;
    }
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
            return Some(format!(
                "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                entry.mac[0], entry.mac[1], entry.mac[2], entry.mac[3], entry.mac[4], entry.mac[5],
            ));
        }
    }
    None
}

use super::ServiceManager;
use lianli_devices::detect::enumerate_devices;
use lianli_shared::ipc::DeviceInfo;
use lianli_shared::screen::screen_info_for;
use tracing::warn;

impl ServiceManager {
    /// Sync current config to IPC shared state.
    pub(super) fn sync_ipc_state(&self) {
        let mut ipc_state = self.ipc_state.lock();
        ipc_state.config = self.config.clone();
    }

    /// Refresh the cached USB device list (full bus enumeration).
    pub(super) fn refresh_usb_device_cache(&mut self) {
        match enumerate_devices() {
            Ok(usb_devices) => {
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

                    // LCD-only USB facets: pump/fan/RGB are owned elsewhere
                    // (register_wired_controllers for HS / Galahad2, wireless dongle
                    // for HS II). Suppress the control-side tags here.
                    let lcd_only = matches!(
                        det.family,
                        lianli_shared::device_id::DeviceFamily::HydroShiftLcd
                            | lianli_shared::device_id::DeviceFamily::Galahad2Lcd
                            | lianli_shared::device_id::DeviceFamily::HydroShift2Lcd
                    );

                    let (firmware_version, supports_c_command) = self
                        .aio_lcd_info
                        .get(&device_id)
                        .cloned()
                        .unwrap_or((None, false));
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
                    });
                }

                self.cached_usb_devices = cached;
            }
            Err(e) => {
                warn!("USB enumeration failed: {e}");
            }
        }

        match crate::desktop_display::enumerate_turzx() {
            Ok(present) => self.desktop_displays.sync(&present),
            Err(e) => warn!("TURZX enumeration failed: {e:#}"),
        }
    }

    /// Update IPC telemetry and device list.
    pub(super) fn sync_ipc_telemetry(&self) {
        let mut ipc_state = self.ipc_state.lock();
        ipc_state.telemetry.streaming_active = !self.targets.is_empty();

        // OpenRGB server status
        let (enabled, _) = self
            .config
            .as_ref()
            .and_then(|c| c.rgb.as_ref())
            .map(|rgb| (rgb.openrgb_server, rgb.openrgb_port))
            .unwrap_or((false, 6743));
        let orgb_state = self.openrgb_state.lock();
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
                WirelessFanType::Slv3Led => DeviceFamily::Slv3Led,
                WirelessFanType::Slv3Lcd => DeviceFamily::Slv3Lcd,
                WirelessFanType::Tlv2Lcd => DeviceFamily::Tlv2Lcd,
                WirelessFanType::Tlv2Led => DeviceFamily::Tlv2Led,
                WirelessFanType::SlInf => DeviceFamily::SlInf,
                WirelessFanType::Clv1 => DeviceFamily::Clv1,
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
                mb_sync_support: dev.fan_type.supports_hw_mobo_sync()
                    || self.wireless.motherboard_pwm().is_some(),
                rgb_zone_count: Some(rgb_zone_count),
                screen_width: None,
                screen_height: None,
                is_unbound_wireless: false,
                pump_rpm_range: dev.fan_type.pump_rpm_range(),
                fan_quantity: None,
                max_fan_quantity: None,
                firmware_version: None,
                supports_c_command: false,
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
                WirelessFanType::Slv3Led => DeviceFamily::Slv3Led,
                WirelessFanType::Slv3Lcd => DeviceFamily::Slv3Lcd,
                WirelessFanType::Tlv2Lcd => DeviceFamily::Tlv2Lcd,
                WirelessFanType::Tlv2Led => DeviceFamily::Tlv2Led,
                WirelessFanType::SlInf => DeviceFamily::SlInf,
                WirelessFanType::Clv1 => DeviceFamily::Clv1,
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
            });
        }

        // Add wired USB/HID fan devices (per-port entries from open_wired_fan_devices)
        devices.extend(self.wired_fan_device_info.clone());

        // Read wired fan RPMs and split per port.
        for (base_id, dev) in self.wired_fan_devices.iter() {
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
        devices.extend(self.cached_usb_devices.clone());

        ipc_state.devices = devices;
    }
}

use lianli_shared::device_id::DeviceFamily;
use lianli_shared::ipc::{DeviceInfo, TelemetrySnapshot};
use slint::{ModelRc, SharedString, VecModel};

pub(super) fn family_display_name(f: DeviceFamily) -> &'static str {
    match f {
        DeviceFamily::Ene6k77 => "UNI FAN SL/AL",
        DeviceFamily::TlFan => "UNI FAN TL",
        DeviceFamily::TlLcd => "UNI FAN TL LCD",
        DeviceFamily::Galahad2Trinity => "Galahad II Trinity",
        DeviceFamily::HydroShiftLcd => "HydroShift LCD",
        DeviceFamily::Galahad2Lcd => "Galahad II LCD",
        DeviceFamily::WirelessTx => "Wireless TX Dongle",
        DeviceFamily::WirelessRx => "Wireless RX Dongle",
        DeviceFamily::Slv3Lcd => "UNI FAN SL Wireless LCD",
        DeviceFamily::Slv3Led => "UNI FAN SL Wireless",
        DeviceFamily::Tlv2Lcd => "UNI FAN TL Wireless LCD",
        DeviceFamily::Tlv2Led => "UNI FAN TL Wireless",
        DeviceFamily::SlInf => "UNI FAN SL-INF Wireless",
        DeviceFamily::Clv1 => "UNI FAN CL Wireless",
        DeviceFamily::HydroShift2Lcd => "HydroShift II LCD Circle",
        DeviceFamily::Lancool207 => "Lancool 207 Digital",
        DeviceFamily::UniversalScreen => "Universal Screen 8.8\"",
        DeviceFamily::HydroShift2LcdDesktop => "HydroShift II LCD (Desktop Mode)",
        DeviceFamily::Lancool207Desktop => "Lancool 207 Digital (Desktop Mode)",
        DeviceFamily::UniversalScreenDesktop => "Universal Screen 8.8\" (Desktop Mode)",
        DeviceFamily::WirelessAio => "HydroShift Wireless AIO",
        DeviceFamily::WirelessStrimer => "Strimer Plus Wireless",
        DeviceFamily::WirelessLc217 => "Lancool 217 Wireless",
        DeviceFamily::WirelessLed88 => "Universal Screen 8.8\" Wireless",
        DeviceFamily::WirelessV150 => "Lancool V150 Wireless",
        DeviceFamily::StrimerPlus => "Strimer Plus",
        DeviceFamily::UniversalScreenLighting => "Universal Screen 8.8\" LED Ring",
        DeviceFamily::Vision9p2 => "Vision 9.2\"",
        DeviceFamily::Vision9p2Desktop => "Vision 9.2\" (Desktop Mode)",
        DeviceFamily::TlFlexLcd => "TL Flex LCD",
        DeviceFamily::SlInfFlexLcd => "SL Infinity Flex LCD",
        DeviceFamily::WiredReceiver => "Wired Controller",
        DeviceFamily::HydroShift2OledCurveLcd => "HydroShift II OLED Curve",
        DeviceFamily::HydroShift2OledCurveLed => "HydroShift II OLED Curve LED",
    }
}

pub fn device_to_slint(
    device: &DeviceInfo,
    telemetry: &TelemetrySnapshot,
    pending_actions: &std::collections::HashMap<
        String,
        (crate::state::PendingAction, std::time::Instant),
    >,
) -> crate::DeviceData {
    let fan_rpms = telemetry
        .fan_rpms
        .get(&device.device_id)
        .map(|rpms| {
            rpms.iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let coolant_temp = telemetry
        .coolant_temps
        .get(&device.device_id)
        .map(|t| format!("{t:.1}\u{00B0}C"))
        .unwrap_or_default();

    let resolution = match (device.screen_width, device.screen_height) {
        (Some(w), Some(h)) => format!("{w}x{h}"),
        _ => String::new(),
    };

    let family_name = family_display_name(device.family);
    let is_bound_wireless = device.device_id.starts_with("wireless:");
    let pending = pending_actions
        .get(&device.device_id)
        .map(|(action, _)| action.as_str())
        .unwrap_or("");

    crate::DeviceData {
        device_id: SharedString::from(&device.device_id),
        family_name: SharedString::from(family_name),
        name: SharedString::from(&device.name),
        serial: SharedString::from(device.serial.as_deref().unwrap_or("")),
        has_lcd: device.has_lcd,
        has_fan: device.has_fan,
        has_pump: device.has_pump,
        has_rgb: device.has_rgb,
        fan_rpms: SharedString::from(&fan_rpms),
        coolant_temp: SharedString::from(&coolant_temp),
        resolution: SharedString::from(&resolution),
        in_desktop_mode: device.family.is_desktop_mode(),
        in_lcd_mode: device.family.supports_display_mode_switch()
            && !device.family.is_desktop_mode(),
        is_unbound_wireless: device.is_unbound_wireless,
        is_bound_wireless,
        pending_action: SharedString::from(pending),
        fan_quantity: device.fan_quantity.unwrap_or(0) as i32,
        max_fan_quantity: device.max_fan_quantity.unwrap_or(0) as i32,
    }
}

pub fn devices_to_model(
    devices: &[DeviceInfo],
    telemetry: &TelemetrySnapshot,
    pending_actions: &std::collections::HashMap<
        String,
        (crate::state::PendingAction, std::time::Instant),
    >,
) -> ModelRc<crate::DeviceData> {
    let items: Vec<crate::DeviceData> = devices
        .iter()
        .filter(|d| {
            !matches!(
                d.family,
                lianli_shared::device_id::DeviceFamily::WirelessTx
                    | lianli_shared::device_id::DeviceFamily::WirelessRx
            )
        })
        .map(|d| device_to_slint(d, telemetry, pending_actions))
        .collect();
    ModelRc::new(VecModel::from(items))
}

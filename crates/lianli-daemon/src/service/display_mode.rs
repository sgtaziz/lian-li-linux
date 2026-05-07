use super::runtime::LcdBackend;
use super::{DaemonEvent, ServiceManager};
use lianli_devices::winusb_lcd::WinUsbLcdDevice;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{info, warn};

impl ServiceManager {
    pub(super) fn handle_display_switch_to_desktop(&mut self, device_id: &str) {
        // Find and remove the active LCD target for this device
        let target_idx = self.targets.iter().find_map(|(&idx, t)| {
            if t.device_identity == *device_id {
                Some(idx)
            } else {
                None
            }
        });

        if let Some(idx) = target_idx {
            if let Some(mut target) = self.targets.remove(&idx) {
                target.stop();
                if let LcdBackend::WinUsb(ref mut lcd) = target.lcd {
                    match lcd.switch_to_desktop_mode() {
                        Ok(()) => {
                            info!("Switched {device_id} to desktop mode");
                            self.mark_mode_switch(device_id);
                        }
                        Err(e) => warn!("Failed to switch {device_id} to desktop mode: {e}"),
                    }
                } else {
                    warn!("Device {device_id} is not a WinUSB LCD, cannot switch to desktop mode");
                }
            }
        } else {
            info!("No active LCD target for {device_id}, opening temporary connection");
            let det = self
                .cached_usb_devices
                .iter()
                .find(|d| d.device_id == *device_id);
            if let Some(det) = det {
                let family = det.family;
                if let Ok(usb_devs) = lianli_devices::detect::enumerate_devices() {
                    for usb_det in usb_devs {
                        if usb_det.family == family && usb_det.device_id() == *device_id {
                            let screen = lianli_shared::screen::screen_info_for(family)
                                .unwrap_or(lianli_shared::screen::ScreenInfo::AIO_LCD_480);
                            match WinUsbLcdDevice::new(usb_det.device, screen, det.name.as_str()) {
                                Ok(mut lcd) => match lcd.switch_to_desktop_mode() {
                                    Ok(()) => {
                                        info!("Switched {device_id} to desktop mode");
                                        self.mark_mode_switch(device_id);
                                    }
                                    Err(e) => warn!("Switch to desktop failed: {e}"),
                                },
                                Err(e) => warn!("Failed to open {device_id} for mode switch: {e}"),
                            }
                            break;
                        }
                    }
                }
            } else {
                warn!("Device {device_id} not found in cached devices");
            }
        }

        self.schedule_post_switch_refresh();
    }

    pub(super) fn handle_display_switch_to_lcd(&mut self, device_id: &str, pid: u16) {
        self.desktop_displays.stop_for_pid(pid);
        self.mark_mode_switch(device_id);
        thread::sleep(Duration::from_millis(300));

        match hidapi::HidApi::new() {
            Ok(api) => match lianli_devices::display_switcher::switch_to_lcd_mode(&api, pid) {
                Ok(()) => info!("Switched {device_id} to LCD mode"),
                Err(e) => warn!("Failed to switch {device_id} to LCD mode: {e:#}"),
            },
            Err(e) => warn!("Failed to open HID for switch-to-LCD: {e:#}"),
        }

        self.schedule_post_switch_refresh();
    }

    /// Wake the USB cache + device poll a few times in the seconds following a
    /// mode switch, so the rebooted device shows up without waiting for the
    /// next 10-second enumeration tick.
    fn schedule_post_switch_refresh(&self) {
        let Some(tx) = self.tx.clone() else { return };
        thread::spawn(move || {
            for delay_secs in [3u64, 3, 3] {
                thread::sleep(Duration::from_secs(delay_secs));
                if tx.send(DaemonEvent::USBCheck).is_err() {
                    return;
                }
                let _ = tx.send(DaemonEvent::DevicePoll);
            }
        });
    }

    fn mark_mode_switch(&mut self, device_id: &str) {
        self.mode_switch_suppression.insert(
            device_id.to_string(),
            Instant::now() + Duration::from_secs(8),
        );
    }

    pub(super) fn mode_switch_suppressed(&self, device_id: &str) -> bool {
        self.mode_switch_suppression
            .get(device_id)
            .is_some_and(|until| Instant::now() < *until)
    }
}

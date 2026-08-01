use anyhow::Result;
use lianli_shared::device_id::DeviceFamily;
use lianli_transport::RusbHid;
use parking_lot::Mutex;
use std::sync::Arc;

/// Result of initializing a wired HID controller that may provide fan, RGB, or both.
pub struct WiredControllerSet {
    pub fan: Option<Box<dyn crate::traits::FanDevice>>,
    pub rgb: Vec<(String, Arc<dyn crate::traits::RgbDevice>)>,
}

/// Create all controllers (fan + RGB) for a device family in a single init pass.
pub fn create_wired_controllers(
    family: DeviceFamily,
    pid: u16,
    backend: Arc<Mutex<RusbHid>>,
) -> Option<Result<WiredControllerSet>> {
    match family {
        DeviceFamily::TlFan => Some(crate::tl_fan::TlFanController::new(backend).map(|ctrl| {
            let ctrl = Arc::new(ctrl);
            let rgb: Vec<_> = ctrl
                .port_devices()
                .into_iter()
                .map(|(port, dev)| {
                    (
                        format!("port{port}"),
                        Arc::new(dev) as Arc<dyn crate::traits::RgbDevice>,
                    )
                })
                .collect();
            WiredControllerSet {
                fan: Some(Box::new(ctrl)),
                rgb,
            }
        })),
        DeviceFamily::Ene6k77 => Some(crate::ene6k77::Ene6k77Controller::new(backend, pid).map(
            |ctrl| {
                let ctrl = Arc::new(ctrl);
                let rgb: Vec<_> = ctrl
                    .group_devices()
                    .into_iter()
                    .map(|(group, dev)| {
                        (
                            format!("group{group}"),
                            Arc::new(dev) as Arc<dyn crate::traits::RgbDevice>,
                        )
                    })
                    .collect();
                WiredControllerSet {
                    fan: Some(Box::new(Arc::clone(&ctrl))),
                    rgb,
                }
            },
        )),
        DeviceFamily::Galahad2Trinity => Some(
            crate::galahad2_trinity::Galahad2TrinityController::new(backend, pid).map(|c| {
                let ctrl = Arc::new(c);
                WiredControllerSet {
                    fan: Some(Box::new(Arc::clone(&ctrl))),
                    rgb: vec![(String::new(), ctrl as Arc<dyn crate::traits::RgbDevice>)],
                }
            }),
        ),
        DeviceFamily::HydroShiftLcd | DeviceFamily::Galahad2Lcd => Some(
            crate::hydroshift_lcd::HydroShiftLcdController::new(Arc::clone(&backend), pid)
                .and_then(|mut lcd_ctrl| {
                    crate::traits::LcdDevice::initialize(&mut lcd_ctrl)?;
                    let rgb_ctrl = crate::hydroshift_lcd::AioLcdRgbController::new(backend, pid)?;
                    Ok(WiredControllerSet {
                        fan: Some(Box::new(Arc::new(lcd_ctrl))),
                        rgb: vec![(
                            String::new(),
                            Arc::new(rgb_ctrl) as Arc<dyn crate::traits::RgbDevice>,
                        )],
                    })
                }),
        ),
        _ => None,
    }
}

/// Create an HID LCD controller from a pre-opened shared backend.
pub fn create_hid_lcd_device(
    family: DeviceFamily,
    pid: u16,
    backend: Arc<Mutex<RusbHid>>,
) -> Option<Result<Box<dyn crate::traits::LcdDevice>>> {
    match family {
        DeviceFamily::HydroShiftLcd | DeviceFamily::Galahad2Lcd => Some(
            crate::hydroshift_lcd::HydroShiftLcdController::new(backend, pid)
                .map(|d| Box::new(d) as Box<dyn crate::traits::LcdDevice>),
        ),
        DeviceFamily::TlLcd => {
            let mut tl = crate::tl_lcd::TlLcdDevice::new(backend);
            Some(
                crate::traits::LcdDevice::initialize(&mut tl)
                    .map(|_| Box::new(tl) as Box<dyn crate::traits::LcdDevice>),
            )
        }
        _ => None,
    }
}

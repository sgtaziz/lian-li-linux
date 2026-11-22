use super::*;
use lianli_devices::traits::RgbFrameDelivery;

#[test]
fn unchanged_rgb_can_be_saved_when_wireless_is_unavailable() {
    use lianli_shared::config::AppConfig;
    use lianli_shared::rgb::{RgbDeviceConfig, RgbZoneConfig};
    let root = tempfile::tempdir().unwrap();
    let mut controller = RgbController::new(HashMap::new(), None);
    controller.wireless_state.insert(
        "wireless:test".into(),
        WirelessDevice {
            mac: [1; 6],
            fan_type: WirelessFanType::Led88,
            fan_count: 0,
            right_attach: false,
        },
    );
    let rgb = RgbAppConfig {
        devices: vec![RgbDeviceConfig {
            device_id: "wireless:test".into(),
            fan_led_count: None,
            mb_rgb_sync: false,
            active_preset: None,
            regions: None,
            effect_memory: Vec::new(),
            port_merge: None,
            reverse_port: false,
            zones: vec![RgbZoneConfig {
                zone_index: 0,
                effect: RgbEffect::default(),
                swap_lr: false,
                swap_tb: false,
            }],
        }],
        ..Default::default()
    };
    assert!(controller
        .validate_config(&rgb)
        .unwrap_err()
        .to_string()
        .contains("wireless RGB controller is unavailable"));
    let mut daemon = crate::ipc::DaemonState::new(root.path().join("config.json"));
    daemon.config = Some(AppConfig {
        rgb: Some(rgb.clone()),
        ..Default::default()
    });
    daemon.rgb_controller = Some(Arc::new(parking_lot::Mutex::new(controller)));
    let state = Arc::new(parking_lot::Mutex::new(daemon));
    assert!(crate::ipc::rgb::validate_saved_config(&state, &rgb).is_none());
    let mut changed = rgb.clone();
    changed.devices[0].zones[0].effect.brightness = 1;
    assert!(crate::ipc::rgb::validate_saved_config(&state, &changed).is_some());
    assert!(crate::ipc::rgb::validate_config(&state, &rgb).is_some());
    let mut config = state.lock().config.clone().unwrap();
    config.hardware_video = true;
    let (tx, rx) = std::sync::mpsc::channel();
    assert!(matches!(
        crate::ipc::persist_and_notify(&mut state.lock(), &tx.into(), "SetConfig", config),
        lianli_shared::ipc::IpcResponse::Ok { .. }
    ));
    assert!(matches!(
        rx.try_recv().unwrap(),
        crate::service::DaemonEvent::IpcUpdate
    ));
    let saved: AppConfig =
        serde_json::from_slice(&std::fs::read(root.path().join("config.json")).unwrap()).unwrap();
    assert!(saved.hardware_video);
    assert_eq!(saved.rgb, Some(rgb));
}

#[test]
fn openrgb_ownership_cannot_resubmit_the_previous_native_loop() {
    let mut controller = RgbController::new(HashMap::new(), None);
    controller.uploads.insert(
        "wireless:old".into(),
        Arc::new(WirelessRgbUpload::new(&[vec![[255, 0, 0]; 26]], 50, None).unwrap()),
    );
    controller.set_openrgb_active(true);
    assert!(controller.uploads.is_empty());
    assert!(controller.is_openrgb_controlled());
}

struct MockRgb {
    rf: bool,
    frames: Option<std::sync::mpsc::Sender<Vec<Vec<[u8; 3]>>>>,
}
impl RgbDevice for MockRgb {
    fn device_name(&self) -> String {
        "mock".into()
    }
    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Static]
    }
    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        if self.frames.is_some() {
            vec![RgbZoneInfo {
                name: "ring".into(),
                name_ui: None,
                led_count: 1,
            }]
        } else {
            vec![]
        }
    }
    fn set_zone_effect(&self, _: u8, _: &RgbEffect) -> anyhow::Result<()> {
        Ok(())
    }
    fn rf_owned(&self) -> bool {
        self.rf
    }
    fn software_frame_delivery(&self) -> Option<RgbFrameDelivery> {
        self.frames.as_ref().map(|_| RgbFrameDelivery::Streaming)
    }
    fn set_software_frames(&self, frames: &[Vec<[u8; 3]>], _: u16) -> anyhow::Result<()> {
        self.frames.as_ref().unwrap().send(frames.to_vec())?;
        Ok(())
    }
}

#[test]
fn exposed_capabilities_hide_rf_owned_and_do_not_infer_software_support() {
    let wired = HashMap::from([
        (
            "hid:kept".into(),
            Arc::new(MockRgb {
                rf: false,
                frames: None,
            }) as Arc<dyn RgbDevice>,
        ),
        (
            "hid:rf".into(),
            Arc::new(MockRgb {
                rf: true,
                frames: None,
            }) as Arc<dyn RgbDevice>,
        ),
    ]);
    let controller = RgbController::new(wired, None);
    let caps = controller.exposed_capabilities();
    assert_eq!(caps.len(), 1);
    assert_eq!(caps[0].device_id, "hid:kept");
    assert!(caps[0].software_modes.is_empty());
    assert_eq!(caps[0].supported_modes, [RgbMode::Static]);
}

#[test]
fn wireless_capabilities_follow_runtime_layout_and_reject_unknown_types() {
    let mut controller = RgbController::new(HashMap::new(), None);
    for (id, fan_type, fan_count) in [
        ("wireless:pump", WirelessFanType::WaterBlock2, 3),
        ("wireless:v150", WirelessFanType::V150, 4),
        ("wireless:unknown", WirelessFanType::Unknown, 1),
        ("wireless:unknown-strimer", WirelessFanType::Strimer(5), 0),
        ("wireless:oversize", WirelessFanType::SlV4, 6),
    ] {
        controller.wireless_state.insert(
            id.into(),
            WirelessDevice {
                mac: [0; 6],
                fan_type,
                fan_count,
                right_attach: false,
            },
        );
    }
    let caps = controller.capabilities();
    assert_eq!(caps.len(), 2);
    assert_eq!(
        caps[0]
            .zones
            .iter()
            .map(|z| z.led_count)
            .collect::<Vec<_>>(),
        [24, 24, 24, 24]
    );
    assert_eq!(caps[1].zones[0].led_count, 88);
    assert!(caps[0].supported_modes.contains(&RgbMode::Rainbow));
    let pump = caps[0]
        .region_parameters
        .iter()
        .find(|r| r.scope == lianli_shared::rgb::RgbScope::Pump)
        .unwrap();
    let outer = caps[0]
        .region_parameters
        .iter()
        .find(|r| r.scope == lianli_shared::rgb::RgbScope::Outer)
        .unwrap();
    assert!(pump.effects.iter().any(|p| p.mode == RgbMode::Pump));
    assert!(!outer.effects.iter().any(|p| p.mode == RgbMode::Pump));
    assert!(outer.effects.iter().any(|p| p.mode == RgbMode::MopUp));
    assert!(!pump.effects.iter().any(|p| p.mode == RgbMode::MopUp));
}

#[test]
fn regional_capabilities_follow_the_effect_engine() {
    use lianli_shared::rgb::RgbScope;
    let mut controller = RgbController::new(HashMap::new(), None);
    for (id, fan_type, fan_count) in [
        ("tl", WirelessFanType::Tlv2Led, 3),
        ("screen", WirelessFanType::Led88, 0),
    ] {
        controller.wireless_state.insert(
            id.into(),
            WirelessDevice {
                mac: [0; 6],
                fan_type,
                fan_count,
                right_attach: false,
            },
        );
    }
    let caps = controller.capabilities();
    let screen = caps.iter().find(|c| c.device_id == "screen").unwrap();
    assert_eq!(screen.effect_regions, [RgbScope::All]);
    assert!(screen.software_modes.contains(&RgbMode::RainbowWave));
    let tl = caps.iter().find(|c| c.device_id == "tl").unwrap();
    assert_eq!(
        tl.effect_regions,
        [RgbScope::All, RgbScope::Top, RgbScope::Bottom]
    );
    assert_eq!(tl.total_led_count, 78);
    assert!(!tl.software_modes.contains(&RgbMode::RainbowWave));
    let breathing = tl
        .effect_parameters
        .iter()
        .find(|p| p.mode == RgbMode::Breathing)
        .unwrap();
    assert!(breathing.per_fan_colors);
    assert_eq!((breathing.min_colors, breathing.max_colors), (3, 3));
    assert!(breathing.directions.is_empty());
}

#[test]
fn raw_frame_requests_reach_wired_software_devices() {
    let (frames, received) = std::sync::mpsc::channel();
    let mut controller = RgbController::new(
        HashMap::from([(
            "wired".into(),
            Arc::new(MockRgb {
                rf: false,
                frames: Some(frames),
            }) as Arc<dyn RgbDevice>,
        )]),
        None,
    );
    assert!(controller.set_rgb_frames("wired", &[], 50).is_err());
    assert!(controller
        .set_rgb_frames("wired", &[vec![[0; 3]; 2]], 50)
        .is_err());
    controller
        .set_rgb_frames("wired", &[vec![[7, 8, 9]]], 50)
        .unwrap();
    assert_eq!(
        received
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap(),
        vec![vec![[7, 8, 9]]]
    );
    assert_eq!(
        controller.get_zone_colors("wired", 0).unwrap(),
        vec![[7, 8, 9]]
    );
}

#[test]
fn device_removal_releases_playback_and_cached_state() {
    let (frames, received) = std::sync::mpsc::channel();
    let mut controller = RgbController::new(
        HashMap::from([(
            "wired".into(),
            Arc::new(MockRgb {
                rf: false,
                frames: Some(frames),
            }) as Arc<dyn RgbDevice>,
        )]),
        None,
    );
    controller
        .set_rgb_frames("wired", &[vec![[1, 2, 3]]], 50)
        .unwrap();
    received
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    controller.retain_wired(&Default::default());
    assert!(controller.get_zone_colors("wired", 0).is_none());
    assert!(controller.capabilities().is_empty());
    assert_eq!(
        received.recv_timeout(std::time::Duration::from_secs(1)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    );
}

struct MergePort {
    port: u8,
    log: Arc<parking_lot::Mutex<Vec<String>>>,
}
impl RgbDevice for MergePort {
    fn device_name(&self) -> String {
        "merge".into()
    }
    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Rainbow]
    }
    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![RgbZoneInfo {
            name: "Fan 1".into(),
            name_ui: None,
            led_count: 1,
        }]
    }
    fn set_zone_effect(&self, _: u8, _: &RgbEffect) -> anyhow::Result<()> {
        self.log.lock().push(format!("effect{}", self.port));
        Ok(())
    }
    fn merged_effect_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Rainbow]
    }
    fn apply_port_merge(
        &self,
        merge: Option<&lianli_shared::rgb::RgbPortMerge>,
    ) -> anyhow::Result<()> {
        let action = if merge.is_some() { "merge" } else { "split" };
        self.log.lock().push(action.into());
        Ok(())
    }
    fn supports_direction(&self) -> bool {
        true
    }
    fn set_fan_direction(&self, zone: u8, swap_lr: bool, _: bool) -> anyhow::Result<()> {
        self.log
            .lock()
            .push(format!("dir{}.{zone}={swap_lr}", self.port));
        Ok(())
    }
    fn supports_port_reverse(&self) -> bool {
        true
    }
    fn set_port_reverse(&self, reversed: bool) -> anyhow::Result<()> {
        self.log
            .lock()
            .push(format!("reverse{}={reversed}", self.port));
        Ok(())
    }
}

#[test]
fn merged_ports_send_one_effect_from_the_lowest_port_and_split_before_reapplying() {
    use lianli_shared::rgb::{RgbDeviceConfig, RgbPortMerge, RgbZoneConfig};
    let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let wired = [0, 2]
        .into_iter()
        .map(|port| {
            (
                format!("hid:tl:port{port}"),
                Arc::new(MergePort {
                    port,
                    log: log.clone(),
                }) as Arc<dyn RgbDevice>,
            )
        })
        .collect();
    let mut controller = RgbController::new(wired, None);
    let port_config = |port: u8, merged: bool, swap_lr: bool| RgbDeviceConfig {
        device_id: format!("hid:tl:port{port}"),
        fan_led_count: None,
        mb_rgb_sync: false,
        active_preset: None,
        regions: None,
        effect_memory: Vec::new(),
        port_merge: Some(RgbPortMerge {
            enabled: merged,
            ..Default::default()
        }),
        reverse_port: port == 2,
        zones: vec![RgbZoneConfig {
            zone_index: 0,
            effect: RgbEffect {
                mode: RgbMode::Rainbow,
                ..Default::default()
            },
            swap_lr,
            swap_tb: false,
        }],
    };
    let mut rgb = RgbAppConfig {
        devices: vec![port_config(2, true, true), port_config(0, true, false)],
        ..Default::default()
    };
    controller.apply_config(&rgb, &[]);
    assert_eq!(
        *log.lock(),
        [
            "reverse2=true",
            "dir2.0=true",
            "merge",
            "reverse0=false",
            "dir0.0=false"
        ]
    );

    log.lock().clear();
    controller.apply_config(&rgb, &[]);
    assert!(log.lock().is_empty());

    rgb.devices = vec![port_config(2, false, false), port_config(0, false, false)];
    controller.apply_config(&rgb, &[]);
    assert_eq!(
        *log.lock(),
        [
            "split",
            "effect2",
            "reverse2=true",
            "dir2.0=false",
            "split",
            "effect0",
            "reverse0=false",
            "dir0.0=false"
        ]
    );
}

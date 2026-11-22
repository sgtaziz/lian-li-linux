use super::*;
use lianli_devices::traits::RgbFrameDelivery;
use lianli_shared::rgb::{
    MergeLightingConfig, RgbDeviceConfig, RgbRenderFamily, RgbRenderProfile, RgbZoneConfig,
};
use std::{sync::mpsc, time::Duration};

struct Screen(mpsc::Sender<Vec<[u8; 3]>>);

#[derive(Default)]
struct CountedFan {
    count: std::sync::atomic::AtomicU16,
    applied: parking_lot::Mutex<Vec<(u16, [u8; 3])>>,
}

impl RgbDevice for CountedFan {
    fn device_name(&self) -> String {
        "Galahad".into()
    }
    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Static]
    }
    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![RgbZoneInfo {
            name: "Fans".into(),
            name_ui: None,
            led_count: self.count.load(std::sync::atomic::Ordering::Relaxed),
        }]
    }
    fn fan_led_count_control(&self) -> Option<lianli_shared::rgb::RgbLedCountControl> {
        Some(lianli_shared::rgb::RgbLedCountControl {
            zone: 0,
            min: 8,
            max: 50,
            default: 24,
        })
    }
    fn configure_fan_led_count(&self, count: Option<u16>) -> anyhow::Result<bool> {
        let count = self
            .fan_led_count_control()
            .unwrap()
            .resolve(count)
            .map_err(anyhow::Error::msg)?;
        Ok(self.count.swap(count, std::sync::atomic::Ordering::Relaxed) != count)
    }
    fn set_zone_effect(&self, _: u8, effect: &RgbEffect) -> anyhow::Result<()> {
        self.applied.lock().push((
            self.count.load(std::sync::atomic::Ordering::Relaxed),
            effect.colors[0],
        ));
        Ok(())
    }
    fn supports_mb_rgb_sync(&self) -> bool {
        true
    }
    fn set_mb_rgb_sync(&self, _: bool) -> anyhow::Result<()> {
        Ok(())
    }
}

#[test]
fn fan_led_count_only_save_reapplies_quick_sync_without_changing_membership_or_effect() {
    let (mut controller, mut config, _) = setup();
    let device = Arc::new(CountedFan::default());
    controller.replace_wired(HashMap::from([(
        "screen".into(),
        device.clone() as Arc<dyn RgbDevice>,
    )]));
    config.merge_lighting.as_mut().unwrap().kind = lianli_shared::rgb::RgbSyncKind::Matched;
    controller.validate_config(&config).unwrap();
    controller.apply_config(&config, &[]);
    assert_eq!(device.applied.lock().last(), Some(&(24, [0, 255, 0])));
    device.applied.lock().clear();
    controller.apply_config(&config, &[]);
    assert!(device.applied.lock().is_empty());
    config.devices[0].fan_led_count = Some(50);
    controller.apply_config(&config, &[]);
    assert_eq!(device.applied.lock().as_slice(), &[(50, [0, 255, 0])]);
    assert!(controller.sync_active.contains("screen"));
    let thermal = crate::thermal_alert::new_shared();
    controller.set_thermal_override(thermal.clone());
    *thermal.lock() = Some([255, 128, 0]);
    assert!(controller.check_thermal_override());
    assert_eq!(device.applied.lock().last(), Some(&(50, [255, 128, 0])));
    config.devices[0].fan_led_count = Some(8);
    controller.apply_config(&config, &[]);
    *thermal.lock() = None;
    assert!(!controller.check_thermal_override());
    assert_eq!(device.applied.lock().last(), Some(&(8, [0, 255, 0])));
    config.devices[0].fan_led_count = Some(50);
    controller.apply_config(&config, &[]);
    config.merge_lighting.as_mut().unwrap().enabled = false;
    controller.apply_config(&config, &[]);
    assert_eq!(device.applied.lock().last(), Some(&(50, [255, 0, 0])));
    config.devices[0].mb_rgb_sync = true;
    controller.apply_config(&config, &[]);
    config.devices[0].mb_rgb_sync = false;
    controller.apply_config(&config, &[]);
    assert_eq!(device.applied.lock().last(), Some(&(50, [255, 0, 0])));
    controller.invalidate_hardware_state();
    controller.apply_config(&config, &[]);
    assert_eq!(device.applied.lock().last(), Some(&(50, [255, 0, 0])));
    let reconnected = Arc::new(CountedFan::default());
    controller.replace_wired(HashMap::from([(
        "screen".into(),
        reconnected.clone() as Arc<dyn RgbDevice>,
    )]));
    controller.apply_config(&config, &[]);
    assert_eq!(reconnected.applied.lock().last(), Some(&(50, [255, 0, 0])));
    config.devices[0].fan_led_count = Some(51);
    assert!(controller.validate_config(&config).is_err());
    controller.replace_wired(HashMap::new());
    assert!(controller.validate_config(&config).is_ok());
}

impl RgbDevice for Screen {
    fn device_name(&self) -> String {
        "screen".into()
    }
    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Static]
    }
    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![RgbZoneInfo {
            name: "ring".into(),
            name_ui: None,
            led_count: 60,
        }]
    }
    fn set_zone_effect(&self, _: u8, _: &RgbEffect) -> anyhow::Result<()> {
        Ok(())
    }
    fn software_render_profile(&self) -> Option<RgbRenderProfile> {
        Some(RgbRenderProfile {
            family: RgbRenderFamily::UniversalScreen,
            fan_count: 0,
            led_count: 60,
            right_attach: false,
        })
    }
    fn software_frame_delivery(&self) -> Option<RgbFrameDelivery> {
        Some(RgbFrameDelivery::Streaming)
    }
    fn set_software_frames(&self, frames: &[Vec<[u8; 3]>], _: u16) -> anyhow::Result<()> {
        self.0.send(frames[0].clone())?;
        Ok(())
    }
}

fn setup() -> (RgbController, RgbAppConfig, mpsc::Receiver<Vec<[u8; 3]>>) {
    let (sender, received) = mpsc::channel();
    let device: Arc<dyn RgbDevice> = Arc::new(Screen(sender));
    let controller = RgbController::new(HashMap::from([("screen".into(), device)]), None);
    let config = RgbAppConfig {
        enabled: true,
        devices: vec![RgbDeviceConfig {
            device_id: "screen".into(),
            fan_led_count: None,
            mb_rgb_sync: false,
            active_preset: None,
            regions: None,
            effect_memory: Vec::new(),
            port_merge: None,
            reverse_port: false,
            zones: vec![RgbZoneConfig {
                zone_index: 0,
                swap_lr: false,
                swap_tb: false,
                effect: RgbEffect {
                    colors: vec![[255, 0, 0]],
                    ..Default::default()
                },
            }],
        }],
        merge_lighting: Some(MergeLightingConfig {
            enabled: true,
            device_order: vec!["screen".into()],
            effect: RgbEffect {
                colors: vec![[0, 255, 0]],
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    (controller, config, received)
}

#[test]
fn sync_deduplicates_configuration_and_restores_individual_settings() {
    let (mut controller, mut config, received) = setup();
    controller.validate_config(&config).unwrap();
    controller.apply_config(&config, &[]);
    let frame = received.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(frame.iter().all(|rgb| rgb[0] == 0 && rgb[1] > 0));
    assert!(controller
        .set_effect("screen", 0, &RgbEffect::default())
        .is_err());

    config
        .merge_lighting
        .as_mut()
        .unwrap()
        .effect_memory
        .push(RgbEffect::default());
    controller.apply_config(&config, &[]);
    assert!(received.recv_timeout(Duration::from_millis(80)).is_err());

    config.merge_lighting.as_mut().unwrap().enabled = false;
    controller.apply_config(&config, &[]);
    let frame = received.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(frame.iter().all(|rgb| rgb[0] > 0 && rgb[1] == 0));
    assert!(controller.sync_active.is_empty());
    controller.stop();
}

#[test]
fn failed_sync_preflight_preserves_current_playback() {
    let (mut controller, mut config, received) = setup();
    controller.apply_config(&config, &[]);
    received.recv_timeout(Duration::from_secs(1)).unwrap();
    let signature = controller.sync_signature.clone();
    config.merge_lighting.as_mut().unwrap().effect.mode = RgbMode::Voice;
    assert!(controller.validate_config(&config).is_err());
    controller.apply_config(&config, &[]);
    assert_eq!(controller.sync_signature, signature);
    assert!(controller.sync_active.contains("screen"));
    assert!(received.recv_timeout(Duration::from_millis(80)).is_err());
    controller.stop();
}

#[test]
fn openrgb_release_restores_sync_without_individual_animation_upload() {
    let (mut controller, config, received) = setup();
    controller.apply_config(&config, &[]);
    received.recv_timeout(Duration::from_secs(1)).unwrap();
    controller.set_openrgb_active(true);
    assert!(controller.sync_active.is_empty());
    controller.set_openrgb_active(false);
    let frame = received.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(frame.iter().all(|rgb| rgb[0] == 0 && rgb[1] > 0));
    assert!(received.recv_timeout(Duration::from_millis(80)).is_err());
    controller.stop();
}

struct UnavailableDevice;

struct SharedPort {
    port: usize,
    colors: Arc<parking_lot::Mutex<[[u8; 3]; 2]>>,
}

impl RgbDevice for SharedPort {
    fn device_name(&self) -> String {
        format!("port {}", self.port)
    }
    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Static]
    }
    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![RgbZoneInfo {
            name: "fan".into(),
            name_ui: None,
            led_count: 1,
        }]
    }
    fn set_zone_effect(&self, _: u8, effect: &RgbEffect) -> anyhow::Result<()> {
        self.colors.lock()[self.port] = effect.colors[0];
        Ok(())
    }
    fn supports_mb_rgb_sync(&self) -> bool {
        true
    }
    fn set_mb_rgb_sync(&self, _: bool) -> anyhow::Result<()> {
        self.colors.lock().fill([0; 3]);
        Ok(())
    }
}

#[test]
fn sync_resets_shared_controller_before_applying_either_port() {
    for order in [
        vec!["port0".into(), "port1".into()],
        vec!["port1".into(), "port0".into()],
    ] {
        let colors = Arc::new(parking_lot::Mutex::new([[0; 3]; 2]));
        let ports = (0..2)
            .map(|port| {
                (
                    format!("port{port}"),
                    Arc::new(SharedPort {
                        port,
                        colors: colors.clone(),
                    }) as Arc<dyn RgbDevice>,
                )
            })
            .collect();
        let mut controller = RgbController::new(ports, None);
        let config = RgbAppConfig {
            enabled: true,
            merge_lighting: Some(MergeLightingConfig {
                enabled: true,
                kind: lianli_shared::rgb::RgbSyncKind::Matched,
                device_order: order,
                effect: RgbEffect {
                    colors: vec![[17, 38, 59]],
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        controller.apply_config(&config, &[]);
        assert_eq!(*colors.lock(), [[17, 38, 59]; 2]);
        controller.stop();
    }
}

#[test]
fn shared_controller_reset_restores_unchanged_individual_port() {
    let colors = Arc::new(parking_lot::Mutex::new([[0; 3]; 2]));
    let ports = (0..2)
        .map(|port| {
            (
                format!("hid:controller:port{port}"),
                Arc::new(SharedPort {
                    port,
                    colors: colors.clone(),
                }) as Arc<dyn RgbDevice>,
            )
        })
        .collect();
    let mut controller = RgbController::new(ports, None);
    let device: RgbDeviceConfig = serde_json::from_value(serde_json::json!({
        "device_id": "hid:controller:port1",
        "zones": [{"zone_index": 0, "effect": RgbEffect { colors: vec![[30, 40, 50]], ..Default::default() }}]
    })).unwrap();
    let mut config = RgbAppConfig {
        enabled: true,
        devices: vec![device],
        ..Default::default()
    };
    controller.apply_config(&config, &[]);
    assert_eq!(colors.lock()[1], [30, 40, 50]);
    config.merge_lighting = Some(MergeLightingConfig {
        enabled: true,
        kind: lianli_shared::rgb::RgbSyncKind::Matched,
        device_order: vec!["hid:controller:port0".into()],
        effect: RgbEffect {
            colors: vec![[1, 2, 3]],
            ..Default::default()
        },
        ..Default::default()
    });
    controller.apply_config(&config, &[]);
    assert_eq!(*colors.lock(), [[1, 2, 3], [30, 40, 50]]);
    controller.stop();
}

impl RgbDevice for UnavailableDevice {
    fn device_name(&self) -> String {
        "unavailable".into()
    }
    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Static]
    }
    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![RgbZoneInfo {
            name: "zone".into(),
            name_ui: None,
            led_count: 1,
        }]
    }
    fn set_zone_effect(&self, _: u8, _: &RgbEffect) -> anyhow::Result<()> {
        anyhow::bail!("device disappeared")
    }
}

#[test]
fn one_failed_sync_device_does_not_block_other_participants_or_restoration() {
    let (mut controller, mut config, restored) = setup();
    controller.apply_config(&config, &[]);
    restored.recv_timeout(Duration::from_secs(1)).unwrap();
    let (sender, participating) = mpsc::channel();
    controller
        .wired
        .insert("unavailable".into(), Arc::new(UnavailableDevice));
    controller
        .wired
        .insert("participant".into(), Arc::new(Screen(sender)));
    let sync = config.merge_lighting.as_mut().unwrap();
    sync.kind = lianli_shared::rgb::RgbSyncKind::Matched;
    sync.device_order = vec!["unavailable".into(), "participant".into()];
    controller.apply_config(&config, &[]);
    assert!(participating
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .iter()
        .all(|rgb| rgb[0] == 0 && rgb[1] > 0));
    assert!(restored
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .iter()
        .all(|rgb| rgb[0] > 0 && rgb[1] == 0));
    assert!(controller.sync_active.contains("unavailable"));
    assert!(controller.sync_signature.is_none());
    controller.stop();
}

struct MergedPort {
    port: usize,
    merged: Arc<std::sync::atomic::AtomicBool>,
    colors: Arc<parking_lot::Mutex<[[u8; 3]; 2]>>,
}

impl RgbDevice for MergedPort {
    fn device_name(&self) -> String {
        format!("port {}", self.port)
    }
    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Static]
    }
    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![RgbZoneInfo {
            name: "fan".into(),
            name_ui: None,
            led_count: 1,
        }]
    }
    fn set_zone_effect(&self, _: u8, effect: &RgbEffect) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.merged.load(std::sync::atomic::Ordering::Relaxed),
            "port is merged"
        );
        self.colors.lock()[self.port] = effect.colors[0];
        Ok(())
    }
    fn merged_effect_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Static]
    }
    fn apply_port_merge(
        &self,
        merge: Option<&lianli_shared::rgb::RgbPortMerge>,
    ) -> anyhow::Result<()> {
        self.merged
            .store(merge.is_some(), std::sync::atomic::Ordering::Relaxed);
        if let Some(merge) = merge {
            self.colors.lock().fill(merge.effect.colors[0]);
        }
        Ok(())
    }
}

#[test]
fn quick_sync_suspends_a_port_merge_and_restores_it_when_sync_ends() {
    let merged = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let colors = Arc::new(parking_lot::Mutex::new([[0; 3]; 2]));
    let ports = (0..2)
        .map(|port| {
            (
                format!("hid:tl:port{port}"),
                Arc::new(MergedPort {
                    port,
                    merged: merged.clone(),
                    colors: colors.clone(),
                }) as Arc<dyn RgbDevice>,
            )
        })
        .collect();
    let mut controller = RgbController::new(ports, None);
    let port = |port: usize, color: [u8; 3]| -> RgbDeviceConfig {
        serde_json::from_value(serde_json::json!({
            "device_id": format!("hid:tl:port{port}"),
            "port_merge": {
                "enabled": true,
                "effect": RgbEffect { colors: vec![[9, 9, 9]], ..Default::default() },
            },
            "zones": [{"zone_index": 0, "effect": RgbEffect { colors: vec![color], ..Default::default() }}]
        }))
        .unwrap()
    };
    let mut config = RgbAppConfig {
        enabled: true,
        devices: vec![port(0, [10, 0, 0]), port(1, [20, 0, 0])],
        ..Default::default()
    };
    controller.apply_config(&config, &[]);
    assert!(merged.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(*colors.lock(), [[9, 9, 9]; 2]);

    config.merge_lighting = Some(MergeLightingConfig {
        enabled: true,
        kind: lianli_shared::rgb::RgbSyncKind::Matched,
        device_order: vec!["hid:tl:port0".into()],
        effect: RgbEffect {
            colors: vec![[1, 2, 3]],
            ..Default::default()
        },
        ..Default::default()
    });
    controller.apply_config(&config, &[]);
    assert!(!merged.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(*colors.lock(), [[1, 2, 3], [20, 0, 0]]);

    config.merge_lighting = None;
    controller.apply_config(&config, &[]);
    assert!(merged.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(*colors.lock(), [[9, 9, 9]; 2]);
    controller.stop();
}

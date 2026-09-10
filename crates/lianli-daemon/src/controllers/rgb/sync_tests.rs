use super::*;
use lianli_devices::traits::RgbFrameDelivery;
use lianli_shared::rgb::{
    MergeLightingConfig, RgbDeviceConfig, RgbRenderFamily, RgbRenderProfile, RgbZoneConfig,
};
use std::{sync::mpsc, time::Duration};

struct Screen(mpsc::Sender<Vec<[u8; 3]>>);

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
            mb_rgb_sync: false,
            active_preset: None,
            regions: None,
            effect_memory: Vec::new(),
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

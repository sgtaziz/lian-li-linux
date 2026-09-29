use super::*;
use lianli_devices::traits::RgbDevice;
use lianli_shared::rgb::RgbRegionParameters;
use std::time::Instant;

fn capabilities(fans: usize) -> RgbDeviceCapabilities {
    let mut cap: RgbDeviceCapabilities = serde_json::from_value(serde_json::json!({
        "device_id": "hid:0cf2:a104:test:group2", "device_name": "AL V2 Group 2",
        "supported_modes": ["Static", "Breathing", "MeteorRainbow", "StaticColorful"],
        "zones": [], "supports_direct": false, "supports_mb_rgb_sync": true,
        "total_led_count": 0, "supported_scopes": [], "hardware_group_effects": true
    }))
    .unwrap();
    cap.zones = (0..fans)
        .map(|i| RgbZoneInfo {
            name: format!("Fan {i}"),
            led_count: 20,
        })
        .collect();
    cap.total_led_count = (fans * 20) as u16;
    for scope in [RgbScope::Inner, RgbScope::Outer] {
        let effects = [
            RgbMode::Static,
            RgbMode::Breathing,
            if scope == RgbScope::Inner {
                RgbMode::MeteorRainbow
            } else {
                RgbMode::StaticColorful
            },
        ]
        .into_iter()
        .map(|mode| RgbEffectParameters {
            mode,
            min_colors: if mode == RgbMode::StaticColorful {
                4
            } else {
                0
            },
            max_colors: if mode == RgbMode::StaticColorful {
                4
            } else {
                0
            },
            per_fan_colors: matches!(mode, RgbMode::Static | RgbMode::Breathing),
            directions: vec![RgbDirection::Clockwise, RgbDirection::CounterClockwise],
            supports_speed: mode != RgbMode::Static,
        })
        .collect();
        cap.region_parameters
            .push(RgbRegionParameters { scope, effects });
    }
    cap
}

fn group() -> RegionGroup {
    RegionGroup::new(capabilities(3), &[])
}

fn mode_command(group: &RegionGroup, region: Option<usize>, mode: RgbMode) -> Command {
    let (specs, state) = region
        .map(|index| (&group.regions[index].modes, &group.regions[index].state))
        .unwrap_or((&group.modes, &group.state));
    let index = specs.iter().position(|spec| spec.mode == mode).unwrap();
    Command::Mode {
        index,
        mode: mode_list(specs, state)[index].clone(),
    }
}

fn mode_packet(command: &Command, version: u32) -> Vec<u8> {
    let Command::Mode { index, mode } = command else {
        panic!("Expected mode");
    };
    let mut data = (*index as u32).to_le_bytes().to_vec();
    mode.write(&mut data, version);
    protocol::sized_packet(&data)
}

#[test]
fn device_modes_follow_and_zone_overrides_preserve_the_other_region() {
    let mut group = group();
    group.regions[1].modes.swap(1, 2);
    let buffer = Mutex::new(DirectColorBuffer::new());
    for spec in group.modes.clone() {
        let command = mode_command(&group, None, spec.mode);
        group.apply(&command, None, &buffer).unwrap();
        assert!(group
            .regions
            .iter()
            .all(|region| region.state.follow && region.state.effect.mode == spec.mode));
    }
    let Command::Mode { index, mode } = mode_command(&group, Some(1), RgbMode::Static) else {
        unreachable!()
    };
    group
        .apply(
            &Command::ZoneMode {
                zone: 1,
                selection: Some((index, mode)),
            },
            None,
            &buffer,
        )
        .unwrap();
    assert!(!group.regions[1].state.follow);
    assert_eq!(group.regions[0].state.effect.mode, RgbMode::Breathing);
    group
        .apply(
            &Command::ZoneMode {
                zone: 1,
                selection: None,
            },
            None,
            &buffer,
        )
        .unwrap();
    assert!(group.regions[1].state.follow);
    assert_eq!(group.regions[1].state.effect.mode, RgbMode::Breathing);
}

#[test]
fn old_and_new_layouts_share_state_and_reconnect_preserves_it() {
    let cap = capabilities(3);
    let shared = Arc::new(Mutex::new(group()));
    let groups = HashMap::from([(cap.device_id.clone(), shared.clone())]);
    let old = targets(std::slice::from_ref(&cap), &groups, 4);
    let new = targets(std::slice::from_ref(&cap), &groups, 6);
    assert_eq!((old.len(), new.len()), (2, 1));
    let Target::Region {
        group: old_group,
        region,
    } = &old[1]
    else {
        panic!()
    };
    let buffer = Mutex::new(DirectColorBuffer::new());
    old_group
        .lock()
        .apply(&Command::Colors(vec![[1, 2, 3]; 3]), *region, &buffer)
        .unwrap();
    assert_eq!(shared.lock().regions[1].state.colors, [[1, 2, 3]; 3]);
    let reconnected = targets(&[cap], &groups, 6);
    let Target::Region {
        group: reconnected, ..
    } = &reconnected[0]
    else {
        panic!()
    };
    assert!(Arc::ptr_eq(reconnected, &shared));
}

#[test]
fn layout_keeps_aio_and_unsupported_devices_and_hides_empty_groups() {
    let mut rgb = RgbController::new(Default::default(), None);
    let mut aio = capabilities(3);
    aio.device_id = "hid:0416:7395:aio".into();
    let mut empty = capabilities(0);
    empty.device_id.push_str(":empty");
    let mut unsupported = capabilities(3);
    unsupported.device_id = "hid:0cf2:a103:other".into();
    let caps = [aio.clone(), empty, unsupported, capabilities(3)];
    let groups = groups(&caps, &rgb, &Default::default(), true);
    let layout = targets(&caps, &groups, 6);
    assert_eq!(layout.len(), 3);
    assert!(matches!(&layout[0], Target::Legacy(cap) if cap.device_id == aio.device_id));
    assert!(matches!(&layout[1], Target::Legacy(_)));
    assert_eq!(targets(&caps, &Default::default(), 6).len(), 4);
    assert!(!capabilities(7).supports_openrgb_regions());
    rgb.stop();
}

#[test]
fn descriptions_encode_every_field_for_v0_through_v6() {
    let group = group();
    for version in 0..=6 {
        let region = if version < 6 { Some(0) } else { None };
        let bytes = group.description(version, region);
        let mut wire = protocol::Reader::new(&bytes);
        wire.size().unwrap();
        assert_eq!(wire.number().unwrap(), DEVICE_TYPE_LED_STRIP);
        wire.string().unwrap();
        if version >= 1 {
            assert_eq!(wire.string().unwrap(), "Lian Li");
        }
        wire.string().unwrap();
        wire.string().unwrap();
        assert!(wire
            .string()
            .unwrap()
            .ends_with(if version < 6 { ":inner" } else { ":regions" }));
        wire.string().unwrap();
        let count = wire.short().unwrap();
        assert!((wire.number().unwrap() as u16) < count);
        for _ in 0..count {
            let mode = ModeData::read(&mut wire, version).unwrap();
            assert!((mode.speed_min..=mode.speed_max).contains(&mode.speed));
            assert!((mode.brightness_min..=mode.brightness_max).contains(&mode.brightness));
        }
        assert_eq!(wire.short().unwrap(), if version < 6 { 3 } else { 2 });
        for _ in 0..if version < 6 { 3 } else { 2 } {
            wire.string().unwrap();
            assert_eq!(wire.number().unwrap(), ZONE_TYPE_LINEAR);
            for _ in 0..3 {
                assert_eq!(wire.number().unwrap(), if version < 6 { 1 } else { 3 });
            }
            assert_eq!(wire.short().unwrap(), 0);
            if version >= 4 {
                assert_eq!(wire.short().unwrap(), 0);
            }
            if version >= 5 {
                assert_eq!(wire.number().unwrap(), 0);
            }
            if version >= 6 {
                let active = wire.number().unwrap();
                let count = wire.short().unwrap();
                assert!(active < count as u32);
                for _ in 0..count {
                    ModeData::read(&mut wire, version).unwrap();
                }
                assert_eq!(wire.string().unwrap(), "");
            }
        }
        let leds = wire.short().unwrap();
        assert_eq!(leds, if version < 6 { 3 } else { 6 });
        for index in 0..leds {
            wire.string().unwrap();
            if version < 6 {
                assert_eq!(wire.number().unwrap(), index as u32);
            }
        }
        assert_eq!(wire.colors().unwrap().len(), leds as usize);
        if version >= 5 {
            assert_eq!(wire.short().unwrap(), 0);
            assert_eq!(wire.number().unwrap(), 0);
        }
        if version >= 6 {
            assert_eq!(wire.string().unwrap(), "");
            assert_eq!(wire.number().unwrap(), 1);
            assert_eq!(wire.take(1).unwrap(), [0]);
        }
        assert!(wire.finished());
    }
}

#[test]
fn capability_revision_disconnects_and_reenumerates_clients() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let (tx, _) = std::sync::mpsc::channel();
    let rgb = Arc::new(Mutex::new(RgbController::new(
        HashMap::from([(
            capabilities(3).device_id,
            Arc::new(RecordingDevice(tx, None)) as Arc<dyn RgbDevice>,
        )]),
        None,
    )));
    let buffer = Arc::new(Mutex::new(DirectColorBuffer::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let state = Arc::new(Mutex::new(OpenRgbServerState::default()));
    let worker = start_openrgb_server(
        rgb.clone(),
        buffer.clone(),
        port,
        true,
        stop.clone(),
        state.clone(),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while !state.lock().running {
        assert!(Instant::now() < deadline, "server did not start");
        thread::sleep(Duration::from_millis(5));
    }
    let mut peer = TcpStream::connect(("127.0.0.1", port)).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    fn request_count(peer: &mut TcpStream) -> u32 {
        let mut packet = MAGIC.to_vec();
        packet.extend_from_slice(&[0; 12]);
        peer.write_all(&packet).unwrap();
        let (_, kind, response) = read_packet_from(peer).unwrap();
        assert_eq!(kind, PKT_REQUEST_CONTROLLER_COUNT);
        u32::from_le_bytes(response[..4].try_into().unwrap())
    }
    assert_eq!(request_count(&mut peer), 2);
    let mut colors = Vec::new();
    protocol::write_colors(&mut colors, &[[20, 30, 40]; 3]);
    let colors = protocol::sized_packet(&colors);
    let mut packet = MAGIC.to_vec();
    packet.extend_from_slice(&0u32.to_le_bytes());
    packet.extend_from_slice(&PKT_UPDATE_LEDS.to_le_bytes());
    packet.extend_from_slice(&(colors.len() as u32).to_le_bytes());
    packet.extend_from_slice(&colors);
    peer.write_all(&packet).unwrap();
    assert_eq!(request_count(&mut peer), 2);

    let id = capabilities(3).device_id;
    let device = rgb.lock().clone_wired_device(&id).unwrap();
    rgb.lock().replace_wired(HashMap::from([
        (id.clone(), device.clone()),
        ("hid:0416:7395:other".into(), device),
    ]));
    assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    let mut reconnected = TcpStream::connect(("127.0.0.1", port)).unwrap();
    reconnected
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    assert_eq!(request_count(&mut reconnected), 3);
    let writes = buffer.lock().take_due_groups(Instant::now());
    assert_eq!(
        writes.len(),
        1,
        "unrelated layout change lost a pending group write"
    );
    assert_eq!(writes[0].effects[0].colors, [[20, 30, 40]; 3]);
    buffer
        .lock()
        .complete_group(&writes[0], true, Instant::now());

    rgb.lock().set_wireless(None);
    let mut peer = TcpStream::connect(("127.0.0.1", port)).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    assert_eq!(request_count(&mut peer), 3);
    assert!(
        buffer.lock().take_due_groups(Instant::now()).is_empty(),
        "wireless revision restarted an unchanged fan animation"
    );
    rgb.lock().invalidate_device_config(&id);
    let deadline = Instant::now() + Duration::from_secs(2);
    let replay = loop {
        let writes = buffer.lock().take_due_groups(Instant::now());
        if !writes.is_empty() {
            break writes;
        }
        assert!(
            Instant::now() < deadline,
            "device invalidation was not delivered"
        );
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        replay.len(),
        1,
        "invalidated device did not replay its desired state"
    );
    assert_eq!(replay[0].id, id);
    assert_eq!(replay[0].effects[0].colors, [[20, 30, 40]; 3]);
    rgb.lock().replace_wired(Default::default());
    assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    let mut reconnected = TcpStream::connect(("127.0.0.1", port)).unwrap();
    reconnected
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    assert_eq!(request_count(&mut reconnected), 0);
    stop.store(true, Ordering::Relaxed);
    worker.join().unwrap();
    rgb.lock().stop();
}

#[test]
fn changed_fan_counts_rebuild_state_without_resetting_other_groups() {
    let mut rgb = RgbController::new(Default::default(), None);
    let caps = [capabilities(3)];
    let old = groups(&caps, &rgb, &Default::default(), true);
    let same = groups(&caps, &rgb, &old, true);
    assert!(Arc::ptr_eq(
        &old[&caps[0].device_id],
        &same[&caps[0].device_id]
    ));
    let changed = groups(&[capabilities(2)], &rgb, &old, true);
    assert!(!Arc::ptr_eq(
        &old[&caps[0].device_id],
        &changed[&caps[0].device_id]
    ));
    assert_eq!(changed[&caps[0].device_id].lock().count(), 2);
    rgb.stop();
}

#[test]
fn saved_all_scope_initializes_both_regions_and_off_is_normalized() {
    for (brightness, disabled) in [(2, false), (255, false), (4, true)] {
        let effect = RgbEffect {
            mode: RgbMode::Breathing,
            scope: RgbScope::All,
            colors: vec![[255, 0, 0]; 3],
            brightness,
            disabled,
            ..Default::default()
        };
        let group = RegionGroup::new(capabilities(3), &[effect]);
        for region in &group.regions {
            if brightness == 255 || disabled {
                assert_eq!(region.state.effect.mode, RgbMode::Direct);
                assert_eq!(region.state.colors, [[0; 3]; 3]);
                assert_eq!(region.state.effect.brightness, 4);
            } else {
                assert_eq!(region.state.effect.mode, RgbMode::Breathing);
                assert_eq!(region.state.colors, [[255, 0, 0]; 3]);
            }
        }
    }
}

#[test]
fn newly_detected_disabled_group_queues_off_without_a_palette() {
    let mut cap = capabilities(3);
    for region in &mut cap.region_parameters {
        region.effects.push(RgbEffectParameters {
            mode: RgbMode::Off,
            min_colors: 0,
            max_colors: 0,
            per_fan_colors: false,
            directions: Vec::new(),
            supports_speed: false,
        });
    }
    let group = RegionGroup::new(
        cap,
        &[RgbEffect {
            disabled: true,
            ..Default::default()
        }],
    );
    let buffer = Mutex::new(DirectColorBuffer::new());
    group.queue_current(&buffer);
    let writes = buffer.lock().take_due_groups(Instant::now());
    assert_eq!(writes.len(), 1);
    assert!(writes[0]
        .effects
        .iter()
        .all(|effect| effect.mode == RgbMode::Off && effect.colors.is_empty()));
}

#[test]
fn mode_commands_roundtrip_all_versions_and_queue_one_complete_group() {
    for version in 0..=6 {
        let mut group = group();
        let buffer = Mutex::new(DirectColorBuffer::new());
        for region in 0..2 {
            for spec in group.regions[region].modes.clone() {
                let packet = mode_packet(&mode_command(&group, Some(region), spec.mode), version);
                let command = Command::decode(PKT_UPDATE_MODE, &packet, version)
                    .unwrap()
                    .unwrap();
                group.apply(&command, Some(region), &buffer).unwrap();
                let writes = buffer.lock().take_due_groups(Instant::now());
                assert_eq!(writes.len(), 1);
                assert_eq!(writes[0].effects.len(), 2);
                assert_eq!(
                    writes[0].effects[region].scope,
                    if region == 0 {
                        RgbScope::Inner
                    } else {
                        RgbScope::Outer
                    }
                );
                assert_eq!(
                    writes[0].effects[region].mode,
                    if spec.mode == RgbMode::Direct {
                        RgbMode::Static
                    } else {
                        spec.mode
                    }
                );
                buffer.lock().clear();
            }
        }
    }
}

#[test]
fn profile_colors_preserve_palettes_animations_and_per_fan_breathing() {
    let mut group = group();
    let buffer = Mutex::new(DirectColorBuffer::new());
    let mut command = mode_command(&group, Some(1), RgbMode::StaticColorful);
    if let Command::Mode { mode, .. } = &mut command {
        mode.colors = vec![[1, 2, 3]; 4];
    }
    group.apply(&command, Some(1), &buffer).unwrap();
    let command = mode_command(&group, Some(0), RgbMode::Breathing);
    group.apply(&command, Some(0), &buffer).unwrap();
    group
        .apply(&Command::Colors(vec![[255, 0, 0]; 6]), None, &buffer)
        .unwrap();
    let writes = buffer.lock().take_due_groups(Instant::now());
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].effects[0].mode, RgbMode::Breathing);
    assert_eq!(writes[0].effects[0].colors, [[255, 0, 0]; 3]);
    assert_eq!(writes[0].effects[1].mode, RgbMode::StaticColorful);
    assert_eq!(writes[0].effects[1].colors, [[1, 2, 3]; 4]);
}

#[test]
fn zone_and_single_colors_preserve_other_fans_and_follow_state() {
    let mut group = group();
    let buffer = Mutex::new(DirectColorBuffer::new());
    group
        .apply(
            &mode_command(&group, None, RgbMode::Breathing),
            None,
            &buffer,
        )
        .unwrap();
    group
        .apply(
            &Command::ZoneColors {
                zone: 1,
                colors: vec![[0, 0, 255]; 3],
            },
            None,
            &buffer,
        )
        .unwrap();
    group
        .apply(
            &Command::SingleColor {
                led: 1,
                color: [255, 0, 0],
            },
            None,
            &buffer,
        )
        .unwrap();
    assert_eq!(
        group.regions[0].state.colors,
        [[255; 3], [255, 0, 0], [255; 3]]
    );
    assert_eq!(group.regions[1].state.colors, [[0, 0, 255]; 3]);
    assert!(group.regions.iter().all(|region| region.state.follow));
}

#[test]
fn invalid_commands_leave_both_regions_and_delivery_queue_untouched() {
    let mut group = group();
    let buffer = Mutex::new(DirectColorBuffer::new());
    let original = group.description(6, None);
    for command in [
        Command::Colors(vec![[1, 2, 3]; 5]),
        Command::ZoneColors {
            zone: 2,
            colors: vec![[1, 2, 3]; 3],
        },
        Command::SingleColor {
            led: 6,
            color: [1, 2, 3],
        },
        Command::ZoneMode {
            zone: 9,
            selection: None,
        },
    ] {
        assert!(group.apply(&command, None, &buffer).is_err());
        assert_eq!(group.description(6, None), original);
        assert!(buffer.lock().take_due_groups(Instant::now()).is_empty());
    }
    let packet = mode_packet(&mode_command(&group, None, RgbMode::Breathing), 6);
    for end in 0..packet.len() {
        assert!(Command::decode(PKT_UPDATE_MODE, &packet[..end], 6).is_err());
    }
}

struct RecordingDevice(
    std::sync::mpsc::Sender<Vec<RgbEffect>>,
    Option<Mutex<std::sync::mpsc::Receiver<()>>>,
);
impl RgbDevice for RecordingDevice {
    fn device_name(&self) -> String {
        "AL V2".into()
    }
    fn supported_modes(&self) -> Vec<RgbMode> {
        capabilities(3).supported_modes
    }
    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        capabilities(3).zones
    }
    fn hardware_regions(&self) -> Vec<RgbRegionParameters> {
        capabilities(3).region_parameters
    }
    fn set_zone_effect(&self, _: u8, _: &RgbEffect) -> anyhow::Result<()> {
        anyhow::bail!("must use group delivery")
    }
    fn set_group_effects(&self, effects: &[RgbEffect]) -> anyhow::Result<()> {
        self.0.send(effects.to_vec())?;
        if let Some(resume) = &self.1 {
            resume.lock().recv_timeout(Duration::from_secs(2))?;
        }
        Ok(())
    }
}

#[test]
fn writer_uses_one_group_call_for_all_fans_and_records_success() {
    let (tx, rx) = std::sync::mpsc::channel();
    let cap = capabilities(3);
    let rgb = Arc::new(Mutex::new(RgbController::new(
        HashMap::from([(
            cap.device_id.clone(),
            Arc::new(RecordingDevice(tx, None)) as Arc<dyn RgbDevice>,
        )]),
        None,
    )));
    rgb.lock().set_openrgb_active(true);
    let buffer = Arc::new(Mutex::new(DirectColorBuffer::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let mut group = group();
    group
        .apply(&Command::Colors(vec![[1, 2, 3]; 6]), None, &buffer)
        .unwrap();
    let writer = crate::controllers::rgb::start_direct_color_writer(
        rgb.clone(),
        buffer.clone(),
        stop.clone(),
    );
    let effects = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(effects.len(), 2);
    assert!(effects.iter().all(|effect| effect.colors == [[1, 2, 3]; 3]));
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
    assert!(rx.try_recv().is_err());
    assert_eq!(
        rgb.lock()
            .get_effect_regions(&cap.device_id)
            .unwrap()
            .iter()
            .map(|region| region.effect.clone())
            .collect::<Vec<_>>(),
        effects
    );
    assert!(buffer.lock().take_due_groups(Instant::now()).is_empty());
    rgb.lock().stop();
}

#[test]
fn completed_group_write_is_cached_only_for_the_device_that_received_it() {
    for replace in [false, true] {
        let (tx, rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let device: Arc<dyn RgbDevice> =
            Arc::new(RecordingDevice(tx.clone(), Some(Mutex::new(resume_rx))));
        let id = capabilities(3).device_id;
        let rgb = Arc::new(Mutex::new(RgbController::new(
            HashMap::from([(id.clone(), device.clone())]),
            None,
        )));
        rgb.lock().set_openrgb_active(true);
        let buffer = Arc::new(Mutex::new(DirectColorBuffer::new()));
        group().queue_current(&buffer);
        let stop = Arc::new(AtomicBool::new(false));
        let writer =
            crate::controllers::rgb::start_direct_color_writer(rgb.clone(), buffer, stop.clone());
        let effects = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let current = if replace {
            Arc::new(RecordingDevice(tx, None)) as Arc<dyn RgbDevice>
        } else {
            device
        };
        rgb.lock()
            .replace_wired(HashMap::from([(id.clone(), current)]));
        stop.store(true, Ordering::Relaxed);
        resume_tx.send(()).unwrap();
        writer.join().unwrap();
        let cached = rgb.lock().get_effect_regions(&id);
        rgb.lock().stop();
        assert!(rx.try_recv().is_err());
        if replace {
            assert!(
                cached.is_none(),
                "old write was cached for a replacement device"
            );
        } else {
            assert_eq!(
                cached
                    .unwrap()
                    .into_iter()
                    .map(|region| region.effect)
                    .collect::<Vec<_>>(),
                effects
            );
        }
    }
}

#[test]
fn malformed_or_unsupported_commands_do_not_disconnect_sdk_clients() {
    let rgb = Arc::new(Mutex::new(RgbController::new(Default::default(), None)));
    let buffer = Arc::new(Mutex::new(DirectColorBuffer::new()));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let (stream, _) = listener.accept().unwrap();
    let mut client = ClientHandler::new(
        stream,
        rgb.clone(),
        buffer,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    client.protocol_version = 6;
    client.targets = Some(vec![Target::Region {
        group: Arc::new(Mutex::new(group())),
        region: None,
    }]);
    for (index, kind, bytes) in [
        (0, PKT_UPDATE_MODE, vec![0]),
        (0, 9999, vec![]),
        (90, PKT_UPDATE_LEDS, vec![6, 0, 0, 0, 0, 0]),
    ] {
        client.handle_packet(index, kind, &bytes).unwrap();
    }
    client
        .handle_packet(90, PKT_REQUEST_CONTROLLER_DATA, &[])
        .unwrap();
    assert!(read_packet_from(&mut peer).unwrap().2.is_empty());
    client
        .handle_packet(0, PKT_REQUEST_CONTROLLER_COUNT, &[])
        .unwrap();
    assert_eq!(
        read_packet_from(&mut peer).unwrap().2,
        [1u32, 0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    client
        .handle_packet(0, PKT_REQUEST_CONTROLLER_DATA, &[])
        .unwrap();
    assert!(!read_packet_from(&mut peer).unwrap().2.is_empty());
    rgb.lock().stop();
}

#[test]
fn profile_notifications_use_compact_colors_and_complete_zone_modes() {
    let mut group = group();
    let buffer = Mutex::new(DirectColorBuffer::new());
    group
        .apply(&mode_command(&group, None, RgbMode::Static), None, &buffer)
        .unwrap();
    let signal = group.signal_update(PKT_UPDATE_MODE);
    assert_eq!(&signal[4..8], &1u32.to_le_bytes());
    assert_eq!(&signal[8..], &group.description(6, None)[4..]);
    group
        .apply(&Command::Colors(vec![[1, 2, 3]; 6]), None, &buffer)
        .unwrap();
    let signal = group.signal_update(PKT_UPDATE_LEDS);
    assert_eq!(signal.len(), 34);
    assert_eq!(&signal[4..8], &0u32.to_le_bytes());
    assert_eq!(&signal[8..10], &6u16.to_le_bytes());
    for (zone, mode) in [(0, RgbMode::Breathing), (1, RgbMode::Static)] {
        let Command::Mode { index, mode } = mode_command(&group, Some(zone), mode) else {
            unreachable!()
        };
        group
            .apply(
                &Command::ZoneMode {
                    zone,
                    selection: Some((index, mode)),
                },
                None,
                &buffer,
            )
            .unwrap();
        let signal = group.signal_update(PKT_UPDATE_ZONE_MODE);
        assert_eq!(&signal[8..], &group.description(6, None)[4..]);
    }
    assert!(group.regions.iter().all(|region| !region.state.follow));
}

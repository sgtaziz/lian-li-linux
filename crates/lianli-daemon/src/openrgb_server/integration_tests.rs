use super::*;
use lianli_devices::traits::RgbDevice;
use lianli_shared::rgb::RgbZoneInfo;
use std::collections::HashMap;
use std::time::Instant;

struct Device {
    regions: bool,
}
impl RgbDevice for Device {
    fn device_name(&self) -> String {
        "Test AIO".into()
    }
    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Static, RgbMode::Breathing]
    }
    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![
            RgbZoneInfo {
                name: "First".into(),
                led_count: 2,
            },
            RgbZoneInfo {
                name: "Second".into(),
                led_count: 2,
            },
        ]
    }
    fn set_zone_effect(&self, _: u8, _: &RgbEffect) -> anyhow::Result<()> {
        Ok(())
    }
    fn hardware_regions(&self) -> Vec<lianli_shared::rgb::RgbRegionParameters> {
        if !self.regions {
            return Vec::new();
        }
        [
            lianli_shared::rgb::RgbScope::Inner,
            lianli_shared::rgb::RgbScope::Outer,
        ]
        .into_iter()
        .map(|scope| lianli_shared::rgb::RgbRegionParameters {
            scope,
            effects: self
                .supported_modes()
                .into_iter()
                .map(|mode| lianli_shared::rgb::RgbEffectParameters {
                    mode,
                    min_colors: 0,
                    max_colors: 0,
                    per_fan_colors: true,
                    directions: Vec::new(),
                    supports_speed: mode == RgbMode::Breathing,
                })
                .collect(),
        })
        .collect()
    }
}

struct Server {
    port: u16,
    rgb: Arc<Mutex<RgbController>>,
    buffer: Arc<Mutex<DirectColorBuffer>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(regions: bool) -> Self {
        Self::with_device(regions, false)
    }

    fn with_device(regions: bool, regional_device: bool) -> Self {
        Self::with_devices(
            regions,
            HashMap::from([(
                if regional_device {
                    "hid:0cf2:a104:test:group1"
                } else {
                    "hid:0416:7395:test"
                }
                .into(),
                Arc::new(Device {
                    regions: regional_device,
                }) as Arc<dyn RgbDevice>,
            )]),
        )
    }

    fn with_devices(regions: bool, devices: HashMap<String, Arc<dyn RgbDevice>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let rgb = Arc::new(Mutex::new(RgbController::new(devices, None)));
        let buffer = Arc::new(Mutex::new(DirectColorBuffer::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(OpenRgbServerState::default()));
        let worker = start_openrgb_server(
            rgb.clone(),
            buffer.clone(),
            port,
            regions,
            stop.clone(),
            state.clone(),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while !state.lock().running {
            assert!(Instant::now() < deadline, "server startup timed out");
            thread::sleep(Duration::from_millis(5));
        }
        Self {
            port,
            rgb,
            buffer,
            stop,
            worker: Some(worker),
        }
    }

    fn connect(&self, version: u32) -> TcpStream {
        let mut peer = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        send(
            &mut peer,
            PKT_REQUEST_PROTOCOL_VERSION,
            &version.to_le_bytes(),
        );
        assert_eq!(
            read_packet_from(&mut peer).unwrap().1,
            PKT_REQUEST_PROTOCOL_VERSION
        );
        if version >= 6 {
            assert_eq!(read_packet_from(&mut peer).unwrap().1, PKT_SET_SERVER_FLAGS);
        }
        send(&mut peer, PKT_REQUEST_CONTROLLER_COUNT, &[]);
        assert_eq!(
            read_packet_from(&mut peer).unwrap().1,
            PKT_REQUEST_CONTROLLER_COUNT
        );
        peer
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
        self.rgb.lock().stop();
    }
}

fn send(peer: &mut TcpStream, kind: u32, payload: &[u8]) {
    send_at(peer, 0, kind, payload);
}

fn send_at(peer: &mut TcpStream, index: u32, kind: u32, payload: &[u8]) {
    let mut packet = MAGIC.to_vec();
    packet.extend_from_slice(&index.to_le_bytes());
    packet.extend_from_slice(&kind.to_le_bytes());
    packet.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    packet.extend_from_slice(payload);
    peer.write_all(&packet).unwrap();
}

fn notification(peer: &mut TcpStream) -> Vec<u8> {
    let (index, kind, payload) = read_packet_from(peer).unwrap();
    assert_eq!((index, kind), (0, PKT_SIGNAL_UPDATE));
    payload
}

fn assert_colors(payload: &[u8], colors: &[[u8; 3]]) {
    let mut wire = protocol::Reader::new(payload);
    wire.size().unwrap();
    assert_eq!(wire.number().unwrap(), 0);
    assert_eq!(wire.colors().unwrap(), colors);
    assert!(wire.finished());
}

fn read_active_mode(description: &[u8]) -> (u32, Vec<ModeData>) {
    let mut wire = protocol::Reader::new(description);
    wire.size().unwrap();
    wire.number().unwrap();
    for _ in 0..6 {
        wire.string().unwrap();
    }
    let count = wire.short().unwrap();
    let active = wire.number().unwrap();
    let modes = (0..count)
        .map(|_| ModeData::read(&mut wire, 6).unwrap())
        .collect();
    (active, modes)
}

#[test]
fn ordinary_devices_notify_all_v6_clients_with_full_current_state() {
    for regions in [false, true] {
        let server = Server::new(regions);
        let mut first = server.connect(6);
        let mut second = server.connect(6);
        send(&mut first, PKT_REQUEST_CONTROLLER_DATA, &[]);
        let description = read_packet_from(&mut first).unwrap().2;
        let (_, modes) = read_active_mode(&description);
        let mut mode = modes[1].clone();
        mode.colors = vec![[10, 20, 30]];
        mode.speed = 3;
        mode.brightness = 2;
        let mut body = 1u32.to_le_bytes().to_vec();
        mode.write(&mut body, 6);
        send(&mut first, PKT_UPDATE_MODE, &protocol::sized_packet(&body));
        for peer in [&mut first, &mut second] {
            let data = notification(peer);
            assert_eq!(u32::from_le_bytes(data[4..8].try_into().unwrap()), 1);
            let full = protocol::sized_packet(&data[8..]);
            let (active, received) = read_active_mode(&full);
            assert_eq!(active, 1);
            assert_eq!(received[1], mode);
        }
        let initial = [[1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12]];
        let mut colors = Vec::new();
        protocol::write_colors(&mut colors, &initial);
        send(
            &mut first,
            PKT_UPDATE_LEDS,
            &protocol::sized_packet(&colors),
        );
        assert_colors(&notification(&mut first), &initial);
        assert_colors(&notification(&mut second), &initial);

        let mut single = 3u32.to_le_bytes().to_vec();
        single.extend_from_slice(&[90, 91, 92, 0]);
        send(&mut second, PKT_UPDATE_SINGLE_LED, &single);
        let mut expected = initial;
        expected[3] = [90, 91, 92];
        assert_colors(&notification(&mut first), &expected);
        assert_colors(&notification(&mut second), &expected);
        let pending = server.buffer.lock().take_all();
        assert_eq!(pending["hid:0416:7395:test"][&1], expected[2..]);

        let mut zone = 0u32.to_le_bytes().to_vec();
        protocol::write_colors(&mut zone, &[[50, 51, 52]; 2]);
        send(
            &mut first,
            PKT_UPDATE_ZONE_LEDS,
            &protocol::sized_packet(&zone),
        );
        expected[..2].fill([50, 51, 52]);
        assert_colors(&notification(&mut first), &expected);
        assert_colors(&notification(&mut second), &expected);
        send(&mut second, PKT_REQUEST_CONTROLLER_DATA, &[]);
        let description = read_packet_from(&mut second).unwrap().2;
        assert_eq!(read_active_mode(&description).0, 1);
        let mut reconnected = server.connect(6);
        send(&mut reconnected, PKT_REQUEST_CONTROLLER_DATA, &[]);
        assert_eq!(read_packet_from(&mut reconnected).unwrap().2, description);
    }
}

#[test]
fn old_clients_update_v6_peers_without_receiving_unsolicited_notifications() {
    let server = Server::new(false);
    let mut old = server.connect(4);
    let mut new = server.connect(6);
    let mut data = Vec::new();
    protocol::write_colors(&mut data, &[[12, 34, 56]; 4]);
    send(&mut old, PKT_UPDATE_LEDS, &protocol::sized_packet(&data));
    assert_colors(&notification(&mut new), &[[12, 34, 56]; 4]);
    send(&mut old, PKT_REQUEST_CONTROLLER_COUNT, &[]);
    assert_eq!(
        read_packet_from(&mut old).unwrap().1,
        PKT_REQUEST_CONTROLLER_COUNT
    );
}

#[test]
fn regional_updates_from_either_layout_notify_all_v6_clients() {
    let server = Server::with_device(true, true);
    let mut first = server.connect(6);
    let mut second = server.connect(6);
    let mut old = server.connect(4);
    let mut data = Vec::new();
    protocol::write_colors(&mut data, &[[12, 34, 56]; 2]);
    send(&mut old, PKT_UPDATE_LEDS, &protocol::sized_packet(&data));
    for peer in [&mut first, &mut second] {
        assert_colors(
            &notification(peer),
            &[[12, 34, 56], [12, 34, 56], [255; 3], [255; 3]],
        );
    }
    send(&mut first, PKT_REQUEST_CONTROLLER_DATA, &[]);
    let (_, modes) = read_active_mode(&read_packet_from(&mut first).unwrap().2);
    let index = modes
        .iter()
        .position(|mode| mode.name == "Breathing")
        .unwrap();
    let mut body = (index as u32).to_le_bytes().to_vec();
    modes[index].write(&mut body, 6);
    send(&mut first, PKT_UPDATE_MODE, &protocol::sized_packet(&body));
    let first_update = notification(&mut first);
    assert_eq!(notification(&mut second), first_update);
    assert_eq!(
        read_active_mode(&protocol::sized_packet(&first_update[8..])).0,
        index as u32
    );
}

#[test]
fn mixed_layouts_route_split_outer_updates_to_the_group_index() {
    let server = Server::with_devices(
        true,
        HashMap::from([
            (
                "hid:0416:7395:aio".into(),
                Arc::new(Device { regions: false }) as Arc<dyn RgbDevice>,
            ),
            (
                "hid:0cf2:a104:test:group1".into(),
                Arc::new(Device { regions: true }) as Arc<dyn RgbDevice>,
            ),
        ]),
    );
    let mut modern = server.connect(6);
    let mut old = server.connect(4);
    let mut data = Vec::new();
    protocol::write_colors(&mut data, &[[12, 34, 56]; 2]);
    send_at(&mut old, 2, PKT_UPDATE_LEDS, &protocol::sized_packet(&data));
    let (index, kind, data) = read_packet_from(&mut modern).unwrap();
    assert_eq!((index, kind), (1, PKT_SIGNAL_UPDATE));
    assert_colors(&data, &[[255; 3], [255; 3], [12, 34, 56], [12, 34, 56]]);
    let mut data = Vec::new();
    protocol::write_colors(&mut data, &[[8, 9, 10]; 4]);
    send_at(&mut old, 0, PKT_UPDATE_LEDS, &protocol::sized_packet(&data));
    assert_colors(&notification(&mut modern), &[[8, 9, 10]; 4]);
}

#[test]
fn ordinary_colors_do_not_wait_for_the_controller_lock() {
    let server = Server::new(false);
    let mut peer = server.connect(6);
    let controller = server.rgb.lock();
    let mut data = Vec::new();
    protocol::write_colors(&mut data, &[[8, 9, 10]; 4]);
    send(&mut peer, PKT_UPDATE_LEDS, &protocol::sized_packet(&data));
    assert_colors(&notification(&mut peer), &[[8, 9, 10]; 4]);
    drop(controller);
}

#[test]
fn wired_reinitialization_preserves_existing_delivery_generations() {
    let mut rgb = RgbController::new(
        HashMap::from([(
            "device".into(),
            Arc::new(Device { regions: false }) as Arc<dyn RgbDevice>,
        )]),
        None,
    );
    rgb.invalidate_device_config("device");
    let before = rgb.delivery_device("device").unwrap();
    let wired = rgb.drain_wired();
    rgb.replace_wired(wired);
    let after = rgb.delivery_device("device").unwrap();
    assert!(Arc::ptr_eq(&before.0, &after.0));
    assert_eq!(before.1, after.1);
    rgb.invalidate_device_config("device");
    assert_ne!(after.1, rgb.delivery_device("device").unwrap().1);
    rgb.stop();
}

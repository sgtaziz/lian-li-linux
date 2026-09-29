//! OpenRGB SDK server: exposes Lian Li devices to OpenRGB/SignalRGB clients.
//!
//! Implements the OpenRGB network protocol (TCP, port 6743 by default).
//! Each physical device is exposed as an OpenRGB controller with its native
//! LED modes. Clients can enumerate devices, set modes, and update per-LED colors.

use crate::controllers::rgb::{DirectColorBuffer, RgbController};
use lianli_shared::rgb::{RgbDeviceCapabilities, RgbDirection, RgbEffect, RgbMode};
use parking_lot::Mutex;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::{debug, error, info, warn};

mod clients;
#[cfg(test)]
mod integration_tests;
mod legacy;
mod protocol;
mod regions;
use protocol::{Command, ModeData, DEFAULT_BRIGHTNESS, DEFAULT_SPEED, MAX_EFFECT_VALUE};
use regions::Target;

const MAGIC: &[u8; 4] = b"ORGB";
const HEADER_SIZE: usize = 16;
const MAX_CLIENTS: usize = 16;
const MAX_PACKET_BYTES: usize = 1024 * 1024;
// OpenRGB matches profiles against this value, keep it independent of daemon releases.
const CONTROLLER_VERSION: &str = "1.0";

#[derive(Default)]
struct Clients(Vec<(TcpStream, thread::JoinHandle<()>)>);

impl Clients {
    fn reap(&mut self) {
        let mut index = 0;
        while index < self.0.len() {
            if self.0[index].1.is_finished() {
                let (_, client) = self.0.swap_remove(index);
                if client.join().is_err() {
                    warn!("OpenRGB client worker panicked");
                }
            } else {
                index += 1;
            }
        }
    }
}

impl Drop for Clients {
    fn drop(&mut self) {
        for (stream, _) in &self.0 {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        for (_, client) in self.0.drain(..) {
            if client.join().is_err() {
                warn!("OpenRGB client worker panicked during shutdown");
            }
        }
    }
}

fn read_packet_from(stream: &mut impl Read) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let mut header = [0u8; HEADER_SIZE];
    stream.read_exact(&mut header)?;
    anyhow::ensure!(&header[0..4] == MAGIC, "Invalid magic bytes");
    let dev_idx = u32::from_le_bytes(header[4..8].try_into()?);
    let pkt_id = u32::from_le_bytes(header[8..12].try_into()?);
    let pkt_size = u32::from_le_bytes(header[12..16].try_into()?) as usize;
    anyhow::ensure!(
        pkt_size <= MAX_PACKET_BYTES,
        "OpenRGB packet exceeds the 1 MiB limit"
    );
    let mut payload = vec![0u8; pkt_size];
    stream.read_exact(&mut payload)?;
    Ok((dev_idx, pkt_id, payload))
}
// v3 adds brightness, v4 segments, v5 flags, and v6 independent zone modes.
const SERVER_PROTOCOL_VERSION: u32 = 6;

// Packet IDs
const PKT_REQUEST_CONTROLLER_COUNT: u32 = 0;
const PKT_REQUEST_CONTROLLER_DATA: u32 = 1;
const PKT_REQUEST_PROTOCOL_VERSION: u32 = 40;
const PKT_SET_CLIENT_NAME: u32 = 50;
const PKT_SET_CLIENT_FLAGS: u32 = 52;
const PKT_SET_SERVER_FLAGS: u32 = 53;
const PKT_SET_CLIENT_HOSTNAME: u32 = 54;
const PKT_REQUEST_PROFILE_LIST: u32 = 150;
const PKT_REQUEST_PLUGIN_LIST: u32 = 200;
const PKT_RESIZE_ZONE: u32 = 1000;
const PKT_UPDATE_LEDS: u32 = 1050;
const PKT_UPDATE_ZONE_LEDS: u32 = 1051;
const PKT_UPDATE_SINGLE_LED: u32 = 1052;
const PKT_SET_CUSTOM_MODE: u32 = 1100;
const PKT_UPDATE_MODE: u32 = 1101;
const PKT_SAVE_MODE: u32 = 1102;
const PKT_UPDATE_ZONE_MODE: u32 = 1103;
const PKT_SIGNAL_UPDATE: u32 = 1150;

// OpenRGB DeviceType
const DEVICE_TYPE_LED_STRIP: u32 = 4;
const DEVICE_TYPE_COOLER: u32 = 3;

// ModeFlags
const MODE_FLAG_HAS_SPEED: u32 = 1 << 0;
const MODE_FLAG_HAS_DIRECTION_LR: u32 = 1 << 1;
const MODE_FLAG_HAS_DIRECTION_UD: u32 = 1 << 2;
const MODE_FLAG_HAS_BRIGHTNESS: u32 = 1 << 4;
const MODE_FLAG_HAS_PER_LED_COLOR: u32 = 1 << 5;
const MODE_FLAG_HAS_MODE_SPECIFIC_COLOR: u32 = 1 << 6;

// Direction
const DIR_LEFT: u32 = 0;
const DIR_RIGHT: u32 = 1;
const DIR_UP: u32 = 2;
const DIR_DOWN: u32 = 3;

// ColorMode
const COLOR_MODE_NONE: u32 = 0;
const COLOR_MODE_PER_LED: u32 = 1;
const COLOR_MODE_MODE_SPECIFIC: u32 = 2;

// Zone types
const ZONE_TYPE_LINEAR: u32 = 1;

/// Shared state reported back from the server thread.
#[derive(Debug, Clone, Default)]
pub struct OpenRgbServerState {
    pub running: bool,
    pub port: Option<u16>,
    pub regions_enabled: bool,
    pub error: Option<String>,
}

/// Starts the OpenRGB SDK server in a background thread.
pub fn start_openrgb_server(
    rgb: Arc<Mutex<RgbController>>,
    direct_buffer: Arc<Mutex<DirectColorBuffer>>,
    port: u16,
    regions_enabled: bool,
    stop_flag: Arc<AtomicBool>,
    state: Arc<Mutex<OpenRgbServerState>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(e) = run_server(
            rgb,
            direct_buffer,
            port,
            regions_enabled,
            &stop_flag,
            &state,
        ) {
            error!("OpenRGB server error: {e}");
            let mut s = state.lock();
            s.running = false;
            s.error = Some(e.to_string());
        } else {
            let mut s = state.lock();
            s.running = false;
            s.error = None;
        }
    })
}

fn run_server(
    rgb: Arc<Mutex<RgbController>>,
    direct_buffer: Arc<Mutex<DirectColorBuffer>>,
    port: u16,
    regions_enabled: bool,
    stop_flag: &Arc<AtomicBool>,
    state: &Arc<Mutex<OpenRgbServerState>>,
) -> anyhow::Result<()> {
    let listener = match TcpListener::bind(format!("0.0.0.0:{port}")) {
        Ok(l) => l,
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::AddrInUse {
                format!("Port {port} is already in use")
            } else {
                format!("Failed to bind port {port}: {e}")
            };
            let mut s = state.lock();
            s.running = false;
            s.port = Some(port);
            s.error = Some(msg.clone());
            anyhow::bail!(msg);
        }
    };
    listener.set_nonblocking(true)?;
    info!("OpenRGB SDK server listening on port {port}");

    {
        let mut s = state.lock();
        s.running = true;
        s.port = Some(port);
        s.regions_enabled = regions_enabled;
        s.error = None;
    }

    let client_count = Arc::new(AtomicUsize::new(0));
    let mut clients = Clients::default();
    let notifications = Arc::new(Mutex::new(clients::Notifications::default()));
    let (mut caps, revision, mut observed_revision, mut groups) = {
        let rgb = rgb.lock();
        let caps = rgb.exposed_capabilities();
        let revision = rgb.capabilities_revision();
        let observed = revision.load(Ordering::Acquire);
        let groups = regions::groups(&caps, &rgb, &Default::default(), regions_enabled);
        (caps, revision, observed, groups)
    };
    let mut legacy_states = legacy::states(&caps, &Default::default());
    let mut delivery_devices: std::collections::HashMap<_, _> = {
        let rgb = rgb.lock();
        groups
            .keys()
            .filter_map(|id| rgb.delivery_device(id).map(|device| (id.clone(), device)))
            .collect()
    };

    while !stop_flag.load(Ordering::Relaxed) {
        clients.reap();
        let current_revision = revision.load(Ordering::Acquire);
        if current_revision != observed_revision {
            let current = rgb.lock().exposed_capabilities();
            if current != caps {
                // Existing clients must not send commands using reassigned device indexes.
                clients = Clients::default();
                notifications.lock().reset();
                let retained: std::collections::HashSet<_> = current
                    .iter()
                    .filter(|cap| caps.contains(cap))
                    .map(|cap| cap.device_id.clone())
                    .collect();
                direct_buffer.lock().retain_devices(&retained);
                legacy_states = legacy::states(&current, &legacy_states);
                groups = regions::groups(&current, &rgb.lock(), &groups, regions_enabled);
                for (id, group) in &groups {
                    if !retained.contains(id) {
                        group.lock().queue_current(&direct_buffer);
                    }
                }
                caps = current;
            }
            let current_devices: std::collections::HashMap<_, _> = {
                let rgb = rgb.lock();
                groups
                    .keys()
                    .filter_map(|id| rgb.delivery_device(id).map(|device| (id.clone(), device)))
                    .collect()
            };
            let changed: std::collections::HashSet<_> = current_devices
                .iter()
                .filter(|(id, (device, generation))| {
                    delivery_devices
                        .get(*id)
                        .is_some_and(|(previous, old_generation)| {
                            !Arc::ptr_eq(device, previous) || generation != old_generation
                        })
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in &changed {
                if let Some(group) = groups.get(id) {
                    group.lock().queue_current(&direct_buffer);
                }
            }
            direct_buffer.lock().invalidate_group_delivery(&changed);
            delivery_devices = current_devices;
            observed_revision = current_revision;
        }
        match listener.accept() {
            Ok((stream, addr)) => {
                if clients.0.len() >= MAX_CLIENTS {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    continue;
                }
                let control = match stream.try_clone() {
                    Ok(control) => control,
                    Err(error) => {
                        warn!("Could not track OpenRGB client: {error}");
                        continue;
                    }
                };
                info!("OpenRGB client connected from {addr}");
                stream.set_nonblocking(false).ok();
                stream.set_read_timeout(Some(Duration::from_secs(300))).ok();
                stream.set_write_timeout(Some(Duration::from_secs(10))).ok();

                let rgb = Arc::clone(&rgb);
                let buf = Arc::clone(&direct_buffer);
                let count = Arc::clone(&client_count);
                let stop = Arc::clone(stop_flag);
                let caps = caps.clone();
                let groups = groups.clone();
                let legacy_states = legacy_states.clone();
                let notifications = notifications.clone();

                let mut client = match ClientHandler::new(stream, rgb.clone(), buf, stop) {
                    Ok(client) => client,
                    Err(error) => {
                        warn!(%addr, %error, "Could not initialize OpenRGB client");
                        continue;
                    }
                };
                client.cached_caps = Some(caps);
                client.targets = Some(regions::targets(client.caps(), &groups, 0));
                client.region_groups = groups;
                client.legacy_states = legacy_states;
                client.notifications = notifications;

                let prev = count.fetch_add(1, Ordering::Relaxed);
                if prev == 0 {
                    rgb.lock().set_openrgb_active(true);
                }

                let client = thread::spawn(move || {
                    if let Err(e) = client.run() {
                        debug!("OpenRGB client disconnected: {e}");
                    }

                    let remaining = count.fetch_sub(1, Ordering::Relaxed) - 1;
                    if remaining == 0 {
                        client.rgb.lock().set_openrgb_active(false);
                    }
                    info!("OpenRGB client disconnected ({remaining} remaining)");
                });
                clients.0.push((control, client));
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                warn!("OpenRGB accept error: {e}");
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    drop(clients);
    rgb.lock().set_openrgb_active(false);
    info!("OpenRGB server stopped");
    Ok(())
}

struct ClientHandler {
    stream: TcpStream,
    output: Arc<clients::Output>,
    writer: Option<thread::JoinHandle<()>>,
    rgb: Arc<Mutex<RgbController>>,
    direct_buffer: Arc<Mutex<DirectColorBuffer>>,
    stop_flag: Arc<AtomicBool>,
    protocol_version: u32,
    client_name: String,
    /// Index meanings remain fixed for the lifetime of a connection.
    cached_caps: Option<Vec<RgbDeviceCapabilities>>,
    targets: Option<Vec<Target>>,
    region_groups: regions::Groups,
    legacy_states: legacy::States,
    notifications: Arc<Mutex<clients::Notifications>>,
}

impl Drop for ClientHandler {
    fn drop(&mut self) {
        self.output.disconnect();
        if let Some(writer) = self.writer.take() {
            if writer.join().is_err() {
                warn!("OpenRGB client writer panicked");
            }
        }
    }
}

impl ClientHandler {
    fn new(
        stream: TcpStream,
        rgb: Arc<Mutex<RgbController>>,
        direct_buffer: Arc<Mutex<DirectColorBuffer>>,
        stop_flag: Arc<AtomicBool>,
    ) -> std::io::Result<Self> {
        let (output, writer) = clients::Output::start(&stream)?;
        Ok(Self {
            stream,
            output,
            writer: Some(writer),
            rgb,
            direct_buffer,
            stop_flag,
            protocol_version: 0,
            client_name: String::new(),
            cached_caps: None,
            targets: None,
            region_groups: Default::default(),
            legacy_states: Default::default(),
            notifications: Default::default(),
        })
    }

    /// Get capabilities, caching on first call to avoid mutex contention during streaming.
    fn caps(&mut self) -> &[RgbDeviceCapabilities] {
        if self.cached_caps.is_none() {
            self.cached_caps = Some(self.rgb.lock().exposed_capabilities());
        }
        self.cached_caps.as_ref().unwrap()
    }

    fn run(&mut self) -> anyhow::Result<()> {
        loop {
            if self.stop_flag.load(Ordering::Relaxed) {
                return Ok(());
            }

            let (dev_idx, pkt_id, payload) = self.read_packet()?;
            if self.stop_flag.load(Ordering::Relaxed) {
                return Ok(());
            }
            self.handle_packet(dev_idx, pkt_id, &payload)?;
        }
    }

    fn read_packet(&mut self) -> anyhow::Result<(u32, u32, Vec<u8>)> {
        read_packet_from(&mut self.stream)
    }

    fn send_packet(&mut self, dev_idx: u32, pkt_id: u32, payload: &[u8]) -> anyhow::Result<()> {
        self.output.send(dev_idx, pkt_id, payload)
    }

    fn handle_packet(&mut self, dev_idx: u32, pkt_id: u32, payload: &[u8]) -> anyhow::Result<()> {
        match pkt_id {
            PKT_REQUEST_PROTOCOL_VERSION => {
                let client_version = payload
                    .get(..4)
                    .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
                    .unwrap_or(0);
                self.protocol_version = client_version.min(SERVER_PROTOCOL_VERSION);
                let caps = self.caps().to_vec();
                self.targets = Some(regions::targets(
                    &caps,
                    &self.region_groups,
                    self.protocol_version,
                ));
                self.send_packet(0, pkt_id, &SERVER_PROTOCOL_VERSION.to_le_bytes())?;
                if self.protocol_version >= 6 {
                    self.send_packet(0, PKT_SET_SERVER_FLAGS, &1u32.to_le_bytes())?;
                }
                self.notifications.lock().subscribe(
                    self.output.clone(),
                    self.targets.as_deref().unwrap_or_default(),
                    self.protocol_version,
                );
                Ok(())
            }
            PKT_SET_CLIENT_FLAGS | PKT_SET_CLIENT_HOSTNAME => Ok(()),
            PKT_SET_CLIENT_NAME => {
                self.client_name = String::from_utf8_lossy(payload)
                    .trim_end_matches('\0')
                    .to_owned();
                debug!(name = %self.client_name, "OpenRGB client named");
                Ok(())
            }
            PKT_REQUEST_CONTROLLER_COUNT => {
                let count = self
                    .targets
                    .as_ref()
                    .map_or_else(|| self.cached_caps.as_ref().map_or(0, Vec::len), Vec::len)
                    as u32;
                let mut response = count.to_le_bytes().to_vec();
                if self.protocol_version >= 6 {
                    for id in 0..count {
                        response.extend_from_slice(&id.to_le_bytes());
                    }
                }
                self.send_packet(0, pkt_id, &response)
            }
            PKT_REQUEST_PROFILE_LIST | PKT_REQUEST_PLUGIN_LIST => {
                self.send_packet(0, pkt_id, &[6, 0, 0, 0, 0, 0])
            }
            _ => self.handle_controller_packet(dev_idx, pkt_id, payload),
        }
    }

    fn resolve_target(&mut self, index: u32) -> Option<Target> {
        if let Some(targets) = &self.targets {
            targets.get(index as usize).cloned()
        } else {
            self.caps()
                .get(index as usize)
                .cloned()
                .map(|cap| Target::Legacy(Arc::new(cap)))
        }
    }

    fn handle_controller_packet(
        &mut self,
        index: u32,
        kind: u32,
        payload: &[u8],
    ) -> anyhow::Result<()> {
        let target = self.resolve_target(index);
        if kind == PKT_REQUEST_CONTROLLER_DATA {
            let data = match target {
                Some(Target::Legacy(cap)) => self
                    .legacy_state(&cap)
                    .lock()
                    .description(self.protocol_version),
                Some(Target::Region { group, region }) => {
                    group.lock().description(self.protocol_version, region)
                }
                None => Vec::new(),
            };
            return self.send_packet(index, kind, &data);
        }
        let command = match Command::decode(kind, payload, self.protocol_version) {
            Ok(Some(command)) => command,
            Ok(None) => {
                debug!(kind, index, "Unsupported OpenRGB command");
                return Ok(());
            }
            Err(error) => {
                debug!(kind, index, %error, "Invalid OpenRGB command");
                return Ok(());
            }
        };
        let Some(target) = target else {
            debug!(index, "Unknown OpenRGB controller");
            return Ok(());
        };
        if let Command::Resize { zone, size } = command {
            return self.handle_resize(index, &target, zone, size);
        }
        match target {
            Target::Legacy(cap) => {
                let state = self.legacy_state(&cap);
                let update = if matches!(
                    command,
                    Command::Colors(_) | Command::ZoneColors { .. } | Command::SingleColor { .. }
                ) {
                    let mut state = state.lock();
                    if let Err(error) = state.apply_colors(&command, &self.direct_buffer) {
                        debug!(index, %error, "Rejected OpenRGB device colors");
                        return Ok(());
                    }
                    state.update(kind)
                } else {
                    let mut rgb = self.rgb.lock();
                    let mut next = state.lock().clone();
                    if let Err(error) = next.apply_mode(&command, &mut rgb) {
                        debug!(index, %error, "Rejected OpenRGB device command");
                        return Ok(());
                    }
                    let mut state = state.lock();
                    state.commit_mode(next);
                    state.update(kind)
                };
                self.notifications
                    .lock()
                    .publish(&cap.device_id, update, || {
                        state.lock().update(PKT_UPDATE_MODE)
                    });
            }
            Target::Region { group, region } => {
                let (id, update) = {
                    let mut group = group.lock();
                    if let Err(error) = group.apply(&command, region, &self.direct_buffer) {
                        debug!(index, %error, "Rejected OpenRGB region command");
                        return Ok(());
                    }
                    (group.cap.device_id.clone(), group.update(kind))
                };
                self.notifications
                    .lock()
                    .publish(&id, update, || group.lock().update(PKT_UPDATE_MODE));
            }
        }
        Ok(())
    }

    fn handle_resize(
        &mut self,
        index: u32,
        target: &Target,
        zone: usize,
        _requested: u32,
    ) -> anyhow::Result<()> {
        let actual = match target {
            Target::Legacy(cap) => cap.zones.get(zone).map_or(0, |zone| zone.led_count as u32),
            Target::Region { group, region } => {
                let group = group.lock();
                if region.is_some() {
                    u32::from(zone < group.count())
                } else if zone < 2 {
                    group.count() as u32
                } else {
                    0
                }
            }
        };
        let mut response = (zone as u32).to_le_bytes().to_vec();
        response.extend_from_slice(&actual.to_le_bytes());
        self.send_packet(index, PKT_RESIZE_ZONE, &response)
    }

    fn legacy_state(&mut self, cap: &RgbDeviceCapabilities) -> Arc<Mutex<legacy::State>> {
        self.legacy_states
            .entry(cap.device_id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(legacy::State::new(cap.clone()))))
            .clone()
    }
}

struct ControllerSerializer {
    protocol_version: u32,
}

struct ZoneModeData {
    active_mode: i32,
    modes: Vec<ModeData>,
}

impl ControllerSerializer {
    #[cfg(test)]
    fn build_controller_data(&self, cap: &RgbDeviceCapabilities) -> Vec<u8> {
        self.build_controller_zones(
            cap,
            &self.build_modes(cap),
            0,
            &vec![[255; 3]; cap.total_led_count as usize],
            None,
        )
    }

    fn build_controller_zones(
        &self,
        cap: &RgbDeviceCapabilities,
        modes: &[ModeData],
        active_mode: u32,
        colors: &[[u8; 3]],
        zone_modes: Option<&[ZoneModeData]>,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1024);

        // data_size placeholder — we'll fill it at the end
        buf.extend_from_slice(&0u32.to_le_bytes());

        // device_type
        let dev_type = if cap.device_name.contains("Galahad")
            || cap.device_name.contains("AIO")
            || cap.device_name.contains("HydroShift")
            || cap.device_name.contains("Pump")
        {
            DEVICE_TYPE_COOLER
        } else {
            DEVICE_TYPE_LED_STRIP
        };
        buf.extend_from_slice(&dev_type.to_le_bytes());

        // name
        write_string(&mut buf, &cap.device_name);

        // vendor (proto >= 1)
        if self.protocol_version >= 1 {
            write_string(&mut buf, "Lian Li");
        }

        // description
        write_string(
            &mut buf,
            &format!("Lian Li {} RGB Controller", cap.device_name),
        );

        write_string(&mut buf, CONTROLLER_VERSION);

        // serial
        write_string(&mut buf, &cap.device_id);

        // location
        write_string(&mut buf, &format!("HID: {}", cap.device_id));

        // Build modes
        // num_modes (u16)
        buf.extend_from_slice(&(modes.len() as u16).to_le_bytes());
        // active_mode (i32)
        buf.extend_from_slice(&active_mode.to_le_bytes());
        // mode data (no u16 prefix — count was already written above)
        for mode_buf in modes {
            mode_buf.write(&mut buf, self.protocol_version);
        }

        // zones (u16 count + data)
        buf.extend_from_slice(&(cap.zones.len() as u16).to_le_bytes());
        for (index, zone) in cap.zones.iter().enumerate() {
            self.write_zone(&mut buf, zone);
            if self.protocol_version >= 6 {
                let state = zone_modes.and_then(|zones| zones.get(index));
                buf.extend_from_slice(&state.map_or(-1, |s| s.active_mode).to_le_bytes()); // active zone mode
                buf.extend_from_slice(&(state.map_or(0, |s| s.modes.len()) as u16).to_le_bytes());
                if let Some(state) = state {
                    for mode in &state.modes {
                        mode.write(&mut buf, self.protocol_version);
                    }
                }
                write_string(&mut buf, ""); // zone alternate name
            }
        }

        // LEDs (u16 count + data)
        let total_leds = cap.total_led_count as usize;
        buf.extend_from_slice(&(total_leds as u16).to_le_bytes());
        let mut led_idx = 0;
        for zone in &cap.zones {
            for i in 0..zone.led_count {
                write_string(&mut buf, &format!("{} LED {}", zone.name, i));
                if self.protocol_version < 6 {
                    buf.extend_from_slice(&(led_idx as u32).to_le_bytes());
                }
                led_idx += 1;
            }
        }

        // colors (u16 count + 4 bytes each)
        buf.extend_from_slice(&(total_leds as u16).to_le_bytes());
        for color in colors {
            buf.extend_from_slice(color);
            buf.push(0);
        }
        if self.protocol_version >= 5 {
            buf.extend_from_slice(&0u16.to_le_bytes()); // alternate names
            buf.extend_from_slice(&0u32.to_le_bytes()); // controller flags
        }
        if self.protocol_version >= 6 {
            write_string(&mut buf, ""); // controller alternate name
            buf.extend_from_slice(&1u32.to_le_bytes()); // configuration string length, including null
            buf.push(0); // empty device-specific configuration
        }

        // SDK size includes the size field.
        let data_size = (buf.len()) as u32;
        buf[0..4].copy_from_slice(&data_size.to_le_bytes());

        buf
    }

    fn build_modes(&self, cap: &RgbDeviceCapabilities) -> Vec<ModeData> {
        let mut modes = Vec::new();

        // Always add a "Direct" mode first (mode index 0)
        modes.push(self.build_mode_entry(
            "Direct",
            0,
            MODE_FLAG_HAS_PER_LED_COLOR,
            COLOR_MODE_PER_LED,
            0,
            0,
            0,
            4,
            0,
            0,
        ));

        // Add each supported mode from the device
        for rgb_mode in &cap.supported_modes {
            if matches!(rgb_mode, RgbMode::Off | RgbMode::Direct | RgbMode::Static) {
                continue; // Skip Off (brightness=0), Direct (already added), Static (same as Direct)
            }

            let name = rgb_mode.display_name();
            let value = rgb_mode.to_tl_mode_byte().unwrap_or(0) as u32;

            let mut flags = MODE_FLAG_HAS_SPEED
                | MODE_FLAG_HAS_BRIGHTNESS
                | MODE_FLAG_HAS_DIRECTION_LR
                | MODE_FLAG_HAS_DIRECTION_UD;

            let (color_mode, colors_min, colors_max) = match rgb_mode {
                RgbMode::Rainbow | RgbMode::RainbowMorph | RgbMode::ColorCycle => {
                    (COLOR_MODE_NONE, 0, 0)
                }
                _ => {
                    flags |= MODE_FLAG_HAS_MODE_SPECIFIC_COLOR;
                    (COLOR_MODE_MODE_SPECIFIC, 1, 4)
                }
            };

            modes.push(self.build_mode_entry(
                name,
                value,
                flags,
                color_mode,
                colors_min,
                colors_max,
                0, // speed_min
                4, // speed_max
                DEFAULT_SPEED,
                DEFAULT_BRIGHTNESS,
            ));
        }

        modes
    }

    #[allow(clippy::too_many_arguments)]
    fn build_mode_entry(
        &self,
        name: &str,
        value: u32,
        flags: u32,
        color_mode: u32,
        colors_min: u32,
        colors_max: u32,
        speed_min: u32,
        speed_max: u32,
        speed: u32,
        brightness: u32,
    ) -> ModeData {
        ModeData {
            name: name.into(),
            value,
            flags,
            color_mode,
            colors_min,
            colors_max,
            speed_min,
            speed_max,
            brightness_min: 0,
            brightness_max: MAX_EFFECT_VALUE,
            speed,
            brightness,
            direction: DIR_RIGHT,
            colors: vec![[255; 3]; colors_min as usize],
        }
    }

    fn write_zone(&self, buf: &mut Vec<u8>, zone: &lianli_shared::rgb::RgbZoneInfo) {
        write_string(buf, &zone.name);
        buf.extend_from_slice(&ZONE_TYPE_LINEAR.to_le_bytes()); // zone_type
        buf.extend_from_slice(&(zone.led_count as u32).to_le_bytes()); // leds_min
        buf.extend_from_slice(&(zone.led_count as u32).to_le_bytes()); // leds_max
        buf.extend_from_slice(&(zone.led_count as u32).to_le_bytes()); // leds_count
        buf.extend_from_slice(&0u16.to_le_bytes()); // matrix_len = 0 (no matrix)

        // segments (proto >= 4)
        if self.protocol_version >= 4 {
            buf.extend_from_slice(&0u16.to_le_bytes()); // 0 segments
        }
        if self.protocol_version >= 5 {
            buf.extend_from_slice(&0u32.to_le_bytes()); // zone flags
        }
    }
}

/// Write an OpenRGB-format string: u16 length (includes null) + bytes + null.
fn write_string(buf: &mut Vec<u8>, s: &str) {
    let len = s.len() as u16 + 1; // +1 for null terminator
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
    buf.push(0); // null terminator
}

/// Map an OpenRGB mode name (from our own exposed modes) back to RgbMode.
fn mode_from_openrgb_name(name: &str, value: u32) -> RgbMode {
    // First try by display name (covers every mode we expose to OpenRGB).
    if let Some(mode) = RgbMode::from_display_name(name) {
        return mode;
    }

    // Fall back to TL mode byte value
    if value > 0 {
        if let Some(mode) = RgbMode::from_tl_mode_byte(value as u8) {
            return mode;
        }
    }

    RgbMode::Static
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn controller_profile_identity_is_stable_across_protocol_versions() {
        let cap: RgbDeviceCapabilities = serde_json::from_value(serde_json::json!({
            "device_id": "led-ring-serial",
            "device_name": "Universal Screen 8.8\" LED Ring",
            "supported_modes": [],
            "zones": [],
            "supports_direct": true,
            "supports_mb_rgb_sync": false,
            "total_led_count": 0,
            "supported_scopes": []
        }))
        .unwrap();

        for protocol_version in 0..=SERVER_PROTOCOL_VERSION {
            let data = ControllerSerializer { protocol_version }.build_controller_data(&cap);
            assert_eq!(
                u32::from_le_bytes(data[..4].try_into().unwrap()) as usize,
                data.len()
            );
            assert_eq!(
                u32::from_le_bytes(data[4..8].try_into().unwrap()),
                DEVICE_TYPE_LED_STRIP
            );
            let mut cursor = Cursor::new(&data[8..]);
            let mut read_string = || {
                let mut length = [0; 2];
                cursor.read_exact(&mut length).unwrap();
                let mut bytes = vec![0; u16::from_le_bytes(length) as usize];
                cursor.read_exact(&mut bytes).unwrap();
                assert_eq!(bytes.pop(), Some(0));
                String::from_utf8(bytes).unwrap()
            };
            assert_eq!(read_string(), "Universal Screen 8.8\" LED Ring");
            if protocol_version >= 1 {
                assert_eq!(read_string(), "Lian Li");
            }
            assert_eq!(
                read_string(),
                "Lian Li Universal Screen 8.8\" LED Ring RGB Controller"
            );
            assert_eq!(read_string(), "1.0");
            assert_eq!(read_string(), "led-ring-serial");
            assert_eq!(read_string(), "HID: led-ring-serial");
        }
    }

    fn header(size: usize) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(5u32.to_le_bytes());
        bytes.extend(105u32.to_le_bytes());
        bytes.extend((size as u32).to_le_bytes());
        bytes
    }

    #[test]
    fn packet_reader_checks_size_before_reading_the_payload() {
        let error = read_packet_from(&mut Cursor::new(header(MAX_PACKET_BYTES + 1))).unwrap_err();
        assert!(error.to_string().contains("1 MiB"));
        let mut bytes = header(MAX_PACKET_BYTES);
        bytes.resize(HEADER_SIZE + MAX_PACKET_BYTES, 42);
        let (device, packet, payload) = read_packet_from(&mut Cursor::new(bytes)).unwrap();
        assert_eq!((device, packet), (5, 105));
        assert_eq!(payload.len(), MAX_PACKET_BYTES);
        assert!(payload.iter().all(|byte| *byte == 42));
        assert!(read_packet_from(&mut Cursor::new(header(1))).is_err());
        let mut invalid = header(0);
        invalid[0] = b'X';
        assert!(read_packet_from(&mut Cursor::new(invalid)).is_err());
    }

    #[test]
    fn client_shutdown_unblocks_and_joins_an_idle_reader() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut reader, _) = listener.accept().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let control = reader.try_clone().unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            assert_eq!(reader.read(&mut [0u8; 1]).unwrap(), 0);
            done_tx.send(()).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(Clients(vec![(control, worker)]));
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(peer);
    }
}

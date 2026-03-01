//! OpenRGB SDK server: exposes Lian Li devices to OpenRGB/SignalRGB clients.
//!
//! Implements the OpenRGB network protocol (TCP, port 6742 by default).
//! Each physical device is exposed as an OpenRGB controller with its native
//! LED modes. Clients can enumerate devices, set modes, and update per-LED colors.

use crate::rgb_controller::{DirectColorBuffer, RgbController};
use lianli_shared::rgb::{RgbDeviceCapabilities, RgbDirection, RgbEffect, RgbMode};
use parking_lot::Mutex;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::{debug, error, info, warn};

const MAGIC: &[u8; 4] = b"ORGB";
const HEADER_SIZE: usize = 16;
/// We support up to protocol version 4 (segments, plugins).
/// Version 3 adds brightness. Version 4 adds segments.
const SERVER_PROTOCOL_VERSION: u32 = 4;

// Packet IDs
const PKT_REQUEST_CONTROLLER_COUNT: u32 = 0;
const PKT_REQUEST_CONTROLLER_DATA: u32 = 1;
const PKT_REQUEST_PROTOCOL_VERSION: u32 = 40;
const PKT_SET_CLIENT_NAME: u32 = 50;
const PKT_RESIZE_ZONE: u32 = 1000;
const PKT_UPDATE_LEDS: u32 = 1050;
const PKT_UPDATE_ZONE_LEDS: u32 = 1051;
const PKT_UPDATE_SINGLE_LED: u32 = 1052;
const PKT_SET_CUSTOM_MODE: u32 = 1100;
const PKT_UPDATE_MODE: u32 = 1101;
const PKT_SAVE_MODE: u32 = 1102;

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
    pub error: Option<String>,
}

/// Starts the OpenRGB SDK server in a background thread.
pub fn start_openrgb_server(
    rgb: Arc<Mutex<RgbController>>,
    direct_buffer: Arc<Mutex<DirectColorBuffer>>,
    port: u16,
    stop_flag: Arc<AtomicBool>,
    state: Arc<Mutex<OpenRgbServerState>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(e) = run_server(rgb, direct_buffer, port, &stop_flag, &state) {
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
        s.error = None;
    }

    let client_count = Arc::new(AtomicUsize::new(0));

    while !stop_flag.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, addr)) => {
                info!("OpenRGB client connected from {addr}");
                stream.set_nonblocking(false).ok();
                stream
                    .set_read_timeout(Some(Duration::from_secs(300)))
                    .ok();
                stream
                    .set_write_timeout(Some(Duration::from_secs(10)))
                    .ok();

                let rgb = Arc::clone(&rgb);
                let buf = Arc::clone(&direct_buffer);
                let count = Arc::clone(&client_count);
                let stop = Arc::clone(&stop_flag);

                let prev = count.fetch_add(1, Ordering::Relaxed);
                if prev == 0 {
                    rgb.lock().set_openrgb_active(true);
                }

                thread::spawn(move || {
                    let mut client = ClientHandler::new(stream, rgb, buf, stop);
                    if let Err(e) = client.run() {
                        debug!("OpenRGB client disconnected: {e}");
                    }

                    let remaining = count.fetch_sub(1, Ordering::Relaxed) - 1;
                    if remaining == 0 {
                        client.rgb.lock().set_openrgb_active(false);
                    }
                    info!("OpenRGB client disconnected ({remaining} remaining)");
                });
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

    info!("OpenRGB server stopped");
    Ok(())
}

struct ClientHandler {
    stream: TcpStream,
    rgb: Arc<Mutex<RgbController>>,
    direct_buffer: Arc<Mutex<DirectColorBuffer>>,
    stop_flag: Arc<AtomicBool>,
    protocol_version: u32,
    client_name: String,
    /// Cached capabilities — avoids locking RgbController on every UpdateLEDs packet.
    /// Populated lazily on first use; static for the lifetime of a connection.
    cached_caps: Option<Vec<RgbDeviceCapabilities>>,
}

impl ClientHandler {
    fn new(
        stream: TcpStream,
        rgb: Arc<Mutex<RgbController>>,
        direct_buffer: Arc<Mutex<DirectColorBuffer>>,
        stop_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            stream,
            rgb,
            direct_buffer,
            stop_flag,
            protocol_version: 0,
            client_name: String::new(),
            cached_caps: None,
        }
    }

    /// Get capabilities, caching on first call to avoid mutex contention during streaming.
    fn caps(&mut self) -> &[RgbDeviceCapabilities] {
        if self.cached_caps.is_none() {
            self.cached_caps = Some(self.rgb.lock().capabilities());
        }
        self.cached_caps.as_ref().unwrap()
    }

    /// Force-refresh the cached capabilities (e.g., after mode changes).
    fn refresh_caps(&mut self) {
        self.cached_caps = Some(self.rgb.lock().capabilities());
    }

    fn run(&mut self) -> anyhow::Result<()> {
        loop {
            if self.stop_flag.load(Ordering::Relaxed) {
                return Ok(());
            }

            let (dev_idx, pkt_id, payload) = self.read_packet()?;
            self.handle_packet(dev_idx, pkt_id, &payload)?;
        }
    }

    fn read_packet(&mut self) -> anyhow::Result<(u32, u32, Vec<u8>)> {
        let mut header = [0u8; HEADER_SIZE];
        self.stream.read_exact(&mut header)?;

        if &header[0..4] != MAGIC {
            anyhow::bail!("Invalid magic bytes");
        }

        let dev_idx = u32::from_le_bytes(header[4..8].try_into()?);
        let pkt_id = u32::from_le_bytes(header[8..12].try_into()?);
        let pkt_size = u32::from_le_bytes(header[12..16].try_into()?) as usize;

        let mut payload = vec![0u8; pkt_size];
        if pkt_size > 0 {
            self.stream.read_exact(&mut payload)?;
        }

        Ok((dev_idx, pkt_id, payload))
    }

    fn send_packet(&mut self, dev_idx: u32, pkt_id: u32, payload: &[u8]) -> anyhow::Result<()> {
        let mut header = [0u8; HEADER_SIZE];
        header[0..4].copy_from_slice(MAGIC);
        header[4..8].copy_from_slice(&dev_idx.to_le_bytes());
        header[8..12].copy_from_slice(&pkt_id.to_le_bytes());
        header[12..16].copy_from_slice(&(payload.len() as u32).to_le_bytes());

        self.stream.write_all(&header)?;
        if !payload.is_empty() {
            self.stream.write_all(payload)?;
        }
        self.stream.flush()?;
        Ok(())
    }

    fn handle_packet(
        &mut self,
        dev_idx: u32,
        pkt_id: u32,
        payload: &[u8],
    ) -> anyhow::Result<()> {
        match pkt_id {
            PKT_REQUEST_PROTOCOL_VERSION => {
                let client_version = if payload.len() >= 4 {
                    u32::from_le_bytes(payload[0..4].try_into()?)
                } else {
                    0
                };
                self.protocol_version = client_version.min(SERVER_PROTOCOL_VERSION);
                debug!(
                    "OpenRGB protocol negotiated: v{} (client={}, server={})",
                    self.protocol_version, client_version, SERVER_PROTOCOL_VERSION
                );
                let resp = SERVER_PROTOCOL_VERSION.to_le_bytes();
                self.send_packet(0, PKT_REQUEST_PROTOCOL_VERSION, &resp)?;
            }

            PKT_SET_CLIENT_NAME => {
                // Raw string + null terminator, no u16 length prefix
                self.client_name = String::from_utf8_lossy(payload)
                    .trim_end_matches('\0')
                    .to_string();
                info!("OpenRGB client name: '{}'", self.client_name);
                // No response
            }

            PKT_REQUEST_CONTROLLER_COUNT => {
                self.refresh_caps();
                let count = self.caps().len() as u32;
                self.send_packet(0, PKT_REQUEST_CONTROLLER_COUNT, &count.to_le_bytes())?;
            }

            PKT_REQUEST_CONTROLLER_DATA => {
                let cap = self.caps().get(dev_idx as usize).cloned();
                if let Some(cap) = cap {
                    let data = self.build_controller_data(&cap);
                    self.send_packet(dev_idx, PKT_REQUEST_CONTROLLER_DATA, &data)?;
                } else {
                    // Empty response for invalid index
                    self.send_packet(dev_idx, PKT_REQUEST_CONTROLLER_DATA, &[])?;
                }
            }

            PKT_SET_CUSTOM_MODE => {
                // Client wants to switch to direct/custom mode. No-op for us.
                debug!("OpenRGB SetCustomMode for device {dev_idx}");
            }

            PKT_UPDATE_LEDS => {
                self.handle_update_leds(dev_idx, payload)?;
            }

            PKT_UPDATE_ZONE_LEDS => {
                self.handle_update_zone_leds(dev_idx, payload)?;
            }

            PKT_UPDATE_SINGLE_LED => {
                self.handle_update_single_led(dev_idx, payload)?;
            }

            PKT_UPDATE_MODE => {
                self.handle_update_mode(dev_idx, payload)?;
            }

            PKT_SAVE_MODE => {
                // Same as update mode for us (no persistent hardware save)
                self.handle_update_mode(dev_idx, payload)?;
            }

            PKT_RESIZE_ZONE => {
                // Ignore zone resize — our zones are fixed hardware
                debug!("OpenRGB ResizeZone for device {dev_idx} (ignored)");
            }

            _ => {
                debug!("OpenRGB unhandled packet: id={pkt_id} dev={dev_idx} size={}", payload.len());
            }
        }

        Ok(())
    }

    fn handle_update_leds(&mut self, dev_idx: u32, payload: &[u8]) -> anyhow::Result<()> {
        // data_size(u32) + num_colors(u16) + colors(4*n)
        if payload.len() < 6 {
            return Ok(());
        }

        let num_colors = u16::from_le_bytes(payload[4..6].try_into()?) as usize;
        let colors = parse_colors(&payload[6..], num_colors);

        // Use cached caps — no RgbController lock needed in the hot path
        if let Some(cap) = self.caps().get(dev_idx as usize) {
            let device_id = cap.device_id.clone();
            let zones: Vec<_> = cap.zones.iter().map(|z| z.led_count as usize).collect();
            // Buffer colors per zone — writer thread will flush asap
            let mut buf = self.direct_buffer.lock();
            let mut offset = 0;
            for (zone_idx, count) in zones.iter().enumerate() {
                let end = (offset + count).min(colors.len());
                if offset < colors.len() {
                    buf.set(device_id.clone(), zone_idx as u8, colors[offset..end].to_vec());
                }
                offset = end;
            }
        }

        Ok(())
    }

    fn handle_update_zone_leds(&mut self, dev_idx: u32, payload: &[u8]) -> anyhow::Result<()> {
        // data_size(u32) + zone_idx(u32) + num_colors(u16) + colors(4*n)
        if payload.len() < 10 {
            return Ok(());
        }

        let zone_idx = u32::from_le_bytes(payload[4..8].try_into()?) as u8;
        let num_colors = u16::from_le_bytes(payload[8..10].try_into()?) as usize;
        let colors = parse_colors(&payload[10..], num_colors);

        if let Some(cap) = self.caps().get(dev_idx as usize) {
            let device_id = cap.device_id.clone();
            self.direct_buffer.lock().set(device_id, zone_idx, colors);
        }

        Ok(())
    }

    fn handle_update_single_led(&mut self, dev_idx: u32, payload: &[u8]) -> anyhow::Result<()> {
        // led_idx(i32/u32) + color(4 bytes)
        if payload.len() < 8 {
            return Ok(());
        }

        let led_idx = u32::from_le_bytes(payload[0..4].try_into()?) as usize;
        let r = payload[4];
        let g = payload[5];
        let b = payload[6];

        if let Some(cap) = self.caps().get(dev_idx as usize) {
            let device_id = cap.device_id.clone();
            let zones: Vec<_> = cap.zones.iter().map(|z| z.led_count as usize).collect();
            let mut offset = 0;
            for (zone_idx, count) in zones.iter().enumerate() {
                if led_idx < offset + count {
                    let colors = vec![[r, g, b]];
                    self.direct_buffer.lock().set(device_id, zone_idx as u8, colors);
                    break;
                }
                offset += count;
            }
        }

        Ok(())
    }

    fn handle_update_mode(&mut self, dev_idx: u32, payload: &[u8]) -> anyhow::Result<()> {
        // data_size(u32) + mode_idx(u32) + ModeData...
        if payload.len() < 8 {
            return Ok(());
        }

        let mode_idx = u32::from_le_bytes(payload[4..8].try_into()?);
        let mode_data = &payload[8..];

        // Parse the mode data to extract what we need
        if let Some(effect) = self.parse_mode_data(mode_data) {
            // Direct mode is controlled exclusively via UpdateLEDs/UpdateZoneLEDs.
            // Applying it here would overwrite per-LED colors with defaults (all black).
            if effect.mode == RgbMode::Direct {
                debug!("OpenRGB UpdateMode: mode=Direct (ignored — use UpdateLEDs)");
                return Ok(());
            }

            // UpdateMode is rare (not streaming hot path), so lock is fine here
            let caps = self.rgb.lock().capabilities();
            if let Some(cap) = caps.get(dev_idx as usize) {
                let device_id = cap.device_id.clone();
                debug!(
                    "OpenRGB UpdateMode: device={device_id} mode_idx={mode_idx} -> {:?}",
                    effect.mode
                );
                for zone_idx in 0..cap.zones.len() {
                    if let Err(e) = self
                        .rgb
                        .lock()
                        .set_effect(&device_id, zone_idx as u8, &effect)
                    {
                        debug!("OpenRGB UpdateMode error for {device_id} zone {zone_idx}: {e}");
                    }
                }
            }
        }

        Ok(())
    }

    /// Parse a ModeData blob from the wire to extract an RgbEffect.
    fn parse_mode_data(&self, data: &[u8]) -> Option<RgbEffect> {
        let mut cursor = 0;

        // name (u16 len + bytes + null)
        let name_str = read_string(data, &mut cursor)?;

        // value (i32)
        let value = read_u32(data, &mut cursor)?;

        // flags (u32)
        let _flags = read_u32(data, &mut cursor)?;

        // speed_min, speed_max (u32 each)
        let _speed_min = read_u32(data, &mut cursor)?;
        let _speed_max = read_u32(data, &mut cursor)?;

        // brightness_min, brightness_max (proto >= 3)
        if self.protocol_version >= 3 {
            let _brightness_min = read_u32(data, &mut cursor)?;
            let _brightness_max = read_u32(data, &mut cursor)?;
        }

        // colors_min, colors_max (u32 each)
        let _colors_min = read_u32(data, &mut cursor)?;
        let _colors_max = read_u32(data, &mut cursor)?;

        // speed (u32)
        let speed = read_u32(data, &mut cursor)?;

        // brightness (proto >= 3)
        let brightness = if self.protocol_version >= 3 {
            read_u32(data, &mut cursor)?
        } else {
            4
        };

        // direction (u32)
        let direction = read_u32(data, &mut cursor)?;

        // color_mode (u32)
        let _color_mode = read_u32(data, &mut cursor)?;

        // colors (u16 count + 4 bytes each)
        let color_count = read_u16(data, &mut cursor)? as usize;
        let mut colors = Vec::new();
        for _ in 0..color_count {
            if cursor + 4 > data.len() {
                break;
            }
            colors.push([data[cursor], data[cursor + 1], data[cursor + 2]]);
            cursor += 4; // skip alpha
        }

        // Map the mode name to our RgbMode
        let mode = mode_from_openrgb_name(&name_str, value);

        // Map direction
        let dir = match direction {
            DIR_LEFT => RgbDirection::CounterClockwise,
            DIR_RIGHT => RgbDirection::Clockwise,
            DIR_UP => RgbDirection::Up,
            DIR_DOWN => RgbDirection::Down,
            _ => RgbDirection::Clockwise,
        };

        // Scale speed: OpenRGB 0..4 maps to our 0..4
        let spd = (speed as u8).min(4);
        let bri = (brightness as u8).min(4);

        Some(RgbEffect {
            mode,
            colors,
            speed: spd,
            brightness: bri,
            direction: dir,
            ..RgbEffect::default()
        })
    }

    /// Build the full ControllerData response for a device.
    fn build_controller_data(&self, cap: &RgbDeviceCapabilities) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1024);

        // data_size placeholder — we'll fill it at the end
        buf.extend_from_slice(&0u32.to_le_bytes());

        // device_type
        let dev_type = if cap.device_name.contains("Galahad") || cap.device_name.contains("AIO") {
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
        write_string(&mut buf, &format!("Lian Li {} RGB Controller", cap.device_name));

        // version
        write_string(&mut buf, env!("CARGO_PKG_VERSION"));

        // serial
        write_string(&mut buf, &cap.device_id);

        // location
        write_string(&mut buf, &format!("HID: {}", cap.device_id));

        // Build modes
        let modes = self.build_modes(cap);
        // num_modes (u16)
        buf.extend_from_slice(&(modes.len() as u16).to_le_bytes());
        // active_mode (i32) — default to 0 (first mode)
        buf.extend_from_slice(&0i32.to_le_bytes());
        // mode data (no u16 prefix — count was already written above)
        for mode_buf in &modes {
            buf.extend_from_slice(mode_buf);
        }

        // zones (u16 count + data)
        buf.extend_from_slice(&(cap.zones.len() as u16).to_le_bytes());
        for zone in &cap.zones {
            self.write_zone(&mut buf, zone);
        }

        // LEDs (u16 count + data)
        let total_leds = cap.total_led_count as usize;
        buf.extend_from_slice(&(total_leds as u16).to_le_bytes());
        let mut led_idx = 0;
        for zone in &cap.zones {
            for i in 0..zone.led_count {
                write_string(&mut buf, &format!("{} LED {}", zone.name, i));
                buf.extend_from_slice(&(led_idx as u32).to_le_bytes()); // value
                led_idx += 1;
            }
        }

        // colors (u16 count + 4 bytes each) — initialize to white
        buf.extend_from_slice(&(total_leds as u16).to_le_bytes());
        for _ in 0..total_leds {
            buf.extend_from_slice(&[255, 255, 255, 0]); // RGBA, A=0
        }

        // Fill in data_size (everything after the data_size field itself)
        let data_size = (buf.len()) as u32;
        buf[0..4].copy_from_slice(&data_size.to_le_bytes());

        buf
    }

    fn build_modes(&self, cap: &RgbDeviceCapabilities) -> Vec<Vec<u8>> {
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
                0,    // speed_min
                4,    // speed_max
                2,    // default speed
                4,    // default brightness
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
        default_speed: u32,
        default_brightness: u32,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(128);

        write_string(&mut buf, name);
        buf.extend_from_slice(&(value as i32).to_le_bytes()); // value
        buf.extend_from_slice(&flags.to_le_bytes());
        buf.extend_from_slice(&speed_min.to_le_bytes());
        buf.extend_from_slice(&speed_max.to_le_bytes());

        if self.protocol_version >= 3 {
            buf.extend_from_slice(&0u32.to_le_bytes()); // brightness_min
            buf.extend_from_slice(&4u32.to_le_bytes()); // brightness_max
        }

        buf.extend_from_slice(&colors_min.to_le_bytes());
        buf.extend_from_slice(&colors_max.to_le_bytes());
        buf.extend_from_slice(&default_speed.to_le_bytes()); // speed

        if self.protocol_version >= 3 {
            buf.extend_from_slice(&default_brightness.to_le_bytes()); // brightness
        }

        buf.extend_from_slice(&(DIR_RIGHT).to_le_bytes()); // direction
        buf.extend_from_slice(&color_mode.to_le_bytes());

        // colors: empty by default for new modes
        buf.extend_from_slice(&0u16.to_le_bytes()); // 0 colors

        buf
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
    }
}

/// Write an OpenRGB-format string: u16 length (includes null) + bytes + null.
fn write_string(buf: &mut Vec<u8>, s: &str) {
    let len = s.len() as u16 + 1; // +1 for null terminator
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
    buf.push(0); // null terminator
}

/// Read an OpenRGB-format string: u16 length + bytes + null.
fn read_string(data: &[u8], cursor: &mut usize) -> Option<String> {
    if *cursor + 2 > data.len() {
        return None;
    }
    let len = u16::from_le_bytes(data[*cursor..*cursor + 2].try_into().ok()?) as usize;
    *cursor += 2;
    if *cursor + len > data.len() || len == 0 {
        return None;
    }
    let bytes = &data[*cursor..*cursor + len - 1]; // exclude null
    *cursor += len;
    Some(String::from_utf8_lossy(bytes).to_string())
}

fn read_u32(data: &[u8], cursor: &mut usize) -> Option<u32> {
    if *cursor + 4 > data.len() {
        return None;
    }
    let val = u32::from_le_bytes(data[*cursor..*cursor + 4].try_into().ok()?);
    *cursor += 4;
    Some(val)
}

fn read_u16(data: &[u8], cursor: &mut usize) -> Option<u16> {
    if *cursor + 2 > data.len() {
        return None;
    }
    let val = u16::from_le_bytes(data[*cursor..*cursor + 2].try_into().ok()?);
    *cursor += 2;
    Some(val)
}

/// Parse colors from OpenRGB wire format (4 bytes each: R, G, B, A).
fn parse_colors(data: &[u8], count: usize) -> Vec<[u8; 3]> {
    let mut colors = Vec::with_capacity(count);
    for i in 0..count {
        let offset = i * 4;
        if offset + 3 > data.len() {
            break;
        }
        colors.push([data[offset], data[offset + 1], data[offset + 2]]);
    }
    colors
}

/// Map an OpenRGB mode name (from our own exposed modes) back to RgbMode.
fn mode_from_openrgb_name(name: &str, value: u32) -> RgbMode {
    // First try by name
    match name {
        "Direct" => return RgbMode::Direct,
        "Static" => return RgbMode::Static,
        "Rainbow" => return RgbMode::Rainbow,
        "Rainbow Morph" => return RgbMode::RainbowMorph,
        "Breathing" => return RgbMode::Breathing,
        "Runway" => return RgbMode::Runway,
        "Meteor" => return RgbMode::Meteor,
        "Color Cycle" => return RgbMode::ColorCycle,
        "Staggered" => return RgbMode::Staggered,
        "Tide" => return RgbMode::Tide,
        "Mixing" => return RgbMode::Mixing,
        "Voice" => return RgbMode::Voice,
        "Door" => return RgbMode::Door,
        "Render" => return RgbMode::Render,
        "Ripple" => return RgbMode::Ripple,
        "Reflect" => return RgbMode::Reflect,
        "Tail Chasing" => return RgbMode::TailChasing,
        "Paint" => return RgbMode::Paint,
        "Ping Pong" => return RgbMode::PingPong,
        "Stack" => return RgbMode::Stack,
        "Cover Cycle" => return RgbMode::CoverCycle,
        "Wave" => return RgbMode::Wave,
        "Racing" => return RgbMode::Racing,
        "Lottery" => return RgbMode::Lottery,
        "Intertwine" => return RgbMode::Intertwine,
        "Meteor Shower" => return RgbMode::MeteorShower,
        "Collide" => return RgbMode::Collide,
        "Electric Current" => return RgbMode::ElectricCurrent,
        "Kaleidoscope" => return RgbMode::Kaleidoscope,
        "Big Bang" => return RgbMode::BigBang,
        "Vortex" => return RgbMode::Vortex,
        "Pump" => return RgbMode::Pump,
        "Colors Morph" => return RgbMode::ColorsMorph,
        _ => {}
    }

    // Fall back to TL mode byte value
    if value > 0 {
        if let Some(mode) = RgbMode::from_tl_mode_byte(value as u8) {
            return mode;
        }
    }

    RgbMode::Static
}

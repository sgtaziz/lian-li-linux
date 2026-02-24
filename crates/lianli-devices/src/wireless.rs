use anyhow::{bail, Context, Result};
use lianli_transport::usb::{UsbTransport, USB_TIMEOUT};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::fmt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{debug, info, warn};

// ── USB VID/PID ──────────────────────────────────────────────────────────────

const TX_VENDOR: u16 = 0x0416;
const TX_PRODUCT: u16 = 0x8040;
const RX_VENDOR: u16 = 0x0416;
const RX_PRODUCT: u16 = 0x8041;

// ── USB command codes (from L-Connect 3 USB_CMD enum) ────────────────────────

const USB_CMD_SEND_RF: u8 = 0x10; // Usb_SendRf
const USB_CMD_GET_MAC: u8 = 0x11; // USB_GetMac

// ── RF command codes (from L-Connect 3 RF_CMD enum) ──────────────────────────

const RF_SELECT: u8 = 18; // RF_Select — carries fan PWM data
const RF_SET_RGB: u8 = 32; // RF_SetRGB (0x20) — carries LED frame data

// ── RF packet geometry ───────────────────────────────────────────────────────

const RF_DATA_SIZE: usize = 240; // Total RF payload
const RF_CHUNK_SIZE: usize = 60; // Payload per USB packet (64 - 4 header)
const RF_CHUNKS: usize = RF_DATA_SIZE / RF_CHUNK_SIZE; // 4

// ── Prebuilt USB commands ────────────────────────────────────────────────────

static CMD_RESET: Lazy<Vec<u8>> = Lazy::new(|| decode_command("11080000"));
static CMD_VIDEO_START: Lazy<Vec<u8>> = Lazy::new(|| decode_command("11010000"));
static CMD_RX_QUERY_34: Lazy<Vec<u8>> = Lazy::new(|| decode_command("10010434"));
static CMD_RX_QUERY_37: Lazy<Vec<u8>> = Lazy::new(|| decode_command("10010437"));
static CMD_RX_LCD_MODE: Lazy<Vec<u8>> = Lazy::new(|| decode_command("10010430"));

fn decode_command(prefix: &str) -> Vec<u8> {
    let mut bytes = hex::decode(prefix).expect("valid hex literal");
    bytes.resize(64, 0u8);
    bytes
}

// ── Fan type classification (from L-Connect 3 device type byte parsing) ──────

/// Wireless fan device type, determines minimum duty and RPM curves.
///
/// Byte ranges from L-Connect 3 `RfDevice.cs`:
/// ```text
/// SLV3  (base 20): 20-26  (LED: 20-23, LCD: 24-26)
/// TLV2  (base 27): 27-35  (LCD: 27,32-35, LED: 28-31)
/// SLINF (base 36): 36-39  (LED only)
/// RL120:           40
/// CLV1:            41-42
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WirelessFanType {
    /// SLV3 120mm/140mm LED fans (no LCD) — 14% minimum duty
    Slv3Led,
    /// SLV3 120mm/140mm LCD fans — 14% minimum duty
    Slv3Lcd,
    /// TLV2 120mm/140mm LCD fans — 10% minimum duty
    Tlv2Lcd,
    /// TLV2 120mm/140mm LED fans (no LCD) — 11% minimum duty
    Tlv2Led,
    /// SL-INF wireless fans — 11% minimum duty
    SlInf,
    /// CL / RL120 fans — 10% minimum duty (special PWM filter)
    Clv1,
    /// Unknown fan type
    Unknown,
}

impl WirelessFanType {
    /// Minimum duty percentage for this fan type (from L-Connect 3 source).
    pub fn min_duty_percent(self) -> u8 {
        match self {
            Self::Slv3Led | Self::Slv3Lcd => 14,
            Self::Tlv2Lcd => 10,
            Self::Tlv2Led | Self::SlInf => 11,
            Self::Clv1 => 10,
            Self::Unknown => 10,
        }
    }

    /// Human-readable display name for this fan type.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Slv3Led => "UNI FAN SL V3 Wireless",
            Self::Slv3Lcd => "UNI FAN SL V3 Wireless LCD",
            Self::Tlv2Lcd => "UNI FAN TL Wireless LCD",
            Self::Tlv2Led => "UNI FAN TL Wireless",
            Self::SlInf => "UNI FAN SL-INF Wireless",
            Self::Clv1 => "UNI FAN CL Wireless",
            Self::Unknown => "Wireless Fan",
        }
    }

    /// Number of addressable LEDs per fan for this device type.
    ///
    /// From L-Connect 3 decompiled source (RfDevice.cs rgb data arrays):
    /// - TLV2: 104 LEDs per zone (UP/DOWN combined, ~26 per fan)
    /// - SLV3: 160 LEDs per zone (inner + outer rings, ~40 per fan)
    /// - SL-INF: 176 LEDs total across all fans (~44 per fan)
    /// - CL: ~24 LEDs per fan (outer + center)
    pub fn leds_per_fan(self) -> u8 {
        match self {
            Self::Tlv2Lcd | Self::Tlv2Led => 26,
            Self::Slv3Led | Self::Slv3Lcd => 40,
            Self::SlInf => 44,
            Self::Clv1 => 24,
            Self::Unknown => 20,
        }
    }

    /// Classify fan type from the fan-type byte in the device record.
    ///
    /// Byte ranges from L-Connect 3 `RfDevice.cs`:
    ///   `RecType[k] = (num < 27) ? SLV3Fan : (num < 36) ? TLV2Fan : SLINF`
    /// Within SLV3/TLV2, bytes base+4..base+7 have LCD (BindLcd=true).
    fn from_fan_type_byte(b: u8) -> Self {
        match b {
            20..=23 => Self::Slv3Led,          // SLV3 LED (120/140, normal/reverse)
            24..=26 => Self::Slv3Lcd,          // SLV3 LCD (120/140, normal/reverse)
            27 | 32..=35 => Self::Tlv2Lcd,     // TLV2 LCD
            28..=31 => Self::Tlv2Led,          // TLV2 LED (120/140, normal/reverse)
            36..=39 => Self::SlInf,            // SL-INF (LED only)
            40 => Self::Clv1,                  // RL120
            41..=42 => Self::Clv1,             // CLV1 variants
            _ => Self::Unknown,
        }
    }
}

// ── Device record parsed from GetDev response ────────────────────────────────

/// A wireless device discovered via the RX GetDev command.
/// Parsed from the 42-byte device record in the response.
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    /// Device MAC address (6 bytes)
    pub mac: [u8; 6],
    /// Master MAC this device is bound to (6 bytes)
    pub master_mac: [u8; 6],
    /// RF channel this device communicates on
    pub channel: u8,
    /// RX type (radio endpoint address, unique per device)
    pub rx_type: u8,
    /// Device type byte (0=fan group, 65=LC217 LCD, 255=master)
    pub device_type: u8,
    /// Number of fans connected (0-4)
    pub fan_count: u8,
    /// Fan type bytes for each slot (determines fan model)
    pub fan_types: [u8; 4],
    /// Current fan RPMs (read from device, big-endian u16 x4)
    pub fan_rpms: [u16; 4],
    /// Current PWM values being applied (0-255 x4)
    pub current_pwm: [u8; 4],
    /// Command sequence number
    pub cmd_seq: u8,
    /// Classified fan type for the device
    pub fan_type: WirelessFanType,
    /// Index in the discovery list (used for video mode prep)
    pub list_index: u8,
}

impl DiscoveredDevice {
    /// MAC address as a colon-separated hex string.
    pub fn mac_str(&self) -> String {
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.mac[0], self.mac[1], self.mac[2],
            self.mac[3], self.mac[4], self.mac[5],
        )
    }
}

impl fmt::Display for DiscoveredDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({:?}, {} fans, ch={}, rx={})",
            self.mac_str(),
            self.fan_type,
            self.fan_count,
            self.channel,
            self.rx_type,
        )
    }
}

/// Parse a 42-byte device record from GetDev response.
///
/// Record layout (from L-Connect 3 RefreshList):
/// ```text
/// [0-5]   Device MAC (6 bytes)
/// [6-11]  Master MAC (6 bytes)
/// [12]    RF Channel
/// [13]    RX Type (radio endpoint)
/// [14-17] System time (ms * 0.625)
/// [18]    Device type (0=fan, 65=LC217, 255=master)
/// [19]    Fan count
/// [20-23] Effect index (4 bytes)
/// [24-27] Fan type bytes (4 bytes, per-slot)
/// [28-35] Fan speeds (4x u16 big-endian RPM)
/// [36-39] Current PWM (4 bytes)
/// [40]    Command sequence number
/// [41]    Validation marker (must be 0x1C = 28)
/// ```
fn parse_device_record(data: &[u8], list_index: u8) -> Option<DiscoveredDevice> {
    if data.len() < 42 {
        return None;
    }

    // Validate marker
    if data[41] != 0x1C {
        debug!(
            "  Device record {list_index}: invalid marker 0x{:02x} (expected 0x1C)",
            data[41]
        );
        return None;
    }

    let device_type = data[18];

    // Skip master device (type 0xFF)
    if device_type == 0xFF {
        debug!("  Device record {list_index}: skipping master device");
        return None;
    }

    let mut mac = [0u8; 6];
    mac.copy_from_slice(&data[0..6]);

    let mut master_mac = [0u8; 6];
    master_mac.copy_from_slice(&data[6..12]);

    let channel = data[12];
    let rx_type = data[13];
    let fan_count = data[19].min(4); // Cap at 4

    let mut fan_types = [0u8; 4];
    fan_types.copy_from_slice(&data[24..28]);

    // Fan RPMs: 4x big-endian u16 at offset 28-35
    let fan_rpms = [
        u16::from_be_bytes([data[28], data[29]]),
        u16::from_be_bytes([data[30], data[31]]),
        u16::from_be_bytes([data[32], data[33]]),
        u16::from_be_bytes([data[34], data[35]]),
    ];

    let mut current_pwm = [0u8; 4];
    current_pwm.copy_from_slice(&data[36..40]);

    let cmd_seq = data[40];

    // Classify fan type from the first non-zero fan type byte
    let fan_type = fan_types
        .iter()
        .find(|&&b| b != 0)
        .map(|&b| WirelessFanType::from_fan_type_byte(b))
        .unwrap_or(WirelessFanType::Unknown);

    Some(DiscoveredDevice {
        mac,
        master_mac,
        channel,
        rx_type,
        device_type,
        fan_count,
        fan_types,
        fan_rpms,
        current_pwm,
        cmd_seq,
        fan_type,
        list_index,
    })
}

// ── WirelessController ───────────────────────────────────────────────────────

pub struct WirelessController {
    tx: Option<Arc<Mutex<UsbTransport>>>,
    rx: Option<Arc<Mutex<UsbTransport>>>,
    poll_stop: Arc<AtomicBool>,
    poll_thread: Option<JoinHandle<()>>,
    video_mode_active: Arc<AtomicBool>,
    master_mac: Arc<Mutex<[u8; 6]>>,
    master_channel: Arc<Mutex<u8>>,
    discovered_devices: Arc<Mutex<Vec<DiscoveredDevice>>>,
}

impl Clone for WirelessController {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            rx: self.rx.clone(),
            poll_stop: Arc::clone(&self.poll_stop),
            poll_thread: None,
            video_mode_active: Arc::clone(&self.video_mode_active),
            master_mac: Arc::clone(&self.master_mac),
            master_channel: Arc::clone(&self.master_channel),
            discovered_devices: Arc::clone(&self.discovered_devices),
        }
    }
}

impl WirelessController {
    pub fn new() -> Self {
        Self {
            tx: None,
            rx: None,
            poll_stop: Arc::new(AtomicBool::new(false)),
            poll_thread: None,
            video_mode_active: Arc::new(AtomicBool::new(false)),
            master_mac: Arc::new(Mutex::new([0u8; 6])),
            master_channel: Arc::new(Mutex::new(8)),
            discovered_devices: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn connect(&mut self) -> Result<()> {
        let mut tx = None;
        let max_retries = 3;

        for attempt in 1..=max_retries {
            match UsbTransport::open(TX_VENDOR, TX_PRODUCT) {
                Ok(device) => {
                    tx = Some(device);
                    break;
                }
                Err(e) if attempt < max_retries => {
                    debug!("TX device not found (attempt {attempt}/{max_retries}): {e}");
                    thread::sleep(Duration::from_millis(1000 * attempt as u64));
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(e))
                        .context("opening wireless TX (0416:8040)");
                }
            }
        }

        let mut tx = tx.context("TX device failed to open after retries")?;
        tx.detach_and_configure("TX")?;
        let tx_arc = Arc::new(Mutex::new(tx));

        let rx_arc = match UsbTransport::open(RX_VENDOR, RX_PRODUCT) {
            Ok(mut rx) => {
                rx.detach_and_configure("RX")?;
                Some(Arc::new(Mutex::new(rx)))
            }
            Err(_) => {
                warn!("RX device (0416:8041) not found – telemetry disabled");
                None
            }
        };

        self.tx = Some(tx_arc);
        self.rx = rx_arc;

        self.discover_master_mac()?;
        Ok(())
    }

    /// Discovers master MAC address and channel by querying TX with USB_GetMac.
    ///
    /// L-Connect 3 tries the configured channel first, then scans.
    /// Channels should be even numbers (L-Connect constraint).
    fn discover_master_mac(&self) -> Result<()> {
        let tx = self.tx.as_ref().context("TX device not available")?;
        info!("Discovering master MAC address and wireless channel...");

        // Try default (8) first, then even channels 2-38, then odd as fallback
        let channels_to_try: Vec<u8> = std::iter::once(8u8)
            .chain((2..=38).filter(|&ch| ch != 8 && ch % 2 == 0))
            .chain((1..=39).filter(|&ch| ch % 2 == 1))
            .collect();

        for channel in channels_to_try {
            let mut cmd = vec![0u8; 64];
            cmd[0] = USB_CMD_GET_MAC;
            cmd[1] = channel;

            let handle = tx.lock();
            if handle.write_bulk(&cmd, USB_TIMEOUT).is_err() {
                drop(handle);
                continue;
            }

            let mut response = [0u8; 64];
            let len = match handle.read_bulk(&mut response, Duration::from_millis(500)) {
                Ok(len) => len,
                Err(_) => {
                    drop(handle);
                    continue;
                }
            };
            drop(handle);

            // Response: [0]=0x11, [1-6]=master MAC, [7-10]=sysTime, [11-12]=fwVer
            if len >= 7 && response[0] == USB_CMD_GET_MAC {
                let mut mac = self.master_mac.lock();
                mac.copy_from_slice(&response[1..7]);
                if mac.iter().any(|&b| b != 0) {
                    *self.master_channel.lock() = channel;
                    info!(
                        "Master MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} channel={}",
                        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], channel
                    );
                    if len >= 13 {
                        let fw_ver = u16::from_be_bytes([response[11], response[12]]);
                        debug!("Master firmware version: {fw_ver}");
                    }
                    return Ok(());
                }
            }
        }

        bail!("Failed to discover master MAC on any channel (tried 1-39)");
    }

    pub fn start_polling(&mut self) -> Result<()> {
        let tx = self
            .tx
            .as_ref()
            .cloned()
            .context("TX device must be connected before polling")?;
        let rx = self
            .rx
            .as_ref()
            .cloned()
            .context("RX device must be connected for device discovery")?;

        {
            let handle = tx.lock();
            handle
                .write_bulk(&CMD_RESET, USB_TIMEOUT)
                .context("sending TX reset")?;
        }

        self.video_mode_active.store(false, Ordering::Release);
        self.poll_stop.store(false, Ordering::SeqCst);

        let stop_flag = self.poll_stop.clone();
        let discovered_devices = Arc::clone(&self.discovered_devices);

        self.poll_thread = Some(thread::spawn(move || {
            while !stop_flag.load(Ordering::SeqCst) {
                if let Err(err) = poll_and_discover(&rx, &discovered_devices) {
                    warn!("RX polling error: {err:?}");
                    break;
                }
                thread::sleep(Duration::from_millis(500));
            }
        }));

        thread::sleep(Duration::from_millis(1500));
        Ok(())
    }

    pub fn ensure_video_mode(&self) -> Result<()> {
        if self.video_mode_active.load(Ordering::Acquire) {
            return Ok(());
        }

        if let Some(tx) = &self.tx {
            let handle = tx.lock();
            handle
                .write_bulk(&CMD_VIDEO_START, USB_TIMEOUT)
                .context("sending TX video start")?;
            thread::sleep(Duration::from_millis(2));

            let devices = self.discovered_devices.lock();
            let device_count = devices.len().max(1);
            let master_ch = *self.master_channel.lock();

            for device_idx in 0..device_count {
                let mut cmd = vec![0u8; 64];
                cmd[0] = USB_CMD_SEND_RF;
                cmd[1] = device_idx as u8;
                cmd[2] = master_ch;
                cmd[3] = 0xFF; // Prep marker
                handle
                    .write_bulk(&cmd, USB_TIMEOUT)
                    .context("sending TX prep command")?;
                thread::sleep(Duration::from_millis(1));
            }

            drop(handle);
            self.video_mode_active.store(true, Ordering::Release);
            info!("Video mode activated with {device_count} device(s)");
        }
        Ok(())
    }

    pub fn send_rx_sequence(&self) -> Result<()> {
        if let Some(rx) = &self.rx {
            for (cmd, capture) in [
                (&*CMD_RX_QUERY_34, true),
                (&*CMD_RX_QUERY_37, true),
                (&*CMD_RX_LCD_MODE, false),
            ] {
                {
                    let handle = rx.lock();
                    handle
                        .write_bulk(cmd, USB_TIMEOUT)
                        .context("sending RX command")?;
                }
                thread::sleep(Duration::from_millis(2));
                if capture {
                    let mut buf = [0u8; 64];
                    let handle = rx.lock();
                    if let Ok(len) = handle.read_bulk(&mut buf, USB_TIMEOUT) {
                        debug!("RX resp: {:02x?}", &buf[..len.min(8)]);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn soft_reset(&mut self) -> bool {
        if self.tx.is_none() {
            if let Ok(mut transport) = UsbTransport::open(TX_VENDOR, TX_PRODUCT) {
                if transport.detach_and_configure("TX").is_ok() {
                    self.tx = Some(Arc::new(Mutex::new(transport)));
                }
            }
        }

        if let Some(tx) = &self.tx {
            {
                let handle = tx.lock();
                if handle.write_bulk(&CMD_RESET, USB_TIMEOUT).is_err() {
                    return false;
                }
            }
            self.video_mode_active.store(false, Ordering::Release);
            thread::sleep(Duration::from_millis(50));
            return self.ensure_video_mode().is_ok();
        }

        false
    }

    /// Whether any wireless devices have been discovered.
    pub fn has_discovered_devices(&self) -> bool {
        !self.discovered_devices.lock().is_empty()
    }

    /// Number of discovered wireless devices.
    pub fn discovered_device_count(&self) -> usize {
        self.discovered_devices.lock().len()
    }

    /// Get a snapshot of all discovered devices.
    pub fn devices(&self) -> Vec<DiscoveredDevice> {
        self.discovered_devices.lock().clone()
    }

    /// Get a snapshot of a single device by its MAC address.
    pub fn device_by_mac(&self, mac: &[u8; 6]) -> Option<DiscoveredDevice> {
        self.discovered_devices
            .lock()
            .iter()
            .find(|d| &d.mac == mac)
            .cloned()
    }

    /// Set fan PWM values for a specific device identified by MAC address.
    ///
    /// Uses the device's own rx_type and channel from discovery, not a global
    /// value. This matches L-Connect 3's SyncPwm behavior.
    ///
    /// ## RF_Select packet layout (240 bytes):
    /// ```text
    /// [0]     = 18 (RF_Select)
    /// [1]     = 18 (RF_Select, repeated per L-Connect 3)
    /// [2-7]   = Device (slave) MAC address
    /// [8-13]  = Master MAC address
    /// [14]    = Target RX type (from device discovery)
    /// [15]    = Target channel (master channel)
    /// [16]    = Bind/group flag
    /// [17-20] = Fan PWM values (4 bytes, one per fan slot)
    /// [21-239]= Reserved
    /// ```
    pub fn set_fan_speeds_by_mac(&self, mac: &[u8; 6], fan_pwm: &[u8; 4]) -> Result<()> {
        let tx = self.tx.as_ref().context("TX device not connected")?;

        let device = self.discovered_devices
            .lock()
            .iter()
            .find(|d| &d.mac == mac)
            .cloned()
            .context(format!(
                "Device MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} not found in discovery",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
            ))?;

        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        // Apply minimum duty enforcement and CLV1 PWM filter
        let mut pwm = *fan_pwm;
        apply_pwm_constraints(&mut pwm, &device);

        // Build RF_Select packet (240 bytes)
        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;            // RF_Select command
        rf_data[1] = RF_SELECT;            // Repeated (L-Connect 3 convention)
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;      // Per-device RX type from discovery
        rf_data[15] = master_ch;           // Target channel = master channel
        rf_data[16] = 0;                   // Bind/group flag
        rf_data[17..21].copy_from_slice(&pwm);

        // Send as 4 USB packets (60-byte chunks)
        let handle = tx.lock();
        for chunk_idx in 0..RF_CHUNKS as u8 {
            let mut packet = vec![0u8; 64];
            packet[0] = USB_CMD_SEND_RF;
            packet[1] = chunk_idx;         // Sequence number
            packet[2] = device.channel;    // Device's current RF channel
            packet[3] = device.rx_type;    // Device's RX type

            let start = chunk_idx as usize * RF_CHUNK_SIZE;
            let end = start + RF_CHUNK_SIZE;
            packet[4..64].copy_from_slice(&rf_data[start..end]);

            handle
                .write_bulk(&packet, USB_TIMEOUT)
                .context("sending fan speed RF packet")?;
            thread::sleep(Duration::from_millis(1));
        }

        debug!(
            "Set fan PWM for {} (rx={}, ch={}): {:?}",
            device.mac_str(), device.rx_type, device.channel, pwm
        );
        Ok(())
    }

    /// Set fan PWM values by device list index (backward compat with old API).
    ///
    /// Index corresponds to the position in the discovery list (0-based).
    pub fn set_fan_speeds(&self, device_index: u8, fan_pwm: &[u8; 4]) -> Result<()> {
        let mac = {
            let devices = self.discovered_devices.lock();
            devices
                .iter()
                .find(|d| d.list_index == device_index)
                .map(|d| d.mac)
                .context(format!(
                    "No device at index {device_index} (discovered {} device(s))",
                    devices.len()
                ))?
        };

        self.set_fan_speeds_by_mac(&mac, fan_pwm)
    }

    /// Send per-LED RGB colors to a wireless device via RF.
    ///
    /// This is the core function for wireless LED control. All effects are
    /// host-rendered — the fan firmware just displays the received frames.
    ///
    /// ## Protocol (from L-Connect 3 MasterDevice.cs SetLightEffect):
    ///
    /// 1. Raw RGB data (`colors`) is compressed using tinyuz (4KB dict).
    /// 2. Compressed data is split into 220-byte chunks.
    /// 3. Header packet (index=0) carries metadata, sent 4× for reliability.
    /// 4. Data packets (index=1..N) carry 220 bytes of compressed data each.
    /// 5. Each 240-byte RF packet is sent as 4× 64-byte USB packets.
    pub fn send_rgb_direct(
        &self,
        mac: &[u8; 6],
        colors: &[[u8; 3]],
        effect_index: &[u8; 4],
    ) -> Result<()> {
        let tx = self.tx.as_ref().context("TX device not connected")?;

        let device = self
            .discovered_devices
            .lock()
            .iter()
            .find(|d| &d.mac == mac)
            .cloned()
            .context("device not found for RGB send")?;

        let master_mac = *self.master_mac.lock();

        // Flatten RGB triplets into a raw byte array
        let mut rgb_data = Vec::with_capacity(colors.len() * 3);
        for color in colors {
            rgb_data.extend_from_slice(color);
        }

        // Compress with tinyuz
        let compressed = crate::tinyuz::compress(&rgb_data)
            .context("failed to compress RGB data")?;

        const LZO_RF_VALID_LEN: usize = 220;
        let total_pk_num =
            (compressed.len() as f64 / LZO_RF_VALID_LEN as f64).ceil() as u8;

        let led_num = colors.len() as u8;
        let total_frame: u16 = 1; // Single frame for direct color mode

        let mut offset: usize = 0;
        let mut index: u8 = 0;

        while offset < compressed.len() || index == 0 {
            let mut rf_data = vec![0u8; RF_DATA_SIZE];

            // Common header for all packets
            rf_data[0] = RF_SELECT;
            rf_data[1] = RF_SET_RGB;
            rf_data[2..8].copy_from_slice(&device.mac);
            rf_data[8..14].copy_from_slice(&master_mac);
            rf_data[14..18].copy_from_slice(effect_index);
            rf_data[18] = index;
            rf_data[19] = total_pk_num + 1;

            if index == 0 {
                // Header packet: metadata only, no compressed data
                let data_len = compressed.len() as u32;
                rf_data[20] = (data_len >> 24) as u8;
                rf_data[21] = ((data_len >> 16) & 0xFF) as u8;
                rf_data[22] = ((data_len >> 8) & 0xFF) as u8;
                rf_data[23] = (data_len & 0xFF) as u8;
                rf_data[24] = 0; // Frame count high byte (always 0 for single frame)
                rf_data[25] = (total_frame >> 8) as u8;
                rf_data[26] = (total_frame & 0xFF) as u8;
                rf_data[27] = led_num;
                // Bytes 28-31: reserved/timing
                // Bytes 32-33: interval (0 for direct mode)
                // Bytes 34-39: sub-interval / flags

                // Send header packet 4 times for reliability (L-Connect 3 convention)
                let handle = tx.lock();
                for repeat in 0..4u8 {
                    self.send_rf_packet(&handle, &device, &rf_data)?;
                    if repeat < 3 {
                        thread::sleep(Duration::from_millis(20));
                    }
                }
                drop(handle);
            } else {
                // Data packet: 220 bytes of compressed data at offset 20
                let remaining = compressed.len() - offset;
                let chunk_len = remaining.min(LZO_RF_VALID_LEN);
                rf_data[20..20 + chunk_len]
                    .copy_from_slice(&compressed[offset..offset + chunk_len]);
                offset += LZO_RF_VALID_LEN;

                let handle = tx.lock();
                self.send_rf_packet(&handle, &device, &rf_data)?;
                drop(handle);
            }

            index += 1;
        }

        debug!(
            "Sent RGB frame to {} ({} LEDs, {} compressed bytes, {} packets)",
            device.mac_str(),
            led_num,
            compressed.len(),
            index
        );
        Ok(())
    }

    /// Send a 240-byte RF packet as 4× 64-byte USB chunks.
    fn send_rf_packet(
        &self,
        handle: &UsbTransport,
        device: &DiscoveredDevice,
        rf_data: &[u8],
    ) -> Result<()> {
        for chunk_idx in 0..RF_CHUNKS as u8 {
            let mut packet = vec![0u8; 64];
            packet[0] = USB_CMD_SEND_RF;
            packet[1] = chunk_idx;
            packet[2] = device.channel;
            packet[3] = device.rx_type;

            let start = chunk_idx as usize * RF_CHUNK_SIZE;
            let end = start + RF_CHUNK_SIZE;
            packet[4..64].copy_from_slice(&rf_data[start..end]);

            handle
                .write_bulk(&packet, USB_TIMEOUT)
                .context("sending RGB RF packet")?;
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        self.poll_stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.poll_thread.take() {
            let _ = handle.join();
        }
        self.tx.take();
        self.rx.take();
    }
}

impl Default for WirelessController {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WirelessController {
    fn drop(&mut self) {
        self.stop();
    }
}

// ── PWM constraints ──────────────────────────────────────────────────────────

/// Apply minimum duty enforcement and CLV1 PWM filter.
///
/// L-Connect 3 enforces per-fan-type minimums and has special PWM
/// remapping for CLV1 devices (values 153-155 → 152/156).
fn apply_pwm_constraints(pwm: &mut [u8; 4], device: &DiscoveredDevice) {
    let min_pwm = ((device.fan_type.min_duty_percent() as f32 / 100.0) * 255.0) as u8;

    for (i, val) in pwm.iter_mut().enumerate() {
        // Only apply to slots that have fans (based on fan_count)
        if i as u8 >= device.fan_count {
            *val = 0; // Unused slots must be 0
            continue;
        }

        // Enforce minimum PWM
        if *val > 0 && *val < min_pwm {
            *val = min_pwm;
        }

        // CLV1 special PWM filter (from L-Connect 3 NeedSyncPwm)
        if device.fan_type == WirelessFanType::Clv1 {
            match *val {
                153 | 154 => *val = 152,
                155 => *val = 156,
                _ => {}
            }
        }
    }
}

// ── Discovery polling ────────────────────────────────────────────────────────

/// Polls the RX device for the current device list.
///
/// Sends GetDev command (0x10, page=1) and parses the response into
/// full 42-byte device records.
fn poll_and_discover(
    rx: &Arc<Mutex<UsbTransport>>,
    discovered_devices: &Arc<Mutex<Vec<DiscoveredDevice>>>,
) -> Result<()> {
    // GetDev command: [0x10, page_number, ...pad...]
    let mut cmd = vec![0u8; 64];
    cmd[0] = USB_CMD_SEND_RF; // GetDev uses the same command byte
    cmd[1] = 0x01;            // Page 1

    let handle = rx.lock();
    handle
        .write_bulk(&cmd, USB_TIMEOUT)
        .context("sending GetDev command")?;

    // Response: [0]=0x10, [1]=device_count, [2-3]=ver/sync, [4+]=42-byte records
    let mut response = [0u8; 512];
    match handle.read_bulk(&mut response, Duration::from_millis(200)) {
        Ok(len) if len >= 4 => {
            if response[0] != USB_CMD_SEND_RF {
                debug!("GetDev: unexpected response 0x{:02x}", response[0]);
                return Ok(());
            }

            let device_count = response[1] as usize;
            debug!("GetDev: {device_count} device(s) reported");

            if device_count == 0 || device_count > 12 {
                return Ok(());
            }

            let mut found = Vec::new();
            let mut offset = 4; // After header [cmd, count, ver[2]]

            for idx in 0..device_count {
                if offset + 42 > len {
                    debug!("GetDev: response truncated at device {idx}");
                    break;
                }

                if let Some(device) = parse_device_record(&response[offset..offset + 42], idx as u8) {
                    debug!(
                        "  [{}] {} type=0x{:02x} fans={} RPM=[{},{},{},{}] PWM=[{},{},{},{}]",
                        idx, device, device.device_type,
                        device.fan_count,
                        device.fan_rpms[0], device.fan_rpms[1],
                        device.fan_rpms[2], device.fan_rpms[3],
                        device.current_pwm[0], device.current_pwm[1],
                        device.current_pwm[2], device.current_pwm[3],
                    );
                    found.push(device);
                }

                offset += 42;
            }

            // Update the shared device list
            let mut devices = discovered_devices.lock();
            if !found.is_empty() {
                let old_count = devices.len();
                *devices = found;
                if old_count != devices.len() {
                    info!(
                        "Discovered {} wireless device(s)",
                        devices.len()
                    );
                }
            }
        }
        Ok(_) => {}
        Err(lianli_transport::TransportError::Usb(rusb::Error::Timeout)) => {}
        Err(err) => {
            debug!("GetDev error: {err}");
        }
    }

    Ok(())
}

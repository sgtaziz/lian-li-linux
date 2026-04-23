use anyhow::{bail, Context, Result};
use lianli_transport::usb::{UsbTransport, USB_TIMEOUT};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::fmt;
use std::sync::{
    atomic::{AtomicBool, AtomicU16, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// TX dongle VID:PID pairs (V1 and V2 hardware).
const TX_IDS: [(u16, u16); 2] = [(0x0416, 0x8040), (0x1A86, 0xE304)];
/// RX dongle VID:PID pairs (V1 and V2 hardware).
const RX_IDS: [(u16, u16); 2] = [(0x0416, 0x8041), (0x1A86, 0xE305)];

const USB_CMD_SEND_RF: u8 = 0x10;
const USB_CMD_GET_MAC: u8 = 0x11;

const RF_SELECT: u8 = 0x12;
const RF_PWM_CMD: u8 = 0x10;
const RF_AIO_SWITCH_WIRELESS: u8 = 0x19;
const RF_SET_RGB: u8 = 0x20;
const RF_AIO_PARAMS: u8 = 0x21;
const RF_AIO_PIC: u8 = 0x22;

const RF_DATA_SIZE: usize = 240;
const RF_CHUNK_SIZE: usize = 60;
const RF_CHUNKS: usize = RF_DATA_SIZE / RF_CHUNK_SIZE;

/// Maximum size of a JPEG uploaded to an AIO's built-in display.
pub const AIO_PIC_MAX_BYTES: usize = 20_480;
/// Width/height of an AIO's built-in display in pixels.
pub const AIO_PIC_DIMENSION: u32 = 480;
/// Size of the aio_param state block sent over RF to wireless AIOs.
pub const AIO_PARAM_LEN: usize = 32;

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

/// Try to open a USB device matching any of the given VID:PID pairs.
fn open_any(ids: &[(u16, u16)]) -> Result<UsbTransport> {
    let mut last_err = None;
    for &(vid, pid) in ids {
        match UsbTransport::open(vid, pid) {
            Ok(transport) => return Ok(transport),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err
        .map(|e| anyhow::anyhow!(e))
        .unwrap_or_else(|| anyhow::anyhow!("no VID:PID pairs to try")))
}

/// Reopen and swap a dongle transport in place after the underlying USB
/// handle goes stale (suspend/resume, hub reset, unplug+replug).
fn reopen_transport(arc: &Arc<Mutex<UsbTransport>>, ids: &[(u16, u16)], name: &str) -> Result<()> {
    let mut new_transport = open_any(ids).context(format!("reopening {name} dongle"))?;
    new_transport.detach_and_configure(name)?;
    let mut guard = arc.lock();
    *guard = new_transport;
    Ok(())
}

/// Run a USB op on a dongle transport with one-shot reopen + retry on failure.
/// `op` must be safe to call twice (idempotent at the protocol level).
fn with_transport_recovery<F, R>(
    arc: &Arc<Mutex<UsbTransport>>,
    ids: &[(u16, u16)],
    name: &str,
    mut op: F,
) -> Result<R>
where
    F: FnMut(&UsbTransport) -> Result<R>,
{
    let first = {
        let handle = arc.lock();
        op(&handle)
    };
    match first {
        Ok(r) => Ok(r),
        Err(e) => {
            warn!("{name} transport op failed ({e}); attempting reopen");
            reopen_transport(arc, ids, name).context("reopen after stale handle")?;
            info!("{name} transport reopened, retrying");
            let handle = arc.lock();
            op(&handle)
        }
    }
}

/// Wireless fan device type, determines minimum duty and RPM curves.
///
/// Byte ranges for classifying fan type:
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
    /// HydroShift II LCD-C (Circle) wireless AIO (device_type 10).
    /// Pump RPM range 1600-2500, 0-4 fans, 24 LEDs on pump head.
    WaterBlock,
    /// HydroShift II LCD-S / H2S (Square) wireless AIO (device_type 11).
    /// Pump RPM range 1600-3200, 0-4 fans, 24 LEDs on pump head.
    WaterBlock2,
    /// Wireless LED strip (device_type 1-9) — RGB only, no fans
    Strimer(u8),
    /// Lancool 217 case RGB ring (device_type 65) — 96 LEDs, no fans
    Lc217,
    /// Universal Screen 8.8" LED ring (device_type 88) — 88 LEDs, no fans
    Led88,
    /// Lancool V150 case fan/RGB controller (device_type 66) — 88 LEDs, dual-zone front/rear
    V150,
    /// Unknown fan type
    Unknown,
}

impl WirelessFanType {
    /// Minimum duty percentage for this fan type.
    pub fn min_duty_percent(self) -> u8 {
        match self {
            Self::Slv3Led | Self::Slv3Lcd => 14,
            Self::Tlv2Lcd => 10,
            Self::Tlv2Led | Self::SlInf => 11,
            Self::Clv1 | Self::WaterBlock | Self::WaterBlock2 | Self::V150 => 10,
            Self::Strimer(_) | Self::Lc217 | Self::Led88 => 0,
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
            Self::WaterBlock => "HydroShift II LCD-C (Wireless)",
            Self::WaterBlock2 => "HydroShift II LCD-S (Wireless)",
            Self::Strimer(_) => "Strimer Plus Wireless",
            Self::Lc217 => "Lancool 217 Wireless",
            Self::Led88 => "Universal Screen 8.8\" Wireless",
            Self::V150 => "Lancool V150 Wireless",
            Self::Unknown => "Wireless Fan",
        }
    }

    /// Number of addressable LEDs per fan for this device type.
    pub fn leds_per_fan(self) -> u8 {
        match self {
            Self::Tlv2Lcd | Self::Tlv2Led => 26,
            Self::Slv3Led | Self::Slv3Lcd => 40,
            Self::SlInf => 44,
            Self::Clv1 | Self::WaterBlock | Self::WaterBlock2 => 24,
            Self::Strimer(_) | Self::Lc217 | Self::Led88 | Self::V150 => 0,
            Self::Unknown => 20,
        }
    }

    /// Whether the receiver firmware supports direct motherboard PWM sync.
    pub fn supports_hw_mobo_sync(self) -> bool {
        matches!(self, Self::Slv3Led | Self::Slv3Lcd)
    }

    /// Whether this is a wireless AIO device with a pump.
    pub fn is_aio(self) -> bool {
        matches!(self, Self::WaterBlock | Self::WaterBlock2)
    }

    /// Whether this is an RGB-only device with no fans or pump.
    pub fn is_rgb_only(self) -> bool {
        matches!(self, Self::Strimer(_) | Self::Lc217 | Self::Led88)
    }

    /// Number of LEDs on the pump head (AIO devices only).
    pub fn pump_led_count(self) -> u8 {
        if self.is_aio() {
            24
        } else {
            0
        }
    }

    /// Supported target pump RPM range (min, max) for this AIO variant, or None
    /// if the device has no pump.
    pub fn pump_rpm_range(self) -> Option<(u32, u32)> {
        match self {
            Self::WaterBlock => Some((1600, 2500)),
            Self::WaterBlock2 => Some((1600, 3200)),
            _ => None,
        }
    }

    /// Total LED count override for non-fan devices.
    /// Returns `Some(count)` for RGB-only devices, `None` for fan-based devices.
    pub fn total_led_count_override(self) -> Option<u16> {
        match self {
            Self::Strimer(dt) => Some(match dt {
                1 => 116,
                2 => 132,
                3 => 174,
                _ => 88,
            }),
            Self::Lc217 => Some(96),
            Self::Led88 => Some(88),
            Self::V150 => Some(88),
            _ => None,
        }
    }

    /// Classify fan type from the fan-type byte in the device record.
    fn from_fan_type_byte(b: u8) -> Self {
        match b {
            20..=23 => Self::Slv3Led,      // SLV3 LED (120/140, normal/reverse)
            24..=26 => Self::Slv3Lcd,      // SLV3 LCD (120/140, normal/reverse)
            27 | 32..=35 => Self::Tlv2Lcd, // TLV2 LCD
            28..=31 => Self::Tlv2Led,      // TLV2 LED (120/140, normal/reverse)
            36..=39 => Self::SlInf,        // SL-INF (LED only)
            40 => Self::Clv1,              // RL120
            41..=42 => Self::Clv1,         // CLV1 variants
            _ => Self::Unknown,
        }
    }
}

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
    /// Coolant temperature in °C (WaterBlock/WaterBlock2 only, from byte 27)
    pub coolant_temp_c: Option<u8>,
}

impl DiscoveredDevice {
    /// MAC address as a colon-separated hex string.
    pub fn mac_str(&self) -> String {
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.mac[0], self.mac[1], self.mac[2], self.mac[3], self.mac[4], self.mac[5],
        )
    }

    pub fn is_aio(&self) -> bool {
        self.fan_type.is_aio()
    }

    /// Pump RPM (from slot 3) for AIO devices.
    pub fn pump_rpm(&self) -> Option<u16> {
        if self.is_aio() {
            Some(self.fan_rpms[3])
        } else {
            None
        }
    }
}

impl fmt::Display for DiscoveredDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mac = self.mac_str();
        if self.fan_type.is_aio() {
            let temp_str = self
                .coolant_temp_c
                .map(|t| format!(", coolant={t}°C"))
                .unwrap_or_default();
            write!(
                f,
                "{} ({:?}, {} fans, pump={}rpm{temp_str}, ch={}, rx={})",
                mac, self.fan_type, self.fan_count, self.fan_rpms[3], self.channel, self.rx_type,
            )
        } else {
            write!(
                f,
                "{} ({:?}, {} fans, ch={}, rx={})",
                mac, self.fan_type, self.fan_count, self.channel, self.rx_type,
            )
        }
    }
}

/// Parse a 42-byte device record from GetDev response.
///
/// Record layout:
/// ```text
/// [0-5]   Device MAC (6 bytes)
/// [6-11]  Master MAC (6 bytes)
/// [12]    RF Channel
/// [13]    RX Type (radio endpoint)
/// [14-17] System time (ms * 0.625)
/// [18]    Device type (0=fan, 65=LC217, 255=master)
/// [19]    Fan count
/// [20-23] Effect index (4 bytes)
/// [24-26] Fan type bytes (3 bytes, per-slot)
/// [27]    Coolant temperature °C (WaterBlock/WaterBlock2 only)
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
    let fan_count = data[19].min(4);

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

    // Classify device by device_type first, then by fan_type bytes for fan groups
    let fan_type = match device_type {
        10 => WirelessFanType::WaterBlock,
        11 => WirelessFanType::WaterBlock2,
        1..=9 => WirelessFanType::Strimer(device_type),
        65 => WirelessFanType::Lc217,
        66 => WirelessFanType::V150,
        88 => WirelessFanType::Led88,
        _ => fan_types
            .iter()
            .find(|&&b| b != 0)
            .map(|&b| WirelessFanType::from_fan_type_byte(b))
            .unwrap_or(WirelessFanType::Unknown),
    };

    // Byte 27 contains coolant temperature for AIO devices
    let coolant_temp_c = if fan_type.is_aio() && data[27] > 0 {
        Some(data[27])
    } else {
        None
    };

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
        coolant_temp_c,
    })
}

pub struct WirelessController {
    tx: Option<Arc<Mutex<UsbTransport>>>,
    rx: Option<Arc<Mutex<UsbTransport>>>,
    poll_stop: Arc<AtomicBool>,
    poll_thread: Option<JoinHandle<()>>,
    video_mode_active: Arc<AtomicBool>,
    master_mac: Arc<Mutex<[u8; 6]>>,
    master_channel: Arc<Mutex<u8>>,
    discovered_devices: Arc<Mutex<Vec<DiscoveredDevice>>>,
    /// Motherboard PWM duty cycle (0-255) extracted from RX GetDev response bytes [2:3].
    /// 0xFFFF means unavailable/not yet read.
    mobo_pwm: Arc<AtomicU16>,
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
            mobo_pwm: Arc::clone(&self.mobo_pwm),
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
            mobo_pwm: Arc::new(AtomicU16::new(0xFFFF)),
        }
    }

    pub fn connect(&mut self) -> Result<()> {
        let mut tx = None;
        let max_retries = 3;

        for attempt in 1..=max_retries {
            match open_any(&TX_IDS) {
                Ok(device) => {
                    tx = Some(device);
                    break;
                }
                Err(e) if attempt < max_retries => {
                    debug!("TX device not found (attempt {attempt}/{max_retries}): {e}");
                    thread::sleep(Duration::from_millis(1000 * attempt as u64));
                }
                Err(e) => {
                    return Err(e).context("opening wireless TX dongle");
                }
            }
        }

        let mut tx = tx.context("TX device failed to open after retries")?;
        tx.detach_and_configure("TX")?;
        let tx_arc = Arc::new(Mutex::new(tx));

        let rx_arc = match open_any(&RX_IDS) {
            Ok(mut rx) => {
                rx.detach_and_configure("RX")?;
                rx.read_flush();
                Some(Arc::new(Mutex::new(rx)))
            }
            Err(_) => {
                warn!("RX dongle not found – telemetry disabled");
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
    /// Tries the default channel first, then scans.
    /// Channels should be even numbers.
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
            if handle.write(&cmd, USB_TIMEOUT).is_err() {
                drop(handle);
                continue;
            }

            let mut response = [0u8; 64];
            let len = match handle.read(&mut response, Duration::from_millis(500)) {
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
                .write(&CMD_RESET, USB_TIMEOUT)
                .context("sending TX reset")?;
        }

        // Brief settle after TX reset before polling
        thread::sleep(Duration::from_millis(500));

        self.video_mode_active.store(false, Ordering::Release);
        self.poll_stop.store(false, Ordering::SeqCst);

        let stop_flag = self.poll_stop.clone();
        let discovered_devices = Arc::clone(&self.discovered_devices);
        let mobo_pwm = Arc::clone(&self.mobo_pwm);
        let master_mac = Arc::clone(&self.master_mac);

        let discovery_done = Arc::new(AtomicBool::new(false));
        let discovery_signal = discovery_done.clone();

        self.poll_thread = Some(thread::spawn(move || {
            let mut found_devices = false;
            let mut consecutive_errors = 0u32;
            let mut consecutive_successes = 0u32;
            let mut total_resets = 0u32;
            const MAX_RESETS: u32 = 3;
            while !stop_flag.load(Ordering::SeqCst) {
                if let Err(err) =
                    poll_and_discover(&rx, &discovered_devices, &mobo_pwm, &master_mac)
                {
                    consecutive_errors += 1;
                    consecutive_successes = 0;
                    info!("RX polling ({consecutive_errors}): {err:?}, continuing");
                    if consecutive_errors >= 5 {
                        total_resets += 1;
                        if total_resets > MAX_RESETS {
                            error!(
                                "RX dongle unresponsive after {MAX_RESETS} resets, \
                                 stopping wireless polling"
                            );
                            break;
                        }
                        warn!(
                            "5 consecutive RX errors, sending RX reset ({total_resets}/{MAX_RESETS})"
                        );
                        let handle = rx.lock();
                        let mut reset_cmd = vec![0u8; 64];
                        reset_cmd[0] = 0x15; // USB_ResetAnother
                        if handle.write(&reset_cmd, USB_TIMEOUT).is_ok() {
                            let mut resp = [0u8; 64];
                            let _ = handle.read(&mut resp, Duration::from_millis(2000));
                        }
                        drop(handle);
                        thread::sleep(Duration::from_millis(500));
                        consecutive_errors = 0;
                        continue;
                    }
                    let backoff = if consecutive_successes == 0
                        && !discovery_signal.load(Ordering::Acquire)
                    {
                        Duration::from_millis(200)
                    } else {
                        Duration::from_secs((1 << consecutive_errors.min(5)).min(30))
                    };
                    thread::sleep(backoff);
                    continue;
                }
                consecutive_errors = 0;
                consecutive_successes += 1;
                total_resets = 0;
                if consecutive_successes >= 2 && !discovery_signal.load(Ordering::Acquire) {
                    discovery_signal.store(true, Ordering::Release);
                }
                if !found_devices && !discovered_devices.lock().is_empty() {
                    found_devices = true;
                }
                thread::sleep(Duration::from_millis(500));
            }
        }));

        // Wait for 2 consecutive successful GetDev responses (up to 5s)
        // so the device list is reliably populated before creating controllers.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if discovery_done.load(Ordering::Acquire) {
                info!("Wireless discovery stable, proceeding with device list");
                break;
            }
            if std::time::Instant::now() >= deadline {
                warn!("Wireless discovery timed out (5s) — will retry in background");
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }

    pub fn ensure_video_mode(&self) -> Result<()> {
        if self.video_mode_active.load(Ordering::Acquire) {
            return Ok(());
        }

        if let Some(tx) = &self.tx {
            let device_count = self.discovered_devices.lock().len().max(1);
            let master_ch = *self.master_channel.lock();
            with_transport_recovery(tx, &TX_IDS, "TX", |handle| {
                handle
                    .write(&CMD_VIDEO_START, USB_TIMEOUT)
                    .context("sending TX video start")?;
                thread::sleep(Duration::from_millis(2));
                for device_idx in 0..device_count {
                    let mut cmd = vec![0u8; 64];
                    cmd[0] = USB_CMD_SEND_RF;
                    cmd[1] = device_idx as u8;
                    cmd[2] = master_ch;
                    cmd[3] = 0xFF; // Prep marker
                    handle
                        .write(&cmd, USB_TIMEOUT)
                        .context("sending TX prep command")?;
                    thread::sleep(Duration::from_millis(1));
                }
                Ok(())
            })?;
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
                with_transport_recovery(rx, &RX_IDS, "RX", |handle| {
                    handle
                        .write(cmd, USB_TIMEOUT)
                        .context("sending RX command")?;
                    Ok(())
                })?;
                thread::sleep(Duration::from_millis(2));
                if capture {
                    let mut buf = [0u8; 64];
                    let handle = rx.lock();
                    if let Ok(len) = handle.read(&mut buf, USB_TIMEOUT) {
                        debug!("RX resp: {:02x?}", &buf[..len.min(8)]);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn soft_reset(&mut self) -> bool {
        if self.tx.is_none() {
            if let Ok(mut transport) = open_any(&TX_IDS) {
                if transport.detach_and_configure("TX").is_ok() {
                    self.tx = Some(Arc::new(Mutex::new(transport)));
                }
            }
        }

        if let Some(tx) = &self.tx {
            {
                let handle = tx.lock();
                if handle.write(&CMD_RESET, USB_TIMEOUT).is_err() {
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

    /// Get a snapshot of discovered devices bound to this PC's dongle.
    pub fn devices(&self) -> Vec<DiscoveredDevice> {
        let local_mac = *self.master_mac.lock();
        self.discovered_devices
            .lock()
            .iter()
            .filter(|d| d.master_mac == local_mac)
            .cloned()
            .collect()
    }

    /// Get a snapshot of discovered devices NOT bound to this dongle.
    pub fn unbound_devices(&self) -> Vec<DiscoveredDevice> {
        let local_mac = *self.master_mac.lock();
        self.discovered_devices
            .lock()
            .iter()
            .filter(|d| d.master_mac != local_mac && d.device_type != 255)
            .cloned()
            .collect()
    }

    /// Bind a wireless device to this dongle by sending an RF bind packet.
    ///
    /// The device firmware updates its stored master MAC and RX endpoint.
    /// A SaveConfig broadcast is sent afterwards to persist the binding.
    pub fn bind_device(&self, mac: &[u8; 6]) -> Result<()> {
        let tx = self.tx.as_ref().context("TX not connected")?;
        let device = self
            .discovered_devices
            .lock()
            .iter()
            .find(|d| &d.mac == mac)
            .cloned()
            .context("device not found in discovery")?;

        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();
        let new_rx = self.get_rx_unused();

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_PWM_CMD;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = new_rx;
        rf_data[15] = master_ch;
        rf_data[16] = new_rx;

        with_transport_recovery(tx, &TX_IDS, "TX", |handle| {
            for _ in 0..3 {
                self.send_rf_packet(handle, &device, &rf_data)?;
                thread::sleep(Duration::from_millis(50));
            }
            Ok(())
        })?;

        self.save_rf_config()?;

        info!(
            "Bind sent to {} ({}) rx={} ch={}",
            device.mac_str(),
            device.fan_type.display_name(),
            new_rx,
            master_ch
        );
        Ok(())
    }

    /// Find an unused RX endpoint (1-14) for a new device binding.
    fn get_rx_unused(&self) -> u8 {
        let devices = self.discovered_devices.lock();
        let local_mac = *self.master_mac.lock();
        for rx in 1..15u8 {
            let in_use = devices
                .iter()
                .any(|d| d.master_mac == local_mac && d.rx_type == rx);
            if !in_use {
                return rx;
            }
        }
        1
    }

    /// Broadcast SaveConfig command to persist device bindings to flash.
    fn save_rf_config(&self) -> Result<()> {
        let tx = self.tx.as_ref().context("TX not connected")?;
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = 0x15; // SaveConfig
        rf_data[2..8].copy_from_slice(&[0xFF; 6]);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = 0xFF;

        with_transport_recovery(tx, &TX_IDS, "TX", |handle| {
            for _ in 0..3 {
                for chunk_idx in 0..RF_CHUNKS as u8 {
                    let mut packet = vec![0u8; 64];
                    packet[0] = USB_CMD_SEND_RF;
                    packet[1] = chunk_idx;
                    packet[2] = master_ch;
                    packet[3] = 0xFF;
                    let start = chunk_idx as usize * RF_CHUNK_SIZE;
                    packet[4..64].copy_from_slice(&rf_data[start..start + RF_CHUNK_SIZE]);
                    handle
                        .write(&packet, USB_TIMEOUT)
                        .context("sending SaveConfig")?;
                    thread::sleep(Duration::from_millis(1));
                }
                thread::sleep(Duration::from_millis(200));
            }
            Ok(())
        })
    }

    /// Get a snapshot of a single device by its MAC address.
    pub fn device_by_mac(&self, mac: &[u8; 6]) -> Option<DiscoveredDevice> {
        self.discovered_devices
            .lock()
            .iter()
            .find(|d| &d.mac == mac)
            .cloned()
    }

    /// Get the current motherboard PWM duty cycle (0-255), or None if unavailable.
    ///
    /// Extracted from the RX GetDev response bytes [2:3] during polling.
    /// Returns None if the high bit of byte[2] is set (mobo PWM not available)
    /// or if no polling data has been received yet.
    pub fn motherboard_pwm(&self) -> Option<u8> {
        match self.mobo_pwm.load(Ordering::Relaxed) {
            0xFFFF => None,
            v => Some(v as u8),
        }
    }

    /// Set fan PWM values for a specific device identified by MAC address.
    ///
    /// Uses the device's own rx_type and channel from discovery, not a global
    /// value.
    ///
    /// ## RF PWM packet layout (240 bytes):
    /// ```text
    /// [0]     = 0x12 (RF_Select — envelope command)
    /// [1]     = 0x10 (RF_Bind — PWM sub-command)
    /// [2-7]   = Device (slave) MAC address
    /// [8-13]  = Master MAC address
    /// [14]    = Target RX type (from device discovery)
    /// [15]    = Target channel (master channel)
    /// [16]    = Sequence index (1 for one-shot commands)
    /// [17-20] = Fan PWM values (4 bytes, one per fan slot)
    /// [21-239]= Reserved
    /// ```
    pub fn set_fan_speeds_by_mac(&self, mac: &[u8; 6], fan_pwm: &[u8; 4]) -> Result<()> {
        let tx = self.tx.as_ref().context("TX device not connected")?;

        let devices = self.discovered_devices.lock();
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        let device = devices
            .iter()
            .find(|d| &d.mac == mac)
            .cloned()
            .context(format!(
                "Device MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} not found in discovery",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
            ))?;

        // Sequential 1-based index of bound devices
        let seq_index = devices
            .iter()
            .filter(|d| d.master_mac == master_mac && d.device_type != 0xFF)
            .position(|d| d.mac == *mac)
            .map(|i| (i + 1) as u8)
            .unwrap_or(1);

        drop(devices);

        let mut pwm = *fan_pwm;
        apply_pwm_constraints(&mut pwm, &device);

        let needs_send = pwm
            .iter()
            .zip(device.current_pwm.iter())
            .any(|(target, reported)| {
                target.abs_diff(*reported) > 5 || (*target <= 10 && *reported != *target)
            });
        if !needs_send {
            return Ok(());
        }

        // Build RF PWM packet (240 bytes)
        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_PWM_CMD;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;
        rf_data[15] = master_ch;
        rf_data[16] = seq_index;
        rf_data[17..21].copy_from_slice(&pwm);

        // Send as 4 USB packets (60-byte chunks)
        with_transport_recovery(tx, &TX_IDS, "TX", |handle| {
            for chunk_idx in 0..RF_CHUNKS as u8 {
                let mut packet = vec![0u8; 64];
                packet[0] = USB_CMD_SEND_RF;
                packet[1] = chunk_idx; // Sequence number
                packet[2] = device.channel; // Device's current RF channel
                packet[3] = device.rx_type; // Device's RX type

                let start = chunk_idx as usize * RF_CHUNK_SIZE;
                let end = start + RF_CHUNK_SIZE;
                packet[4..64].copy_from_slice(&rf_data[start..end]);

                handle
                    .write(&packet, USB_TIMEOUT)
                    .context("sending fan speed RF packet")?;
                thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        })?;

        debug!(
            "Set fan PWM for {} (rx={}, ch={}): {:?}",
            device.mac_str(),
            device.rx_type,
            device.channel,
            pwm
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

    /// Send a single frame of per-LED RGB colors to a wireless device.
    ///
    /// Wrapper around `send_rgb_frames` for single-frame (static/direct) use.
    pub fn send_rgb_direct(
        &self,
        mac: &[u8; 6],
        colors: &[[u8; 3]],
        effect_index: &[u8; 4],
        header_repeats: u8,
    ) -> Result<()> {
        let led_num = colors.len() as u8;
        let mut raw_rgb = Vec::with_capacity(colors.len() * 3);
        for color in colors {
            raw_rgb.extend_from_slice(color);
        }
        self.send_rgb_payload(
            mac,
            &raw_rgb,
            led_num,
            1,
            5000,
            effect_index,
            header_repeats,
        )
    }

    /// Send a multi-frame animation to a wireless device.
    ///
    /// Firmware stores the compressed blob and loops all frames at `interval_ms`.
    /// Used for batched OpenRGB streaming — collect N frames, send once, let
    /// firmware play them back smoothly with zero host involvement.
    pub fn send_rgb_frames(
        &self,
        mac: &[u8; 6],
        frames: &[Vec<[u8; 3]>],
        interval_ms: u16,
        effect_index: &[u8; 4],
        header_repeats: u8,
    ) -> Result<()> {
        if frames.is_empty() {
            return Ok(());
        }
        let led_num = frames[0].len() as u8;
        let total_frames = frames.len() as u16;

        let mut raw_rgb = Vec::with_capacity(frames.len() * led_num as usize * 3);
        for frame in frames {
            for color in frame {
                raw_rgb.extend_from_slice(color);
            }
        }

        self.send_rgb_payload(
            mac,
            &raw_rgb,
            led_num,
            total_frames,
            interval_ms,
            effect_index,
            header_repeats,
        )
    }

    /// Core RF RGB payload sender.
    ///
    /// Compresses raw RGB data, splits into 220-byte chunks, and sends via RF.
    /// Header packet (index=0) carries metadata and is repeated for reliability.
    /// Data packets (index=1..N) carry compressed data chunks.
    fn send_rgb_payload(
        &self,
        mac: &[u8; 6],
        raw_rgb: &[u8],
        led_num: u8,
        total_frames: u16,
        interval_ms: u16,
        effect_index: &[u8; 4],
        header_repeats: u8,
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

        let compressed = crate::tinyuz::compress(raw_rgb).context("failed to compress RGB data")?;

        const LZO_RF_VALID_LEN: usize = 220;
        let total_pk_num = (compressed.len() as f64 / LZO_RF_VALID_LEN as f64).ceil() as u8;

        let mut packets_sent: u8 = 0;
        // Hold TX lock for the entire transfer to prevent interleaving
        // with PWM or other TX operations.
        with_transport_recovery(tx, &TX_IDS, "TX", |handle| {
            let mut offset: usize = 0;
            let mut index: u8 = 0;
            while offset < compressed.len() || index == 0 {
                let mut rf_data = vec![0u8; RF_DATA_SIZE];

                rf_data[0] = RF_SELECT;
                rf_data[1] = RF_SET_RGB;
                rf_data[2..8].copy_from_slice(&device.mac);
                rf_data[8..14].copy_from_slice(&master_mac);
                rf_data[14..18].copy_from_slice(effect_index);
                rf_data[18] = index;
                rf_data[19] = total_pk_num + 1;

                if index == 0 {
                    // Header packet: metadata
                    let data_len = compressed.len() as u32;
                    rf_data[20] = (data_len >> 24) as u8;
                    rf_data[21] = ((data_len >> 16) & 0xFF) as u8;
                    rf_data[22] = ((data_len >> 8) & 0xFF) as u8;
                    rf_data[23] = (data_len & 0xFF) as u8;
                    rf_data[24] = 0;
                    rf_data[25] = (total_frames >> 8) as u8;
                    rf_data[26] = (total_frames & 0xFF) as u8;
                    rf_data[27] = led_num;
                    rf_data[32] = (interval_ms >> 8) as u8;
                    rf_data[33] = (interval_ms & 0xFF) as u8;

                    let repeats = header_repeats.max(1);
                    let gap_ms = if repeats <= 2 { 2 } else { 20 };
                    for repeat in 0..repeats {
                        self.send_rf_packet(handle, &device, &rf_data)?;
                        if repeat < repeats - 1 {
                            thread::sleep(Duration::from_millis(gap_ms));
                        }
                    }
                } else {
                    // Data packet: 220 bytes of compressed data
                    let remaining = compressed.len() - offset;
                    let chunk_len = remaining.min(LZO_RF_VALID_LEN);
                    rf_data[20..20 + chunk_len]
                        .copy_from_slice(&compressed[offset..offset + chunk_len]);
                    offset += LZO_RF_VALID_LEN;

                    self.send_rf_packet(handle, &device, &rf_data)?;
                }

                index += 1;
            }
            packets_sent = index;
            Ok(())
        })?;

        debug!(
            "Sent RGB to {} ({} frame(s), {} LEDs, {} compressed, {} packets, {}ms interval)",
            device.mac_str(),
            total_frames,
            led_num,
            compressed.len(),
            packets_sent,
            interval_ms
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
                .write(&packet, USB_TIMEOUT)
                .context("sending RGB RF packet")?;
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    /// Signal an AIO device to start honouring RF-driven theme / pump state.
    /// Must be sent once per AIO MAC after discovery, before the first `set_aio_params`.
    /// Idempotent — safe to re-invoke on reconnects.
    pub fn switch_to_wireless_theme(&self, mac: &[u8; 6]) -> Result<()> {
        let tx = self.tx.as_ref().context("TX device not connected")?;
        let device = self.device_by_mac_snapshot(mac)?;
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_AIO_SWITCH_WIRELESS;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;
        rf_data[15] = master_ch;

        with_transport_recovery(tx, &TX_IDS, "TX", |handle| {
            for _ in 0..10 {
                send_rf_frame_via(handle, &device, &rf_data)?;
                thread::sleep(Duration::from_millis(2));
            }
            Ok(())
        })?;

        debug!("switch_to_wireless_theme sent to {}", device.mac_str());
        Ok(())
    }

    /// Send the 32-byte aio_param block to a wireless AIO. Carries pump speed,
    /// on-screen sensor values + enables, text colors, LCD brightness, rotation,
    /// theme index, loop interval.
    pub fn set_aio_params(&self, mac: &[u8; 6], aio_param: &[u8; AIO_PARAM_LEN]) -> Result<()> {
        let tx = self.tx.as_ref().context("TX device not connected")?;
        let device = self.device_by_mac_snapshot(mac)?;
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();
        let seq_index = self.next_seq_index(&device);

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_AIO_PARAMS;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;
        rf_data[15] = master_ch;
        rf_data[16] = seq_index;
        rf_data[18..18 + AIO_PARAM_LEN].copy_from_slice(aio_param);

        with_transport_recovery(tx, &TX_IDS, "TX", |handle| {
            send_rf_frame_via(handle, &device, &rf_data)
        })?;

        debug!(
            "set_aio_params sent to {} (pump_timer={}, theme={})",
            device.mac_str(),
            u16::from_be_bytes([aio_param[28], aio_param[29]]),
            aio_param[27]
        );
        Ok(())
    }

    /// Upload a JPEG to the AIO's built-in display for custom theme mode.
    /// Must be ≤ `AIO_PIC_MAX_BYTES` bytes; should be 480×480 JPEG.
    pub fn send_aio_pic(&self, mac: &[u8; 6], jpeg: &[u8]) -> Result<()> {
        if jpeg.len() > AIO_PIC_MAX_BYTES {
            bail!(
                "AIO image {} bytes exceeds maximum {}",
                jpeg.len(),
                AIO_PIC_MAX_BYTES
            );
        }
        if jpeg.is_empty() {
            bail!("AIO image payload is empty");
        }

        let tx = self.tx.as_ref().context("TX device not connected")?;
        let device = self.device_by_mac_snapshot(mac)?;
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        const PIC_CHUNK: usize = 220;
        let total_len = jpeg.len() as u16;
        let total_chunks = jpeg.len().div_ceil(PIC_CHUNK);

        with_transport_recovery(tx, &TX_IDS, "TX", |handle| {
            for idx in 0..total_chunks {
                let start = idx * PIC_CHUNK;
                let end = (start + PIC_CHUNK).min(jpeg.len());
                let mut rf_data = vec![0u8; RF_DATA_SIZE];
                rf_data[0] = RF_SELECT;
                rf_data[1] = RF_AIO_PIC;
                rf_data[2..8].copy_from_slice(&device.mac);
                rf_data[8..14].copy_from_slice(&master_mac);
                rf_data[14] = device.rx_type;
                rf_data[15] = master_ch;
                rf_data[18] = idx as u8;
                rf_data[19..19 + (end - start)].copy_from_slice(&jpeg[start..end]);
                send_rf_frame_via(handle, &device, &rf_data)?;
                thread::sleep(Duration::from_millis(2));
            }

            let mut terminator = vec![0u8; RF_DATA_SIZE];
            terminator[0] = RF_SELECT;
            terminator[1] = RF_AIO_PIC;
            terminator[2..8].copy_from_slice(&device.mac);
            terminator[8..14].copy_from_slice(&master_mac);
            terminator[14] = device.rx_type;
            terminator[15] = master_ch;
            terminator[18] = 0xFF;
            terminator[19] = (total_len >> 8) as u8;
            terminator[20] = (total_len & 0xFF) as u8;
            send_rf_frame_via(handle, &device, &terminator)?;
            Ok(())
        })?;

        info!(
            "send_aio_pic sent to {}: {} bytes in {} chunks",
            device.mac_str(),
            total_len,
            total_chunks
        );
        Ok(())
    }

    fn next_seq_index(&self, device: &DiscoveredDevice) -> u8 {
        let devices = self.discovered_devices.lock();
        let master_mac = *self.master_mac.lock();
        devices
            .iter()
            .filter(|d| d.master_mac == master_mac && d.device_type != 0xFF)
            .position(|d| d.mac == device.mac)
            .map(|i| (i + 1) as u8)
            .unwrap_or(1)
    }

    fn device_by_mac_snapshot(&self, mac: &[u8; 6]) -> Result<DiscoveredDevice> {
        let devices = self.discovered_devices.lock();
        devices
            .iter()
            .find(|d| &d.mac == mac)
            .cloned()
            .with_context(|| {
                format!(
                    "Device MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} not found in discovery",
                    mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
                )
            })
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

fn send_rf_frame_via(
    handle: &UsbTransport,
    device: &DiscoveredDevice,
    rf_data: &[u8],
) -> Result<()> {
    assert_eq!(rf_data.len(), RF_DATA_SIZE);
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
            .write(&packet, USB_TIMEOUT)
            .context("sending RF packet chunk")?;
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

/// Map pump target RPM → firmware PWM timer value for the given AIO variant.
/// Returns `None` for non-AIO device types.
pub fn pump_rpm_to_timer(rpm: u32, variant: WirelessFanType) -> Option<u16> {
    match variant {
        WirelessFanType::WaterBlock => Some(circle_pump_timer(rpm)),
        WirelessFanType::WaterBlock2 => Some(square_pump_timer(rpm)),
        _ => None,
    }
}

fn circle_pump_timer(rpm: u32) -> u16 {
    let rpm = rpm.clamp(1600, 2500) as f32;
    let t = if rpm <= 1720.0 {
        1500.0 - (rpm - 1600.0) * 1.667
    } else if rpm <= 1870.0 {
        1300.0 - (rpm - 1720.0) * 2.0
    } else if rpm <= 2000.0 {
        1000.0 - (rpm - 1870.0) * 1.23
    } else if rpm <= 2300.0 {
        840.0 - (rpm - 2000.0) * 2.0
    } else if rpm <= 2400.0 {
        240.0 - (rpm - 2300.0) * 1.8
    } else {
        60.0 - (rpm - 2400.0) * 0.5
    };
    t.clamp(0.0, u16::MAX as f32) as u16
}

fn square_pump_timer(rpm: u32) -> u16 {
    let rpm = rpm.clamp(1600, 3200) as f32;
    let t = if rpm <= 1800.0 {
        1590.0 - (rpm - 1600.0) * 0.95
    } else if rpm <= 2000.0 {
        1400.0 - (rpm - 1800.0)
    } else if rpm <= 2200.0 {
        1200.0 - (rpm - 2000.0)
    } else if rpm <= 2400.0 {
        1000.0 - (rpm - 2200.0)
    } else if rpm <= 2600.0 {
        800.0 - (rpm - 2400.0)
    } else if rpm <= 2800.0 {
        580.0 - (rpm - 2600.0) * 1.11
    } else if rpm <= 3000.0 {
        330.0 - (rpm - 2800.0) * 1.2
    } else {
        90.0 - (rpm - 3000.0) * 0.45
    };
    t.clamp(0.0, u16::MAX as f32) as u16
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

/// Apply minimum duty enforcement and CLV1 PWM filter.
///
/// Enforces per-fan-type minimums and special PWM remapping
/// for CLV1 devices (values 153-155 to 152/156).
fn apply_pwm_constraints(pwm: &mut [u8; 4], device: &DiscoveredDevice) {
    let min_pwm = ((device.fan_type.min_duty_percent() as f32 / 100.0) * 255.0) as u8;

    for (i, val) in pwm.iter_mut().enumerate() {
        // Only apply to slots that have fans (based on fan_count).
        // For AIO devices, slot 3 is the pump — don't zero it.
        let is_pump_slot = i == 3 && device.fan_type.is_aio();
        if i as u8 >= device.fan_count && !is_pump_slot {
            *val = 0;
            continue;
        }

        // Enforce minimum PWM
        if *val > 0 && *val < min_pwm {
            *val = min_pwm;
        }

        // CLV1 special PWM filter
        if device.fan_type == WirelessFanType::Clv1 {
            match *val {
                153 | 154 => *val = 152,
                155 => *val = 156,
                _ => {}
            }
        }
    }
}

/// Polls the RX device for the current device list.
///
/// Sends GetDev command (0x10, page=1) and parses the response into
/// full 42-byte device records.
fn poll_and_discover(
    rx: &Arc<Mutex<UsbTransport>>,
    discovered_devices: &Arc<Mutex<Vec<DiscoveredDevice>>>,
    mobo_pwm: &Arc<AtomicU16>,
    master_mac: &Arc<Mutex<[u8; 6]>>,
) -> Result<()> {
    // GetDev command: [0x10, page_number, ...pad...]
    let mut cmd = vec![0u8; 64];
    cmd[0] = USB_CMD_SEND_RF;
    cmd[1] = 0x01; // Page 1

    with_transport_recovery(rx, &RX_IDS, "RX", |handle| {
        handle.read_flush();
        handle
            .write(&cmd, USB_TIMEOUT)
            .context("sending GetDev command")?;
        Ok(())
    })?;
    let handle = rx.lock();

    // Response: [0]=0x10, [1]=device_count, [2-3]=mobo_pwm or version, [4+]=42-byte records
    let mut response = [0u8; 512];
    match handle.read(&mut response, Duration::from_millis(200)) {
        Ok(len) if len >= 4 => {
            if response[0] != USB_CMD_SEND_RF {
                info!(
                    "GetDev: unexpected response 0x{:02x}, will retry",
                    response[0]
                );
                bail!("GetDev: unexpected response 0x{:02x}", response[0]);
            }

            let device_count = response[1] as usize;

            // Extract motherboard PWM from response bytes [2:3].
            // Byte [2] high bit = unavailable flag. When clear:
            //   off_time = byte[2] & 0x7F, on_time = byte[3]
            //   pwm = 255 * on_time / (on_time + off_time)
            let indicator = response[2];
            if indicator >> 7 == 1 {
                // High bit set — mobo PWM unavailable (bytes are firmware version instead)
                mobo_pwm.store(0xFFFF, Ordering::Relaxed);
            } else {
                let off_time = (indicator & 0x7F) as u16;
                let on_time = response[3] as u16;
                let denominator = off_time + on_time;
                if denominator > 0 {
                    let pwm = (255u16 * on_time / denominator).min(255);
                    mobo_pwm.store(pwm, Ordering::Relaxed);
                } else {
                    mobo_pwm.store(0xFFFF, Ordering::Relaxed);
                }
            }

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

                if let Some(device) = parse_device_record(&response[offset..offset + 42], idx as u8)
                {
                    debug!(
                        "  [{}] {} type=0x{:02x} fans={} RPM=[{},{},{},{}] PWM=[{},{},{},{}]",
                        idx,
                        device,
                        device.device_type,
                        device.fan_count,
                        device.fan_rpms[0],
                        device.fan_rpms[1],
                        device.fan_rpms[2],
                        device.fan_rpms[3],
                        device.current_pwm[0],
                        device.current_pwm[1],
                        device.current_pwm[2],
                        device.current_pwm[3],
                    );
                    found.push(device);
                }

                offset += 42;
            }

            let mut devices = discovered_devices.lock();
            if !found.is_empty() {
                let old_count = devices.len();
                *devices = found;
                if old_count != devices.len() {
                    let local_mac = *master_mac.lock();
                    let bound = devices.iter().filter(|d| d.master_mac == local_mac).count();
                    let unbound = devices.len() - bound;
                    info!(
                        "Discovered {} wireless device(s) ({bound} bound, {unbound} unbound)",
                        devices.len()
                    );
                    for d in devices.iter().filter(|d| d.master_mac != local_mac) {
                        info!(
                            "  {} ({}) not bound to this dongle",
                            d.mac_str(),
                            d.fan_type.display_name()
                        );
                    }
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

#[cfg(test)]
mod aio_tests {
    use super::*;

    #[test]
    fn circle_curve_clamps_to_range() {
        assert_eq!(circle_pump_timer(1000), circle_pump_timer(1600));
        assert_eq!(circle_pump_timer(5000), circle_pump_timer(2500));
    }

    #[test]
    fn circle_curve_spans_each_segment() {
        assert_eq!(circle_pump_timer(1600), 1500);
        assert_eq!(circle_pump_timer(1700), 1333);
        assert_eq!(circle_pump_timer(1800), 1140);
        assert_eq!(circle_pump_timer(1900), 963);
        assert_eq!(circle_pump_timer(2100), 640);
        assert_eq!(circle_pump_timer(2350), 150);
        assert_eq!(circle_pump_timer(2450), 35);
        assert_eq!(circle_pump_timer(2500), 10);
    }

    #[test]
    fn square_curve_clamps_to_range() {
        assert_eq!(square_pump_timer(100), square_pump_timer(1600));
        assert_eq!(square_pump_timer(9999), square_pump_timer(3200));
    }

    #[test]
    fn square_curve_spans_each_segment() {
        assert_eq!(square_pump_timer(1600), 1590);
        assert_eq!(square_pump_timer(1700), 1495);
        assert_eq!(square_pump_timer(1900), 1300);
        assert_eq!(square_pump_timer(2100), 1100);
        assert_eq!(square_pump_timer(2300), 900);
        assert_eq!(square_pump_timer(2500), 700);
        assert_eq!(square_pump_timer(2700), 469);
        assert_eq!(square_pump_timer(2900), 210);
        assert_eq!(square_pump_timer(3100), 45);
        assert_eq!(square_pump_timer(3200), 0);
    }

    #[test]
    fn pump_rpm_to_timer_dispatches_by_variant() {
        assert_eq!(
            pump_rpm_to_timer(2000, WirelessFanType::WaterBlock),
            Some(circle_pump_timer(2000))
        );
        assert_eq!(
            pump_rpm_to_timer(2000, WirelessFanType::WaterBlock2),
            Some(square_pump_timer(2000))
        );
        assert_eq!(pump_rpm_to_timer(2000, WirelessFanType::Slv3Led), None);
    }

    #[test]
    fn pump_rpm_range_per_variant() {
        assert_eq!(WirelessFanType::WaterBlock.pump_rpm_range(), Some((1600, 2500)));
        assert_eq!(WirelessFanType::WaterBlock2.pump_rpm_range(), Some((1600, 3200)));
        assert_eq!(WirelessFanType::Slv3Led.pump_rpm_range(), None);
    }
}

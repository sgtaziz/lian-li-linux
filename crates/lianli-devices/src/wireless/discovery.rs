use super::transport::with_transport_recovery;
use super::transport::SharedTransport;
use super::{WirelessFanType, RX_IDS, USB_CMD_SEND_RF};
use anyhow::{Context, Result};
use lianli_transport::usb::USB_TIMEOUT;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// A wireless device discovered via the RX GetDev command.
/// Parsed from the 42-byte device record in the response.
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub mac: [u8; 6],
    pub master_mac: [u8; 6],
    pub channel: u8,
    pub rx_type: u8,
    pub device_type: u8,
    pub fan_count: u8,
    /// True when the bind group chains right-to-left (SL-INF daisy-chain
    /// with `fan_num >= 10` reported). Per-fan PWM/RGB ordering must be
    /// reversed before sending.
    pub is_inf_right_attach: bool,
    pub fan_types: [u8; 4],
    pub fan_rpms: [u16; 4],
    pub current_pwm: [u8; 4],
    pub cmd_seq: u8,
    pub fan_type: WirelessFanType,
    pub list_index: u8,
    /// Coolant temperature in °C (WaterBlock/WaterBlock2 only, from byte 27)
    pub coolant_temp_c: Option<u8>,
    /// Effect index the device firmware is currently running. Drifts to
    /// device-default if the firmware resets idle; compare against the desired
    /// effect_index to detect that and re-send the RGB packet.
    pub effect_index: [u8; 4],
    /// Status flags decoded from `fans_speed[0]` bitfield.
    pub is_sync_mb_light: bool,
    pub is_pwm_line_on: bool,
    /// Host-side binding intent; true while we consider this device ours,
    /// independent of the dongle-reported master MAC.
    pub bind_intent: bool,
}

/// The record fields that define a device's layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceIdentity {
    device_type: u8,
    fan_count: u8,
    right_attach: bool,
    slots: [u8; 4],
}

impl DiscoveredDevice {
    fn identity(&self) -> DeviceIdentity {
        let mut slots = self.fan_types;
        // An AIO reports coolant temperature in the last slot.
        if self.fan_type.is_aio() {
            slots[3] = 0;
        }
        DeviceIdentity {
            device_type: self.device_type,
            fan_count: self.fan_count,
            right_attach: self.is_inf_right_attach,
            slots,
        }
    }

    pub fn lcd_group_size(&self) -> Option<u8> {
        if self.device_type != 0 || !(1..=4).contains(&self.fan_count) {
            return None;
        }
        let family = match self.fan_type {
            WirelessFanType::Slv3Led | WirelessFanType::Slv3Lcd => 20..=26,
            WirelessFanType::Tlv2Led | WirelessFanType::Tlv2Lcd => 27..=35,
            WirelessFanType::SlInfV3 { .. } => 43..=50,
            WirelessFanType::TlV3 { .. } => 51..=58,
            _ => return None,
        };
        let mut count = 0;
        for &kind in &self.fan_types[..usize::from(self.fan_count)] {
            if !family.contains(&kind) {
                return None;
            }
            match WirelessFanType::from_fan_type_byte(kind) {
                WirelessFanType::Unknown => return None,
                WirelessFanType::Slv3Lcd
                | WirelessFanType::Tlv2Lcd
                | WirelessFanType::SlInfV3 { lcd: true }
                | WirelessFanType::TlV3 { lcd: true } => count += 1,
                _ => {}
            }
        }
        Some(count)
    }

    pub fn mac_str(&self) -> String {
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.mac[0], self.mac[1], self.mac[2], self.mac[3], self.mac[4], self.mac[5],
        )
    }

    pub fn is_aio(&self) -> bool {
        self.fan_type.is_aio()
    }

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
pub(super) fn parse_device_record(data: &[u8], list_index: u8) -> Option<DiscoveredDevice> {
    if data.len() < 42 {
        return None;
    }

    if data[41] != 0x1C {
        debug!(
            "  Device record {list_index}: invalid marker 0x{:02x} (expected 0x1C)",
            data[41]
        );
        return None;
    }

    let device_type = data[18];

    if device_type == 0xFF {
        debug!("  Device record {list_index}: skipping master device");
        return None;
    }

    let mut mac = [0u8; 6];
    mac.copy_from_slice(&data[0..6]);

    let channel = data[12];
    let rx_type = data[13];

    if mac == [0u8; 6] {
        debug!(
            "  Device record {list_index}: invalid mac ({:02x?}, ch={channel}, rx={rx_type})",
            mac
        );
        return None;
    }

    let mut master_mac = [0u8; 6];
    master_mac.copy_from_slice(&data[6..12]);
    // fan_num >= 10 flags SL-INF right-attach (chains right-to-left).
    let raw_fan_count = data[19];
    let (fan_count, is_inf_right_attach) = if raw_fan_count >= 10 {
        (raw_fan_count.saturating_sub(10).min(4), true)
    } else {
        (raw_fan_count.min(4), false)
    };

    let mut fan_types = [0u8; 4];
    fan_types.copy_from_slice(&data[24..28]);

    let status_byte = data[28];
    let is_sync_mb_light = (status_byte & 0x40) != 0;
    let is_pwm_line_on = (status_byte & 0x20) != 0;

    let fan_rpms = [
        u16::from_be_bytes([data[28] & 0x0F, data[29]]),
        u16::from_be_bytes([data[30] & 0x0F, data[31]]),
        u16::from_be_bytes([data[32] & 0x0F, data[33]]),
        u16::from_be_bytes([data[34] & 0x0F, data[35]]),
    ];

    let mut current_pwm = [0u8; 4];
    current_pwm.copy_from_slice(&data[36..40]);

    let cmd_seq = data[40];

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

    let coolant_temp_c = if fan_type.is_aio() && data[27] > 0 {
        Some(data[27])
    } else {
        None
    };

    let mut effect_index = [0u8; 4];
    effect_index.copy_from_slice(&data[20..24]);

    Some(DiscoveredDevice {
        mac,
        master_mac,
        channel,
        rx_type,
        device_type,
        fan_count,
        is_inf_right_attach,
        fan_types,
        fan_rpms,
        current_pwm,
        cmd_seq,
        fan_type,
        list_index,
        coolant_temp_c,
        effect_index,
        is_sync_mb_light,
        is_pwm_line_on,
        bind_intent: false,
    })
}

pub(super) const LIVENESS_TIMEOUT: Duration = Duration::from_secs(15);
const DEBOUNCE_SIGHTINGS: u32 = 3;
const IDENTITY_WARN_INTERVAL: Duration = Duration::from_secs(60);

/// Exclusive upper bound for RX slots. `get_rx_unused()` allocates from 1..14,
/// so anything at or above this was never a slot this daemon handed out.
pub(super) const RX_SLOT_LIMIT: u8 = 14;

pub(super) fn is_valid_rx(rx: u8) -> bool {
    rx != 0 && rx < RX_SLOT_LIMIT
}

/// The vendor client offers odd channels up to 39 and auto-assigns even ones
/// from 8 upward; reported values outside that range are corrupted state.
pub(super) fn is_valid_channel(channel: u8) -> bool {
    (1..=39).contains(&channel)
}

/// Normal traffic keeps a device's last valid channel; without one it uses ours.
pub(super) fn traffic_channel(published: u8, master: u8) -> u8 {
    if is_valid_channel(published) {
        published
    } else {
        master
    }
}

/// Devices that latched sensor bytes from an old clock broadcast keep
/// reporting them as their address (channel = CPU usage, rx = CPU temperature)
/// until rebound. Such records still prove the device is alive.
pub(super) fn is_valid_address(channel: u8, rx: u8) -> bool {
    is_valid_channel(channel) && is_valid_rx(rx)
}

/// Receivers can store a bind record where one byte of our master MAC repeats
/// through the rest of the master, the channel and rx. Requiring the exact
/// smear keeps another controller's devices from ever matching.
pub(super) fn is_smeared_local_master(
    reported: &[u8; 6],
    local: &[u8; 6],
    channel: u8,
    rx: u8,
) -> bool {
    if reported == local || *local == [0; 6] {
        return false;
    }
    (2..=4).any(|k| {
        let byte = local[k];
        reported[..=k] == local[..=k]
            && reported[k + 1..].iter().all(|&b| b == byte)
            && channel == byte
            && rx == byte
    })
}

pub(super) const ACK_FRESHNESS: Duration = Duration::from_secs(3);
pub(super) const REBIND_FOREIGN_AFTER: Duration = Duration::from_secs(10);
/// Devices that swap slots briefly look like duplicates until both commit.
pub(super) const REBIND_DUPLICATE_AFTER: Duration = Duration::from_secs(5);

/// A master dongle heard on the current channel, reported by GetDev as a
/// device record with device_type 0xFF. Foreign masters stay online only
/// while freshly sighted, which guards bind and unbind against touching
/// devices another live controller owns.
pub(super) struct MasterEntry {
    pub channel: u8,
    pub last_seen: Instant,
}

pub(super) type MasterEntryMap = Arc<Mutex<BTreeMap<[u8; 6], MasterEntry>>>;

pub(super) fn parse_master_record(data: &[u8]) -> Option<([u8; 6], u8)> {
    if data.len() < 42 || data[41] != 0x1C || data[18] != 0xFF {
        return None;
    }
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&data[0..6]);
    if mac == [0u8; 6] {
        return None;
    }
    Some((mac, data[12]))
}

pub(super) fn merge_master_sightings(
    found: &[([u8; 6], u8)],
    local: &[u8; 6],
    masters: &MasterEntryMap,
) {
    let now = Instant::now();
    let mut masters = masters.lock();
    for (mac, channel) in found {
        // The dongle reports itself as a master record with our own MAC.
        // Keeping it would log our own address as a foreign master and
        // disagree with the channel the discovery probe picked.
        if mac == local {
            continue;
        }
        if !masters.contains_key(mac) {
            info!(
                "foreign master dongle {:02x?} online on channel {channel}",
                mac
            );
        }
        masters.insert(
            *mac,
            MasterEntry {
                channel: *channel,
                last_seen: now,
            },
        );
    }
    masters.retain(|_, m| now.duration_since(m.last_seen) <= LIVENESS_TIMEOUT);
}

#[derive(Default)]
pub(super) struct ChannelCorrection {
    target: u8,
    attempts: u8,
    next_attempt: Option<Instant>,
    exhausted_reported: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ChannelCorrectionAction {
    Wait,
    Send,
    Exhausted,
}

pub(super) struct DeviceHealth {
    pub channel_correction: ChannelCorrection,
    pub published: DiscoveredDevice,
    pub last_seen: Instant,
    pub bind_intent: bool,
    /// Sticky flag set by an explicit unbind. Suppresses auto claiming and
    /// rebinds until the user binds the device again.
    pub man_unbind: bool,
    pub dead: bool,
    pub raw_master: [u8; 6],
    pub raw_rx: u8,
    pub raw_channel: u8,
    pub raw_seen: Instant,
    pub foreign_since: Option<Instant>,
    pub observed_master: [u8; 6],
    pub confirmed_invalid_address: Option<(u8, u8)>,
    pub duplicate_since: Option<Instant>,
    rx_published_at: Instant,
    master_cand: Option<([u8; 6], u32)>,
    addr_cand: Option<((u8, u8), u32)>,
    identity_cand: Option<(DeviceIdentity, u32)>,
    identity_warned_at: Option<Instant>,
}

impl DeviceHealth {
    pub(super) fn channel_correction_action(
        &mut self,
        master: [u8; 6],
        channel: u8,
        now: Instant,
    ) -> ChannelCorrectionAction {
        if !self.bind_intent
            || self.man_unbind
            || self.dead
            || self.raw_master != master
            || master == [0; 6]
            || self.raw_rx == 0
            || now.saturating_duration_since(self.raw_seen) > ACK_FRESHNESS
            || self.published.master_mac != self.raw_master
            || self.published.channel != self.raw_channel
            || self.published.rx_type != self.raw_rx
        {
            return ChannelCorrectionAction::Wait;
        }
        if self.raw_channel == channel {
            self.channel_correction = ChannelCorrection::default();
            return ChannelCorrectionAction::Wait;
        }
        let correction = &mut self.channel_correction;
        if correction.target != channel {
            *correction = ChannelCorrection {
                target: channel,
                ..Default::default()
            };
        }
        if correction.next_attempt.is_some_and(|next| now < next) {
            return ChannelCorrectionAction::Wait;
        }
        if correction.attempts >= 5 {
            if correction.exhausted_reported {
                return ChannelCorrectionAction::Wait;
            }
            correction.exhausted_reported = true;
            return ChannelCorrectionAction::Exhausted;
        }
        correction.next_attempt = Some(now + Duration::from_secs(2 << correction.attempts));
        correction.attempts += 1;
        ChannelCorrectionAction::Send
    }

    pub(super) fn new(mut rec: DiscoveredDevice) -> Self {
        if !is_valid_channel(rec.channel) {
            rec.channel = 0;
        }
        if !is_valid_address(rec.channel, rec.rx_type) {
            rec.rx_type = 0;
        }
        Self {
            channel_correction: ChannelCorrection::default(),
            observed_master: rec.master_mac,
            confirmed_invalid_address: None,
            duplicate_since: None,
            rx_published_at: Instant::now(),
            published: rec,
            last_seen: Instant::now(),
            bind_intent: false,
            man_unbind: false,
            dead: false,
            raw_master: [0u8; 6],
            raw_rx: 0,
            raw_channel: 0,
            raw_seen: Instant::now(),
            foreign_since: None,
            master_cand: None,
            addr_cand: None,
            identity_cand: None,
            identity_warned_at: None,
        }
    }

    /// A single record can carry bad type bytes between good ones. Unknown never
    /// replaces a known layout, so RGB and fan control keep their last valid
    /// target while the device stays alive.
    fn merge_identity(&mut self, rec: &DiscoveredDevice, now: Instant) {
        let reported = rec.identity();
        let published = self.published.identity();
        if reported == published {
            self.identity_cand = None;
            self.published.fan_types = rec.fan_types;
            return;
        }
        if self
            .identity_warned_at
            .is_none_or(|at| now.saturating_duration_since(at) >= IDENTITY_WARN_INTERVAL)
        {
            self.identity_warned_at = Some(now);
            warn!(
                "{} reported {:?} {reported:?} while published as {:?} {published:?}",
                rec.mac_str(),
                rec.fan_type,
                self.published.fan_type
            );
        }
        let committed = commit_streak(&mut self.identity_cand, reported).is_some();
        let keeps_layout =
            rec.fan_type != WirelessFanType::Unknown || self.published.fan_type == rec.fan_type;
        if committed && keeps_layout {
            info!(
                "{} changed from {:?} {published:?} to {:?} {reported:?}",
                rec.mac_str(),
                self.published.fan_type,
                rec.fan_type
            );
            let p = &mut self.published;
            p.device_type = rec.device_type;
            p.fan_count = rec.fan_count;
            p.is_inf_right_attach = rec.is_inf_right_attach;
            p.fan_types = rec.fan_types;
            p.fan_type = rec.fan_type;
        }
    }

    pub(super) fn publish_rx(&mut self, rx: u8) {
        if self.published.rx_type != rx {
            self.published.rx_type = rx;
            self.rx_published_at = Instant::now();
        }
    }
}

pub(super) type DeviceHealthMap = Arc<Mutex<BTreeMap<[u8; 6], DeviceHealth>>>;

fn commit_streak<T: Copy + Eq>(cand: &mut Option<(T, u32)>, observed: T) -> Option<T> {
    match cand {
        Some((v, n)) if *v == observed => {
            *n += 1;
            if *n >= DEBOUNCE_SIGHTINGS {
                *cand = None;
                Some(observed)
            } else {
                None
            }
        }
        _ => {
            *cand = Some((observed, 1));
            None
        }
    }
}

pub(super) struct ReceiverState {
    pub pwm: AtomicU16,
    pub pages: AtomicU8,
    pub fg_sync: AtomicBool,
}

impl Default for ReceiverState {
    fn default() -> Self {
        Self {
            pwm: AtomicU16::new(0xFFFF),
            pages: AtomicU8::new(1),
            fg_sync: AtomicBool::new(false),
        }
    }
}

/// Polls the RX device for the current device list.
///
/// Sends GetDev command (0x10) and parses the response into
/// full 42-byte device records. Results are merged into the persistent
/// health map; the published device list is rebuilt from it.
pub(super) fn poll_and_discover(
    rx: &SharedTransport,
    discovered_devices: &Arc<Mutex<Vec<DiscoveredDevice>>>,
    health_map: &DeviceHealthMap,
    master_entries: &MasterEntryMap,
    receiver: &ReceiverState,
    stop: &AtomicBool,
    master_mac: &Arc<Mutex<[u8; 6]>>,
) -> Result<()> {
    sweep(health_map, discovered_devices, master_mac);

    let pages = receiver.pages.load(Ordering::Relaxed).clamp(1, 26);

    let mut cmd = vec![0u8; 64];
    cmd[0] = USB_CMD_SEND_RF;
    cmd[1] = pages;

    if receiver.fg_sync.load(Ordering::Relaxed) {
        let rpm = discovered_devices
            .lock()
            .iter()
            .flat_map(|d| d.fan_rpms.iter())
            .copied()
            .find(|&r| r > 0)
            .unwrap_or(0);
        cmd[2] = (rpm >> 8) as u8;
        cmd[3] = (rpm & 0xFF) as u8;
    }

    let mut response = [0u8; 26 * 512];
    let len = with_transport_recovery(rx, &RX_IDS, "RX", stop, |handle| {
        handle.read_flush();
        handle
            .write(&cmd, USB_TIMEOUT)
            .context("sending GetDev command")?;
        let len = handle.read_silence_checked(
            &mut response[..usize::from(pages) * 512],
            Duration::from_millis(100),
            Duration::from_millis(10),
        )?;
        Ok(len)
    });
    let len = match len.and_then(|len| {
        validate_discovery_response(&response[..len], pages).inspect_err(|error| {
            debug!(pages, len, header = ?&response[..len.min(16)], %error, "Rejected wireless discovery reply");
        })?;
        Ok(len)
    }) {
        Ok(len) => len,
        Err(error) => {
            receiver.pwm.store(0xFFFF, Ordering::Relaxed);
            return Err(error);
        }
    };
    receiver
        .pages
        .store(discovery_page_count(response[1]), Ordering::Relaxed);

    {
        let device_count = usize::from(response[1]).min(usize::from(pages) * 10);

        let indicator = response[2];
        if indicator >> 7 == 1 {
            receiver.pwm.store(0xFFFF, Ordering::Relaxed);
        } else {
            let off_time = (indicator & 0x7F) as u16;
            let on_time = response[3] as u16;
            let denominator = off_time + on_time;
            if let Some(duty) = (255u16 * on_time).checked_div(denominator) {
                let pwm = duty.min(255);
                receiver.pwm.store(pwm, Ordering::Relaxed);
            } else {
                receiver.pwm.store(0xFFFF, Ordering::Relaxed);
            }
        }

        debug!("GetDev: {device_count} device(s) reported");

        if device_count > 0 {
            let mut found = Vec::new();
            let mut found_masters = Vec::new();
            let mut offset = 4;

            for idx in 0..device_count {
                if offset + 42 > len {
                    debug!("GetDev: response truncated at device {idx}");
                    break;
                }

                if let Some((mac, channel)) = parse_master_record(&response[offset..offset + 42]) {
                    debug!("  [{}] master dongle {:02x?} ch={channel}", idx, mac);
                    found_masters.push((mac, channel));
                } else if let Some(device) =
                    parse_device_record(&response[offset..offset + 42], idx as u8)
                {
                    debug!(
                        "  [{}] {} type=0x{:02x} fans={} slots={:?} RPM=[{},{},{},{}] PWM=[{},{},{},{}]",
                        idx,
                        device,
                        device.device_type,
                        device.fan_count,
                        device.fan_types,
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

            let master_changes =
                merge_sightings(&found, health_map, discovered_devices, master_mac);
            let local = *master_mac.lock();
            merge_master_sightings(&found_masters, &local, master_entries);
            log_master_changes(&master_changes, &local, master_entries);
        }
    }

    Ok(())
}

fn discovery_page_count(total: u8) -> u8 {
    total.div_ceil(10).max(1)
}

fn validate_discovery_response(response: &[u8], pages: u8) -> Result<()> {
    anyhow::ensure!(
        response.len() >= 4 && response[0] == USB_CMD_SEND_RF,
        "missing or invalid GetDev response"
    );
    let records = usize::from(response[1]).min(usize::from(pages) * 10);
    anyhow::ensure!(
        response.len() >= 4 + records * 42,
        "truncated GetDev response"
    );
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct MasterChange {
    device: [u8; 6],
    from: [u8; 6],
    to: [u8; 6],
}

fn log_master_changes(changes: &[MasterChange], local: &[u8; 6], masters: &MasterEntryMap) {
    if changes.is_empty() {
        return;
    }
    let masters = masters.lock();
    for change in changes {
        let owner = if change.to == *local {
            "this dongle"
        } else if change.to == [0; 6] {
            "no master"
        } else if masters.contains_key(&change.to) {
            "another online master"
        } else {
            "a master not heard by this dongle"
        };
        info!(
            "{:02x?} now reports master {:02x?} ({owner}), previously {:02x?}",
            change.device, change.to, change.from
        );
    }
}

fn merge_sightings(
    found: &[DiscoveredDevice],
    health_map: &DeviceHealthMap,
    discovered_devices: &Arc<Mutex<Vec<DiscoveredDevice>>>,
    master_mac: &Arc<Mutex<[u8; 6]>>,
) -> Vec<MasterChange> {
    let now = Instant::now();
    let local = *master_mac.lock();
    let mut health = health_map.lock();
    let mut master_changes = Vec::new();

    for rec in found {
        let rx_taken_by = rx_holder(&health, rec, &local);
        let h = health
            .entry(rec.mac)
            .or_insert_with(|| DeviceHealth::new(rec.clone()));
        if h.dead {
            let intent = h.bind_intent;
            let man_unbind = h.man_unbind;
            *h = DeviceHealth::new(rec.clone());
            h.bind_intent = intent;
            h.man_unbind = man_unbind;
        }
        let intent = h.bind_intent;
        h.last_seen = now;
        h.raw_seen = now;
        h.raw_master = rec.master_mac;
        h.raw_rx = rec.rx_type;
        if is_valid_address(rec.channel, rec.rx_type) && rx_taken_by.is_none() {
            h.confirmed_invalid_address = None;
            h.duplicate_since = None;
        }
        h.raw_channel = rec.channel;

        h.merge_identity(rec, now);

        let p = &mut h.published;
        p.fan_rpms = rec.fan_rpms;
        p.current_pwm = rec.current_pwm;
        p.cmd_seq = rec.cmd_seq;
        p.coolant_temp_c = rec.coolant_temp_c;
        p.effect_index = rec.effect_index;
        p.is_sync_mb_light = rec.is_sync_mb_light;
        p.is_pwm_line_on = rec.is_pwm_line_on;
        p.bind_intent = intent;

        if let Some(m) = commit_streak(&mut h.master_cand, rec.master_mac) {
            if m != h.observed_master {
                master_changes.push(MasterChange {
                    device: rec.mac,
                    from: h.observed_master,
                    to: m,
                });
            }
            h.observed_master = m;
            let steal_back = intent && h.published.master_mac == local && m != local;
            if !steal_back {
                h.published.master_mac = m;
            }
            if intent {
                if m != local {
                    if h.foreign_since.is_none() {
                        h.foreign_since = Some(now);
                    }
                } else {
                    h.foreign_since = None;
                }
            }
        }

        if let Some((ch, rx)) = commit_streak(&mut h.addr_cand, (rec.channel, rec.rx_type)) {
            if is_valid_channel(ch) {
                h.published.channel = ch;
            }
            if let Some(holder) = rx_taken_by {
                if h.confirmed_invalid_address.is_none() {
                    warn!(
                        "{} reported rx={rx} already used by {:02x?}; keeping ch={} rx={} pending recovery",
                        rec.mac_str(),
                        holder,
                        h.published.channel,
                        h.published.rx_type
                    );
                }
                h.confirmed_invalid_address = Some((ch, rx));
                h.duplicate_since.get_or_insert(now);
            } else if is_valid_address(ch, rx) {
                h.publish_rx(rx);
            } else {
                h.duplicate_since = None;
                if h.confirmed_invalid_address.is_none() && (rx != 0 || intent) {
                    warn!(
                        "{} reported invalid address ch={ch} rx={rx}; keeping ch={} rx={} pending recovery",
                        rec.mac_str(),
                        h.published.channel,
                        h.published.rx_type
                    );
                }
                h.confirmed_invalid_address = Some((ch, rx));
            }
        }

        if !h.bind_intent && !h.man_unbind && rec.master_mac == local {
            h.bind_intent = true;
            h.published.bind_intent = true;
            info!(
                "{} reports this dongle as master, claiming binding intent",
                rec.mac_str()
            );
        }
    }

    rebuild_published_vec(&health, discovered_devices, &local);
    master_changes
}

/// The vendor treats two devices under its own master reporting the same rx
/// as a conflict. The device that published that rx first keeps it, and an
/// already flagged device holds nothing, so exactly one side is rebound.
fn rx_holder(
    health: &BTreeMap<[u8; 6], DeviceHealth>,
    rec: &DiscoveredDevice,
    local: &[u8; 6],
) -> Option<[u8; 6]> {
    if *local == [0; 6] || rec.master_mac != *local || !is_valid_rx(rec.rx_type) {
        return None;
    }
    let own_claim = health
        .get(&rec.mac)
        .filter(|own| !own.dead && own.published.rx_type == rec.rx_type)
        .map(|own| (own.rx_published_at, rec.mac));
    health
        .iter()
        .find(|(mac, h)| {
            **mac != rec.mac
                && h.bind_intent
                && !h.dead
                && h.raw_master == *local
                && h.raw_seen.elapsed() <= ACK_FRESHNESS
                && h.confirmed_invalid_address.is_none()
                && h.published.rx_type == rec.rx_type
                && own_claim.is_none_or(|own| (h.rx_published_at, **mac) < own)
        })
        .map(|(mac, _)| *mac)
}

fn sweep(
    health_map: &DeviceHealthMap,
    discovered_devices: &Arc<Mutex<Vec<DiscoveredDevice>>>,
    master_mac: &Arc<Mutex<[u8; 6]>>,
) {
    let now = Instant::now();
    let mut health = health_map.lock();
    let mut changed = false;
    for h in health.values_mut() {
        if !h.dead && now.duration_since(h.last_seen) > LIVENESS_TIMEOUT {
            h.dead = true;
            changed = true;
        }
    }
    let before = health.len();
    health.retain(|_, h| !h.dead || h.bind_intent || h.man_unbind);
    changed |= health.len() != before;
    if changed {
        let local = *master_mac.lock();
        rebuild_published_vec(&health, discovered_devices, &local);
    }
}

fn rebuild_published_vec(
    health: &BTreeMap<[u8; 6], DeviceHealth>,
    discovered_devices: &Arc<Mutex<Vec<DiscoveredDevice>>>,
    local: &[u8; 6],
) {
    let mut devices = discovered_devices.lock();
    let mut new_vec: Vec<DiscoveredDevice> = Vec::with_capacity(devices.len());
    for old in devices.iter() {
        if let Some(h) = health.get(&old.mac) {
            if !h.dead {
                new_vec.push(h.published.clone());
            }
        }
    }
    for h in health.values() {
        if !h.dead && !new_vec.iter().any(|d| d.mac == h.published.mac) {
            new_vec.push(h.published.clone());
        }
    }
    for (i, d) in new_vec.iter_mut().enumerate() {
        d.list_index = i as u8;
    }

    let old_sig: Vec<([u8; 6], [u8; 6])> = devices.iter().map(|d| (d.mac, d.master_mac)).collect();
    let new_sig: Vec<([u8; 6], [u8; 6])> = new_vec.iter().map(|d| (d.mac, d.master_mac)).collect();
    if old_sig != new_sig {
        let bound = new_vec
            .iter()
            .filter(|d| d.bind_intent || d.master_mac == *local)
            .count();
        let unbound = new_vec.len() - bound;
        info!(
            "Discovered {} wireless device(s) ({bound} bound, {unbound} unbound)",
            new_vec.len()
        );
        for d in new_vec
            .iter()
            .filter(|d| !d.bind_intent && d.master_mac != *local)
        {
            info!(
                "  {} ({}) not bound to this dongle; reported master={:02x?}",
                d.mac_str(),
                d.fan_type.display_name(),
                d.master_mac
            );
        }
    }

    *devices = new_vec;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_rx_drift_requires_debounce_and_recovers_without_losing_ownership() {
        for initial_rx in [1, 41] {
            let controller = super::super::controller::WirelessController::new();
            let local = [9; 6];
            let mac = [1; 6];
            *controller.master_mac.lock() = local;
            let mut sighting = rec(mac, local);
            sighting.rx_type = initial_rx;
            let merge = |sighting: &DiscoveredDevice| {
                merge_sightings(
                    std::slice::from_ref(sighting),
                    &controller.device_health,
                    &controller.discovered_devices,
                    &controller.master_mac,
                );
            };
            merge(&sighting);
            assert!(controller.rebind_candidates().is_empty());
            assert_eq!(
                controller.device_by_mac(&mac).unwrap().rx_type,
                if initial_rx == 1 { 1 } else { 0 }
            );
            sighting.rx_type = 41;
            merge(&sighting);
            assert!(controller.rebind_candidates().is_empty());
            merge(&sighting);
            if initial_rx == 1 {
                assert!(controller.rebind_candidates().is_empty());
                merge(&sighting);
            }
            assert_eq!(controller.rebind_candidates(), vec![mac]);
            assert!(controller.unbound_devices().is_empty());

            sighting.rx_type = 2;
            merge(&sighting);
            assert!(controller.rebind_candidates().is_empty());
            merge(&sighting);
            merge(&sighting);
            assert_eq!(controller.device_by_mac(&mac).unwrap().rx_type, 2);
        }
    }

    #[test]
    fn lcd_group_count_uses_each_reported_slot_and_rejects_unknown_data() {
        let mut device = rec([1; 6], [2; 6]);
        for (types, count) in [
            ([23, 20, 26, 0], 2),
            ([27, 28, 35, 0], 2),
            ([43, 45, 48, 0], 2),
            ([51, 53, 56, 0], 2),
            ([20, 21, 22, 0], 0),
        ] {
            device.fan_types = types;
            device.fan_type = WirelessFanType::from_fan_type_byte(types[0]);
            assert_eq!(device.lcd_group_size(), Some(count));
        }
        device.fan_type = WirelessFanType::TlV3 { lcd: true };
        device.fan_types = [51, 23, 56, 0];
        assert_eq!(device.lcd_group_size(), None);
        device.fan_types = [51, 0, 56, 0];
        assert_eq!(device.lcd_group_size(), None);
        device.fan_types = [51; 4];
        device.fan_count = 5;
        assert_eq!(device.lcd_group_size(), None);
        device.fan_count = 3;
        device.device_type = 10;
        assert_eq!(device.lcd_group_size(), None);
    }

    fn rec(mac: [u8; 6], master: [u8; 6]) -> DiscoveredDevice {
        DiscoveredDevice {
            mac,
            master_mac: master,
            channel: 8,
            rx_type: 1,
            device_type: 0,
            fan_count: 3,
            is_inf_right_attach: false,
            fan_types: [0; 4],
            fan_rpms: [0; 4],
            current_pwm: [0; 4],
            cmd_seq: 0,
            fan_type: WirelessFanType::Unknown,
            list_index: 0,
            coolant_temp_c: None,
            effect_index: [0; 4],
            is_sync_mb_light: false,
            is_pwm_line_on: false,
            bind_intent: false,
        }
    }

    type TestCtx = (
        DeviceHealthMap,
        Arc<Mutex<Vec<DiscoveredDevice>>>,
        Arc<Mutex<[u8; 6]>>,
    );

    fn setup() -> TestCtx {
        (
            Arc::new(Mutex::new(Default::default())),
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(Mutex::new([9u8; 6])),
        )
    }

    fn correction_health(now: Instant) -> DeviceHealth {
        let mut health = DeviceHealth::new(rec([1; 6], [9; 6]));
        health.bind_intent = true;
        health.raw_master = [9; 6];
        health.raw_channel = 8;
        health.raw_rx = 1;
        health.raw_seen = now;
        health
    }

    #[test]
    fn channel_correction_backs_off_and_stops_until_channel_changes() {
        use ChannelCorrectionAction::*;
        let now = Instant::now();
        let mut health = correction_health(now);
        for seconds in [0, 2, 6, 14, 30] {
            let time = now + Duration::from_secs(seconds);
            health.raw_seen = time;
            assert_eq!(health.channel_correction_action([9; 6], 4, time), Send);
            assert_eq!(health.channel_correction_action([9; 6], 4, time), Wait);
        }
        let time = now + Duration::from_secs(62);
        health.raw_seen = time;
        assert_eq!(health.channel_correction_action([9; 6], 4, time), Exhausted);
        assert_eq!(health.channel_correction_action([9; 6], 4, time), Wait);
        let later = time + Duration::from_secs(3600);
        health.raw_seen = later;
        assert_eq!(health.channel_correction_action([9; 6], 4, later), Wait);
        assert_eq!(health.channel_correction_action([9; 6], 12, later), Send);
    }

    #[test]
    fn channel_correction_requires_fresh_confirmed_ownership_and_address() {
        use ChannelCorrectionAction::*;
        let now = Instant::now();
        for variant in 0..8 {
            let mut health = correction_health(now);
            match variant {
                0 => health.bind_intent = false,
                1 => health.man_unbind = true,
                2 => health.dead = true,
                3 => health.raw_master = [3; 6],
                4 => health.raw_seen = now - ACK_FRESHNESS - Duration::from_millis(1),
                5 => health.raw_channel = 2,
                6 => health.raw_rx = 2,
                _ => health.published.master_mac = [3; 6],
            }
            assert_eq!(health.channel_correction_action([9; 6], 4, now), Wait);
            assert_eq!(health.channel_correction.attempts, 0);
        }
    }

    #[test]
    fn confirmed_channel_match_clears_retry_budget() {
        use ChannelCorrectionAction::*;
        let now = Instant::now();
        let mut health = correction_health(now);
        assert_eq!(health.channel_correction_action([9; 6], 4, now), Send);
        health.raw_channel = 4;
        health.published.channel = 4;
        assert_eq!(health.channel_correction_action([9; 6], 4, now), Wait);
        health.raw_channel = 8;
        health.published.channel = 8;
        assert_eq!(health.channel_correction_action([9; 6], 4, now), Send);
    }

    #[test]
    fn corrupt_type_records_do_not_replace_a_known_layout() {
        let local = [9; 6];
        let controller = controller_with_master(local);
        let typed = |slots: [u8; 4]| {
            let mut device = rec([1; 6], local);
            device.fan_types = slots;
            device.fan_type = WirelessFanType::from_fan_type_byte(slots[0]);
            device
        };
        let tl = typed([28, 28, 28, 0]);
        let sl = typed([20, 20, 20, 0]);
        let unknown = typed([0xEE, 0, 0, 0]);
        let published = || controller.device_by_mac(&[1; 6]).unwrap();

        merge_into(&controller, std::slice::from_ref(&tl), 1);
        for _ in 0..DEBOUNCE_SIGHTINGS {
            merge_into(&controller, std::slice::from_ref(&sl), 1);
            merge_into(&controller, std::slice::from_ref(&unknown), 1);
            merge_into(&controller, std::slice::from_ref(&tl), 1);
            assert_eq!(published().fan_type, WirelessFanType::Tlv2Led);
        }

        merge_into(
            &controller,
            std::slice::from_ref(&unknown),
            DEBOUNCE_SIGHTINGS * 2,
        );
        assert_eq!(published().fan_type, WirelessFanType::Tlv2Led);
        assert_eq!(published().fan_types, tl.fan_types);

        merge_into(&controller, std::slice::from_ref(&sl), DEBOUNCE_SIGHTINGS);
        assert_eq!(published().fan_type, WirelessFanType::Slv3Led);
    }

    #[test]
    fn unknown_first_sighting_adopts_a_stable_type() {
        let local = [9; 6];
        let controller = controller_with_master(local);
        let mut device = rec([1; 6], local);
        merge_into(&controller, std::slice::from_ref(&device), 1);
        device.fan_types = [28, 28, 28, 0];
        device.fan_type = WirelessFanType::Tlv2Led;
        merge_into(
            &controller,
            std::slice::from_ref(&device),
            DEBOUNCE_SIGHTINGS,
        );
        assert_eq!(
            controller.device_by_mac(&[1; 6]).unwrap().fan_type,
            WirelessFanType::Tlv2Led
        );
    }

    #[test]
    fn aio_coolant_changes_do_not_count_as_identity_changes() {
        let local = [9; 6];
        let controller = controller_with_master(local);
        let mut aio = rec([1; 6], local);
        aio.device_type = 10;
        aio.fan_type = WirelessFanType::WaterBlock;
        aio.fan_types = [0, 0, 0, 30];
        merge_into(&controller, std::slice::from_ref(&aio), 1);
        aio.fan_types[3] = 31;
        merge_into(&controller, std::slice::from_ref(&aio), 1);
        let health = controller.device_health.lock();
        let health = health.get(&[1; 6]).unwrap();
        assert_eq!(health.published.fan_types[3], 31);
        assert!(health.identity_warned_at.is_none());
    }

    #[test]
    fn commit_streak_requires_three_agreeing_sightings() {
        let mut cand = None;
        assert!(commit_streak(&mut cand, 5u8).is_none());
        assert!(commit_streak(&mut cand, 5u8).is_none());
        assert_eq!(commit_streak(&mut cand, 5u8), Some(5));
        assert!(commit_streak(&mut cand, 5u8).is_none());
    }

    #[test]
    fn streak_resets_on_disagreement() {
        let mut cand = None;
        commit_streak(&mut cand, 1u8);
        commit_streak(&mut cand, 1u8);
        commit_streak(&mut cand, 2u8);
        assert!(commit_streak(&mut cand, 2u8).is_none());
        assert_eq!(commit_streak(&mut cand, 2u8), Some(2));
    }

    #[test]
    fn parse_rejects_only_zero_mac() {
        let mut buf = [0u8; 42];
        buf[41] = 0x1C;
        assert!(parse_device_record(&buf, 0).is_none());
        buf[0..6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        assert!(parse_device_record(&buf, 0).is_some());
        buf[13] = 15;
        assert!(parse_device_record(&buf, 0).is_some());
        buf[13] = 1;
        assert!(parse_device_record(&buf, 0).is_some());
    }

    /// An RX slot outside the allocatable range must never be adopted.
    ///
    /// The master-clock broadcast writes `cpu_temp` into `rf_data[14]`, the
    /// byte targeted frames use for `rx_type`, so devices report slots that
    /// track CPU temperature. Adopting one addresses the device where it was
    /// never bound, and two chains on the same broadcast collide on the same
    /// slot.
    #[test]
    fn out_of_range_rx_is_not_adopted() {
        assert!(is_valid_rx(1));
        assert!(is_valid_rx(13));
        assert!(!is_valid_rx(0));
        assert!(!is_valid_rx(RX_SLOT_LIMIT));
        // The values actually observed on hardware, all CPU temperatures.
        for observed in [40u8, 41, 42, 44, 49, 61, 62, 64, 67, 71] {
            assert!(!is_valid_rx(observed), "rx={observed} must be rejected");
        }
    }

    #[test]
    fn latched_channel_zero_device_stays_alive_and_becomes_rebind_candidate() {
        let controller = super::super::controller::WirelessController::new();
        let local = [9; 6];
        let mac = [1; 6];
        *controller.master_mac.lock() = local;
        let merge = |sighting: &DiscoveredDevice| {
            merge_sightings(
                std::slice::from_ref(sighting),
                &controller.device_health,
                &controller.discovered_devices,
                &controller.master_mac,
            );
        };

        let mut buf = [0u8; 42];
        buf[0..6].copy_from_slice(&mac);
        buf[6..12].copy_from_slice(&local);
        buf[12] = 8;
        buf[13] = 1;
        buf[41] = 0x1C;
        merge(&parse_device_record(&buf, 0).unwrap());
        assert!(controller.rebind_candidates().is_empty());

        buf[12] = 0;
        buf[13] = 82;
        let latched = parse_device_record(&buf, 0).expect("latched record must parse");
        for _ in 0..DEBOUNCE_SIGHTINGS {
            merge(&latched);
        }
        let device = controller
            .device_by_mac(&mac)
            .expect("device must stay live");
        assert_eq!((device.channel, device.rx_type), (8, 1));
        assert_eq!(controller.rebind_candidates(), vec![mac]);
    }

    fn merge_into(
        controller: &super::super::controller::WirelessController,
        sightings: &[DiscoveredDevice],
        times: u32,
    ) {
        for _ in 0..times {
            merge_sightings(
                sightings,
                &controller.device_health,
                &controller.discovered_devices,
                &controller.master_mac,
            );
        }
    }

    fn controller_with_master(local: [u8; 6]) -> super::super::controller::WirelessController {
        let controller = super::super::controller::WirelessController::new();
        *controller.master_mac.lock() = local;
        controller
    }

    fn age_duplicate_flags(controller: &super::super::controller::WirelessController) {
        for h in controller.device_health.lock().values_mut() {
            h.duplicate_since = h
                .duplicate_since
                .map(|since| since - REBIND_DUPLICATE_AFTER);
        }
    }

    #[test]
    fn device_reporting_another_masters_mac_is_never_a_rebind_candidate() {
        let local = [9; 6];
        let foreign = [7; 6];
        let mac = [1; 6];
        for claimed_earlier in [false, true] {
            for foreign_online in [false, true] {
                for (channel, rx) in [(8, 1), (8, 0xA7), (0, 82), (131, 1), (8, 2)] {
                    let controller = controller_with_master(local);
                    let mut holder = rec([2; 6], local);
                    holder.rx_type = 2;
                    merge_into(&controller, &[holder.clone()], DEBOUNCE_SIGHTINGS);
                    if claimed_earlier {
                        merge_into(&controller, &[rec(mac, local)], DEBOUNCE_SIGHTINGS);
                    }
                    if foreign_online {
                        merge_master_sightings(&[(foreign, 8)], &local, &controller.master_entries);
                    }
                    let mut sighting = rec(mac, foreign);
                    sighting.channel = channel;
                    sighting.rx_type = rx;
                    merge_into(
                        &controller,
                        &[holder.clone(), sighting],
                        DEBOUNCE_SIGHTINGS * 2,
                    );
                    if let Some(h) = controller.device_health.lock().get_mut(&mac) {
                        h.foreign_since = h.foreign_since.map(|_| {
                            Instant::now() - REBIND_FOREIGN_AFTER - Duration::from_secs(1)
                        });
                    }
                    assert!(
                        controller.rebind_candidates().is_empty(),
                        "claimed={claimed_earlier} online={foreign_online} ch={channel} rx={rx}"
                    );
                    assert_eq!(controller.device_by_mac(&[2; 6]).unwrap().rx_type, 2);
                }
            }
        }
    }

    const REPORTER_MASTER: [u8; 6] = [0xda, 0x82, 0x10, 0x7a, 0x4e, 0xe0];

    #[test]
    fn smeared_local_master_matches_only_the_exact_smear() {
        let local = REPORTER_MASTER;
        let cases = [
            ([0xda, 0x82, 0x10, 0x7a, 0x7a, 0x7a], 0x7a, 0x7a, true),
            ([0xda, 0x82, 0x10, 0x10, 0x10, 0x10], 0x10, 0x10, true),
            ([0xda, 0x82, 0x10, 0x7a, 0x4e, 0x4e], 0x4e, 0x4e, true),
            ([0xda, 0x82, 0x82, 0x82, 0x82, 0x82], 0x82, 0x82, false),
            ([0xda, 0x82, 0x10, 0x7a, 0x7a, 0x7a], 0x7a, 8, false),
            ([0xda, 0x82, 0x10, 0x7a, 0x7a, 0x7a], 8, 0x7a, false),
            ([0xda, 0x82, 0x10, 0x7a, 0x7a, 0x7b], 0x7a, 0x7a, false),
            ([0xda, 0x82, 0x10, 0x7a, 0x4e, 0xe1], 0x4e, 0x4e, false),
            ([0x11, 0x22, 0x33, 0x44, 0x55, 0x66], 0x44, 0x44, false),
            (local, 0x4e, 0x4e, false),
        ];
        for (reported, channel, rx, expected) in cases {
            assert_eq!(
                is_smeared_local_master(&reported, &local, channel, rx),
                expected,
                "{reported:02x?} ch={channel} rx={rx}"
            );
        }
        assert!(!is_smeared_local_master(&[0; 6], &[0; 6], 0, 0));
    }

    fn smeared_sighting(mac: [u8; 6]) -> DiscoveredDevice {
        let mut sighting = rec(mac, [0xda, 0x82, 0x10, 0x7a, 0x7a, 0x7a]);
        sighting.channel = 0x7a;
        sighting.rx_type = 0x7a;
        sighting
    }

    fn age_foreign_flags(controller: &super::super::controller::WirelessController) {
        for h in controller.device_health.lock().values_mut() {
            h.foreign_since = h.foreign_since.map(|since| since - REBIND_FOREIGN_AFTER);
        }
    }

    #[test]
    fn our_device_with_a_smeared_master_becomes_a_rebind_candidate() {
        let local = REPORTER_MASTER;
        let mac = [1; 6];
        let smear = smeared_sighting(mac).master_mac;
        for case in ["aged", "fresh", "smear online", "manually unbound"] {
            let controller = controller_with_master(local);
            merge_into(&controller, &[rec(mac, local)], DEBOUNCE_SIGHTINGS);
            if case == "smear online" {
                merge_master_sightings(&[(smear, 8)], &local, &controller.master_entries);
            }
            if case == "manually unbound" {
                controller.confirm_binding(&mac, false);
            }
            merge_into(&controller, &[smeared_sighting(mac)], DEBOUNCE_SIGHTINGS);
            if case != "fresh" {
                age_foreign_flags(&controller);
            }
            let expected = if case == "aged" { vec![mac] } else { vec![] };
            assert_eq!(controller.rebind_candidates(), expected, "{case}");
        }
    }

    #[test]
    fn device_never_ours_with_a_lookalike_master_is_never_a_rebind_candidate() {
        let local = REPORTER_MASTER;
        let mac = [1; 6];
        let controller = controller_with_master(local);
        merge_into(
            &controller,
            &[smeared_sighting(mac)],
            DEBOUNCE_SIGHTINGS * 2,
        );
        age_foreign_flags(&controller);
        assert!(controller.rebind_candidates().is_empty());
        assert!(!controller.needs_rebind(&mac));
        assert!(controller.unbound_devices().iter().any(|d| d.mac == mac));
    }

    #[test]
    fn leaked_effect_byte_as_rx_under_our_master_is_a_rebind_candidate() {
        let local = [9; 6];
        let mac = [1; 6];
        let controller = controller_with_master(local);
        merge_into(&controller, &[rec(mac, local)], DEBOUNCE_SIGHTINGS);
        assert!(controller.rebind_candidates().is_empty());

        let mut leaked = rec(mac, local);
        leaked.rx_type = 0xA7;
        merge_into(&controller, &[leaked], DEBOUNCE_SIGHTINGS);
        assert_eq!(controller.rebind_candidates(), vec![mac]);
        assert_eq!(controller.device_by_mac(&mac).unwrap().rx_type, 1);
        assert!(!controller.reports_valid_address(&mac));
    }

    #[test]
    fn duplicate_rx_under_our_master_rebinds_only_the_newcomer() {
        let local = [9; 6];
        let holder = [1; 6];
        let newcomer = [2; 6];
        let controller = controller_with_master(local);
        let mut moved = rec(newcomer, local);
        moved.rx_type = 2;
        merge_into(
            &controller,
            &[rec(holder, local), moved.clone()],
            DEBOUNCE_SIGHTINGS,
        );
        assert!(controller.rebind_candidates().is_empty());

        moved.rx_type = 1;
        merge_into(
            &controller,
            &[rec(holder, local), moved.clone()],
            DEBOUNCE_SIGHTINGS * 2,
        );
        assert!(controller.rebind_candidates().is_empty());
        age_duplicate_flags(&controller);
        assert_eq!(controller.rebind_candidates(), vec![newcomer]);
        assert_eq!(controller.device_by_mac(&holder).unwrap().rx_type, 1);
        assert_eq!(controller.device_by_mac(&newcomer).unwrap().rx_type, 2);
        assert!(controller.reports_valid_address(&holder));
        assert!(!controller.reports_valid_address(&newcomer));

        moved.rx_type = 3;
        merge_into(&controller, &[rec(holder, local), moved], 1);
        controller.confirm_binding(&newcomer, true);
        assert!(controller.rebind_candidates().is_empty());
        assert_eq!(controller.device_by_mac(&newcomer).unwrap().rx_type, 3);
        assert_eq!(controller.device_by_mac(&holder).unwrap().rx_type, 1);
    }

    #[test]
    fn duplicate_rx_seen_at_startup_rebinds_exactly_one_device() {
        let local = [9; 6];
        let controller = controller_with_master(local);
        merge_into(
            &controller,
            &[rec([1; 6], local), rec([2; 6], local)],
            DEBOUNCE_SIGHTINGS * 2,
        );
        age_duplicate_flags(&controller);
        assert_eq!(controller.rebind_candidates(), vec![[2; 6]]);
        assert_eq!(controller.device_by_mac(&[1; 6]).unwrap().rx_type, 1);
        assert_eq!(
            controller.device_by_mac(&[2; 6]).unwrap().rx_type,
            1,
            "a flagged device keeps its last address so it stays under fan control"
        );
        assert!(controller.reports_valid_address(&[1; 6]));
        assert!(!controller.reports_valid_address(&[2; 6]));
    }

    #[test]
    fn swapped_rx_reports_never_flag_both_devices() {
        let local = [9; 6];
        let controller = controller_with_master(local);
        let mut first = rec([1; 6], local);
        let mut second = rec([2; 6], local);
        second.rx_type = 2;
        merge_into(
            &controller,
            &[first.clone(), second.clone()],
            DEBOUNCE_SIGHTINGS,
        );
        first.rx_type = 2;
        second.rx_type = 1;
        let mut transient_flag = false;
        for _ in 0..DEBOUNCE_SIGHTINGS * 3 {
            merge_into(&controller, &[first.clone(), second.clone()], 1);
            let flagged = controller
                .device_health
                .lock()
                .values()
                .filter(|h| h.duplicate_since.is_some())
                .count();
            assert!(flagged <= 1);
            transient_flag |= flagged == 1;
            assert!(controller.rebind_candidates().is_empty());
        }
        assert!(transient_flag, "the swap must exercise the debounce");
        age_duplicate_flags(&controller);
        assert!(controller.rebind_candidates().is_empty());
        assert_eq!(controller.device_by_mac(&[1; 6]).unwrap().rx_type, 2);
        assert_eq!(controller.device_by_mac(&[2; 6]).unwrap().rx_type, 1);
    }

    #[test]
    fn silent_holder_does_not_flag_a_device_on_its_rx() {
        let local = [9; 6];
        let controller = controller_with_master(local);
        let mut moved = rec([2; 6], local);
        moved.rx_type = 2;
        merge_into(
            &controller,
            &[rec([1; 6], local), moved.clone()],
            DEBOUNCE_SIGHTINGS,
        );
        controller
            .device_health
            .lock()
            .get_mut(&[1; 6])
            .unwrap()
            .raw_seen = Instant::now() - ACK_FRESHNESS - Duration::from_secs(1);

        moved.rx_type = 1;
        merge_into(&controller, &[moved], DEBOUNCE_SIGHTINGS);
        assert!(controller.rebind_candidates().is_empty());
        assert_eq!(controller.device_by_mac(&[2; 6]).unwrap().rx_type, 1);
    }

    #[test]
    fn duplicate_rx_under_a_foreign_master_is_ignored() {
        let local = [9; 6];
        let controller = controller_with_master(local);
        merge_into(
            &controller,
            &[rec([1; 6], local), rec([2; 6], [7; 6])],
            DEBOUNCE_SIGHTINGS * 2,
        );
        assert!(controller.rebind_candidates().is_empty());
        assert_eq!(controller.device_by_mac(&[1; 6]).unwrap().rx_type, 1);
        assert_eq!(controller.device_by_mac(&[2; 6]).unwrap().rx_type, 1);
        assert!(controller.reports_valid_address(&[1; 6]));
    }

    #[test]
    fn out_of_range_channel_is_never_used_for_traffic_but_still_rebinds() {
        let controller = super::super::controller::WirelessController::new();
        let local = [9; 6];
        let mac = [1; 6];
        *controller.master_mac.lock() = local;
        *controller.master_channel.lock() = 12;
        let merge = |sighting: &DiscoveredDevice| {
            merge_sightings(
                std::slice::from_ref(sighting),
                &controller.device_health,
                &controller.discovered_devices,
                &controller.master_mac,
            );
        };
        let mut sighting = rec(mac, local);
        merge(&sighting);

        sighting.channel = 131;
        for _ in 0..DEBOUNCE_SIGHTINGS {
            merge(&sighting);
        }
        let device = controller
            .device_by_mac(&mac)
            .expect("device must stay live");
        assert_eq!((device.channel, device.rx_type), (8, 1));
        assert_eq!(controller.rebind_candidates(), vec![mac]);
        assert_eq!(controller.bind_channel(&device, 12), 131);

        sighting.channel = 0;
        merge(&sighting);
        assert_eq!(controller.bind_channel(&device, 12), 12);
        assert_eq!(traffic_channel(device.channel, 12), 8);
    }

    #[test]
    fn channels_observed_from_corrupted_devices_are_invalid() {
        assert!(is_valid_channel(1));
        assert!(is_valid_channel(39));
        assert!(!is_valid_channel(0));
        assert!(!is_valid_channel(40));
        for observed in [64u8, 68, 85, 131, 146, 175, 209, 210, 220, 226, 228] {
            assert!(
                !is_valid_channel(observed),
                "ch={observed} must be rejected"
            );
        }
    }

    #[test]
    fn out_of_range_channel_at_startup_publishes_no_address() {
        let mut latched = rec([1; 6], [9; 6]);
        latched.channel = 45;
        let health = DeviceHealth::new(latched);
        assert_eq!((health.published.channel, health.published.rx_type), (0, 0));
        assert_eq!(traffic_channel(health.published.channel, 12), 12);
    }

    #[test]
    fn latched_record_at_startup_publishes_no_address() {
        let mut latched = rec([1; 6], [9; 6]);
        latched.channel = 0;
        latched.rx_type = 1;
        let health = DeviceHealth::new(latched);
        assert_eq!(health.published.rx_type, 0);
    }

    #[test]
    fn parse_master_record_validates() {
        let mut buf = [0u8; 42];
        buf[41] = 0x1C;
        buf[18] = 0xFF;
        assert!(parse_master_record(&buf).is_none());
        buf[0..6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        buf[12] = 8;
        assert_eq!(parse_master_record(&buf), Some(([1, 2, 3, 4, 5, 6], 8)));
        buf[18] = 0x00;
        assert!(parse_master_record(&buf).is_none());
        buf[18] = 0xFF;
        buf[41] = 0x00;
        assert!(parse_master_record(&buf).is_none());
    }

    #[test]
    fn master_sightings_track_liveness() {
        let masters: MasterEntryMap = Arc::new(Mutex::new(Default::default()));
        let mac = [9u8; 6];
        let local = [1u8; 6];

        merge_master_sightings(&[(mac, 8)], &local, &masters);
        assert!(masters.lock().contains_key(&mac));

        let stale = Instant::now()
            .checked_sub(LIVENESS_TIMEOUT + Duration::from_secs(1))
            .expect("uptime too short for test");
        masters.lock().get_mut(&mac).unwrap().last_seen = stale;
        merge_master_sightings(&[], &local, &masters);
        assert!(!masters.lock().contains_key(&mac));
    }

    #[test]
    fn own_master_record_is_not_tracked_as_foreign() {
        let masters: MasterEntryMap = Arc::new(Mutex::new(Default::default()));
        let local = [1u8; 6];

        merge_master_sightings(&[(local, 8), ([9u8; 6], 8)], &local, &masters);
        assert!(!masters.lock().contains_key(&local));
        assert!(masters.lock().contains_key(&[9u8; 6]));
    }

    #[test]
    fn master_flip_debounced_without_intent() {
        let (health, devices, master) = setup();
        let mac = [1, 2, 3, 4, 5, 6];
        let foreign_a = [6u8; 6];
        let foreign_b = [7u8; 6];

        for _ in 0..3 {
            merge_sightings(&[rec(mac, foreign_a)], &health, &devices, &master);
        }
        assert_eq!(
            health.lock().get(&mac).unwrap().published.master_mac,
            foreign_a
        );

        for _ in 0..2 {
            merge_sightings(&[rec(mac, foreign_b)], &health, &devices, &master);
        }
        assert_eq!(
            health.lock().get(&mac).unwrap().published.master_mac,
            foreign_a
        );

        merge_sightings(&[rec(mac, foreign_b)], &health, &devices, &master);
        assert_eq!(
            health.lock().get(&mac).unwrap().published.master_mac,
            foreign_b
        );
        assert_eq!(devices.lock().len(), 1);
    }

    #[test]
    fn committed_master_changes_are_reported_once_per_transition() {
        let (health, devices, master) = setup();
        let mac = [1, 2, 3, 4, 5, 6];
        let local = *master.lock();
        let foreign = [7u8; 6];
        let sightings = |owners: &[[u8; 6]]| -> Vec<MasterChange> {
            owners
                .iter()
                .flat_map(|owner| merge_sightings(&[rec(mac, *owner)], &health, &devices, &master))
                .collect()
        };
        let change = |from, to| MasterChange {
            device: mac,
            from,
            to,
        };

        assert!(sightings(&[local; 6]).is_empty());
        assert!(sightings(&[foreign, local, foreign, local]).is_empty());
        assert_eq!(sightings(&[foreign; 9]), vec![change(local, foreign)]);
        assert_eq!(sightings(&[[0; 6]; 3]), vec![change(foreign, [0; 6])]);
    }

    #[test]
    fn local_master_sighting_claims_intent() {
        let (health, devices, master) = setup();
        let mac = [1, 2, 3, 4, 5, 6];
        let local = *master.lock();

        merge_sightings(&[rec(mac, local)], &health, &devices, &master);
        let guard = health.lock();
        let h = guard.get(&mac).unwrap();
        assert!(h.bind_intent);
        assert!(h.published.bind_intent);
        assert!(!h.man_unbind);
    }

    #[test]
    fn claim_skipped_for_manually_unbound() {
        let (health, devices, master) = setup();
        let mac = [1, 2, 3, 4, 5, 6];
        let local = *master.lock();

        merge_sightings(&[rec(mac, local)], &health, &devices, &master);
        health.lock().get_mut(&mac).unwrap().man_unbind = true;
        health.lock().get_mut(&mac).unwrap().bind_intent = false;

        merge_sightings(&[rec(mac, local)], &health, &devices, &master);
        assert!(!health.lock().get(&mac).unwrap().bind_intent);
    }

    #[test]
    fn transient_foreign_sighting_does_not_flip_master() {
        let (health, devices, master) = setup();
        let mac = [1, 2, 3, 4, 5, 6];
        let local = *master.lock();
        let foreign = [7u8; 6];

        for _ in 0..3 {
            merge_sightings(&[rec(mac, local)], &health, &devices, &master);
        }
        merge_sightings(&[rec(mac, foreign)], &health, &devices, &master);
        merge_sightings(&[rec(mac, local)], &health, &devices, &master);
        merge_sightings(&[rec(mac, foreign)], &health, &devices, &master);
        merge_sightings(&[rec(mac, local)], &health, &devices, &master);

        let guard = health.lock();
        let h = guard.get(&mac).unwrap();
        assert_eq!(h.published.master_mac, local);
        assert_eq!(h.observed_master, local);
        assert!(h.foreign_since.is_none());
    }

    #[test]
    fn steal_back_keeps_published_master() {
        let (health, devices, master) = setup();
        let mac = [1, 2, 3, 4, 5, 6];
        let local = *master.lock();
        let foreign = [7u8; 6];

        for _ in 0..3 {
            merge_sightings(&[rec(mac, local)], &health, &devices, &master);
        }
        health.lock().get_mut(&mac).unwrap().bind_intent = true;
        devices.lock().iter_mut().next().unwrap().bind_intent = true;

        for _ in 0..3 {
            merge_sightings(&[rec(mac, foreign)], &health, &devices, &master);
        }

        let guard = health.lock();
        let h = guard.get(&mac).unwrap();
        assert_eq!(h.published.master_mac, local);
        assert_eq!(h.observed_master, foreign);
        assert!(h.foreign_since.is_some());
    }

    #[test]
    fn sweep_evicts_dead_without_intent_and_keeps_intents() {
        let (health, devices, master) = setup();
        let mac_a = [1, 2, 3, 4, 5, 6];
        let mac_b = [2, 2, 3, 4, 5, 6];
        let local = *master.lock();
        let foreign = [7u8; 6];

        for _ in 0..3 {
            merge_sightings(&[rec(mac_a, foreign)], &health, &devices, &master);
        }
        for _ in 0..3 {
            merge_sightings(&[rec(mac_b, local)], &health, &devices, &master);
        }

        let stale = Instant::now()
            .checked_sub(LIVENESS_TIMEOUT + Duration::from_secs(1))
            .expect("uptime too short for test");
        for h in health.lock().values_mut() {
            h.last_seen = stale;
        }

        sweep(&health, &devices, &master);

        let health = health.lock();
        assert!(health.get(&mac_a).is_none());
        let b = health.get(&mac_b).unwrap();
        assert!(b.dead);
        assert!(b.bind_intent);
        assert!(devices.lock().is_empty());
    }

    #[test]
    fn sweep_keeps_manually_unbound() {
        let (health, devices, master) = setup();
        let mac = [1, 2, 3, 4, 5, 6];
        let foreign = [7u8; 6];

        for _ in 0..3 {
            merge_sightings(&[rec(mac, foreign)], &health, &devices, &master);
        }
        {
            let mut h = health.lock();
            let entry = h.get_mut(&mac).unwrap();
            entry.man_unbind = true;
        }

        let stale = Instant::now()
            .checked_sub(LIVENESS_TIMEOUT + Duration::from_secs(1))
            .expect("uptime too short for test");
        for h in health.lock().values_mut() {
            h.last_seen = stale;
        }

        sweep(&health, &devices, &master);
        assert!(health.lock().get(&mac).is_some());
    }

    #[test]
    fn revive_restores_intent_and_state() {
        let (health, devices, master) = setup();
        let mac = [1, 2, 3, 4, 5, 6];
        let local = *master.lock();

        for _ in 0..3 {
            merge_sightings(&[rec(mac, local)], &health, &devices, &master);
        }
        {
            let mut h = health.lock();
            let entry = h.get_mut(&mac).unwrap();
            entry.bind_intent = true;
            entry.dead = true;
        }

        let mut revived = rec(mac, [7u8; 6]);
        revived.fan_rpms = [100, 200, 0, 0];
        merge_sightings(&[revived], &health, &devices, &master);

        let guard = health.lock();
        let h = guard.get(&mac).unwrap();
        assert!(!h.dead);
        assert!(h.bind_intent);
        assert_eq!(h.published.fan_rpms, [100, 200, 0, 0]);
        assert_eq!(h.raw_master, [7u8; 6]);
        assert_eq!(devices.lock().len(), 1);
    }
}

#[cfg(test)]
mod response_tests {
    use super::*;

    #[test]
    fn receiver_total_bootstraps_followup_pages_without_known_devices() {
        let mut first = vec![0; 4 + 10 * 42];
        first[0] = USB_CMD_SEND_RF;
        first[1] = 23;
        assert!(validate_discovery_response(&first, 1).is_ok());
        let pages = discovery_page_count(first[1]);
        assert_eq!(pages, 3);
        assert!(validate_discovery_response(&first, pages).is_err());
        first.resize(4 + 23 * 42, 0);
        assert!(validate_discovery_response(&first, pages).is_ok());
        assert_eq!(discovery_page_count(0), 1);
        assert_eq!(discovery_page_count(255), 26);
    }

    #[test]
    fn empty_scan_is_healthy_but_silence_wrong_echo_and_truncation_are_errors() {
        assert!(validate_discovery_response(&[0x10, 0, 0x80, 0], 1).is_ok());
        assert!(validate_discovery_response(&[], 1).is_err());
        assert!(validate_discovery_response(&[0x10, 0, 0], 1).is_err());
        assert!(validate_discovery_response(&[0x11, 0, 0, 0], 1).is_err());
        assert!(validate_discovery_response(&[0x10, 1, 0, 0], 1).is_err());
    }
}

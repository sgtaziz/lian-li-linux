use super::transport::with_transport_recovery;
use super::{WirelessFanType, RX_IDS, USB_CMD_SEND_RF};
use anyhow::{Context, Result};
use lianli_transport::usb::{RusbBulk, USB_TIMEOUT};
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info};

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

impl DiscoveredDevice {
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
pub(super) const ACK_FRESHNESS: Duration = Duration::from_secs(3);
pub(super) const REBIND_FOREIGN_AFTER: Duration = Duration::from_secs(10);

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
    master_cand: Option<([u8; 6], u32)>,
    addr_cand: Option<((u8, u8), u32)>,
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

    pub(super) fn new(rec: DiscoveredDevice) -> Self {
        Self {
            channel_correction: ChannelCorrection::default(),
            observed_master: rec.master_mac,
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
    rx: &Arc<Mutex<RusbBulk>>,
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
            if denominator > 0 {
                let pwm = (255u16 * on_time / denominator).min(255);
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

            merge_sightings(&found, health_map, discovered_devices, master_mac);
            let local = *master_mac.lock();
            merge_master_sightings(&found_masters, &local, master_entries);
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

fn merge_sightings(
    found: &[DiscoveredDevice],
    health_map: &DeviceHealthMap,
    discovered_devices: &Arc<Mutex<Vec<DiscoveredDevice>>>,
    master_mac: &Arc<Mutex<[u8; 6]>>,
) {
    let now = Instant::now();
    let local = *master_mac.lock();
    let mut health = health_map.lock();

    for rec in found {
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
        h.raw_channel = rec.channel;

        let p = &mut h.published;
        p.device_type = rec.device_type;
        p.fan_count = rec.fan_count;
        p.is_inf_right_attach = rec.is_inf_right_attach;
        p.fan_types = rec.fan_types;
        p.fan_rpms = rec.fan_rpms;
        p.current_pwm = rec.current_pwm;
        p.cmd_seq = rec.cmd_seq;
        p.fan_type = rec.fan_type;
        p.coolant_temp_c = rec.coolant_temp_c;
        p.effect_index = rec.effect_index;
        p.is_sync_mb_light = rec.is_sync_mb_light;
        p.is_pwm_line_on = rec.is_pwm_line_on;
        p.bind_intent = intent;

        if let Some(m) = commit_streak(&mut h.master_cand, rec.master_mac) {
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
            h.published.channel = ch;
            h.published.rx_type = rx;
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
}

fn sweep(
    health_map: &DeviceHealthMap,
    discovered_devices: &Arc<Mutex<Vec<DiscoveredDevice>>>,
    master_mac: &Arc<Mutex<[u8; 6]>>,
) {
    let now = Instant::now();
    let mut health = health_map.lock();
    let mut changed = false;
    for (_, h) in health.iter_mut() {
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
        buf[12] = 8;
        assert!(parse_device_record(&buf, 0).is_some());
        buf[13] = 15;
        assert!(parse_device_record(&buf, 0).is_some());
        buf[13] = 1;
        assert!(parse_device_record(&buf, 0).is_some());
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
        for (_, h) in health.lock().iter_mut() {
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

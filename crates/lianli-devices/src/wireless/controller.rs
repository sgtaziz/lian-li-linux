use super::convergence::{PendingQueue, TargetSeqMap};
use super::discovery::{
    poll_and_discover, DeviceHealthMap, DiscoveredDevice, MasterEntryMap, ReceiverState,
    ACK_FRESHNESS, REBIND_FOREIGN_AFTER,
};
use super::mb_sync::MbRgbTargetMap;
use super::transport::{open_any, with_transport_recovery};
use super::{
    CMD_RESET, CMD_RX_LCD_MODE, CMD_RX_QUERY_34, CMD_RX_QUERY_37, CMD_VIDEO_START, RF_CHUNKS,
    RF_CHUNK_SIZE, RF_DATA_SIZE, RF_SELECT, RX_IDS, TX_IDS, USB_CMD_GET_MAC, USB_CMD_SEND_RF,
};
use anyhow::{bail, Context, Result};
use lianli_transport::usb::{RusbBulk, USB_TIMEOUT};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

const TX_FAILURE_THRESHOLD: u32 = 5;

pub struct WirelessController {
    pub(super) tx: Option<Arc<Mutex<RusbBulk>>>,
    pub(super) rx: Option<Arc<Mutex<RusbBulk>>>,
    pub(super) receiver_state: Arc<ReceiverState>,
    pub(super) rx_running: Arc<AtomicBool>,
    pub(super) poll_stop: Arc<AtomicBool>,
    pub(super) poll_thread: Option<JoinHandle<()>>,
    pub(super) video_mode_active: Arc<AtomicBool>,
    pub(super) master_mac: Arc<Mutex<[u8; 6]>>,
    pub(super) master_channel: Arc<Mutex<u8>>,
    pub(super) discovered_devices: Arc<Mutex<Vec<DiscoveredDevice>>>,
    pub(super) device_health: DeviceHealthMap,
    pub(super) master_entries: MasterEntryMap,
    pub(super) clock_init_sent: Arc<AtomicBool>,
    pub(super) fg_sync: Arc<AtomicBool>,
    pub(super) tx_failures: Arc<AtomicU32>,
    pub(super) desired_effects: Arc<Mutex<std::collections::HashMap<[u8; 6], [u8; 4]>>>,
    pub(super) mb_rgb_targets: MbRgbTargetMap,
    pub(super) command_order: Arc<Mutex<()>>,
    pub(super) binding_mac: Arc<Mutex<Option<[u8; 6]>>>,
    /// Pending commands awaiting device ack, drained by the convergence loop.
    pub(super) pending_commands: Option<PendingQueue>,
    /// Per-device target `cmd_seq`; incremented per state-changing command.
    pub(super) target_cmd_seqs: Option<TargetSeqMap>,
    pub(super) convergence_thread: Option<JoinHandle<()>>,
}

impl Clone for WirelessController {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            rx: self.rx.clone(),
            receiver_state: Arc::clone(&self.receiver_state),
            rx_running: Arc::clone(&self.rx_running),
            poll_stop: Arc::clone(&self.poll_stop),
            poll_thread: None,
            video_mode_active: Arc::clone(&self.video_mode_active),
            master_mac: Arc::clone(&self.master_mac),
            master_channel: Arc::clone(&self.master_channel),
            discovered_devices: Arc::clone(&self.discovered_devices),
            device_health: Arc::clone(&self.device_health),
            master_entries: Arc::clone(&self.master_entries),
            clock_init_sent: Arc::clone(&self.clock_init_sent),
            fg_sync: Arc::clone(&self.fg_sync),
            tx_failures: Arc::clone(&self.tx_failures),
            desired_effects: Arc::clone(&self.desired_effects),
            mb_rgb_targets: Arc::clone(&self.mb_rgb_targets),
            command_order: Arc::clone(&self.command_order),
            binding_mac: Arc::clone(&self.binding_mac),
            pending_commands: self.pending_commands.clone(),
            target_cmd_seqs: self.target_cmd_seqs.clone(),
            convergence_thread: None,
        }
    }
}

impl WirelessController {
    pub fn new() -> Self {
        let pending_commands: PendingQueue = Arc::new(Mutex::new(Default::default()));
        let target_cmd_seqs: TargetSeqMap = Arc::new(Mutex::new(Default::default()));
        Self {
            tx: None,
            rx: None,
            receiver_state: Arc::new(ReceiverState::default()),
            rx_running: Arc::new(AtomicBool::new(false)),
            poll_stop: Arc::new(AtomicBool::new(false)),
            poll_thread: None,
            video_mode_active: Arc::new(AtomicBool::new(false)),
            master_mac: Arc::new(Mutex::new([0u8; 6])),
            master_channel: Arc::new(Mutex::new(8)),
            discovered_devices: Arc::new(Mutex::new(Vec::new())),
            device_health: Arc::new(Mutex::new(Default::default())),
            master_entries: Arc::new(Mutex::new(Default::default())),
            clock_init_sent: Arc::new(AtomicBool::new(false)),
            fg_sync: Arc::new(AtomicBool::new(false)),
            tx_failures: Arc::new(AtomicU32::new(0)),
            desired_effects: Arc::new(Mutex::new(std::collections::HashMap::new())),
            mb_rgb_targets: Arc::new(Mutex::new(Default::default())),
            command_order: Arc::new(Mutex::new(())),
            binding_mac: Arc::new(Mutex::new(None)),
            pending_commands: Some(pending_commands),
            target_cmd_seqs: Some(target_cmd_seqs),
            convergence_thread: None,
        }
    }

    pub fn connect(&mut self) -> Result<()> {
        self.stop();
        self.poll_stop = Arc::new(AtomicBool::new(false));
        self.receiver_state.pages.store(1, Ordering::Relaxed);
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

        let mut rx = open_any(&RX_IDS).context("opening wireless RX dongle")?;
        rx.detach_and_configure("RX")?;
        rx.read_flush();
        self.tx = Some(tx_arc);
        self.rx = Some(Arc::new(Mutex::new(rx)));
        self.tx_failures.store(0, Ordering::Relaxed);

        if let Err(error) = self.discover_master_mac() {
            self.stop();
            return Err(error);
        }
        Ok(())
    }

    /// Discovers master MAC address and channel by querying TX with USB_GetMac.
    /// Tries the default channel first, then scans even, then odd as fallback.
    fn discover_master_mac(&self) -> Result<()> {
        let tx = self.tx.as_ref().context("TX device not available")?;
        info!("Discovering master MAC address and wireless channel...");

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

            if valid_master_response(&response[..len]) {
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
        anyhow::ensure!(
            self.poll_thread.is_none() && self.convergence_thread.is_none(),
            "wireless workers already started"
        );
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

        thread::sleep(Duration::from_millis(500));

        self.video_mode_active.store(false, Ordering::Release);
        self.poll_stop.store(false, Ordering::SeqCst);
        self.clock_init_sent.store(false, Ordering::Release);

        let stop_flag = self.poll_stop.clone();
        let discovered_devices = Arc::clone(&self.discovered_devices);
        let device_health = Arc::clone(&self.device_health);
        let master_entries = Arc::clone(&self.master_entries);
        let receiver_state = Arc::clone(&self.receiver_state);
        let fg_sync = Arc::clone(&self.fg_sync);
        let master_mac = Arc::clone(&self.master_mac);
        let retarget_ctrl = self.clone();
        let rx_running = Arc::clone(&self.rx_running);
        rx_running.store(true, Ordering::Release);

        let discovery_done = Arc::new(AtomicBool::new(false));
        let discovery_signal = discovery_done.clone();

        self.poll_thread = Some(thread::spawn(move || {
            let mut found_devices = false;
            let mut consecutive_errors = 0u32;
            let mut consecutive_successes = 0u32;
            let mut total_resets = 0u32;
            let mut last_retarget = Instant::now();
            const MAX_RESETS: u32 = 3;
            while !stop_flag.load(Ordering::SeqCst) {
                if let Err(err) = poll_and_discover(
                    &rx,
                    &discovered_devices,
                    &device_health,
                    &master_entries,
                    &receiver_state,
                    &fg_sync,
                    &master_mac,
                ) {
                    consecutive_errors += 1;
                    consecutive_successes = 0;
                    info!("RX polling ({consecutive_errors}): {err:#}, continuing");
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
                        wait_for_poll(&stop_flag, Duration::from_millis(500));
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
                    wait_for_poll(&stop_flag, backoff);
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
                if last_retarget.elapsed() >= Duration::from_secs(1) {
                    last_retarget = Instant::now();
                    retarget_ctrl.retarget_mischannelled_devices();
                }
                wait_for_poll(&stop_flag, Duration::from_millis(500));
            }
            rx_running.store(false, Ordering::Release);
        }));

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

        if let Some(queue) = self.pending_commands.clone() {
            let conv_stop = self.poll_stop.clone();
            let device_health = Arc::clone(&self.device_health);
            self.convergence_thread = Some(Self::spawn_convergence_loop(
                Arc::clone(&tx),
                queue,
                device_health,
                Arc::clone(&self.desired_effects),
                Arc::clone(&self.binding_mac),
                conv_stop,
            ));
        }

        Ok(())
    }

    pub fn ensure_video_mode(&self) -> Result<()> {
        if self.video_mode_active.load(Ordering::Acquire) {
            return Ok(());
        }

        if self.tx.is_some() {
            let device_count = self.discovered_devices.lock().len().max(1);
            let master_ch = *self.master_channel.lock();
            self.tx_recover(|handle| {
                handle
                    .write(&CMD_VIDEO_START, USB_TIMEOUT)
                    .context("sending TX video start")?;
                thread::sleep(Duration::from_millis(2));
                for device_idx in 0..device_count {
                    let mut cmd = vec![0u8; 64];
                    cmd[0] = USB_CMD_SEND_RF;
                    cmd[1] = device_idx as u8;
                    cmd[2] = master_ch;
                    cmd[3] = 0xFF;
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

    /// Broadcast a "master clock" sync packet (RF sub-command 0x14) carrying
    /// 220 bytes of CPU/GPU info. This must be sent once per second; missing
    /// it appears to put the fan firmware into an autonomous fallback that
    /// occasionally spikes RPM.
    pub fn send_master_clock(&self, payload: &[u8; 220]) -> Result<()> {
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = super::RF_CLOCK_SYNC;
        rf_data[8..14].copy_from_slice(&master_mac);
        let init = !self.clock_init_sent.load(Ordering::Acquire);
        if init {
            // vendor init frame: fixedData region carries the 0x14 "unset" sentinel
            rf_data[14..64].fill(0x14);
            rf_data[64..234].copy_from_slice(&payload[50..220]);
        } else {
            rf_data[14..234].copy_from_slice(payload);
        }

        self.tx_recover(|handle| {
            for chunk_idx in 0..RF_CHUNKS as u8 {
                let mut packet = vec![0u8; 64];
                packet[0] = USB_CMD_SEND_RF;
                packet[1] = chunk_idx;
                packet[2] = master_ch;
                packet[3] = 0xFF;

                let start = chunk_idx as usize * RF_CHUNK_SIZE;
                let end = start + RF_CHUNK_SIZE;
                packet[4..64].copy_from_slice(&rf_data[start..end]);

                handle
                    .write(&packet, USB_TIMEOUT)
                    .context("sending master clock packet")?;
                thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        })?;
        if init {
            self.clock_init_sent.store(true, Ordering::Release);
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
                with_transport_recovery(rx, &RX_IDS, "RX", Some(&self.poll_stop), |handle| {
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

    pub fn is_connected(&self) -> bool {
        self.tx.is_some()
            && self.rx.is_some()
            && !self.poll_stop.load(Ordering::Acquire)
            && self.rx_running.load(Ordering::Acquire)
            && self.tx_failures.load(Ordering::Relaxed) < TX_FAILURE_THRESHOLD
    }

    /// Returns true if any wireless device's currently-running effect_index
    /// drifted away from what we last sent. Indicates the device firmware
    /// reset its lighting state (e.g. idle watchdog) and we should re-apply.
    pub fn rgb_drifted(&self) -> bool {
        let desired = self.desired_effects.lock();
        if desired.is_empty() {
            return false;
        }
        let devices = self.discovered_devices.lock();
        devices.iter().any(|d| match desired.get(&d.mac) {
            Some(want) => d.effect_index != *want,
            None => false,
        })
    }

    pub(super) fn tx_recover<F, R>(&self, op: F) -> Result<R>
    where
        F: FnMut(&RusbBulk) -> Result<R>,
    {
        let tx = self.tx.as_ref().context("TX device not connected")?;
        let result = with_transport_recovery(tx, &TX_IDS, "TX", Some(&self.poll_stop), op);
        match &result {
            Ok(_) => self.tx_failures.store(0, Ordering::Relaxed),
            Err(_) => {
                let n = self.tx_failures.fetch_add(1, Ordering::Relaxed) + 1;
                if n == TX_FAILURE_THRESHOLD {
                    warn!("Wireless TX: {n} consecutive failures, marking disconnected");
                }
            }
        }
        result
    }

    pub fn has_discovered_devices(&self) -> bool {
        !self.discovered_devices.lock().is_empty()
    }

    pub fn discovered_device_count(&self) -> usize {
        self.discovered_devices.lock().len()
    }

    /// Snapshot of devices considered ours: binding intent or dongle-reported
    /// master match, restricted to live entries.
    pub fn devices(&self) -> Vec<DiscoveredDevice> {
        let local_mac = *self.master_mac.lock();
        self.discovered_devices
            .lock()
            .iter()
            .filter(|d| d.bind_intent || d.master_mac == local_mac)
            .cloned()
            .collect()
    }

    /// Snapshot of devices available for binding (observed foreign, no intent).
    pub fn unbound_devices(&self) -> Vec<DiscoveredDevice> {
        let local_mac = *self.master_mac.lock();
        self.discovered_devices
            .lock()
            .iter()
            .filter(|d| !d.bind_intent && d.master_mac != local_mac)
            .cloned()
            .collect()
    }

    /// MACs eligible for automatic recovery: live masterless devices that
    /// were not explicitly unbound by the user. Bound devices that drifted
    /// away must have been masterless for [`REBIND_FOREIGN_AFTER`] first.
    /// Devices owned by another controller are never candidates.
    pub fn rebind_candidates(&self) -> Vec<[u8; 6]> {
        self.device_health
            .lock()
            .iter()
            .filter(|(_, h)| {
                if h.dead || h.man_unbind || h.observed_master != [0u8; 6] {
                    return false;
                }
                if h.bind_intent {
                    h.foreign_since
                        .is_some_and(|t| t.elapsed() >= REBIND_FOREIGN_AFTER)
                } else {
                    true
                }
            })
            .map(|(mac, _)| *mac)
            .collect()
    }

    pub(super) fn confirm_binding(&self, mac: &[u8; 6], intent: bool) {
        let mut health = self.device_health.lock();
        let Some(h) = health.get_mut(mac) else { return };
        h.bind_intent = intent;
        h.man_unbind = !intent;
        h.foreign_since = None;
        h.observed_master = h.raw_master;
        h.published.bind_intent = intent;
        h.published.master_mac = h.raw_master;
        h.published.rx_type = h.raw_rx;
        h.published.channel = h.raw_channel;
        if let Some(device) = self
            .discovered_devices
            .lock()
            .iter_mut()
            .find(|d| d.mac == *mac)
        {
            *device = h.published.clone();
        }
    }

    /// True when the given master MAC belongs to another dongle that is
    /// currently live on our channel.
    pub fn foreign_master_online(&self, master: &[u8; 6]) -> bool {
        let local = *self.master_mac.lock();
        if *master == [0u8; 6] || *master == local {
            return false;
        }
        self.master_entries.lock().contains_key(master)
    }

    /// MAC of the master a device currently reports, straight from the last
    /// GetDev sighting.
    pub(super) fn observed_master_of(&self, mac: &[u8; 6]) -> Option<[u8; 6]> {
        self.device_health.lock().get(mac).map(|h| h.raw_master)
    }

    /// Channel our dongle should occupy given the masters visible via
    /// GetDev. Masters sort by MAC and take channels 8, 12, 16 ... so two
    /// controllers in radio range never share one. Our own dongle also
    /// appears as a master record, so a move is only warranted when at
    /// least one FOREIGN master is live. A lone dongle stays on whatever
    /// channel the pair runs on.
    pub fn arbitration_target(&self) -> Option<u8> {
        let current = *self.master_channel.lock();
        if !current.is_multiple_of(2) {
            return None;
        }
        let ours = *self.master_mac.lock();
        if ours == [0u8; 6] {
            return None;
        }
        let mut macs: Vec<[u8; 6]> = {
            let masters = self.master_entries.lock();
            masters.keys().copied().filter(|m| *m != ours).collect()
        };
        if macs.is_empty() {
            return None;
        }
        let others: Vec<([u8; 6], u8)> = {
            let masters = self.master_entries.lock();
            macs.iter()
                .filter_map(|m| masters.get(m).map(|e| (*m, e.channel)))
                .collect()
        };
        macs.push(ours);
        macs.sort_unstable();
        let idx = macs.iter().position(|m| *m == ours)?;
        let target = (8 + idx * 4) as u8;
        if target > 38 || target == current {
            return None;
        }
        info!(
            "{:?} foreign master dongles in range {:02x?}, moving to channel {target}",
            others.len(),
            others
        );
        Some(target)
    }

    /// Move the dongle and all bound devices to a new channel, mirroring the
    /// vendor: host state now, one bind packet per device carrying the new
    /// channel, and the poll loop keeps re pushing until every device
    /// reports it. Devices remain controllable while straddling channels
    /// because every RF send is routed on the device s own channel.
    pub fn switch_channel(&self, target: u8) -> Result<()> {
        if !(1..=39).contains(&target) {
            bail!("invalid channel {target}");
        }
        let current = *self.master_channel.lock();
        if current == target {
            return Ok(());
        }
        *self.master_channel.lock() = target;
        info!("switching wireless channel {current} -> {target}");
        self.retarget_mischannelled_devices();
        Ok(())
    }

    /// Send one bind packet to every bound device that does not yet report
    /// the master channel. Vendor firmware applies the channel from these
    /// packets lazily, so this runs on the poll cadence without a deadline.
    pub(super) fn retarget_mischannelled_devices(&self) {
        if self.binding_mac.lock().is_some() {
            return;
        }
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();
        let targets: Vec<([u8; 6], u8)> = {
            let health = self.device_health.lock();
            health
                .iter()
                .filter(|(_, h)| {
                    h.bind_intent
                        && !h.dead
                        && h.raw_master == master_mac
                        && h.raw_rx != 0
                        && h.raw_channel != master_ch
                })
                .map(|(mac, h)| (*mac, h.raw_rx))
                .collect()
        };
        for (mac, rx) in targets {
            if let Err(e) = self.send_bind_packet(&mac, &master_mac, rx) {
                debug!("channel retarget for {:02x?} failed: {e:#}", mac);
            }
        }
    }

    pub fn device_by_mac(&self, mac: &[u8; 6]) -> Option<DiscoveredDevice> {
        self.discovered_devices
            .lock()
            .iter()
            .find(|d| &d.mac == mac)
            .cloned()
    }

    /// Current motherboard PWM duty cycle (0-255), or `None` if unavailable.
    /// Extracted from RX GetDev response bytes [2:3] during polling.
    pub fn motherboard_pwm(&self) -> Option<u8> {
        match self.receiver_state.pwm.load(Ordering::Relaxed) {
            0xFFFF => None,
            v => Some(v as u8),
        }
    }

    /// Enable/disable FgSync mode. When enabled, the GetDev polling command
    /// includes the current fan RPM so the RX dongle can measure motherboard
    /// PWM duty from the FG signal.
    pub fn set_fg_sync(&self, enabled: bool) {
        self.fg_sync.store(enabled, Ordering::Relaxed);
    }

    /// Send a 240-byte RF packet as 4× 64-byte USB chunks.
    pub(super) fn send_rf_packet(
        &self,
        handle: &RusbBulk,
        device: &DiscoveredDevice,
        rf_data: &[u8],
    ) -> Result<()> {
        anyhow::ensure!(
            rf_data.len() == RF_DATA_SIZE,
            "invalid RGB RF packet length"
        );
        for chunk_idx in 0..RF_CHUNKS as u8 {
            let mut packet = [0u8; 64];
            packet[0] = USB_CMD_SEND_RF;
            packet[1] = chunk_idx;
            packet[2] = device.channel;
            packet[3] = device.rx_type;

            let start = chunk_idx as usize * RF_CHUNK_SIZE;
            let end = start + RF_CHUNK_SIZE;
            packet[4..64].copy_from_slice(&rf_data[start..end]);

            let written = handle
                .write(&packet, USB_TIMEOUT)
                .context("sending RGB RF packet")?;
            anyhow::ensure!(
                written == packet.len(),
                "short RGB RF packet write: {written}/64 bytes"
            );
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    pub(super) fn next_slot_index(&self, device: &DiscoveredDevice) -> u8 {
        let devices = self.discovered_devices.lock();
        let master_mac = *self.master_mac.lock();
        let mut next_slot = 1u8;
        for bound in devices
            .iter()
            .filter(|d| (d.bind_intent || d.master_mac == master_mac) && d.device_type != 0xFF)
        {
            if bound.mac == device.mac {
                return next_slot;
            }
            next_slot = next_slot.saturating_add(1);
        }
        next_slot
    }

    pub(super) fn device_by_mac_snapshot(&self, mac: &[u8; 6]) -> Result<DiscoveredDevice> {
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
        let owns_runtime = self.poll_thread.is_some() || self.convergence_thread.is_some();
        if owns_runtime {
            self.mb_rgb_targets.lock().clear();
            self.poll_stop.store(true, Ordering::SeqCst);
            for handle in [&self.poll_thread, &self.convergence_thread]
                .into_iter()
                .flatten()
            {
                handle.thread().unpark();
            }
            if let Some(handle) = self.poll_thread.take() {
                let _ = handle.join();
            }
            if let Some(handle) = self.convergence_thread.take() {
                let _ = handle.join();
            }
            self.rx_running.store(false, Ordering::Release);
            self.receiver_state.pwm.store(0xFFFF, Ordering::Relaxed);
            if let Some(queue) = &self.pending_commands {
                queue.lock().clear();
            }
        }
        self.tx.take();
        self.rx.take();
    }

    pub fn rgb_upload_applied(&self, mac: &[u8; 6], effect_index: [u8; 4]) -> bool {
        // A matching firmware index can survive a daemon restart without a local upload.
        if self.desired_effects.lock().get(mac) != Some(&effect_index) {
            return false;
        }
        let observed = self.device_health.lock().get(mac).is_some_and(|health| {
            health.raw_seen.elapsed() <= ACK_FRESHNESS
                && !health.published.is_sync_mb_light
                && health.published.effect_index == effect_index
        });
        observed && !self.has_pending_mb_rgb_transition(mac)
    }
}

fn valid_master_response(response: &[u8]) -> bool {
    response.len() >= 13
        && response[0] == USB_CMD_GET_MAC
        && response[1..7].iter().any(|&b| b != 0)
        && u32::from_be_bytes(response[7..11].try_into().unwrap()) > 1
}

fn wait_for_poll(stop: &AtomicBool, duration: Duration) {
    let deadline = Instant::now() + duration;
    while !stop.load(Ordering::Acquire) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        thread::park_timeout(remaining);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wireless::discovery::{DeviceHealth, DiscoveredDevice};
    use crate::wireless::WirelessFanType;
    use std::collections::BTreeMap;

    #[test]
    fn new_binding_uses_next_slot_without_claiming_ownership_first() {
        let controller = WirelessController::new();
        *controller.master_mac.lock() = [9; 6];
        let first = entry([9; 6]).published;
        let mut second = first.clone();
        second.mac = [2; 6];
        let mut unbound = entry([0; 6]).published;
        unbound.mac = [3; 6];
        *controller.discovered_devices.lock() =
            vec![first.clone(), second.clone(), unbound.clone()];
        assert_eq!(controller.next_slot_index(&first), 1);
        assert_eq!(controller.next_slot_index(&second), 2);
        assert_eq!(controller.next_slot_index(&unbound), 3);
        assert!(!unbound.bind_intent);
    }

    #[test]
    fn stop_joins_both_workers_and_clone_drop_does_not_stop_owner() {
        let mut controller = WirelessController::new();
        let finished = Arc::new(AtomicU32::new(0));
        for slot in [
            &mut controller.poll_thread,
            &mut controller.convergence_thread,
        ] {
            let stop = Arc::clone(&controller.poll_stop);
            let finished = Arc::clone(&finished);
            *slot = Some(thread::spawn(move || {
                wait_for_poll(&stop, Duration::from_secs(30));
                finished.fetch_add(1, Ordering::Release);
            }));
        }
        drop(controller.clone());
        assert!(!controller.poll_stop.load(Ordering::Acquire));
        controller.stop();
        assert_eq!(finished.load(Ordering::Acquire), 2);
        assert!(controller.poll_thread.is_none());
        assert!(controller.convergence_thread.is_none());
        controller.stop();
    }

    #[test]
    fn convergence_only_runtime_is_stopped_and_joined() {
        let mut controller = WirelessController::new();
        let stop = Arc::clone(&controller.poll_stop);
        controller.convergence_thread = Some(thread::spawn(move || {
            wait_for_poll(&stop, Duration::from_secs(30));
        }));
        controller.stop();
        assert!(controller.poll_stop.load(Ordering::Acquire));
        assert!(controller.convergence_thread.is_none());
    }

    #[test]
    fn master_discovery_rejects_missing_clock_and_truncated_identity() {
        let mut response = [0x11, 1, 2, 3, 4, 5, 6, 0, 0, 0, 2, 1, 2];
        assert!(valid_master_response(&response));
        assert!(!valid_master_response(&response[..7]));
        response[10] = 0;
        assert!(!valid_master_response(&response));
        response[10] = 1;
        assert!(!valid_master_response(&response));
        response[10] = 2;
        response[1..7].fill(0);
        assert!(!valid_master_response(&response));
    }

    fn entry(master: [u8; 6]) -> DeviceHealth {
        let rec = DiscoveredDevice {
            mac: [1, 2, 3, 4, 5, 6],
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
            fan_type: WirelessFanType::Slv3Led,
            list_index: 0,
            coolant_temp_c: None,
            effect_index: [0; 4],
            is_sync_mb_light: false,
            is_pwm_line_on: false,
            bind_intent: false,
        };
        let mut h = DeviceHealth::new(rec);
        h.observed_master = master;
        h.raw_master = master;
        h
    }

    #[test]
    fn matching_effect_is_not_applied_during_motherboard_sync_or_transition() {
        let controller = WirelessController::new();
        controller
            .desired_effects
            .lock()
            .insert([1, 2, 3, 4, 5, 6], [7; 4]);
        let mut health = entry([9; 6]);
        health.published.effect_index = [7; 4];
        health.published.is_sync_mb_light = true;
        health.raw_seen = Instant::now();
        controller
            .device_health
            .lock()
            .insert([1, 2, 3, 4, 5, 6], health);

        assert!(!controller.rgb_upload_applied(&[1, 2, 3, 4, 5, 6], [7; 4]));
        controller
            .device_health
            .lock()
            .get_mut(&[1, 2, 3, 4, 5, 6])
            .unwrap()
            .published
            .is_sync_mb_light = false;
        assert!(controller.rgb_upload_applied(&[1, 2, 3, 4, 5, 6], [7; 4]));

        let now = Instant::now();
        controller.reserve_mb_rgb_transition(&[1, 2, 3, 4, 5, 6], false, true, 0, now);
        assert!(!controller.rgb_upload_applied(&[1, 2, 3, 4, 5, 6], [7; 4]));
    }

    #[test]
    fn retained_firmware_effect_requires_an_upload_in_this_session() {
        let controller = WirelessController::new();
        let mac = [1, 2, 3, 4, 5, 6];
        let mut health = entry([9; 6]);
        health.published.effect_index = [7; 4];
        health.raw_seen = Instant::now();
        controller.device_health.lock().insert(mac, health);

        assert!(!controller.rgb_upload_applied(&mac, [7; 4]));
        controller.desired_effects.lock().insert(mac, [7; 4]);
        assert!(controller.rgb_upload_applied(&mac, [7; 4]));
        assert!(!controller.rgb_upload_applied(&mac, [8; 4]));
        controller.clear_rgb_targets();
        assert!(!controller.rgb_upload_applied(&mac, [7; 4]));
    }

    fn controller_with_health(entries: Vec<([u8; 6], DeviceHealth)>) -> WirelessController {
        let c = WirelessController::new();
        *c.master_mac.lock() = [9u8; 6];
        let mut health: BTreeMap<[u8; 6], DeviceHealth> = BTreeMap::new();
        for (mac, h) in entries {
            health.insert(mac, h);
        }
        *c.device_health.lock() = health;
        c
    }

    fn mac() -> [u8; 6] {
        [1, 2, 3, 4, 5, 6]
    }

    #[test]
    fn masterless_intent_needs_timer() {
        let mut h = entry([0u8; 6]);
        h.bind_intent = true;
        h.foreign_since = Some(std::time::Instant::now());
        let c = controller_with_health(vec![(mac(), h)]);
        assert!(c.rebind_candidates().is_empty());

        let mut h = entry([0u8; 6]);
        h.bind_intent = true;
        h.foreign_since = Some(
            std::time::Instant::now()
                .checked_sub(REBIND_FOREIGN_AFTER + Duration::from_secs(1))
                .unwrap(),
        );
        let c = controller_with_health(vec![(mac(), h)]);
        assert_eq!(c.rebind_candidates(), vec![mac()]);
    }

    #[test]
    fn masterless_without_intent_is_candidate() {
        let c = controller_with_health(vec![(mac(), entry([0u8; 6]))]);
        assert_eq!(c.rebind_candidates(), vec![mac()]);
    }

    #[test]
    fn foreign_owned_and_manual_unbind_are_not_candidates() {
        let mut foreign = entry([7u8; 6]);
        foreign.foreign_since = Some(
            std::time::Instant::now()
                .checked_sub(REBIND_FOREIGN_AFTER + Duration::from_secs(1))
                .unwrap(),
        );

        let mut unbound = entry([0u8; 6]);
        unbound.man_unbind = true;

        let mut dead = entry([0u8; 6]);
        dead.dead = true;

        let healthy = entry([9u8; 6]);

        let c = controller_with_health(vec![
            ([1, 2, 3, 4, 5, 6], foreign),
            ([2, 2, 3, 4, 5, 6], unbound),
            ([3, 2, 3, 4, 5, 6], dead),
            ([4, 2, 3, 4, 5, 6], healthy),
        ]);
        assert!(c.rebind_candidates().is_empty());
    }

    #[test]
    fn observed_master_of_reads_raw_master() {
        let c = controller_with_health(vec![(mac(), entry([7u8; 6]))]);
        assert_eq!(c.observed_master_of(&mac()), Some([7u8; 6]));
        assert_eq!(c.observed_master_of(&[9, 9, 9, 9, 9, 9]), None);
    }

    #[test]
    fn arbitration_targets_spread_masters_by_mac() {
        let c = WirelessController::new();
        *c.master_mac.lock() = [9u8; 6];
        assert_eq!(c.arbitration_target(), None);

        let mut masters = c.master_entries.lock();
        masters.insert(
            [7u8; 6],
            crate::wireless::discovery::MasterEntry {
                channel: 8,
                last_seen: std::time::Instant::now(),
            },
        );
        drop(masters);
        assert_eq!(c.arbitration_target(), Some(12));

        *c.master_channel.lock() = 12;
        assert_eq!(c.arbitration_target(), None);

        *c.master_channel.lock() = 7;
        assert_eq!(c.arbitration_target(), None);
    }

    #[test]
    fn lone_dongle_is_never_moved() {
        let c = WirelessController::new();
        *c.master_mac.lock() = [9u8; 6];
        *c.master_channel.lock() = 2;
        assert_eq!(c.arbitration_target(), None);

        // Defense in depth: merge skips our own record now, but a stale or
        // directly inserted own entry must not count as a conflict either.
        c.master_entries.lock().insert(
            [9u8; 6],
            crate::wireless::discovery::MasterEntry {
                channel: 2,
                last_seen: std::time::Instant::now(),
            },
        );
        assert_eq!(c.arbitration_target(), None);
    }

    #[test]
    fn arbitration_slot_follows_mac_order() {
        let c = WirelessController::new();
        *c.master_mac.lock() = [8u8; 6];
        let mut masters = c.master_entries.lock();
        for m in [[7u8; 6], [9u8; 6]] {
            masters.insert(
                m,
                crate::wireless::discovery::MasterEntry {
                    channel: 8,
                    last_seen: std::time::Instant::now(),
                },
            );
        }
        drop(masters);
        assert_eq!(c.arbitration_target(), Some(12));
    }
}

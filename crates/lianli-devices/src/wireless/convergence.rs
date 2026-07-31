use super::controller::WirelessController;
use super::discovery::DiscoveredDevice;
use super::{RF_CHUNKS, RF_CHUNK_SIZE, RF_DATA_SIZE, USB_CMD_SEND_RF};
use anyhow::{Context, Result};
use lianli_transport::usb::{RusbBulk, USB_TIMEOUT};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Initial retry budget for a pending command. The convergence loop re-sends
/// every [`TICK_INTERVAL`] until the device acknowledges or the budget is
/// exhausted. On exhaustion the command is silently force-acknowledged — it
/// is never dropped with a warning.
const INITIAL_RETRIES: u32 = 10;

/// Convergence loop tick interval. Commands are re-sent on each tick until
/// acknowledged. 10 retries × 100 ms = 1 s hard timeout.
const TICK_INTERVAL: Duration = Duration::from_millis(100);

/// How a pending command is acknowledged. Different RF command types use
/// different ack signals.
pub(super) enum AckSignal {
    /// Device's reported `current_pwm` must match.
    Pwm([u8; 4]),
    /// Device's reported `cmd_seq` must reach this value.
    CmdSeq(u8),
}

/// A command awaiting device acknowledgment.
pub(super) struct PendingCommand {
    pub mac: [u8; 6],
    pub channel: u8,
    pub rx_type: u8,
    pub rf_data: Vec<u8>,
    pub ack: AckSignal,
    pub remaining_retries: u32,
    pub last_sent: Instant,
    pub description: String,
}

impl PendingCommand {
    fn mac_str(&self) -> String {
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.mac[0], self.mac[1], self.mac[2], self.mac[3], self.mac[4], self.mac[5],
        )
    }
}

pub(super) type PendingQueue = Arc<Mutex<VecDeque<PendingCommand>>>;

/// Per-device target `cmd_seq`. The host increments this on each state-changing
/// command; the device echoes its applied `cmd_seq` in the GetDev record. When
/// they match, the command is acknowledged.
pub(super) type TargetSeqMap = Arc<Mutex<std::collections::HashMap<[u8; 6], u8>>>;

impl WirelessController {
    /// Enqueue a state-changing RF command for convergence-tracked delivery.
    /// Sends the command immediately, then the background loop re-sends every
    /// [`TICK_INTERVAL`] until the device acknowledges or the retry budget runs out.
    pub(super) fn enqueue_rf_command(
        &self,
        device: &DiscoveredDevice,
        rf_data: Vec<u8>,
        ack: AckSignal,
        description: impl Into<String>,
    ) {
        if let Some(queue) = self.pending_commands.as_ref() {
            let cmd = PendingCommand {
                mac: device.mac,
                channel: device.channel,
                rx_type: device.rx_type,
                rf_data,
                ack,
                remaining_retries: INITIAL_RETRIES,
                last_sent: Instant::now(),
                description: description.into(),
            };

            if let Err(e) = self.send_command_once(&cmd) {
                warn!("initial send failed for {}: {e:#}", cmd.mac_str());
            }

            queue.lock().push_back(cmd);
        }
    }

    /// Compute the next target `cmd_seq` for a device. Seeded from the device's
    /// last reported `cmd_seq`; increments by 1 for each queued command so rapid
    /// successive sends each get a distinct target.
    pub(super) fn bump_target_cmd_seq(&self, mac: &[u8; 6], device_cmd_seq: u8) -> u8 {
        if let Some(map) = self.target_cmd_seqs.as_ref() {
            let mut guard = map.lock();
            let next = match guard.get(mac) {
                Some(&last_pending) if last_pending >= device_cmd_seq => {
                    last_pending.wrapping_add(1)
                }
                _ => device_cmd_seq.wrapping_add(1),
            };
            let next = if next == 0 { 1 } else { next };
            guard.insert(*mac, next);
            next
        } else {
            device_cmd_seq.wrapping_add(1).max(1)
        }
    }

    /// Send a single RF command packet set (4 × 64-byte USB chunks).
    fn send_command_once(&self, cmd: &PendingCommand) -> Result<()> {
        self.tx_recover(|handle| {
            send_rf_frame(handle, &cmd.channel, &cmd.rx_type, &cmd.rf_data)?;
            Ok(())
        })
    }

    /// Spawn the convergence TX loop. Runs until `stop` is set; re-sends any
    /// pending command whose `target_cmd_seq` has not yet been acknowledged by
    /// the device.
    pub(super) fn spawn_convergence_loop(
        tx: Arc<Mutex<RusbBulk>>,
        queue: PendingQueue,
        target_seqs: TargetSeqMap,
        discovered: Arc<Mutex<Vec<DiscoveredDevice>>>,
        stop: Arc<AtomicBool>,
    ) -> thread::JoinHandle<()> {
        thread::Builder::new()
            .name("wireless-convergence".into())
            .spawn(move || {
                info!("wireless convergence loop started (100ms tick)");
                while !stop.load(Ordering::SeqCst) {
                    let tick_start = Instant::now();

                    drain_pending(&tx, &queue, &target_seqs, &discovered, &stop);

                    let elapsed = tick_start.elapsed();
                    if elapsed < TICK_INTERVAL {
                        thread::sleep(TICK_INTERVAL - elapsed);
                    }
                }
                debug!("wireless convergence loop stopped");
            })
            .expect("spawning convergence thread")
    }
}

fn drain_pending(
    tx: &Arc<Mutex<RusbBulk>>,
    queue: &PendingQueue,
    _target_seqs: &TargetSeqMap,
    discovered: &Arc<Mutex<Vec<DiscoveredDevice>>>,
    _stop: &Arc<AtomicBool>,
) {
    let mut guard = queue.lock();
    if guard.is_empty() {
        return;
    }

    let devices = discovered.lock();
    let now = Instant::now();

    let mut retain = VecDeque::with_capacity(guard.len());
    while let Some(mut cmd) = guard.pop_front() {
        let acked = devices
            .iter()
            .find(|d| d.mac == cmd.mac)
            .map(|d| match &cmd.ack {
                AckSignal::Pwm(target) => pwm_acked(&d.current_pwm, target),
                AckSignal::CmdSeq(target) => d.cmd_seq == *target,
            })
            .unwrap_or(false);

        if acked {
            debug!(
                "ack received for {} ({}) after {} retries",
                cmd.mac_str(),
                cmd.description,
                INITIAL_RETRIES.saturating_sub(cmd.remaining_retries),
            );
            continue;
        }

        let due = now.duration_since(cmd.last_sent) >= TICK_INTERVAL;
        if due {
            cmd.remaining_retries = cmd.remaining_retries.saturating_sub(1);
            if cmd.remaining_retries == 0 {
                debug!(
                    "force-ack {} ({}) after {} retries — giving up convergence",
                    cmd.mac_str(),
                    cmd.description,
                    INITIAL_RETRIES,
                );
                continue;
            }
            cmd.last_sent = now;
            if let Err(e) = send_rf_frame(&tx.lock(), &cmd.channel, &cmd.rx_type, &cmd.rf_data) {
                warn!(
                    "re-send failed for {} ({}): {e:#}",
                    cmd.mac_str(),
                    cmd.description,
                );
            }
        }
        retain.push_back(cmd);
    }
    *guard = retain;
}

fn send_rf_frame(handle: &RusbBulk, channel: &u8, rx_type: &u8, rf_data: &[u8]) -> Result<()> {
    assert_eq!(rf_data.len(), RF_DATA_SIZE);
    for chunk_idx in 0..RF_CHUNKS as u8 {
        let mut packet = [0u8; 64];
        packet[0] = USB_CMD_SEND_RF;
        packet[1] = chunk_idx;
        packet[2] = *channel;
        packet[3] = *rx_type;
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

/// Check whether the device's reported PWM values match the target. Allows a
/// tolerance of 5 because the device rounds to its nearest internal step.
fn pwm_acked(reported: &[u8; 4], target: &[u8; 4]) -> bool {
    reported
        .iter()
        .zip(target.iter())
        .all(|(r, t)| r.abs_diff(*t) <= 5 || (*t <= 10 && *r == *t))
}

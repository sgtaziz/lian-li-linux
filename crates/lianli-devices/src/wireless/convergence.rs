use super::controller::WirelessController;
use super::discovery::{DeviceHealthMap, DiscoveredDevice, ACK_FRESHNESS};
use super::transport::{with_transport_recovery, RecoveryBackoff, SharedTransport};
use super::{RF_CHUNKS, RF_CHUNK_SIZE, RF_DATA_SIZE, USB_CMD_SEND_RF};
use anyhow::{ensure, Context, Result};
use lianli_transport::usb::{RusbBulk, USB_TIMEOUT};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

const INITIAL_RETRIES: u32 = 10;

const TICK_INTERVAL: Duration = Duration::from_millis(100);
// RGB acknowledgement must not indefinitely suppress cooling keepalives.
const RGB_CONTROL_INTERVAL: Duration = Duration::from_secs(1);
const MAX_PENDING_COMMANDS: usize = 256;

pub(super) type BindingMac = Arc<Mutex<Option<[u8; 6]>>>;

pub(super) type RgbTargets = Arc<Mutex<std::collections::HashMap<[u8; 6], [u8; 4]>>>;

/// How a pending command is acknowledged. Different RF command types use
/// different ack signals.
#[derive(Clone)]
pub(super) enum AckSignal {
    /// Device's reported `current_pwm` must match.
    Pwm([u8; 4]),
    /// Device's reported `cmd_seq` must reach this value.
    CmdSeq(u8),
}

/// A command awaiting device acknowledgment.
#[derive(Clone)]
pub(super) struct PendingCommand {
    pub mac: [u8; 6],
    pub channel: u8,
    pub rx_type: u8,
    pub rf_data: Vec<u8>,
    pub ack: AckSignal,
    pub remaining_retries: u32,
    pub last_sent: Instant,
    queued_at: Instant,
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
    pub(super) fn enqueue_rf_command(
        &self,
        device: &DiscoveredDevice,
        rf_data: Vec<u8>,
        ack: AckSignal,
        description: impl Into<String>,
    ) -> Result<()> {
        let _order = self.command_order.lock();
        ensure!(self.tx.is_some(), "wireless TX is unavailable");
        ensure!(
            !self.poll_stop.load(Ordering::Acquire),
            "wireless controller is stopping"
        );
        ensure!(
            rf_data.len() == RF_DATA_SIZE,
            "invalid wireless command length"
        );
        ensure!(
            !binding_blocks_control(&self.binding_mac, &self.device_health, &device.mac),
            "wireless binding change prevents this control command"
        );
        let queue = self
            .pending_commands
            .as_ref()
            .context("wireless command queue is unavailable")?;
        let cmd = PendingCommand {
            mac: device.mac,
            channel: device.channel,
            rx_type: device.rx_type,
            rf_data,
            ack,
            remaining_retries: INITIAL_RETRIES,
            last_sent: Instant::now(),
            queued_at: Instant::now(),
            description: description.into(),
        };

        admit_command(&mut queue.lock(), cmd.clone())?;
        if let Err(e) = self.send_command_once(&cmd) {
            warn!("initial send failed for {}: {e:#}", cmd.mac_str());
        }
        Ok(())
    }

    pub(super) fn bump_target_cmd_seq(&self, mac: &[u8; 6], device_cmd_seq: u8) -> u8 {
        if let Some(map) = self.target_cmd_seqs.as_ref() {
            let mut guard = map.lock();
            let next = next_target_sequence(guard.get(mac).copied(), device_cmd_seq);
            guard.insert(*mac, next);
            next
        } else {
            next_target_sequence(None, device_cmd_seq)
        }
    }

    fn send_command_once(&self, cmd: &PendingCommand) -> Result<()> {
        self.tx_recover(|handle| {
            send_rf_frame(handle, &cmd.channel, &cmd.rx_type, &cmd.rf_data)?;
            Ok(())
        })
    }

    pub(super) fn spawn_convergence_loop(
        tx: SharedTransport,
        queue: PendingQueue,
        health_map: DeviceHealthMap,
        rgb_targets: RgbTargets,
        binding_mac: BindingMac,
        stop: Arc<AtomicBool>,
    ) -> Result<thread::JoinHandle<()>> {
        thread::Builder::new()
            .name("wireless-convergence".into())
            .spawn(move || {
                info!("wireless convergence loop started (100ms tick)");
                while !stop.load(Ordering::SeqCst) {
                    let tick_start = Instant::now();

                    drain_pending(&tx, &queue, &health_map, &rgb_targets, &binding_mac, &stop);

                    let elapsed = tick_start.elapsed();
                    if elapsed < TICK_INTERVAL {
                        thread::sleep(TICK_INTERVAL - elapsed);
                    }
                }
                debug!("wireless convergence loop stopped");
            })
            .context("spawning wireless convergence thread")
    }
}

fn next_target_sequence(previous: Option<u8>, observed: u8) -> u8 {
    match previous.unwrap_or(observed) {
        value @ 0..=253 => value + 1,
        _ => 1,
    }
}

fn drain_pending(
    tx: &SharedTransport,
    queue: &PendingQueue,
    health_map: &DeviceHealthMap,
    rgb_targets: &RgbTargets,
    binding_mac: &BindingMac,
    stop: &Arc<AtomicBool>,
) {
    if lianli_transport::usb::shutting_down() {
        return;
    }
    let mut commands = {
        let mut pending = queue.lock();
        std::mem::take(&mut *pending)
    };
    if commands.is_empty() {
        return;
    }

    let now = Instant::now();
    while let Some(mut cmd) = commands.pop_front() {
        if stop.load(Ordering::Acquire) || lianli_transport::usb::shutting_down() {
            break;
        }
        let target = rgb_targets.lock().get(&cmd.mac).copied();
        let (acked, changing_rgb) = {
            let health = health_map.lock();
            let recent = health
                .get(&cmd.mac)
                .filter(|h| h.raw_seen.elapsed() <= ACK_FRESHNESS);
            let acked = recent
                .map(|h| match &cmd.ack {
                    AckSignal::Pwm(target) => pwm_acked(&h.published.current_pwm, target),
                    AckSignal::CmdSeq(target) => h.published.cmd_seq == *target,
                })
                .unwrap_or(false);
            let changing_rgb = target
                .is_some_and(|target| recent.is_none_or(|h| h.published.effect_index != target));
            (acked, changing_rgb)
        };

        if acked {
            debug!(
                "ack received for {} ({}) after {} retries",
                cmd.mac_str(),
                cmd.description,
                INITIAL_RETRIES.saturating_sub(cmd.remaining_retries),
            );
            continue;
        }

        let due = retry_due(now.duration_since(cmd.last_sent), changing_rgb, &cmd.ack);
        if due {
            if cmd.remaining_retries == 0 {
                debug!(
                    "command remained unacknowledged for {} ({}) after {} retries",
                    cmd.mac_str(),
                    cmd.description,
                    INITIAL_RETRIES,
                );
                continue;
            }
            cmd.last_sent = now;
            let mut obsolete = false;
            let mut attempted = false;
            let result = with_transport_recovery(tx, &super::TX_IDS, "TX", stop, |handle| {
                if binding_blocks_control(binding_mac, health_map, &cmd.mac)
                    || superseded_command(&queue.lock(), &cmd)
                {
                    obsolete = true;
                    return Ok(());
                }
                attempted = true;
                send_rf_frame(handle, &cmd.channel, &cmd.rx_type, &cmd.rf_data)
            });
            if obsolete {
                continue;
            }
            account_retry(&mut cmd.remaining_retries, attempted, &result);
            if !result
                .as_ref()
                .is_err_and(|error| error.is::<RecoveryBackoff>())
            {
                if let Err(e) = result {
                    warn!(
                        "re-send failed for {} ({}): {e:#}",
                        cmd.mac_str(),
                        cmd.description,
                    );
                }
            }
        }
        let mut pending = queue.lock();
        let superseded = superseded_command(&pending, &cmd);
        if !superseded && pending.len() < MAX_PENDING_COMMANDS {
            pending.push_back(cmd);
        } else if !superseded {
            warn!(mac = ?cmd.mac, operation = %cmd.description, "Wireless command retry discarded because the queue is full");
        }
    }
}

fn account_retry(remaining: &mut u32, attempted: bool, result: &Result<()>) {
    if attempted
        || !result
            .as_ref()
            .is_err_and(|error| error.is::<RecoveryBackoff>())
    {
        *remaining = remaining.saturating_sub(1);
    }
}

fn binding_blocks_control(binding: &BindingMac, health: &DeviceHealthMap, mac: &[u8; 6]) -> bool {
    *binding.lock() == Some(*mac) || health.lock().get(mac).is_some_and(|h| h.man_unbind)
}

fn retry_due(elapsed: Duration, changing_rgb: bool, acknowledgement: &AckSignal) -> bool {
    elapsed
        >= if changing_rgb && matches!(acknowledgement, AckSignal::Pwm(_)) {
            RGB_CONTROL_INTERVAL
        } else {
            TICK_INTERVAL
        }
}

fn admit_command(pending: &mut VecDeque<PendingCommand>, command: PendingCommand) -> Result<()> {
    let replaces = pending.iter().any(|old| same_target_slot(old, &command));
    ensure!(
        pending.len() < MAX_PENDING_COMMANDS || replaces,
        "wireless command queue is full"
    );
    pending.retain(|old| !same_target_slot(old, &command));
    pending.push_back(command);
    Ok(())
}

fn same_target_slot(left: &PendingCommand, right: &PendingCommand) -> bool {
    left.mac == right.mac
        && match (&left.ack, &right.ack) {
            (AckSignal::Pwm(_), AckSignal::Pwm(_)) => true,
            (AckSignal::CmdSeq(_), AckSignal::CmdSeq(_)) => left.rf_data[1] == right.rf_data[1],
            _ => false,
        }
}

fn superseded_command(pending: &VecDeque<PendingCommand>, command: &PendingCommand) -> bool {
    pending
        .iter()
        .any(|new| same_target_slot(new, command) && new.queued_at > command.queued_at)
}

fn send_rf_frame(handle: &RusbBulk, channel: &u8, rx_type: &u8, rf_data: &[u8]) -> Result<()> {
    ensure!(
        rf_data.len() == RF_DATA_SIZE,
        "invalid wireless command length"
    );
    for chunk_idx in 0..RF_CHUNKS as u8 {
        let mut packet = [0u8; 64];
        packet[0] = USB_CMD_SEND_RF;
        packet[1] = chunk_idx;
        packet[2] = *channel;
        packet[3] = *rx_type;
        let start = chunk_idx as usize * RF_CHUNK_SIZE;
        let end = start + RF_CHUNK_SIZE;
        packet[4..64].copy_from_slice(&rf_data[start..end]);
        let written = handle
            .write(&packet, USB_TIMEOUT)
            .context("sending RF packet chunk")?;
        ensure!(
            written == packet.len(),
            "short wireless command write: {written}/64 bytes"
        );
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

/// Check whether the device's reported PWM values match the target. Allows a
/// tolerance of 5 because the device rounds to its nearest internal step.
fn pwm_acked(reported: &[u8; 4], target: &[u8; 4]) -> bool {
    reported.iter().zip(target.iter()).all(|(r, t)| {
        if *t <= 10 {
            *r == *t
        } else {
            r.abs_diff(*t) <= 5
        }
    })
}

#[cfg(test)]
#[path = "convergence_tests.rs"]
mod tests;

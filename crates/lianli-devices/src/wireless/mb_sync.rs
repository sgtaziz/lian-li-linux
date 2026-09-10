use super::controller::WirelessController;
use super::convergence::AckSignal;
use super::{RF_DATA_SIZE, RF_MB_LIGHT_SYNC, RF_SELECT};
use anyhow::{bail, Result};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::debug;

const MB_RGB_TRANSITION_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
pub(super) struct MbRgbTarget {
    enabled: bool,
    cmd_seq: u8,
    expires_at: Instant,
}

pub(super) type MbRgbTargetMap = Arc<Mutex<HashMap<[u8; 6], MbRgbTarget>>>;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum MbRgbReservation {
    Applied,
    Pending(u8),
    Start(u8),
}

impl WirelessController {
    pub fn set_mb_rgb_sync(&self, mac: &[u8; 6], enabled: bool) -> Result<()> {
        self.request_mb_rgb_sync(mac, enabled).map(|_| ())
    }

    fn request_mb_rgb_sync(&self, mac: &[u8; 6], enabled: bool) -> Result<MbRgbReservation> {
        let devices = self.discovered_devices.lock();
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        let device = devices.iter().find(|d| &d.mac == mac).cloned();
        let Some(device) = device else {
            drop(devices);
            self.forget_mb_rgb_target(mac);
            bail!("device not found for MB RGB sync");
        };

        let slave_index = devices
            .iter()
            .filter(|d| d.master_mac == master_mac && d.device_type != 0xFF)
            .position(|d| d.mac == *mac)
            .map(|i| i as u8)
            .unwrap_or(0);
        drop(devices);

        let target_cmd_seq = match self.reserve_mb_rgb_transition(
            mac,
            device.is_sync_mb_light,
            enabled,
            device.cmd_seq,
            Instant::now(),
        ) {
            reservation @ (MbRgbReservation::Applied | MbRgbReservation::Pending(_)) => {
                if enabled {
                    self.desired_effects.lock().remove(mac);
                }
                return Ok(reservation);
            }
            MbRgbReservation::Start(target_cmd_seq) => target_cmd_seq,
        };

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_MB_LIGHT_SYNC;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;
        rf_data[15] = master_ch;
        rf_data[16] = slave_index;
        rf_data[17] = target_cmd_seq;
        rf_data[20] = if enabled { 1 } else { 0 };

        if let Err(error) = self.enqueue_rf_command(
            &device,
            rf_data,
            AckSignal::CmdSeq(target_cmd_seq),
            format!("MB RGB sync {}", if enabled { "on" } else { "off" }),
        ) {
            self.cancel_mb_rgb_transition(mac, target_cmd_seq);
            return Err(error);
        }
        if enabled {
            self.desired_effects.lock().remove(mac);
        }

        debug!(
            "MB RGB sync {}: {} (target_cmd_seq={})",
            if enabled { "enabled" } else { "disabled" },
            device.mac_str(),
            target_cmd_seq,
        );
        Ok(MbRgbReservation::Start(target_cmd_seq))
    }

    pub fn mb_rgb_ready_for_upload(&self, mac: &[u8; 6]) -> Result<bool> {
        let device = match self.device_by_mac_snapshot(mac) {
            Ok(device) => device,
            Err(error) => {
                self.mb_rgb_targets.lock().remove(mac);
                return Err(error);
            }
        };
        if !device.fan_type.supports_mb_rgb_sync() {
            self.mb_rgb_targets.lock().remove(mac);
            return Ok(true);
        }
        Ok(matches!(
            self.request_mb_rgb_sync(mac, false)?,
            MbRgbReservation::Applied
        ))
    }

    pub(super) fn forget_mb_rgb_target(&self, mac: &[u8; 6]) {
        self.mb_rgb_targets.lock().remove(mac);
    }

    pub(super) fn has_pending_mb_rgb_transition(&self, mac: &[u8; 6]) -> bool {
        self.mb_rgb_targets.lock().contains_key(mac)
    }

    pub(super) fn reserve_mb_rgb_transition(
        &self,
        mac: &[u8; 6],
        observed: bool,
        enabled: bool,
        observed_cmd_seq: u8,
        now: Instant,
    ) -> MbRgbReservation {
        let mut targets = self.mb_rgb_targets.lock();
        if let Some(target) = targets.get(mac) {
            if target.enabled == enabled {
                if observed == enabled && observed_cmd_seq == target.cmd_seq {
                    targets.remove(mac);
                    return MbRgbReservation::Applied;
                }
                if now < target.expires_at {
                    return MbRgbReservation::Pending(target.cmd_seq);
                }
            }
        } else if observed == enabled {
            return MbRgbReservation::Applied;
        }
        let cmd_seq = self.bump_target_cmd_seq(mac, observed_cmd_seq);
        targets.insert(
            *mac,
            MbRgbTarget {
                enabled,
                cmd_seq,
                expires_at: now + MB_RGB_TRANSITION_TIMEOUT,
            },
        );
        MbRgbReservation::Start(cmd_seq)
    }

    fn cancel_mb_rgb_transition(&self, mac: &[u8; 6], cmd_seq: u8) {
        let mut targets = self.mb_rgb_targets.lock();
        if targets
            .get(mac)
            .is_some_and(|target| target.cmd_seq == cmd_seq)
        {
            targets.remove(mac);
        }
    }

    pub fn has_mb_pwm_companion(&self) -> bool {
        let master_mac = *self.master_mac.lock();
        self.discovered_devices
            .lock()
            .iter()
            .any(|d| d.master_mac == master_mac && d.mac[5] == 0xE1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_disable_reuses_the_pending_command_sequence() {
        let controller = WirelessController::new();
        let mac = [1, 2, 3, 4, 5, 6];
        let now = Instant::now();

        assert_eq!(
            controller.reserve_mb_rgb_transition(&mac, true, false, 10, now),
            MbRgbReservation::Start(11)
        );
        assert_eq!(
            controller.reserve_mb_rgb_transition(
                &mac,
                false,
                false,
                10,
                now + Duration::from_millis(500)
            ),
            MbRgbReservation::Pending(11)
        );
        assert_eq!(
            controller
                .target_cmd_seqs
                .as_ref()
                .unwrap()
                .lock()
                .get(&mac),
            Some(&11)
        );

        assert_eq!(
            controller.reserve_mb_rgb_transition(
                &mac,
                true,
                false,
                10,
                now + MB_RGB_TRANSITION_TIMEOUT
            ),
            MbRgbReservation::Start(12)
        );
        assert_eq!(
            controller.reserve_mb_rgb_transition(
                &mac,
                false,
                false,
                12,
                now + MB_RGB_TRANSITION_TIMEOUT
            ),
            MbRgbReservation::Applied
        );
        assert!(!controller.mb_rgb_targets.lock().contains_key(&mac));
    }

    #[test]
    fn opposite_request_supersedes_a_pending_transition() {
        let controller = WirelessController::new();
        let mac = [6, 5, 4, 3, 2, 1];
        let now = Instant::now();

        assert_eq!(
            controller.reserve_mb_rgb_transition(&mac, true, false, 20, now),
            MbRgbReservation::Start(21)
        );
        assert_eq!(
            controller.reserve_mb_rgb_transition(
                &mac,
                true,
                true,
                20,
                now + Duration::from_millis(100)
            ),
            MbRgbReservation::Start(22)
        );
        drop(controller.clone());
        assert_eq!(controller.mb_rgb_targets.lock()[&mac].cmd_seq, 22);
        assert_eq!(
            controller.reserve_mb_rgb_transition(
                &mac,
                true,
                true,
                21,
                now + Duration::from_millis(200)
            ),
            MbRgbReservation::Pending(22)
        );
        assert_eq!(
            controller.reserve_mb_rgb_transition(
                &mac,
                true,
                true,
                22,
                now + Duration::from_millis(300)
            ),
            MbRgbReservation::Applied
        );
    }
}

use super::RgbController;
use lianli_shared::rgb::RgbEffect;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::debug;

pub struct DirectColorBuffer {
    pending: HashMap<String, HashMap<u8, Vec<[u8; 3]>>>,
    groups: HashMap<String, GroupDelivery>,
    next_revision: u64,
}

impl DirectColorBuffer {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            groups: HashMap::new(),
            next_revision: 0,
        }
    }

    pub fn set(&mut self, device_id: String, zone: u8, colors: Vec<[u8; 3]>) {
        self.pending
            .entry(device_id)
            .or_default()
            .insert(zone, colors);
    }

    pub fn take_all(&mut self) -> HashMap<String, HashMap<u8, Vec<[u8; 3]>>> {
        std::mem::take(&mut self.pending)
    }

    pub fn set_group(&mut self, id: String, effects: Vec<RgbEffect>) {
        self.next_revision = self.next_revision.wrapping_add(1);
        let revision = self.next_revision;
        let entry = self.groups.entry(id).or_insert_with(|| GroupDelivery {
            effects: effects.clone(),
            revision,
            delivered: None,
            in_flight: false,
            attempts: 0,
            retry_at: Instant::now(),
            warned: false,
        });
        if entry.effects != effects {
            entry.effects = effects;
            entry.revision = revision;
            entry.attempts = 0;
        } else if entry.attempts >= MAX_GROUP_ATTEMPTS {
            // An explicit retry may restart a bounded attempt sequence, after the backoff.
            entry.attempts = 0;
        }
    }

    pub fn take_due_groups(&mut self, now: Instant) -> Vec<GroupWrite> {
        self.groups
            .iter_mut()
            .filter_map(|(id, entry)| {
                if entry.in_flight
                    || entry.delivered == Some(entry.revision)
                    || entry.attempts >= MAX_GROUP_ATTEMPTS
                    || now < entry.retry_at
                {
                    return None;
                }
                entry.in_flight = true;
                Some(GroupWrite {
                    id: id.clone(),
                    revision: entry.revision,
                    effects: entry.effects.clone(),
                })
            })
            .collect()
    }

    pub fn complete_group(&mut self, write: &GroupWrite, success: bool, now: Instant) -> bool {
        let Some(entry) = self.groups.get_mut(&write.id) else {
            return false;
        };
        entry.in_flight = false;
        if success {
            if entry.revision == write.revision {
                entry.delivered = Some(write.revision);
            }
            entry.attempts = 0;
            entry.retry_at = now;
            entry.warned = false;
            false
        } else {
            entry.attempts = entry.attempts.saturating_add(1);
            entry.retry_at = now + Duration::from_millis(100 * (1u64 << entry.attempts.min(4)));
            let first_failure = !entry.warned;
            entry.warned = true;
            first_failure
        }
    }

    pub fn clear(&mut self) {
        self.pending.clear();
        self.groups.clear();
    }

    pub fn retain_devices(&mut self, ids: &std::collections::HashSet<String>) {
        self.pending.retain(|id, _| ids.contains(id));
        self.groups.retain(|id, _| ids.contains(id));
    }

    pub fn invalidate_group_delivery(&mut self, ids: &std::collections::HashSet<String>) {
        for (id, entry) in &mut self.groups {
            if !ids.contains(id) {
                continue;
            }
            self.next_revision = self.next_revision.wrapping_add(1);
            entry.revision = self.next_revision;
            entry.delivered = None;
            entry.attempts = 0;
            entry.retry_at = Instant::now();
        }
    }
}

const MAX_GROUP_ATTEMPTS: u8 = 3;

struct GroupDelivery {
    effects: Vec<RgbEffect>,
    revision: u64,
    delivered: Option<u64>,
    in_flight: bool,
    attempts: u8,
    retry_at: Instant,
    warned: bool,
}

pub struct GroupWrite {
    pub id: String,
    revision: u64,
    pub effects: Vec<RgbEffect>,
}

pub fn start_direct_color_writer(
    rgb: Arc<Mutex<RgbController>>,
    buffer: Arc<Mutex<DirectColorBuffer>>,
    stop_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        debug!("Direct color writer started");

        loop {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            let updates = buffer.lock().take_all();
            let groups = buffer.lock().take_due_groups(Instant::now());
            for write in groups {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                let prepared = rgb.lock().prepare_group_write(&write.id, &write.effects);
                let result = prepared.and_then(|device| {
                    device.set_group_effects(&write.effects)?;
                    rgb.lock().cache_group_effects(
                        write.id.clone(),
                        &device,
                        write.effects.clone(),
                    );
                    Ok(())
                });
                let (first_failure, exhausted) = {
                    let mut buffer = buffer.lock();
                    let first = buffer.complete_group(&write, result.is_ok(), Instant::now());
                    let exhausted = result.is_err()
                        && buffer
                            .groups
                            .get(&write.id)
                            .is_some_and(|entry| entry.attempts >= MAX_GROUP_ATTEMPTS);
                    (first, exhausted)
                };
                if first_failure || exhausted {
                    if let Err(error) = result {
                        tracing::warn!(
                            device = %write.id, %error, retrying = !exhausted,
                            "OpenRGB group delivery failed or was refused"
                        );
                    }
                }
            }

            if !updates.is_empty() {
                let mut wireless = Vec::new();
                let mut wired = Vec::new();
                {
                    let mut rgb = rgb.lock();
                    rgb.cache_direct_batch(&updates);
                    for (device_id, zones) in &updates {
                        if rgb.software_controlled(device_id) {
                            wireless.push((device_id.clone(), zones.clone()));
                        } else if let Some(dev) = rgb.clone_wired_device(device_id) {
                            let z: Vec<_> = zones.iter().map(|(&k, v)| (k, v.clone())).collect();
                            wired.push((dev, device_id.clone(), z));
                        }
                    }
                }

                if !wireless.is_empty() {
                    let mut rgb = rgb.lock();
                    for (device_id, zones) in wireless {
                        if let Err(e) = rgb.apply_wireless_batch(&device_id, &zones) {
                            debug!("Wireless flush error for {device_id}: {e}");
                        }
                    }
                }

                for (dev, device_id, zones) in wired {
                    if stop_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    for (zone, colors) in zones {
                        if stop_flag.load(Ordering::Relaxed) {
                            break;
                        }
                        if let Err(e) = dev.set_direct_colors(zone, &colors) {
                            debug!("Wired flush error for {device_id} zone {zone}: {e}");
                        }
                    }
                }
            }
            thread::sleep(Duration::from_millis(16));
        }

        debug!("Direct color writer stopped");
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effects(color: [u8; 3]) -> Vec<RgbEffect> {
        [
            lianli_shared::rgb::RgbScope::Inner,
            lianli_shared::rgb::RgbScope::Outer,
        ]
        .map(|scope| RgbEffect {
            scope,
            colors: vec![color; 3],
            ..Default::default()
        })
        .to_vec()
    }

    #[test]
    fn failed_group_delivery_retries_with_backoff_and_identical_commands_can_rearm_it() {
        let mut buffer = DirectColorBuffer::new();
        let red = effects([255, 0, 0]);
        buffer.set_group("fans".into(), red.clone());
        let mut now = Instant::now() + Duration::from_secs(1);
        for attempt in 0..MAX_GROUP_ATTEMPTS {
            let writes = buffer.take_due_groups(now);
            assert_eq!(writes.len(), 1);
            assert_eq!(buffer.complete_group(&writes[0], false, now), attempt == 0);
            assert!(buffer
                .take_due_groups(now + Duration::from_millis(99))
                .is_empty());
            now += Duration::from_secs(2);
        }
        assert!(buffer.take_due_groups(now).is_empty());
        buffer.set_group("fans".into(), red.clone());
        let writes = buffer.take_due_groups(now);
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].effects, red);
        buffer.complete_group(&writes[0], true, now);
        buffer.set_group("fans".into(), red);
        assert!(buffer.take_due_groups(now).is_empty());
    }

    #[test]
    fn new_state_replaces_pending_work_without_stale_completions_marking_it_delivered() {
        let mut buffer = DirectColorBuffer::new();
        let now = Instant::now() + Duration::from_secs(1);
        buffer.set_group("fans".into(), effects([255, 0, 0]));
        let old = buffer.take_due_groups(now).pop().unwrap();
        buffer.set_group("fans".into(), effects([0, 255, 0]));
        assert!(buffer.take_due_groups(now).is_empty());
        buffer.complete_group(&old, true, now);
        let next = buffer.take_due_groups(now).pop().unwrap();
        assert_eq!(next.effects, effects([0, 255, 0]));
        buffer.clear();
        buffer.set_group("fans".into(), effects([0, 0, 255]));
        buffer.complete_group(&next, true, now);
        let next = buffer.take_due_groups(now).pop().unwrap();
        assert_eq!(next.effects, effects([0, 0, 255]));
        buffer.complete_group(&next, true, now);
        assert!(buffer.take_due_groups(now).is_empty());
    }

    #[test]
    fn retained_devices_keep_pending_retries_and_completed_delivery() {
        let mut buffer = DirectColorBuffer::new();
        let now = Instant::now() + Duration::from_secs(1);
        for id in ["pending", "delivered", "removed"] {
            buffer.set_group(id.into(), effects([1, 2, 3]));
            buffer.set(id.into(), 0, vec![[4, 5, 6]]);
        }
        for write in buffer.take_due_groups(now) {
            buffer.complete_group(&write, write.id == "delivered", now);
        }
        buffer.retain_devices(&["pending".into(), "delivered".into()].into());
        let direct = buffer.take_all();
        assert_eq!(direct.len(), 2);
        assert!(!direct.contains_key("removed"));
        assert!(buffer.take_due_groups(now).is_empty());
        let retry = buffer.take_due_groups(now + Duration::from_secs(1));
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].id, "pending");
        buffer.complete_group(&retry[0], true, now);
        buffer.invalidate_group_delivery(&["pending".into()].into());
        let replay = buffer.take_due_groups(now + Duration::from_secs(2));
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].id, "pending");
    }
}

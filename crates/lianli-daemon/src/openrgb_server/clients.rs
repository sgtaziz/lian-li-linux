use super::*;
use std::collections::HashMap;
use std::sync::mpsc::{self, SyncSender};

const QUEUE_PACKETS: usize = 16;
const QUEUE_BYTES: usize = 2 * MAX_PACKET_BYTES;

pub(super) struct Output {
    sender: SyncSender<Vec<u8>>,
    control: TcpStream,
    queued_bytes: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
}

impl Output {
    pub fn start(stream: &TcpStream) -> std::io::Result<(Arc<Self>, thread::JoinHandle<()>)> {
        let mut writer = stream.try_clone()?;
        writer.set_write_timeout(Some(Duration::from_secs(10)))?;
        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(QUEUE_PACKETS);
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicBool::new(false));
        let output = Arc::new(Self {
            sender,
            control: stream.try_clone()?,
            queued_bytes: queued_bytes.clone(),
            closed: closed.clone(),
        });
        let worker = thread::spawn(move || {
            while !closed.load(Ordering::Acquire) {
                let packet = match receiver.recv() {
                    Ok(packet) => packet,
                    Err(_) => break,
                };
                if closed.load(Ordering::Acquire) {
                    break;
                }
                let result = writer.write_all(&packet);
                queued_bytes.fetch_sub(packet.len(), Ordering::Relaxed);
                if result.is_err() {
                    break;
                }
            }
            closed.store(true, Ordering::Release);
            let _ = writer.shutdown(std::net::Shutdown::Both);
        });
        Ok((output, worker))
    }

    pub fn disconnect(&self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.control.shutdown(std::net::Shutdown::Both);
        // Wake an idle writer; a full queue already has a wakeup pending.
        let _ = self.sender.try_send(Vec::new());
    }

    pub fn send(&self, index: u32, kind: u32, payload: &[u8]) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.closed.load(Ordering::Acquire),
            "SDK client disconnected"
        );
        let size = HEADER_SIZE + payload.len();
        let reserved =
            self.queued_bytes
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                    used.checked_add(size).filter(|total| *total <= QUEUE_BYTES)
                });
        if payload.len() > MAX_PACKET_BYTES || reserved.is_err() {
            if reserved.is_ok() {
                self.queued_bytes.fetch_sub(size, Ordering::Relaxed);
            }
            self.disconnect();
            anyhow::bail!("SDK client output limit exceeded");
        }
        let mut packet = Vec::with_capacity(size);
        packet.extend_from_slice(MAGIC);
        packet.extend_from_slice(&index.to_le_bytes());
        packet.extend_from_slice(&kind.to_le_bytes());
        packet.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        packet.extend_from_slice(payload);
        if self.sender.try_send(packet).is_err() {
            self.queued_bytes.fetch_sub(size, Ordering::Relaxed);
            self.disconnect();
            anyhow::bail!("SDK client output queue full or closed");
        }
        Ok(())
    }
}

struct Subscriber {
    output: Arc<Output>,
    indexes: HashMap<String, u32>,
}

#[derive(Default)]
pub(super) struct Notifications {
    subscribers: Vec<Subscriber>,
    revisions: HashMap<String, u64>,
}

impl Notifications {
    pub fn subscribe(&mut self, output: Arc<Output>, targets: &[Target], version: u32) {
        self.subscribers.retain(|client| {
            !Arc::ptr_eq(&client.output, &output) && !client.output.closed.load(Ordering::Acquire)
        });
        if version >= 6 {
            let indexes = targets
                .iter()
                .enumerate()
                .map(|(index, target)| {
                    let id = match target {
                        Target::Legacy(cap) => cap.device_id.clone(),
                        Target::Region { group, .. } => group.lock().cap.device_id.clone(),
                    };
                    (id, index as u32)
                })
                .collect();
            self.subscribers.push(Subscriber { output, indexes });
        }
    }

    pub fn publish(&mut self, id: &str, mut update: Update, full_state: impl FnOnce() -> Update) {
        if !self.subscribers.iter().any(|client| {
            client.indexes.contains_key(id) && !client.output.closed.load(Ordering::Acquire)
        }) {
            return;
        }
        let previous = self.revisions.entry(id.to_owned()).or_default();
        if update.revision <= *previous {
            return;
        }
        // Concurrent senders can publish snapshots out of order; fill any skipped mode change.
        if update.revision != previous.wrapping_add(1) {
            update = full_state();
        }
        let data = &update.data;
        *previous = update.revision;
        self.subscribers.retain(|client| {
            if client.output.closed.load(Ordering::Acquire) {
                return false;
            }
            client
                .indexes
                .get(id)
                .is_none_or(|index| client.output.send(*index, PKT_SIGNAL_UPDATE, data).is_ok())
        });
    }

    pub fn reset(&mut self) {
        self.subscribers.clear();
        self.revisions.clear();
    }
}

pub(super) struct Update {
    pub revision: u64,
    pub data: Vec<u8>,
}

pub(super) fn is_color_update(kind: u32) -> bool {
    matches!(
        kind,
        PKT_UPDATE_LEDS | PKT_UPDATE_ZONE_LEDS | PKT_UPDATE_SINGLE_LED
    )
}

pub(super) fn signal(kind: u32, colors: &[[u8; 3]], description: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    if is_color_update(kind) {
        data.extend_from_slice(&0u32.to_le_bytes());
        protocol::write_colors(&mut data, colors);
    } else {
        data.extend_from_slice(&(if kind == PKT_SAVE_MODE { 2u32 } else { 1u32 }).to_le_bytes());
        data.extend_from_slice(&description[4..]);
    }
    protocol::sized_packet(&data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::time::Instant;

    fn connection() -> (Arc<Output>, thread::JoinHandle<()>, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let size: libc::c_int = 4096;
        // A small kernel buffer makes a non-reading peer exercise the bounded user-space queue.
        let result = unsafe {
            libc::setsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                (&size as *const libc::c_int).cast(),
                std::mem::size_of_val(&size) as libc::socklen_t,
            )
        };
        assert_eq!(result, 0);
        let (output, worker) = Output::start(&stream).unwrap();
        (output, worker, peer)
    }

    #[test]
    fn stalled_client_is_disconnected_without_blocking_other_clients() {
        let (slow, slow_worker, _slow_peer) = connection();
        let (fast, fast_worker, mut fast_peer) = connection();
        let mut notifications = Notifications::default();
        for output in [&slow, &fast] {
            notifications.subscribers.push(Subscriber {
                output: output.clone(),
                indexes: HashMap::from([("device".into(), 0)]),
            });
        }
        let started = Instant::now();
        for revision in 1..=32 {
            let payload = vec![revision as u8; 256 * 1024];
            notifications.publish(
                "device",
                Update {
                    revision,
                    data: payload.clone(),
                },
                || panic!("sequential updates must not serialize full state"),
            );
            assert_eq!(read_packet_from(&mut fast_peer).unwrap().2, payload);
            if slow.closed.load(Ordering::Acquire) {
                break;
            }
        }
        assert!(slow.closed.load(Ordering::Acquire));
        assert!(!fast.closed.load(Ordering::Acquire));
        assert!(started.elapsed() < Duration::from_secs(3));
        fast.send(0, PKT_REQUEST_CONTROLLER_COUNT, &[1, 0, 0, 0])
            .unwrap();
        assert_eq!(
            read_packet_from(&mut fast_peer).unwrap().1,
            PKT_REQUEST_CONTROLLER_COUNT
        );
        fast.disconnect();
        slow_worker.join().unwrap();
        fast_worker.join().unwrap();
    }

    #[test]
    fn out_of_order_snapshots_resynchronize_modes_and_never_rewind_state() {
        let (output, worker, mut peer) = connection();
        let mut notifications = Notifications::default();
        notifications.subscribers.push(Subscriber {
            output: output.clone(),
            indexes: HashMap::from([("device".into(), 7)]),
        });
        for revision in [1, 3, 2] {
            notifications.publish(
                "device",
                Update {
                    revision,
                    data: vec![revision as u8],
                },
                || {
                    assert_eq!(revision, 3);
                    Update {
                        revision,
                        data: vec![revision as u8, 99],
                    }
                },
            );
        }
        assert_eq!(
            read_packet_from(&mut peer).unwrap(),
            (7, PKT_SIGNAL_UPDATE, vec![1])
        );
        assert_eq!(
            read_packet_from(&mut peer).unwrap(),
            (7, PKT_SIGNAL_UPDATE, vec![3, 99])
        );
        output.send(0, PKT_REQUEST_CONTROLLER_COUNT, &[]).unwrap();
        assert_eq!(
            read_packet_from(&mut peer).unwrap().1,
            PKT_REQUEST_CONTROLLER_COUNT
        );
        output.disconnect();
        worker.join().unwrap();
    }

    #[test]
    fn no_subscribers_do_not_build_full_state() {
        Notifications::default().publish(
            "device",
            Update {
                revision: 8,
                data: vec![],
            },
            || panic!("no subscriber needs a description"),
        );
    }
}

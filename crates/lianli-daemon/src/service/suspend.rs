use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use crate::service::DaemonEvent;

pub fn spawn(tx: Sender<DaemonEvent>) {
    thread::spawn(move || {
        let interval = Duration::from_secs(2);
        loop {
            let before = Instant::now();
            thread::sleep(interval);
            let elapsed = before.elapsed();
            if elapsed.as_secs() >= 6 {
                tracing::info!(
                    "System resume detected (clock jumped {:?}s, expected {:?})",
                    elapsed.as_secs(),
                    interval
                );
                let _ = tx.send(DaemonEvent::SystemResumed);
            }
        }
    });
}

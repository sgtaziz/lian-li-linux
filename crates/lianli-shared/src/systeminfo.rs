use std::fs::File;
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

// SysSensor is a singleton.
// It can be used as follows: At the very beginning of daemon start SysSensor::init() is called which initializes the singleton
// After that the CPU usage values get refreshed 3 times per second automatically.
// You can retrieve the stored values at any time by SysSensor::get_cpu_usage();

pub struct SysSensor {
    // list of usage per core (cpu0, cpu1, ...)
    per_core_usage: Arc<Vec<AtomicU32>>,
    last_global_usage: AtomicU32,
}

static INSTANCE: OnceLock<SysSensor> = OnceLock::new();

impl SysSensor {
    pub fn init() {
        INSTANCE.get_or_init(|| {
            // Let's retrieve the number of CPU cores in the system
            let core_count = std::fs::read_to_string("/proc/cpuinfo")
                .unwrap_or_default()
                .matches("processor")
                .count()
                .max(1);

            let per_core = Arc::new((0..core_count).map(|_| AtomicU32::new(0)).collect());

            let sensor = Self {
                per_core_usage: per_core,
                last_global_usage: AtomicU32::new(0),
            };

            thread::spawn(|| Self::worker_loop());
            sensor
        });
    }

    pub fn get_core_usage() -> Vec<u32> {
        if let Some(s) = INSTANCE.get() {
            let cores: Vec<u32> = s
                .per_core_usage
                .iter()
                .map(|c| (c.load(Ordering::Relaxed) / 100) as u32)
                .collect();

            cores
        } else {
            vec![]
        }
    }

    /// ranges from 0 to 10000 (i.e. multiplied by 100 to be able to have 2 decimal places after the percent value (e.g. 78.67%), so mathematically 4 decimals...)
    pub fn get_cpu_usage() -> u32 {
        if let Some(s) = INSTANCE.get() {
            s.last_global_usage.load(Ordering::Relaxed)
        } else {
            0
        }
    }

    fn worker_loop() {
        // CPU Usage and usage per core calculation.

        let mut stat_file = File::open("/proc/stat").ok();
        let mut buffer = [0u8; 4096];

        // Mem for the previous ticks (global and per core)
        let mut last_stats: Vec<(u64, u64)> = vec![];

        loop {
            let start_time = Instant::now();

            if let Some(ref mut f) = stat_file {
                if let Ok(n) = f.read_at(&mut buffer, 0) {
                    let s = std::str::from_utf8(&buffer[..n]).unwrap_or("");
                    let cpu_lines: Vec<&str> = s.lines().filter(|l| l.starts_with("cpu")).collect();

                    // Initialize last_stats if necessary
                    if last_stats.len() < cpu_lines.len() {
                        last_stats = vec![(0, 0); cpu_lines.len()];
                    }

                    for (i, line) in cpu_lines.iter().enumerate() {
                        let parts: Vec<u64> = line
                            .split_whitespace()
                            .skip(1)
                            .filter_map(|p| p.parse().ok())
                            .collect();

                        let idle = parts.get(3).copied().unwrap_or(0)
                            + parts.get(4).copied().unwrap_or(0);
                        let total: u64 = parts.iter().sum();

                        let (prev_total, prev_idle) = last_stats[i];
                        let total_delta = total.saturating_sub(prev_total);
                        let idle_delta = idle.saturating_sub(prev_idle);
                        last_stats[i] = (total, idle);

                        if total_delta > 0 {
                            let usage = (1.0 - (idle_delta as f32 / total_delta as f32)) * 100.0;
                            let val = (usage * 100.0) as u32;

                            if i == 0 {
                                // Global cpu usage (first line "cpu ")
                                if let Some(inst) = INSTANCE.get() {
                                    inst.last_global_usage.store(val, Ordering::Relaxed);
                                }
                            } else {
                                // Core usage (lines "cpu0", "cpu1", ...)
                                if let Some(inst) = INSTANCE.get() {
                                    if let Some(atomic) = inst.per_core_usage.get(i - 1) {
                                        atomic.store(val, Ordering::Relaxed);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let sleep_dur = Duration::from_millis(333).saturating_sub(start_time.elapsed());

            thread::sleep(sleep_dur);
        }
    }
}

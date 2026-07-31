use chrono::{Datelike, Local, Timelike};

const PAYLOAD_LEN: usize = 220;

/// Sensor data that drives wireless LCD fan embedded themes.
/// Populated once per second and broadcast via RF_ClockSync (0x14).
#[derive(Debug, Clone, Default)]
pub struct SensorSnapshot {
    pub cpu_temp: u8,
    pub cpu_usage: u8,
    pub gpu_temp: u8,
    pub gpu_usage: u8,
    pub cpu_freq_mhz: u16,
    pub gpu_freq_mhz: u16,
}

/// Build the 220-byte `cpuInfoParam` blob from sensor data + system time.
///
/// Layout (from vendor `MasterDevice.SetFanLcdInfo` / `RFController.GetFixedData`):
/// ```text
/// [0]    CPU temp °C
/// [1]    CPU usage %
/// [2]    GPU temp °C
/// [3]    GPU usage %
/// [4..6] CPU freq MHz (BE)
/// [6..8] GPU freq MHz (BE)
/// [8..32] other sensor fields (disk, memory, power, etc.) — zero for now
/// [32..34] year (BE)
/// [34]   month
/// [35]   day
/// [36]   hour
/// [37]   minute
/// [38]   second
/// [39]   reserved
/// [40..50] unused
/// [50..218] per-receiver fan blocks (14 × 12 bytes) — defaults for now
/// [218..220] unused
/// ```
pub fn build_payload(snap: &SensorSnapshot) -> [u8; PAYLOAD_LEN] {
    let mut buf = [0u8; PAYLOAD_LEN];

    buf[0] = snap.cpu_temp;
    buf[1] = snap.cpu_usage;
    buf[2] = snap.gpu_temp;
    buf[3] = snap.gpu_usage;
    buf[4..6].copy_from_slice(&snap.cpu_freq_mhz.to_be_bytes());
    buf[6..8].copy_from_slice(&snap.gpu_freq_mhz.to_be_bytes());

    let now = Local::now();
    let year = now.year() as u16;
    buf[32..34].copy_from_slice(&year.to_be_bytes());
    buf[34] = now.month() as u8;
    buf[35] = now.day() as u8;
    buf[36] = now.hour() as u8;
    buf[37] = now.minute() as u8;
    buf[38] = now.second() as u8;

    buf
}

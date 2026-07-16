//! HydroShift II fan controller — persistent local service.
//!
//! Replaces the pulsating firmware fan curve with a smooth, coolant-temp-driven
//! curve streamed over the same WinUSB channel L-Connect uses. Protocol lives in
//! `docs/hydroshift2-fan-protocol.md`; this is a working consumer of it.
//!
//! Why it doesn't pulsate: the firmware curve chases the spiky CPU die temp. This
//! service follows the *coolant* temp (naturally smooth — thermal mass of the loop),
//! then adds an EMA low-pass on the reading plus a slew-rate limit on the duty, so
//! fan speed only ever drifts gently. A single 0xFB decays back to firmware in a few
//! seconds, so we re-stream every `interval_secs` to hold the setpoint; if the service
//! dies, the firmware curve simply resumes (built-in failsafe).
//!
//! Config: /etc/hs2-fanctl.json (falls back to built-in defaults). Reloaded on SIGHUP
//! is not implemented — edit + `systemctl restart hs2-fanctl`.

use anyhow::{Context, Result};
use cbc::Encryptor;
use des::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use des::Des;
use lianli_transport::usb::{UsbTransport, LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use serde::Deserialize;
use std::time::{Duration, Instant};

const DES_KEY: [u8; 8] = *b"slv3tuzx";
const VID: u16 = 0x1CBE;
const PID: u16 = 0xA021;
const CMD_GET_STATUS: u8 = 0xFA;
const CMD_SET_FANS: u8 = 0xFB;
const CONFIG_PATH: &str = "/etc/hs2-fanctl.json";

// ─────────────────────────── config ───────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct Config {
    /// "coolant" (default, recommended for an AIO) or "cpu" (k10temp Tctl).
    temp_source: String,
    /// Streaming/poll interval in seconds.
    interval_secs: f32,
    /// Curve points [temp_c, duty_pct], any order; linearly interpolated.
    /// Below the first point → first duty; above the last → last duty.
    curve: Vec<[f32; 2]>,
    /// Hard clamps on duty percent.
    min_duty_pct: f32,
    max_duty_pct: f32,
    /// EMA time constant (s) low-passing the temp reading. Bigger = smoother/slower.
    smoothing_tau_secs: f32,
    /// Max duty change per second (percent). Caps how fast fans ramp — kills audible steps.
    slew_pct_per_sec: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            temp_source: "coolant".into(),
            interval_secs: 2.0,
            // Coolant-driven. Real coolant idles ~36-40 °C and only crosses ~44 °C under
            // sustained all-core load, so hold a quiet floor through the normal band and
            // ramp only when the loop is genuinely saturated (full tilt by 50 °C).
            curve: vec![
                [38.0, 30.0],
                [41.0, 45.0],
                [44.0, 65.0],
                [47.0, 85.0],
                [50.0, 100.0],
            ],
            min_duty_pct: 25.0,
            max_duty_pct: 100.0,
            smoothing_tau_secs: 8.0,
            slew_pct_per_sec: 5.0,
        }
    }
}

impl Config {
    fn load() -> Self {
        match std::fs::read_to_string(CONFIG_PATH) {
            Ok(s) => match serde_json::from_str::<Config>(&s) {
                Ok(mut c) => {
                    c.curve.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());
                    c
                }
                Err(e) => {
                    eprintln!("config parse error ({CONFIG_PATH}): {e}; using defaults");
                    Config::default()
                }
            },
            Err(_) => {
                eprintln!("no {CONFIG_PATH}; using built-in defaults");
                Config::default()
            }
        }
    }

    fn interp(&self, temp: f32) -> f32 {
        let c = &self.curve;
        if c.is_empty() {
            return self.min_duty_pct;
        }
        if temp <= c[0][0] {
            return c[0][1].clamp(self.min_duty_pct, self.max_duty_pct);
        }
        if temp >= c[c.len() - 1][0] {
            return c[c.len() - 1][1].clamp(self.min_duty_pct, self.max_duty_pct);
        }
        for w in c.windows(2) {
            let (t0, d0) = (w[0][0], w[0][1]);
            let (t1, d1) = (w[1][0], w[1][1]);
            if temp >= t0 && temp <= t1 {
                let f = (temp - t0) / (t1 - t0);
                return (d0 + f * (d1 - d0)).clamp(self.min_duty_pct, self.max_duty_pct);
            }
        }
        self.min_duty_pct
    }
}

// ─────────────────────────── protocol ───────────────────────────

fn build_set_fans(duty_255: u8, sensor_mirror: u16, ts_ms: u32) -> Vec<u8> {
    fn crc16_xmodem(data: &[u8]) -> u16 {
        let mut crc: u16 = 0;
        for &b in data {
            crc ^= (b as u16) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
            }
        }
        crc
    }
    let mut params = vec![0xff, 0x0f, 0xa2, 0x00];
    params.extend_from_slice(&sensor_mirror.to_be_bytes());
    params.extend_from_slice(&[duty_255, duty_255, duty_255]);
    params.extend_from_slice(&[0, 0, 0]);
    let crc = crc16_xmodem(&params);
    params.extend_from_slice(&crc.to_be_bytes());

    let mut buf = vec![0u8; 508];
    buf[0] = CMD_SET_FANS;
    buf[2] = 0x1A;
    buf[3] = 0x6D;
    buf[4..8].copy_from_slice(&ts_ms.to_le_bytes());
    buf[8..8 + params.len()].copy_from_slice(&params);
    let enc = Encryptor::<Des>::new_from_slices(&DES_KEY, &DES_KEY)
        .unwrap()
        .encrypt_padded_mut::<Pkcs7>(&mut buf, 500)
        .unwrap()
        .to_vec();
    let mut out = vec![0u8; 512];
    out[..504].copy_from_slice(&enc);
    out[510] = 0xa1;
    out[511] = 0x1a;
    out
}

fn build_cmd(cmd: u8, ts_ms: u32) -> Vec<u8> {
    let mut buf = vec![0u8; 508];
    buf[0] = cmd;
    buf[2] = 0x1A;
    buf[3] = 0x6D;
    buf[4..8].copy_from_slice(&ts_ms.to_le_bytes());
    let enc = Encryptor::<Des>::new_from_slices(&DES_KEY, &DES_KEY)
        .unwrap()
        .encrypt_padded_mut::<Pkcs7>(&mut buf, 500)
        .unwrap()
        .to_vec();
    let mut out = vec![0u8; 512];
    out[..504].copy_from_slice(&enc);
    out[510] = 0xa1;
    out[511] = 0x1a;
    out
}

struct Status {
    coolant_c: f32,
    fan_rpm: [u16; 3],
    pump_rpm: u16,
}

fn parse_status(buf: &[u8; 512]) -> Option<Status> {
    if buf[0] != CMD_GET_STATUS || buf[8] != 0x0a {
        return None;
    }
    let be = |i: usize| u16::from_be_bytes([buf[i], buf[i + 1]]);
    Some(Status {
        fan_rpm: [be(14), be(16), be(18)],
        pump_rpm: be(20),
        // [13] = coolant temp, whole °C (verified vs the pump's own LCD readout).
        // [30:32] is a constant (0x0116), NOT temperature.
        coolant_c: buf[13] as f32,
    })
}

// ─────────────────────────── device ───────────────────────────

struct Device {
    transport: UsbTransport,
    start: Instant,
}

impl Device {
    fn open() -> Result<Self> {
        let mut transport = UsbTransport::open(VID, PID).context("opening HydroShift II")?;
        transport
            .detach_and_configure("hs2-fanctl")
            .context("configuring device")?;
        let _ = transport.clear_halt(0x01);
        let _ = transport.clear_halt(0x81);
        transport.read_flush();
        let dev = Self { transport, start: Instant::now() };
        dev.wake()?;
        Ok(dev)
    }

    fn ts(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn xfer(&self, pkt: &[u8]) -> Result<Option<[u8; 512]>> {
        self.transport.read_flush();
        self.transport.write(pkt, LCD_WRITE_TIMEOUT)?;
        let mut buf = [0u8; 512];
        match self.transport.read(&mut buf, LCD_READ_TIMEOUT) {
            Ok(n) if n > 0 => Ok(Some(buf)),
            _ => Ok(None),
        }
    }

    /// Re-arm the command channel (device ignores fan cmds after LCD play mode).
    fn wake(&self) -> Result<()> {
        for cmd in [0x7Bu8, 0x34, 0x0A] {
            let _ = self.xfer(&build_cmd(cmd, self.ts()))?;
            std::thread::sleep(Duration::from_millis(150));
        }
        Ok(())
    }

    fn status(&self) -> Result<Option<Status>> {
        Ok(self.xfer(&build_cmd(CMD_GET_STATUS, self.ts()))?
            .as_ref()
            .and_then(parse_status))
    }

    fn set_fans(&self, duty_255: u8, sensor_mirror: u16) -> Result<()> {
        self.xfer(&build_set_fans(duty_255, sensor_mirror, self.ts()))?;
        Ok(())
    }
}

// ─────────────────────────── sensors ───────────────────────────

/// Read k10temp Tctl (°C) from sysfs.
fn read_cpu_tctl() -> Option<f32> {
    for hw in std::fs::read_dir("/sys/class/hwmon").ok()? {
        let dir = hw.ok()?.path();
        let name = std::fs::read_to_string(dir.join("name")).ok()?;
        if name.trim() != "k10temp" {
            continue;
        }
        for entry in std::fs::read_dir(&dir).ok()? {
            let p = entry.ok()?.path();
            let fname = p.file_name()?.to_str()?.to_string();
            if fname.ends_with("_label") {
                if let Ok(label) = std::fs::read_to_string(&p) {
                    if label.trim() == "Tctl" {
                        let input = p.to_str()?.replace("_label", "_input");
                        let milli: f32 = std::fs::read_to_string(&input).ok()?.trim().parse().ok()?;
                        return Some(milli / 1000.0);
                    }
                }
            }
        }
    }
    None
}

fn pct_to_255(pct: f32) -> u8 {
    (pct.clamp(0.0, 100.0) * 255.0 / 100.0).round() as u8
}

// ─────────────────────────── control loop ───────────────────────────

fn run() -> Result<()> {
    let cfg = Config::load();
    let use_cpu = cfg.temp_source.eq_ignore_ascii_case("cpu");
    let dt = cfg.interval_secs.max(0.5);
    let alpha = 1.0 - (-dt / cfg.smoothing_tau_secs.max(0.1)).exp();
    let max_step = cfg.slew_pct_per_sec * dt;

    println!(
        "hs2-fanctl: source={} interval={:.1}s tau={:.1}s slew={:.1}%/s curve={:?}",
        cfg.temp_source, dt, cfg.smoothing_tau_secs, cfg.slew_pct_per_sec, cfg.curve
    );

    let mut dev = Device::open()?;
    let mut ema_temp: Option<f32> = None;
    let mut duty_pct = cfg.min_duty_pct;
    let mut fails = 0u32;
    let mut ticks = 0u64;

    loop {
        let status = match dev.status() {
            Ok(Some(s)) => {
                fails = 0;
                Some(s)
            }
            Ok(None) | Err(_) => {
                fails += 1;
                if fails == 3 {
                    eprintln!("no telemetry x3 — re-waking");
                    let _ = dev.wake();
                }
                if fails >= 8 {
                    eprintln!("device unresponsive — reopening");
                    std::thread::sleep(Duration::from_secs(2));
                    match Device::open() {
                        Ok(d) => {
                            dev = d;
                            fails = 0;
                        }
                        Err(e) => eprintln!("reopen failed: {e}"),
                    }
                }
                None
            }
        };

        // Pick the driving temperature.
        let raw_temp = if use_cpu {
            read_cpu_tctl()
        } else {
            status.as_ref().map(|s| s.coolant_c)
        };

        if let Some(t) = raw_temp {
            let e = ema_temp.map_or(t, |prev| prev + alpha * (t - prev));
            ema_temp = Some(e);
            let target = cfg.interp(e);
            // slew-rate limit
            duty_pct = if target > duty_pct {
                (duty_pct + max_step).min(target)
            } else {
                (duty_pct - max_step).max(target)
            };
            duty_pct = duty_pct.clamp(cfg.min_duty_pct, cfg.max_duty_pct);

            // sensor mirror = the temp L-Connect would display; feed it the die temp
            // (or coolant) so the pump's on-screen readout stays plausible.
            let mirror = (read_cpu_tctl().unwrap_or(t) * 10.0) as u16;
            if let Err(err) = dev.set_fans(pct_to_255(duty_pct), mirror) {
                eprintln!("set_fans failed: {err}");
            }

            // Log once every ~10 s.
            ticks += 1;
            if ticks % ((10.0 / dt).round() as u64).max(1) == 0 {
                if let Some(s) = &status {
                    println!(
                        "coolant {:.1}C (ema {:.1}) -> duty {:.0}% | fans {}/{}/{} rpm  pump {} rpm",
                        s.coolant_c, e, duty_pct, s.fan_rpm[0], s.fan_rpm[1], s.fan_rpm[2], s.pump_rpm
                    );
                } else {
                    println!("temp {:.1}C (ema {:.1}) -> duty {:.0}% (no telemetry this tick)", t, e, duty_pct);
                }
            }
        }

        std::thread::sleep(Duration::from_secs_f32(dt));
    }
}

fn main() -> Result<()> {
    run()
}

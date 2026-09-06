//! HydroShift II AIO controller — pump + fan + RGB ring.
//!
//! Shares the LCD device's USB handle via [`SharedTransport`] (`Arc<LcdLink>`).
//! While the LCD is streaming H.264, control writes are handed to the stream
//! thread (see `LcdLink`) instead of going straight on the wire.

use super::lcd::{PendingCmd, SharedTransport};
use crate::crypto::PacketBuilder;
use crate::traits::{AioDevice, FanDevice, RgbDevice};
use anyhow::{Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbZoneInfo};
use lianli_transport::usb::{LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::debug;

const PUMP_MIN_RPM: u16 = 1600;
const PUMP_MAX_RPM_CIRCLE: u16 = 2500;
const PUMP_MAX_RPM_SQUARE: u16 = 3200;
const RING_LED_COUNT: usize = 24;

/// Telemetry parsed from GetH2Params response.
#[derive(Clone)]
pub struct H2Params {
    pub cpu_temp: u8,
    pub cpu_load: u8,
    pub gpu_temp: u8,
    pub gpu_load: u8,
    pub pump_rpm: u16,
    pub fan_rpm: [u16; 3],
    pub coolant_temp: u8,
    pub mac: Option<[u8; 6]>,
}

/// After LCD play mode the device ignores control commands until this
/// StopPlay → StopClock → GetVer preamble re-arms the channel.
/// Skipped while the LCD streams, with the same transition guard as the
/// other control writes, so re-arming never lands mid playback.
fn wake(transport: &SharedTransport) {
    let mut builder = PacketBuilder::new();
    let cmds = [
        builder.stop_play_header_winusb(),
        builder.stop_clock_header_winusb(),
        builder.get_ver_header_winusb(),
    ];
    for cmd in &cmds {
        let t = transport.lock();
        if transport.is_streaming() {
            debug!("H2 control channel: wake skipped, LCD streaming");
            return;
        }
        let _ = t.write(cmd, LCD_WRITE_TIMEOUT);
        let mut buf = [0u8; 512];
        let _ = t.read(&mut buf, LCD_READ_TIMEOUT);
        drop(t);
        std::thread::sleep(Duration::from_millis(150));
    }
    debug!("H2 control channel: wake preamble sent");
}

/// HydroShift II AIO controller (pump + fan + RGB ring via shared handle).
pub struct H2AioController {
    transport: SharedTransport,
    builder: Mutex<PacketBuilder>,
    last_fan_duties: Mutex<[u8; 3]>,
    last_pump_duty: Mutex<u8>,
    is_square: bool,
    is_wireless: AtomicBool,
    mac: Mutex<Option<[u8; 6]>>,
    /// Last GetH2Params reply plus when it arrived. Callers ask for coolant and
    /// fan RPM separately, but both live in the same 512-byte response; issuing
    /// two exchanges back to back made each one drain the other's reply, so the
    /// two fields alternated between real data and zeros. One exchange feeds
    /// both within this window.
    params_cache: Mutex<Option<(std::time::Instant, H2Params)>>,
    /// Last SyncPumpFan actually put on the wire: (when, pump duty, fan duties).
    last_sync: Mutex<Option<(std::time::Instant, u8, [u8; 3])>>,
    /// When the "telemetry held back while streaming" line was last logged.
    stale_params_logged_at: Mutex<Option<std::time::Instant>>,
    /// Raw ring bytes and interval of the last PushRgbData that went out
    /// or was queued, so an unchanged ring is not resent.
    last_rgb_payload: Mutex<Option<(Vec<u8>, u8)>>,
}

impl H2AioController {
    pub fn new(transport: SharedTransport, pid: u16) -> Self {
        let ctrl = Self {
            transport: Arc::clone(&transport),
            builder: Mutex::new(PacketBuilder::new()),
            last_fan_duties: Mutex::new([50, 50, 50]),
            last_pump_duty: Mutex::new(128),
            is_square: pid == 0xA034,
            is_wireless: AtomicBool::new(false),
            mac: Mutex::new(None),
            params_cache: Mutex::new(None),
            last_sync: Mutex::new(None),
            stale_params_logged_at: Mutex::new(None),
            last_rgb_payload: Mutex::new(None),
        };
        wake(&transport);
        tracing::info!("HydroShift II control channel opened (shared transport)");
        ctrl
    }

    pub fn set_wireless_mode(&self, enabled: bool) {
        self.is_wireless.store(enabled, Ordering::Relaxed);
    }

    pub fn is_wireless_mode(&self) -> bool {
        self.is_wireless.load(Ordering::Relaxed)
    }

    pub fn mac(&self) -> Option<[u8; 6]> {
        *self.mac.lock()
    }

    /// Put a fire-and-forget control command on the wire, or — while the LCD
    /// is streaming — hand it to the stream thread. A control write landing on
    /// a full ingest buffer hangs the MCU (usbmon, 2026-08-22/23), so nothing
    /// is written from here mid-stream. `play_safe` commands are sent by the
    /// stream thread once the panel reports headroom; the rest wait for the
    /// stream to end. Returns true if it was sent now.
    fn send_control(
        &self,
        label: &'static str,
        packet: Vec<u8>,
        reply_wait: Duration,
        play_safe: bool,
    ) -> Result<bool> {
        if !self.transport.is_streaming() {
            // Not streaming at the check. The write rechecks the flag under
            // the bulk mutex, the same mutex stream_begin holds while
            // flipping it, so a stream beginning in between is caught
            // before any byte reaches the pipe.
            if self.write_control(label, &packet, reply_wait)? {
                return Ok(true);
            }
        }
        debug!("H2: {label} deferred — LCD streaming");
        self.transport.defer(PendingCmd {
            label,
            packet,
            reply_wait,
            queued_at: std::time::Instant::now(),
            play_safe,
        });
        // The stream can end between the check above and the queueing:
        // stream_end() has then already drained the queue, and nothing
        // would ever send this packet — it would sit there until the *next*
        // stream ended and go out stale. Drain it here instead.
        if !self.transport.is_streaming() {
            self.send_stranded();
        }
        Ok(false)
    }

    /// Write one control packet and discard its reply, holding the transport
    /// across both halves so no other command's answer is consumed here.
    /// Returns false, nothing written, when a stream began while waiting
    /// for the transport, so the caller must queue the command instead.
    fn write_control(&self, label: &str, packet: &[u8], reply_wait: Duration) -> Result<bool> {
        let transport = self.transport.lock();
        if self.transport.is_streaming() {
            return Ok(false);
        }
        transport
            .write_full(packet, LCD_WRITE_TIMEOUT)
            .with_context(|| format!("H2: {label} write"))?;
        let mut buf = [0u8; 512];
        let _ = transport.read(&mut buf, reply_wait);
        Ok(true)
    }

    /// Send play-safe commands left in the queue by the teardown race above.
    /// The stream is over, so these go straight out. Commands that need the
    /// panel reinitialised first (PushRgbData) stay queued for the LCD
    /// stream thread, which owns the raw device handle.
    fn send_stranded(&self) {
        for cmd in self.transport.take_play_safe() {
            debug!("H2: sending {} stranded by stream teardown", cmd.label);
            match self.write_control(cmd.label, &cmd.packet, cmd.reply_wait) {
                Ok(true) => {}
                Ok(false) => {
                    // A new stream began before this could go out. Requeue
                    // it so the new stream sends it at a safe point or its
                    // teardown drains it, rather than losing the command.
                    debug!("H2: requeueing {} until the new stream ends", cmd.label);
                    self.transport.defer(cmd);
                }
                Err(e) => {
                    tracing::warn!("H2: stranded {} write failed: {e:#}", cmd.label);
                }
            }
        }
    }

    /// Log held-back telemetry at most once every STALE_PARAMS_LOG_INTERVAL, so
    /// a long stream does not fill the log with one line per poll.
    fn note_stale_params(&self, age: Duration) {
        const STALE_PARAMS_LOG_INTERVAL: Duration = Duration::from_secs(10);
        let mut last = self.stale_params_logged_at.lock();
        if last.is_none_or(|at| at.elapsed() >= STALE_PARAMS_LOG_INTERVAL) {
            debug!(
                "H2: serving telemetry from cache ({} ms old) — LCD streaming",
                age.as_millis()
            );
            *last = Some(std::time::Instant::now());
        }
    }

    /// How long a GetH2Params reply is reused before going back to the wire.
    /// Long enough to cover a poll cycle's coolant+RPM pair, far shorter than
    /// the 1s telemetry tick, so readings stay live.
    const PARAMS_CACHE_TTL: Duration = Duration::from_millis(300);

    pub fn get_h2_params(&self) -> Result<H2Params> {
        if let Some((at, cached)) = self.params_cache.lock().as_ref() {
            if at.elapsed() < Self::PARAMS_CACHE_TTL {
                return Ok(cached.clone());
            }
        }
        // A control write landing on a full ingest buffer hangs the MCU, which
        // is why every command in send_control defers while the LCD streams.
        // This one is a read, so there is nothing to queue — but it is the same
        // 512-byte header on the same pipe, and on a cache miss it goes out
        // twice. Hold the last reading instead: telemetry goes stale for the
        // length of the stream, which is recoverable, and a wedged pump is not.
        if self.transport.is_streaming() {
            if let Some((at, cached)) = self.params_cache.lock().as_ref() {
                self.note_stale_params(at.elapsed());
                return Ok(cached.clone());
            }
            anyhow::bail!("H2: GetH2Params withheld — LCD streaming, no cached reading yet");
        }
        let header = self.builder.lock().get_h2_params_header_winusb();

        // The transport stays locked across both halves of each exchange; it
        // used to be released between them, letting another command's reply be
        // consumed here.
        // Two attempts. sync_pump_fan() fires once a second on this shared
        // transport and only waits before discarding its own reply, so a late
        // answer can still be sitting in the pipe. The first exchange then
        // consumes that stale frame and the second gets the real one — which is
        // why coolant read a constant 105 C (a field of the fixed SyncPumpFan
        // reply) instead of the true ~26 C.
        let mut buf = [0u8; 512];
        let mut last_err: Option<anyhow::Error> = None;
        let mut got = false;
        let mut stream_began = false;
        for attempt in 0..2 {
            let hdr = if attempt == 0 {
                header.clone()
            } else {
                self.builder.lock().get_h2_params_header_winusb()
            };
            let res = {
                let transport = self.transport.lock();
                if self.transport.is_streaming() {
                    // Same transition guard as write_control. The earlier
                    // check passed, but a stream began before the lock was
                    // acquired, so do not touch the pipe.
                    stream_began = true;
                    None
                } else {
                    Some(
                        transport
                            .write_full(&hdr, LCD_WRITE_TIMEOUT)
                            .context("H2: GetH2Params write")
                            .and_then(|_| {
                                transport
                                    .read(&mut buf, LCD_READ_TIMEOUT)
                                    .context("H2: GetH2Params read")
                            }),
                    )
                }
            };
            match res {
                None => break,
                Some(Ok(k)) if k >= 32 => {
                    got = true;
                    break;
                }
                Some(Ok(k)) => last_err = Some(anyhow::anyhow!("response too short ({k} bytes)")),
                Some(Err(e)) => last_err = Some(e),
            }
        }
        if stream_began {
            // Serve the last reading rather than failing the poll, the
            // stream will end and refresh it.
            if let Some((at, cached)) = self.params_cache.lock().as_ref() {
                self.note_stale_params(at.elapsed());
                return Ok(cached.clone());
            }
            anyhow::bail!("H2: GetH2Params withheld — LCD streaming began mid exchange");
        }
        if !got {
            return Err(last_err.unwrap_or_else(|| anyhow::anyhow!("H2: GetH2Params failed")));
        }

        let mac = {
            let m = [buf[22], buf[23], buf[24], buf[25], buf[26], buf[27]];
            if m.iter().all(|&b| b == 0) {
                None
            } else {
                Some(m)
            }
        };
        if mac.is_some() {
            *self.mac.lock() = mac;
        }

        let parsed = H2Params {
            cpu_temp: 0,
            cpu_load: 0,
            gpu_temp: 0,
            gpu_load: 0,
            pump_rpm: u16::from_be_bytes([buf[20], buf[21]]),
            fan_rpm: [
                u16::from_be_bytes([buf[14], buf[15]]),
                u16::from_be_bytes([buf[16], buf[17]]),
                u16::from_be_bytes([buf[18], buf[19]]),
            ],
            coolant_temp: buf[13],
            mac,
        };
        *self.params_cache.lock() = Some((std::time::Instant::now(), parsed.clone()));
        Ok(parsed)
    }

    /// Send pump + fan PWM via SyncPumpFan (0xFB).
    pub fn sync_pump_fan(&self, pump_duty: u8, fan_duties: [u8; 3]) -> Result<()> {
        if self.is_wireless.load(Ordering::Relaxed) {
            return Ok(());
        }
        // FIX: drop a SyncPumpFan that repeats the previous one too soon. The
        // controller calls set_fan_speeds() and then set_pump_speed(), and each
        // sends a full packet — but SyncPumpFan already carries pump *and* fans,
        // so the second is an exact duplicate ~0.1ms behind the first. That put
        // two commands per second on the wire against the ~3.6s L-Connect uses,
        // roughly seven times the vendor's rate, and this controller stops
        // answering writes after about 26s of it (measured at +26.294s,
        // +26.498s and +26.522s across three runs). Reads keep working, so the
        // fans just decay to minimum with nothing in the log.
        //
        // Identical packets still refresh every RESYNC_INTERVAL, far inside the
        // ~13s the firmware waits before falling back on its own, and any change
        // in pump or fan duty goes out immediately.
        // Rate limit: L-Connect re-sends about every 3.6s, and this controller
        // stops answering writes after a fixed number of them regardless of
        // content — 2/s died at 26.5s, 1/s at 47.1s. Skipping only *identical*
        // packets was not enough, because a curve's duty jitters by a point or
        // two every tick and every jittered packet went out anyway.
        //
        // So gate on elapsed time, not equality, and let a meaningful change
        // through immediately so the UI still feels responsive.
        const RESYNC_INTERVAL: Duration = Duration::from_millis(3600);
        const SIGNIFICANT_DUTY_STEP: u8 = 8;
        {
            let last = self.last_sync.lock();
            if let Some((at, prev_pump, prev_fans)) = *last {
                // The pump needs the same deadband as the fans, and for the same
                // reason: its duty comes from a curve evaluated every second and
                // jitters by a point or two per tick. Comparing it exactly let
                // every jittered tick through, which put the packet rate back
                // where this gate exists to stop it. Compared in duty space so
                // one threshold covers both, rather than in the pump's PWM
                // period, whose scale is model-dependent and non-linear.
                let changed = prev_pump.abs_diff(pump_duty) >= SIGNIFICANT_DUTY_STEP
                    || prev_fans
                        .iter()
                        .zip(fan_duties.iter())
                        .any(|(a, b)| a.abs_diff(*b) >= SIGNIFICANT_DUTY_STEP);
                if !changed && at.elapsed() < RESYNC_INTERVAL {
                    return Ok(());
                }
            }
        }
        let pump_pwm = self.duty_to_pwm(pump_duty);

        let header = self.builder.lock().sync_pump_fan_header_winusb(
            pump_pwm,
            fan_duties[0],
            fan_duties[1],
            fan_duties[2],
        );
        // Reply wait 250 ms: at 50 ms a slower answer stayed queued and
        // poisoned the next read.
        let sent = self.send_control("SyncPumpFan", header, Duration::from_millis(250), true)?;
        *self.last_sync.lock() = Some((std::time::Instant::now(), pump_duty, fan_duties));
        debug!(
            "H2: SyncPumpFan pump_pwm={pump_pwm} fans={:?}{}",
            fan_duties,
            if sent { "" } else { " (deferred)" }
        );
        Ok(())
    }

    /// Upload full-ring RGB frames via PushRgbData (0xFC); firmware loops
    /// them at `interval_ms`.
    pub fn send_rgb_frames(&self, frames: &[Vec<[u8; 3]>], interval_ms: u8) -> Result<()> {
        if frames.is_empty() {
            return Ok(());
        }
        // Bridged to a wireless AIO: the same 24-LED ring is driven over RF
        // through the pump-head device, as the fan and pump paths already
        // are, so this packet is redundant here and the wired write is
        // skipped.
        if self.is_wireless.load(Ordering::Relaxed) {
            debug!("H2: PushRgbData skipped — ring is driven over RF (wireless mode)");
            return Ok(());
        }
        let total_frames = frames.len();

        let mut raw = Vec::with_capacity(total_frames * RING_LED_COUNT * 3);
        for frame in frames {
            for led in 0..RING_LED_COUNT {
                let c = frame.get(led).copied().unwrap_or([0, 0, 0]);
                raw.extend_from_slice(&c);
            }
        }

        // The daemon re-applies every configured effect on each config
        // reload, including LCD media switches. Each write here costs a
        // stop/push/reopen cycle and a second of LCD pause, so an unchanged
        // ring is not resent.
        let payload_key = (raw.clone(), interval_ms);
        if self.last_rgb_payload.lock().as_ref() == Some(&payload_key) {
            debug!("H2: PushRgbData skipped — ring unchanged");
            return Ok(());
        }

        let compressed = crate::tinyuz::compress(&raw).context("compressing RGB data")?;

        let mut payload = compressed;
        payload.push((total_frames >> 8) as u8);
        payload.push((total_frames & 0xFF) as u8);
        payload.push(interval_ms);
        payload.push(RING_LED_COUNT as u8);

        let header = self
            .builder
            .lock()
            .push_rgb_data_header_winusb(payload.len());
        let mut packet = Vec::with_capacity(512 + payload.len());
        packet.extend_from_slice(&header);
        packet.extend_from_slice(&payload);

        // This packet is a full 512-byte header plus the compressed RGB payload,
        // so it exceeds one bulk packet; send_control uses write_full so every
        // byte goes out (a plain write() merely warned on the short write).
        //
        // Not play-safe: a PushRgbData during H.264 play mode hangs the panel
        // even at buffer level 1 (2026-08-23), and one sent after the stream
        // ended hangs it too (2026-09-06), even after an acknowledged
        // StopPlay. Once the panel has played anything, the packet is
        // therefore handed to the LCD stream thread, which stops play,
        // reopens the handle, reruns the init sequence, sends it, and
        // resumes (`reinit_and_flush_unsafe`). Straight after enumeration it
        // goes out directly, as it always did.
        let cmd = PendingCmd {
            label: "PushRgbData",
            packet,
            reply_wait: Duration::from_millis(100),
            queued_at: std::time::Instant::now(),
            play_safe: false,
        };
        let sent = if self.transport.is_streaming() {
            debug!("H2: PushRgbData queued — the stream thread will stop play, send it and reopen");
            self.transport.defer(cmd);
            false
        } else {
            // No stream thread is going to run the cycle, so run it here.
            // Straight after enumeration the panel copes with a bare write
            // (pid 22766: one lost GetVer reply, then fine), but a bare
            // write between two streams silenced it (pid 39118, 250 s), so
            // every write off the stream thread takes the full cycle.
            // StopPlay/StopClock re-arm the control channel first; if a
            // stream begins under us, queue instead.
            // A ring write still queued from an earlier stream is stale
            // now: latest wins, as `defer` does.
            self.transport.take_unsafe();
            let mut builder = PacketBuilder::new();
            let stop = builder.stop_play_header_winusb();
            let stop_clock = builder.stop_clock_header_winusb();
            let armed = self.write_control("StopPlay", &stop, LCD_READ_TIMEOUT)?
                && self.write_control("StopClock", &stop_clock, LCD_READ_TIMEOUT)?;
            let pushed = if armed {
                match self.transport.push_and_recover(
                    "HydroShift II control",
                    std::slice::from_ref(&cmd),
                    LCD_WRITE_TIMEOUT,
                    false,
                ) {
                    Ok(pushed) => pushed,
                    Err(e) => {
                        // Not delivered: keep it queued for the next stream
                        // start, and let the caller see the failure.
                        self.transport.defer(cmd);
                        return Err(e);
                    }
                }
            } else {
                false
            };
            if !pushed {
                debug!("H2: PushRgbData queued — a stream began first");
                self.transport.defer(cmd);
            }
            pushed
        };
        // Queued writes count too: the queue keeps the latest ring write
        // until it is delivered (stream end no longer drops it), so a later
        // identical apply has nothing to add.
        *self.last_rgb_payload.lock() = Some(payload_key);
        debug!(
            "H2: PushRgbData {} frame(s), {} LEDs, {} bytes{}",
            total_frames,
            RING_LED_COUNT,
            payload.len(),
            if sent { "" } else { " (queued)" }
        );
        Ok(())
    }

    fn pump_max_rpm(&self) -> u16 {
        if self.is_square {
            PUMP_MAX_RPM_SQUARE
        } else {
            PUMP_MAX_RPM_CIRCLE
        }
    }

    fn rpm_to_pwm(&self, rpm: u16) -> u16 {
        let rpm = rpm.clamp(PUMP_MIN_RPM, self.pump_max_rpm()) as f32;
        let pwm = if self.is_square {
            if rpm <= 1800.0 {
                1590.0 - (rpm - 1600.0) * 0.95
            } else if rpm <= 2000.0 {
                1400.0 - (rpm - 1800.0)
            } else if rpm <= 2200.0 {
                1200.0 - (rpm - 2000.0)
            } else if rpm <= 2400.0 {
                1000.0 - (rpm - 2200.0)
            } else if rpm <= 2600.0 {
                800.0 - (rpm - 2400.0)
            } else if rpm <= 2800.0 {
                580.0 - (rpm - 2600.0) * 1.11
            } else if rpm <= 3000.0 {
                330.0 - (rpm - 2800.0) * 1.2
            } else {
                90.0 - (rpm - 3000.0) * 0.45
            }
        } else {
            if rpm < 1720.0 {
                1500.0 - (rpm - 1600.0) * 1.625
            } else if rpm < 1870.0 {
                1300.0 - (rpm - 1720.0) * 2.0
            } else if rpm < 2000.0 {
                1000.0 - (rpm - 1870.0) * 1.23
            } else if rpm < 2300.0 {
                840.0 - (rpm - 2000.0) * 2.0
            } else if rpm < 2400.0 {
                240.0 - (rpm - 2300.0) * 1.8
            } else {
                60.0 - (rpm - 2400.0) * 0.5
            }
        };
        pwm.round() as u16
    }

    fn duty_to_pwm(&self, duty: u8) -> u16 {
        let pct = (duty as f32 / 255.0).clamp(0.0, 1.0);
        let rpm = PUMP_MIN_RPM as f32 + pct * (self.pump_max_rpm() - PUMP_MIN_RPM) as f32;
        self.rpm_to_pwm(rpm.round() as u16)
    }
}

fn scale_brightness([r, g, b]: [u8; 3], brightness: u8) -> [u8; 3] {
    let scale = (lianli_shared::rgb::brightness_scale(brightness) as f32) / 4.0;
    [
        (r as f32 * scale).round() as u8,
        (g as f32 * scale).round() as u8,
        (b as f32 * scale).round() as u8,
    ]
}

impl FanDevice for H2AioController {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        let mut duties = *self.last_fan_duties.lock();
        // FIX: SyncPumpFan's fan bytes are 0-255, NOT 0-100. Verified on
        // hardware: raw byte 150 -> 1256 RPM (model RPM = 8.43 x byte, 0.6% error).
        duties[slot as usize % 3] = duty;
        *self.last_fan_duties.lock() = duties;
        self.sync_pump_fan(*self.last_pump_duty.lock(), duties)
    }

    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        let mut fan_duties = [0u8; 3];
        for (i, &d) in duties.iter().enumerate().take(3) {
            // FIX: raw 0-255, see note in set_fan_speed.
            fan_duties[i] = d;
        }
        *self.last_fan_duties.lock() = fan_duties;
        self.sync_pump_fan(*self.last_pump_duty.lock(), fan_duties)
    }

    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        if self.is_wireless_mode() {
            return Ok(Vec::new());
        }
        let params = self.get_h2_params()?;
        Ok(params.fan_rpm.to_vec())
    }

    fn fan_slot_count(&self) -> u8 {
        3
    }

    fn has_pump_control(&self) -> bool {
        true
    }

    fn poll_coolant_temp(&self) -> Option<f32> {
        if self.is_wireless_mode() {
            return None;
        }
        self.get_h2_params().ok().map(|p| p.coolant_temp as f32)
    }

    fn set_pump_speed(&self, duty: u8) -> Result<()> {
        *self.last_pump_duty.lock() = duty;
        let fans = *self.last_fan_duties.lock();
        self.sync_pump_fan(duty, fans)
    }

    fn wireless_link_mac(&self) -> Option<[u8; 6]> {
        self.mac()
    }

    fn set_wireless_bound(&self, bound: bool) {
        self.set_wireless_mode(bound);
    }
}

impl AioDevice for H2AioController {
    fn read_pump_rpm(&self) -> Result<u16> {
        if self.is_wireless_mode() {
            return Ok(0);
        }
        let params = self.get_h2_params()?;
        Ok(params.pump_rpm)
    }

    fn read_coolant_temp(&self) -> Result<f32> {
        if self.is_wireless_mode() {
            return Ok(0.0);
        }
        let params = self.get_h2_params()?;
        Ok(params.coolant_temp as f32)
    }
}

impl RgbDevice for H2AioController {
    fn device_name(&self) -> String {
        "HydroShift II LCD RGB Ring".to_string()
    }

    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Off, RgbMode::Static, RgbMode::Direct]
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![RgbZoneInfo {
            name: "Ring".to_string(),
            led_count: RING_LED_COUNT as u16,
        }]
    }

    fn supports_direct(&self) -> bool {
        true
    }

    fn rf_owned(&self) -> bool {
        self.is_wireless_mode()
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        if zone != 0 {
            anyhow::bail!("H2 RGB: zone {zone} out of range (only zone 0)");
        }
        let color = if effect.mode == RgbMode::Off || effect.disabled {
            [0, 0, 0]
        } else {
            let base = effect.colors.first().copied().unwrap_or([255, 255, 255]);
            scale_brightness(base, effect.brightness)
        };
        let frame = vec![color; RING_LED_COUNT];
        self.send_rgb_frames(&[frame], 100)
    }

    fn set_direct_colors(&self, zone: u8, colors: &[[u8; 3]]) -> Result<()> {
        if zone != 0 {
            anyhow::bail!("H2 RGB: zone {zone} out of range (only zone 0)");
        }
        self.send_rgb_frames(&[colors.to_vec()], 100)
    }
}

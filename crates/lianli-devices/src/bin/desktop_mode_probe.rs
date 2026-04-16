//! Probe a desktop-mode Lian Li display (VID=0x1A86, PID 0xAD10..0xAD3F).
//!
//! Walks the TURZX init sequence (see target/TURZX.md), decodes every
//! response, picks a supported codec, sends the start-config control packet,
//! pushes one test frame, then sends the power-off packet on exit.

use anyhow::{bail, Context, Result};
use clap::Parser;
use image::{ImageBuffer, Rgb};
use lianli_devices::turzx::{
    self, build_config_packet, build_power_off, parse_vendor_desc, pick_format, pick_mode,
    Mode, TurzxDisplay, VendorCaps, FMT_H264, FMT_MJPEG,
};
use lianli_transport::usb::{UsbTransport, LCD_READ_TIMEOUT};
use std::io::{BufRead, Write};
use std::time::Duration;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value = "0xad21")]
    pid: String,

    /// Override auto-detected codec (mjpeg | h264).
    #[arg(long)]
    format: Option<String>,

    /// RGB color used for the single test frame.
    #[arg(long, default_value = "255,0,0")]
    color: String,

    /// JPEG quality for MJPEG path.
    #[arg(long, default_value_t = 85)]
    quality: u8,

    /// Seconds to wait after sending the frame before power-off.
    #[arg(long, default_value_t = 10)]
    hold_secs: u64,

    /// Skip interactive Enter-to-continue prompts.
    #[arg(long)]
    no_pause: bool,
}

fn parse_hex_u16(s: &str) -> Result<u16> {
    let trimmed = s.trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(trimmed, 16).with_context(|| format!("parsing PID '{s}'"))
}

fn parse_color(s: &str) -> Result<[u8; 3]> {
    let parts: Vec<_> = s.split(',').collect();
    if parts.len() != 3 {
        bail!("color must be 'r,g,b' e.g. 255,0,0");
    }
    Ok([parts[0].trim().parse()?, parts[1].trim().parse()?, parts[2].trim().parse()?])
}

fn press_enter(no_pause: bool, prompt: &str) {
    if no_pause {
        println!("\n>>> {prompt}");
        return;
    }
    print!("\n>>> {prompt}\n    press Enter (Ctrl-C to abort)... ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok();
}

fn hex_first(label: &str, data: &[u8], n: usize) {
    let n = data.len().min(n);
    println!("  {label} [{} bytes, first {n}]: {:02x?}", data.len(), &data[..n]);
}

fn print_caps_summary(caps: &VendorCaps) {
    println!();
    println!("  capability summary:");
    println!("    size range : {}×{} .. {}×{}", caps.min_w, caps.min_h, caps.max_w, caps.max_h);
    println!("    max xfer   : {} bytes", caps.max_transfer);
    println!(
        "    codecs     : {}{}",
        if caps.supports_mjpeg { "MJPEG " } else { "" },
        if caps.supports_h264 { "H.264" } else { "" }
    );
    println!("    modes      : {} entries", caps.modes.len());
    for (i, m) in caps.modes.iter().enumerate() {
        println!("      [{i}] {}×{} @ {}Hz", m.width, m.height, m.refresh_hz);
    }
}

fn parse_edid(buf: &[u8]) {
    println!("  EDID hex ({} bytes):", buf.len());
    for chunk in buf.chunks(16) {
        println!("    {:02x?}", chunk);
    }
    if buf.len() < 128 {
        println!("  ! EDID too short to parse");
        return;
    }
    let sig = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
    if buf[..8] != sig {
        println!("  ! EDID signature invalid");
        return;
    }
    let m = ((buf[8] as u16) << 8) | buf[9] as u16;
    let letter = |v: u16| (((v & 0x1F) as u8).saturating_sub(1) + b'A') as char;
    let mfr: String = [letter(m >> 10), letter(m >> 5), letter(m)].iter().collect();
    let product = u16::from_le_bytes([buf[10], buf[11]]);
    let serial = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
    let week = buf[16];
    let year = 1990 + buf[17] as u16;
    let version = (buf[18], buf[19]);
    let phys_w_cm = buf[21];
    let phys_h_cm = buf[22];
    let pixel_clock_khz = u16::from_le_bytes([buf[54], buf[55]]) as u32 * 10;
    let h_active = (((buf[58] as u16) & 0xF0) << 4) | buf[56] as u16;
    let v_active = (((buf[61] as u16) & 0xF0) << 4) | buf[59] as u16;
    let checksum: u8 = buf[..128].iter().copied().fold(0u8, |a, b| a.wrapping_add(b));
    println!();
    println!("  EDID decoded:");
    println!("    manufacturer     : {mfr}");
    println!("    product code     : {product:#06x}");
    println!("    serial number    : {serial}");
    println!("    manufacture date : week {week}, year {year}");
    println!("    EDID version     : {}.{}", version.0, version.1);
    println!("    physical size    : {phys_w_cm} × {phys_h_cm} cm");
    println!("    preferred timing : {h_active} × {v_active} @ {pixel_clock_khz} kHz");
    println!(
        "    checksum         : 0x{checksum:02x} ({})",
        if checksum == 0 { "OK" } else { "BAD" }
    );
}

fn dump_config_packet(pkt: &[u8]) {
    println!("  config packet ({} bytes):", pkt.len());
    for tlv in pkt.chunks(4) {
        let label = match (tlv[0], tlv[1], tlv[2]) {
            (0xAF, 0x20, 0x00) => "start marker",
            (0xAF, 0x20, 0x01) => "width hi",
            (0xAF, 0x20, 0x02) => "width lo",
            (0xAF, 0x20, 0x03) => "height hi",
            (0xAF, 0x20, 0x04) => "height lo",
            (0xAF, 0x20, 0x11) => "format MJPEG",
            (0xAF, 0x20, 0x12) => "format H.264",
            (0xAF, 0x20, 0x1F) if tlv.len() > 3 && tlv[3] == 0x01 => "power on",
            (0xAF, 0x20, 0x1F) if tlv.len() > 3 && tlv[3] == 0x02 => "power off",
            _ => "?",
        };
        println!("    {:02x?}   {label}", tlv);
    }
}

fn make_jpeg(width: u32, height: u32, color: [u8; 3], quality: u8) -> Result<Vec<u8>> {
    let img = ImageBuffer::from_pixel(width, height, Rgb(color));
    let tj = turbojpeg::Image {
        pixels: img.as_raw().as_slice(),
        width: width as usize,
        pitch: width as usize * 3,
        height: height as usize,
        format: turbojpeg::PixelFormat::RGB,
    };
    let buf = turbojpeg::compress(tj, quality as i32, turbojpeg::Subsamp::Sub2x2)
        .map_err(|e| anyhow::anyhow!("turbojpeg: {e}"))?;
    Ok(buf.to_vec())
}

fn control_in(
    transport: &UsbTransport,
    label: &str,
    req_type: u8,
    b_request: u8,
    w_value: u16,
    w_index: u16,
    len: u16,
) -> Result<Vec<u8>> {
    println!(
        "  control IN [{label}]  bmReq=0x{:02x} bReq=0x{:02x} wValue=0x{:04x} wIndex=0x{:04x} wLength={len}",
        req_type, b_request, w_value, w_index
    );
    let mut buf = vec![0u8; len as usize];
    match transport.control_in(req_type, b_request, w_value, w_index, &mut buf, LCD_READ_TIMEOUT) {
        Ok(n) => {
            buf.truncate(n);
            println!("    ok: {n} bytes");
            Ok(buf)
        }
        Err(e) => {
            println!("    FAILED: {e}");
            Err(anyhow::anyhow!("{e}"))
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    let pid = parse_hex_u16(&args.pid)?;
    let color = parse_color(&args.color)?;

    println!("Target: {:04x}:{:04x}", turzx::VID, pid);

    let mut transport = UsbTransport::open(turzx::VID, pid)
        .map_err(|e| anyhow::anyhow!("open: {e}"))?;

    press_enter(args.no_pause, "Phase 0: USB reset + claim interface 0");
    match transport.reset() {
        Ok(()) => println!("  reset ok"),
        Err(e) => println!("  reset failed: {e} (continuing)"),
    }
    std::thread::sleep(Duration::from_millis(300));
    transport
        .detach_and_configure("desktop-mode-probe")
        .map_err(|e| anyhow::anyhow!("claim: {e}"))?;
    println!("  claimed interface 0 alt 0");

    press_enter(args.no_pause, "Phase 1: read vendor mode descriptor (0x5F)");
    let desc = control_in(&transport, "vendor mode descriptor", 0x81, 0x06, 0x5F00, 0, 512)?;
    hex_first("  raw", &desc, desc.len().min(64));
    let caps = parse_vendor_desc(&desc).context("parsing vendor descriptor")?;
    print_caps_summary(&caps);

    press_enter(args.no_pause, "Phase 2: poll status until ready bit 0x10");
    let mut ready = false;
    for attempt in 0..100 {
        let resp = control_in(
            &transport,
            &format!("status poll #{}", attempt + 1),
            0xC1,
            0x01,
            0,
            0,
            1,
        )?;
        if !resp.is_empty() {
            let s = resp[0];
            println!(
                "    status = 0x{:02x}  (ready=0x10 {}, 0x08 {})",
                s,
                if s & 0x10 != 0 { "SET" } else { "clear" },
                if s & 0x08 != 0 { "SET" } else { "clear" }
            );
            if s & 0x10 != 0 {
                ready = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !ready {
        bail!("device never reported ready (bit 0x10 never set)");
    }

    press_enter(args.no_pause, "Phase 3: read EDID");
    let edid = control_in(&transport, "EDID", 0xC1, 0x02, 0, 0, 128)?;
    parse_edid(&edid);

    // Probe ships only an MJPEG encoder; default to that so the frame test
    // actually renders. Pass `--format h264` to exercise the config packet
    // without a frame payload.
    let format_forced = args.format.as_deref().unwrap_or("mjpeg");
    let format = pick_format(&caps, Some(format_forced))?;
    let mode: Mode = pick_mode(&caps)?;
    let (width, height) = (mode.width, mode.height);
    let fmt_name = if format == FMT_MJPEG { "MJPEG" } else { "H.264" };
    println!();
    println!("  chosen: {width}×{height} @ {}Hz, {fmt_name} ({:#06x})", mode.refresh_hz, format);

    // Probe releases the low-level transport and hands off to TurzxDisplay for
    // the post-init streaming phases. That way we exercise the exact code path
    // the daemon bridge will use.
    drop(transport);
    std::thread::sleep(Duration::from_millis(150));

    let mut display = TurzxDisplay::open(pid).context("reopening via TurzxDisplay")?;

    press_enter(args.no_pause, "Phase 4: start_streaming (display info + fmt + power on)");
    let cfg = build_config_packet(width, height, format);
    dump_config_packet(&cfg);
    display.start_streaming(mode, format).context("start_streaming")?;
    println!("    ok");
    std::thread::sleep(Duration::from_millis(50));

    if format == FMT_MJPEG {
        press_enter(args.no_pause, "Phase 5: send one JPEG frame via send_jpeg_frame");
        let jpeg = make_jpeg(width as u32, height as u32, color, args.quality)?;
        println!("  encoded JPEG: {} bytes", jpeg.len());
        display.send_jpeg_frame(&jpeg).context("send_jpeg_frame")?;
        println!("    ok");
        println!(
            "\n  frame sent. Holding for {} seconds — watch the panel...",
            args.hold_secs
        );
        std::thread::sleep(Duration::from_secs(args.hold_secs));
    } else if format == FMT_H264 {
        println!();
        println!(
            "  note: the probe does not ship an H.264 encoder. Use the daemon \
             bridge for real H.264 output. Skipping frame send (held {}s).",
            args.hold_secs
        );
        std::thread::sleep(Duration::from_secs(args.hold_secs));
    }

    press_enter(args.no_pause, "Phase 6: send_power_off and exit");
    let off = build_power_off();
    dump_config_packet(&off);
    if let Err(e) = display.send_power_off() {
        println!("  (power-off failed, ignoring: {e})");
    }

    Ok(())
}

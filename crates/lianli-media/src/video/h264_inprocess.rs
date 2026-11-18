use anyhow::{anyhow, bail, Context, Result};
use ffmpeg_next as ffmpeg;
use std::time::Instant;
use tracing::{debug, info};

static FFMPEG_INIT: std::sync::Once = std::sync::Once::new();

/// Initialise the libavcodec library exactly once per process. Must be called
/// before any `ffmpeg_next` API. Safe to call multiple times.
pub fn ensure_ffmpeg_initialized() {
    FFMPEG_INIT.call_once(|| {
        if let Err(e) = ffmpeg::init() {
            tracing::error!("ffmpeg::init failed: {e}");
        }
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Error);
    });
}

/// libavcodec H.264 encoder specialised for 32 bit framebuffers arriving
/// from evdi, in either channel order the negotiated DRM fourcc dictates.
/// Kept persistent across frames. Returns complete NAL packets
/// synchronously, unlike the CLI-based [`super::LiveH264Encoder`] which
/// pipelines via subprocess stdio.
pub struct H264Encoder {
    encoder: ffmpeg::encoder::Video,
    scaler: ffmpeg::software::scaling::Context,
    frame_in: ffmpeg::frame::Video,
    frame_out: ffmpeg::frame::Video,
    width: u32,
    height: u32,
    start: Instant,
    packet: ffmpeg::Packet,
    backend: &'static str,
}

impl H264Encoder {
    /// `rgb_byte_order` selects the input interpretation: true means the
    /// framebuffer stores red in the first byte of each pixel, as AB24 and
    /// XB24 do, false means blue first, as XR24 and AR24 do.
    pub fn new(
        width: u32,
        height: u32,
        fps: u32,
        rgb_byte_order: bool,
        hardware_video: bool,
    ) -> Result<Self> {
        ensure_ffmpeg_initialized();

        let src_pixel = if rgb_byte_order {
            ffmpeg::util::format::Pixel::RGBA
        } else {
            ffmpeg::util::format::Pixel::BGRA
        };
        let gop = (fps / 2).max(1);
        let mut last_err: Option<anyhow::Error> = None;
        for &name in encoder_names(hardware_video) {
            match try_open_encoder(name, width, height, fps, gop) {
                Ok(encoder) => {
                    info!("H.264 encoder: {name}");
                    let scaler = ffmpeg::software::scaling::Context::get(
                        src_pixel,
                        width,
                        height,
                        ffmpeg::util::format::Pixel::YUV420P,
                        width,
                        height,
                        ffmpeg::software::scaling::Flags::BILINEAR,
                    )
                    .with_context(|| format!("building sws scaler {src_pixel:?} -> YUV420P"))?;
                    let frame_in = ffmpeg::frame::Video::new(src_pixel, width, height);
                    let frame_out = ffmpeg::frame::Video::new(
                        ffmpeg::util::format::Pixel::YUV420P,
                        width,
                        height,
                    );
                    return Ok(Self {
                        encoder,
                        scaler,
                        frame_in,
                        frame_out,
                        width,
                        height,
                        start: Instant::now(),
                        packet: ffmpeg::Packet::empty(),
                        backend: name,
                    });
                }
                Err(e) => {
                    debug!("H.264 encoder {name} unavailable: {e:#}");
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("no H.264 encoder available")))
    }

    pub fn encode(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        self.encode_strided(frame, self.width as usize * 4)
    }

    pub fn backend(&self) -> &'static str {
        self.backend
    }

    pub fn encode_strided(&mut self, frame: &[u8], source_stride: usize) -> Result<Vec<u8>> {
        self.copy_pixels_in(frame, source_stride)?;
        // Encoders may retain the previous frame's storage after returning a packet.
        let result = unsafe { ffmpeg::ffi::av_frame_make_writable(self.frame_out.as_mut_ptr()) };
        if result < 0 {
            return Err(ffmpeg::Error::from(result)).context("Preparing reusable H.264 frame");
        }
        self.scaler
            .run(&self.frame_in, &mut self.frame_out)
            .context("sws scale to YUV420P")?;
        self.frame_out
            .set_pts(Some(self.start.elapsed().as_micros() as i64));
        self.encoder
            .send_frame(&self.frame_out)
            .context("encoder.send_frame")?;

        let mut out = Vec::new();
        loop {
            match self.encoder.receive_packet(&mut self.packet) {
                Ok(()) => {
                    if let Some(data) = self.packet.data() {
                        out.extend_from_slice(data);
                    }
                }
                Err(ffmpeg::Error::Other { errno }) if errno == libc::EAGAIN => break,
                Err(error) => return Err(error).context("Receiving H.264 output"),
            }
        }
        Ok(out)
    }

    fn copy_pixels_in(&mut self, frame: &[u8], source_stride: usize) -> Result<()> {
        let row_bytes = self.width as usize * 4;
        if source_stride < row_bytes {
            bail!("frame stride {source_stride} is smaller than a {row_bytes}-byte row");
        }
        let expected = source_stride
            .checked_mul(self.height as usize - 1)
            .and_then(|bytes| bytes.checked_add(row_bytes))
            .context("frame layout overflow")?;
        if frame.len() < expected {
            bail!("frame buffer too small: {} < {}", frame.len(), expected);
        }
        let stride = self.frame_in.stride(0);
        if stride == row_bytes && source_stride == row_bytes {
            self.frame_in.data_mut(0)[..expected].copy_from_slice(&frame[..expected]);
        } else {
            let dst = self.frame_in.data_mut(0);
            for y in 0..self.height as usize {
                let src_off = y * source_stride;
                let dst_off = y * stride;
                dst[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&frame[src_off..src_off + row_bytes]);
            }
        }
        Ok(())
    }
}

fn encoder_names(hardware_video: bool) -> &'static [&'static str] {
    if hardware_video {
        &["h264_nvenc", "h264_amf", "libx264"]
    } else {
        &["libx264"]
    }
}

fn try_open_encoder(
    name: &str,
    width: u32,
    height: u32,
    fps: u32,
    gop: u32,
) -> Result<ffmpeg::encoder::Video> {
    let codec = ffmpeg::encoder::find_by_name(name)
        .ok_or_else(|| anyhow!("codec {name} not built into libavcodec"))?;
    let ctx = ffmpeg::codec::context::Context::new_with_codec(codec);

    let mut opts = ffmpeg::Dictionary::new();
    match name {
        "h264_nvenc" => {
            opts.set("preset", "p1");
            opts.set("tune", "ull");
            opts.set("rc", "vbr");
            opts.set("forced-idr", "1");
            opts.set("zerolatency", "1");
            opts.set("delay", "0");
        }
        "h264_amf" => {
            opts.set("usage", "ultralowlatency");
            opts.set("quality", "speed");
            opts.set("rc", "cbr");
        }
        _ => {
            opts.set("preset", "ultrafast");
            opts.set("tune", "zerolatency");
            opts.set("threads", "1");
            opts.set("x264-params", "bframes=0:slices=1");
            opts.set("crf", "23");
        }
    }

    let mut enc = ctx.encoder().video()?;
    enc.set_width(width);
    enc.set_height(height);
    enc.set_format(ffmpeg::util::format::Pixel::YUV420P);
    enc.set_time_base(ffmpeg::Rational(1, 1_000_000));
    enc.set_frame_rate(Some(ffmpeg::Rational(fps as i32, 1)));
    desktop_rate_control(&mut enc, width, height, fps);
    enc.set_gop(gop);
    enc.set_max_b_frames(0);
    Ok(enc.open_with(opts)?)
}

/// Below this, small panels starve keyframes and visibly soften at every GOP.
const DESKTOP_MIN_BITRATE: u64 = 6_000_000;
const DESKTOP_MAX_BITRATE: u64 = 20_000_000;

fn desktop_bitrate(width: u32, height: u32, fps: u32) -> u64 {
    super::h264::bitrate(width, height, fps).clamp(DESKTOP_MIN_BITRATE, DESKTOP_MAX_BITRATE)
}

pub(super) fn desktop_rate_control(
    encoder: &mut ffmpeg::encoder::video::Video,
    width: u32,
    height: u32,
    fps: u32,
) {
    let bitrate = desktop_bitrate(width, height, fps).min(i32::MAX as u64);
    encoder.set_bit_rate(bitrate as usize);
    // Bound bursts to two frames instead of NVENC's default two seconds of bitrate.
    unsafe {
        let context = encoder.as_mut_ptr();
        (*context).rc_max_rate = bitrate as i64;
        (*context).rc_buffer_size =
            (bitrate * 2 / u64::from(fps.max(1))).min(i32::MAX as u64) as i32;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn small_desktop_panels_get_enough_bitrate_for_sharp_keyframes() {
        assert_eq!(super::desktop_bitrate(480, 480, 30), 6_000_000);
        assert_eq!(
            super::desktop_bitrate(1920, 480, 30),
            super::super::h264::bitrate(1920, 480, 30)
        );
        assert_eq!(super::desktop_bitrate(2288, 1080, 60), 20_000_000);
    }

    #[test]
    fn sustained_motion_has_bounded_packets_without_delayed_output() {
        let mut encoder = super::H264Encoder::new(320, 240, 60, false, false).unwrap();
        let mut pixels = vec![0; 320 * 240 * 4];
        let mut seed = 1u32;
        let allowance = super::desktop_bitrate(320, 240, 60) as usize / 60 / 8 * 4;
        for _ in 0..20 {
            for pixel in pixels.as_chunks_mut::<4>().0 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                pixel.copy_from_slice(&seed.to_le_bytes());
            }
            let packet = encoder.encode(&pixels).unwrap();
            assert!(
                !packet.is_empty(),
                "A desktop frame was buffered in the encoder"
            );
            assert!(
                packet.len() <= allowance,
                "Unbounded motion burst: {} bytes",
                packet.len()
            );
        }
    }

    #[test]
    fn moving_cursor_frames_decode_without_leaving_old_pixels() {
        use ffmpeg_next as ffmpeg;
        let mut encoder = super::H264Encoder::new(1920, 480, 60, false, false).unwrap();
        let codec = ffmpeg::decoder::find(ffmpeg::codec::Id::H264).unwrap();
        let mut decoder = ffmpeg::codec::context::Context::new_with_codec(codec)
            .decoder()
            .video()
            .unwrap();
        let mut decoded = ffmpeg::frame::Video::empty();
        let mut pixels = vec![0; 1920 * 480 * 4];
        let positions = [16, 1800, 320, 1504, 640, 1216];
        for &x in &positions {
            pixels.fill(0);
            for y in 224..256 {
                pixels[(y * 1920 + x) * 4..(y * 1920 + x + 32) * 4].fill(255);
            }
            let packet = encoder.encode(&pixels).unwrap();
            decoder.send_packet(&ffmpeg::Packet::copy(&packet)).unwrap();
            decoder.receive_frame(&mut decoded).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (1920, 480));
            for &other in &positions {
                let luma = decoded.data(0)[240 * decoded.stride(0) + other + 16];
                if other == x {
                    assert!(luma > 200, "Current cursor was lost at {x}");
                } else {
                    assert!(luma < 40, "Stale cursor remained at {other}");
                }
            }
        }
    }

    #[test]
    fn encoding_does_not_overwrite_a_retained_reference_frame() {
        let mut encoder = super::H264Encoder::new(64, 64, 30, false, false).unwrap();
        encoder.encode(&vec![32; 64 * 64 * 4]).unwrap();
        let mut retained = ffmpeg_next::frame::Video::empty();
        let result = unsafe {
            ffmpeg_next::ffi::av_frame_ref(retained.as_mut_ptr(), encoder.frame_out.as_ptr())
        };
        assert_eq!(result, 0);
        let previous = retained.data(0).to_vec();
        encoder.encode(&vec![220; 64 * 64 * 4]).unwrap();
        assert_eq!(retained.data(0), previous);
        assert_ne!(encoder.frame_out.data(0), previous);
    }

    #[test]
    fn desktop_frames_use_one_slice_like_live_lcd_frames() {
        let mut encoder = super::H264Encoder::new(1920, 480, 60, false, false).unwrap();
        let pixels = vec![80; 1920 * 480 * 4];
        for _ in 0..3 {
            let packet = encoder.encode(&pixels).unwrap();
            let slices = packet
                .windows(4)
                .filter(|bytes| bytes[..3] == [0, 0, 1] && matches!(bytes[3] & 31, 1 | 5))
                .count();
            assert_eq!(
                slices, 1,
                "Each submitted frame must contain one complete slice"
            );
        }
    }

    #[test]
    fn padded_capture_rows_reach_the_encoder_without_padding_or_truncation() {
        let mut encoder = super::H264Encoder::new(16, 16, 30, false, false).unwrap();
        let mut pixels = vec![0xff; 80 * 15 + 64];
        for row in 0..16 {
            pixels[row * 80..row * 80 + 64].fill(row as u8);
        }
        encoder.copy_pixels_in(&pixels, 80).unwrap();
        let stride = encoder.frame_in.stride(0);
        for row in 0..16 {
            assert_eq!(
                &encoder.frame_in.data(0)[row * stride..row * stride + 64],
                &[row as u8; 64]
            );
        }
        assert!(encoder
            .copy_pixels_in(&pixels[..pixels.len() - 1], 80)
            .is_err());
        assert!(encoder.copy_pixels_in(&pixels, 63).is_err());
        assert!(encoder.copy_pixels_in(&pixels, usize::MAX).is_err());
    }

    #[test]
    fn desktop_encoding_respects_software_only_policy() {
        assert_eq!(super::encoder_names(false), &["libx264"]);
        assert_eq!(
            super::encoder_names(true),
            &["h264_nvenc", "h264_amf", "libx264"]
        );
    }
}

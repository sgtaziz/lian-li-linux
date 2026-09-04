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
}

impl H264Encoder {
    /// `rgb_byte_order` selects the input interpretation: true means the
    /// framebuffer stores red in the first byte of each pixel, as AB24 and
    /// XB24 do, false means blue first, as XR24 and AR24 do.
    pub fn new(width: u32, height: u32, fps: u32, rgb_byte_order: bool) -> Result<Self> {
        ensure_ffmpeg_initialized();

        let src_pixel = if rgb_byte_order {
            ffmpeg::util::format::Pixel::RGBA
        } else {
            ffmpeg::util::format::Pixel::BGRA
        };
        let gop = (fps / 2).max(1);
        let mut last_err: Option<anyhow::Error> = None;
        for name in ["h264_nvenc", "h264_amf", "libx264"] {
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
        self.copy_pixels_in(frame)?;
        self.scaler
            .run(&self.frame_in, &mut self.frame_out)
            .context("sws scale to YUV420P")?;
        self.frame_out
            .set_pts(Some(self.start.elapsed().as_micros() as i64));
        self.encoder
            .send_frame(&self.frame_out)
            .context("encoder.send_frame")?;

        let mut out = Vec::new();
        while self.encoder.receive_packet(&mut self.packet).is_ok() {
            if let Some(data) = self.packet.data() {
                out.extend_from_slice(data);
            }
        }
        Ok(out)
    }

    fn copy_pixels_in(&mut self, frame: &[u8]) -> Result<()> {
        let expected = (self.width as usize) * 4 * (self.height as usize);
        if frame.len() < expected {
            bail!("frame buffer too small: {} < {}", frame.len(), expected);
        }
        let stride = self.frame_in.stride(0);
        let row_bytes = (self.width as usize) * 4;
        if stride == row_bytes {
            self.frame_in.data_mut(0)[..expected].copy_from_slice(&frame[..expected]);
        } else {
            let dst = self.frame_in.data_mut(0);
            for y in 0..self.height as usize {
                let src_off = y * row_bytes;
                let dst_off = y * stride;
                dst[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&frame[src_off..src_off + row_bytes]);
            }
        }
        Ok(())
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
            opts.set("rc", "cbr");
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
            opts.set("x264-params", "bframes=0");
        }
    }

    let mut enc = ctx.encoder().video()?;
    enc.set_width(width);
    enc.set_height(height);
    enc.set_format(ffmpeg::util::format::Pixel::YUV420P);
    enc.set_time_base(ffmpeg::Rational(1, 1_000_000));
    enc.set_frame_rate(Some(ffmpeg::Rational(fps as i32, 1)));
    enc.set_bit_rate(5_000_000);
    enc.set_gop(gop);
    enc.set_max_b_frames(0);
    Ok(enc.open_with(opts)?)
}

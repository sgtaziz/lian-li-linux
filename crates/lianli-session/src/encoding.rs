use anyhow::{ensure, Context, Result};
use lianli_display::frame::{Mode, PixelFormat};
use lianli_display::Capture;
use lianli_media::common::{encode_jpeg_to_fit, JpegQualityMemo};
use lianli_media::video::vaapi::{DmaFrame, DmaPlane, VaapiEncoder};
use lianli_media::video::H264Encoder;
use lianli_shared::display::{
    CpuReadbackReason, DesktopEncoder, DesktopEncodingStatus, DisplayCodec, SoftwareEncodingReason,
};
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub struct Encoder {
    vaapi: Option<VaapiEncoder>,
    cpu_input: Option<H264Encoder>,
    gpu_unavailable: bool,
    cpu_readback_reason: Option<CpuReadbackReason>,
    software_reason: Option<SoftwareEncodingReason>,
    status: Option<DesktopEncodingStatus>,
    jpeg_quality: JpegQualityMemo,
}

#[derive(Clone, Copy)]
pub struct FrameRequest {
    pub codec: DisplayCodec,
    pub mode: Mode,
    pub format: PixelFormat,
    pub fps: u32,
    pub hardware_video: bool,
    pub max_frame_bytes: Option<u32>,
}

impl Encoder {
    pub fn status(&self) -> Option<DesktopEncodingStatus> {
        self.status
    }

    pub fn reset(&mut self, capture: &mut dyn Capture) -> Result<()> {
        self.vaapi = None;
        self.cpu_input = None;
        self.gpu_unavailable = false;
        self.cpu_readback_reason = None;
        self.software_reason = None;
        self.status = None;
        self.jpeg_quality = JpegQualityMemo::default();
        capture.discard_gpu()
    }

    pub fn encode(
        &mut self,
        capture: &mut dyn Capture,
        request: FrameRequest,
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>> {
        if request.codec == DisplayCodec::H264 && request.hardware_video && !self.gpu_unavailable {
            match self.gpu_packet(capture, request, cancel) {
                Ok(Some(packet)) => {
                    self.status = Some(DesktopEncodingStatus {
                        encoder: DesktopEncoder::H264Vaapi,
                        gpu_input: true,
                        cpu_readback_reason: None,
                        software_reason: None,
                    });
                    return Ok(packet);
                }
                Ok(None) => {
                    tracing::info!("Selected capture implementation supplies CPU buffers; GPU-buffer input is unavailable on this path");
                    self.gpu_unavailable = true;
                    self.cpu_readback_reason = Some(CpuReadbackReason::NoDmaBuf);
                }
                Err(error) => {
                    self.vaapi = None;
                    capture.discard_gpu()?;
                    ensure!(
                        !cancel.load(Ordering::Relaxed),
                        "Desktop encoding cancelled"
                    );
                    tracing::warn!(
                        "GPU desktop encoding unavailable; using CPU readback: {error:#}"
                    );
                    self.gpu_unavailable = true;
                    self.cpu_readback_reason = Some(CpuReadbackReason::GpuFailure);
                }
            }
        }
        let frame = capture.frame(cancel)?;
        ensure!(
            frame.mode == request.mode && frame.format == request.format,
            "Desktop frame layout changed unexpectedly"
        );
        let result = match request.codec {
            DisplayCodec::Jpeg => Ok(encode_jpeg_to_fit(
                turbojpeg::Image {
                    pixels: frame.pixels,
                    width: frame.mode.width as usize,
                    height: frame.mode.height as usize,
                    pitch: frame.stride,
                    format: if frame.format.rgb_byte_order() {
                        turbojpeg::PixelFormat::RGBX
                    } else {
                        turbojpeg::PixelFormat::BGRX
                    },
                },
                90,
                request
                    .max_frame_bytes
                    .map_or(usize::MAX, |bytes| bytes as usize),
                &self.jpeg_quality,
            )?),
            DisplayCodec::H264 => {
                if self.cpu_input.is_none() {
                    self.cpu_input = Some(H264Encoder::new(
                        request.mode.width,
                        request.mode.height,
                        request.fps,
                        request.format.rgb_byte_order(),
                        request.hardware_video,
                    )?);
                }
                let result = complete_cpu_packet(
                    self.cpu_input.as_mut().unwrap(),
                    frame.pixels,
                    frame.stride,
                );
                match result {
                    Ok(packet) => Ok(packet),
                    Err(error) if self.cpu_input.as_ref().unwrap().backend() != "libx264" => {
                        tracing::warn!(
                            "Desktop hardware encoding failed; switching to libx264: {error:#}"
                        );
                        self.cpu_input = None;
                        self.software_reason = Some(SoftwareEncodingReason::HardwareFailed);
                        ensure!(
                            !cancel.load(Ordering::Relaxed),
                            "Desktop encoding cancelled"
                        );
                        self.cpu_input = Some(H264Encoder::new(
                            request.mode.width,
                            request.mode.height,
                            request.fps,
                            request.format.rgb_byte_order(),
                            false,
                        )?);
                        complete_cpu_packet(
                            self.cpu_input.as_mut().unwrap(),
                            frame.pixels,
                            frame.stride,
                        )
                    }
                    Err(error) => Err(error),
                }
            }
        };
        if result.is_ok() {
            let encoder = if request.codec == DisplayCodec::Jpeg {
                DesktopEncoder::Turbojpeg
            } else {
                match self
                    .cpu_input
                    .as_ref()
                    .context("Missing desktop encoder")?
                    .backend()
                {
                    "libx264" => DesktopEncoder::Libx264,
                    "h264_nvenc" => DesktopEncoder::H264Nvenc,
                    "h264_amf" => DesktopEncoder::H264Amf,
                    _ => DesktopEncoder::Unknown,
                }
            };
            if encoder == DesktopEncoder::Libx264
                && request.hardware_video
                && self.software_reason.is_none()
            {
                self.software_reason = Some(SoftwareEncodingReason::HardwareUnavailable);
            }
            self.status = Some(DesktopEncodingStatus {
                encoder,
                gpu_input: false,
                cpu_readback_reason: self.cpu_readback_reason,
                software_reason: self.software_reason,
            });
        }
        result
    }

    fn gpu_packet(
        &mut self,
        capture: &mut dyn Capture,
        request: FrameRequest,
        cancel: &AtomicBool,
    ) -> Result<Option<Vec<u8>>> {
        let Some(frame) = capture.gpu_frame(cancel)? else {
            return Ok(None);
        };
        ensure!(
            (frame.image.width, frame.image.height) == (request.mode.width, request.mode.height),
            "GPU frame geometry changed unexpectedly"
        );
        let planes: Vec<_> = frame
            .image
            .planes
            .iter()
            .map(|plane| DmaPlane {
                descriptor: plane.descriptor.as_fd(),
                allocation_bytes: plane.allocation_bytes,
                pitch: plane.pitch,
                offset: plane.offset,
            })
            .collect();
        let input = DmaFrame {
            width: frame.image.width,
            height: frame.image.height,
            fourcc: frame.image.fourcc,
            modifier: frame.image.modifier,
            planes: &planes,
        };
        let initializing = self.vaapi.is_none();
        if initializing {
            ensure!(
                !cancel.load(Ordering::Relaxed),
                "Desktop encoding cancelled"
            );
            self.vaapi = Some(VaapiEncoder::new(frame.device, &input, request.fps)?);
        }
        let packet = self
            .vaapi
            .as_mut()
            .context("VAAPI encoder is missing")?
            .encode(&input, cancel)?;
        if initializing {
            tracing::info!("Desktop encoding applied: h264_vaapi with GPU composition and conversion, without CPU pixel readback");
        }
        Ok(Some(packet))
    }
}

fn complete_cpu_packet(encoder: &mut H264Encoder, pixels: &[u8], stride: usize) -> Result<Vec<u8>> {
    let packet = encoder.encode_strided(pixels, stride)?;
    ensure!(
        !packet.is_empty(),
        "Desktop encoding requires a complete packet for a static frame"
    );
    Ok(packet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_display::{frame::Frame, gpu::GpuFrame, Event};
    use std::time::Duration;

    struct SoftwareCapture {
        pixels: Vec<u8>,
        frames: usize,
    }

    impl Capture for SoftwareCapture {
        fn poll_events(&mut self, _: Duration, _: &AtomicBool) -> Result<Vec<Event>> {
            Ok(Vec::new())
        }
        fn request_update(&mut self) -> Result<bool> {
            Ok(true)
        }
        fn frame(&mut self, _: &AtomicBool) -> Result<Frame<'_>> {
            self.frames += 1;
            Frame::new(mode(), PixelFormat::Argb8888, 64, &self.pixels)
        }
        fn gpu_frame(&mut self, _: &AtomicBool) -> Result<Option<GpuFrame<'_>>> {
            panic!("Software/JPEG encoding must not initialize DMA-BUF video")
        }
        fn invalidate(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn mode() -> Mode {
        Mode {
            width: 16,
            height: 16,
            refresh_hz: 30,
        }
    }

    #[test]
    fn software_policy_and_jpeg_firmware_never_initialize_gpu_video() {
        for (codec, hardware_video) in [
            (DisplayCodec::H264, false),
            (DisplayCodec::Jpeg, false),
            (DisplayCodec::Jpeg, true),
        ] {
            let mut capture = SoftwareCapture {
                pixels: vec![80; 16 * 16 * 4],
                frames: 0,
            };
            let mut encoder = Encoder::default();
            let packet = encoder
                .encode(
                    &mut capture,
                    FrameRequest {
                        codec,
                        mode: mode(),
                        format: PixelFormat::Argb8888,
                        fps: 30,
                        hardware_video,
                        max_frame_bytes: None,
                    },
                    &AtomicBool::new(false),
                )
                .unwrap();
            assert!(!packet.is_empty());
            assert_eq!(capture.frames, 1);
            let status = encoder.status().unwrap();
            assert_eq!(
                status.encoder,
                if codec == DisplayCodec::Jpeg {
                    DesktopEncoder::Turbojpeg
                } else {
                    DesktopEncoder::Libx264
                }
            );
            assert!(!status.gpu_input);
            assert!(status.software_reason.is_none());
            encoder.reset(&mut capture).unwrap();
            assert!(encoder.status().is_none());
        }
    }

    #[test]
    fn jpeg_frames_step_down_to_the_frame_limit_until_reset() {
        let mut state = 0x2545_f491_u32;
        let mut capture = SoftwareCapture {
            pixels: (0..16 * 16 * 4)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    state as u8
                })
                .collect(),
            frames: 0,
        };
        let request = |max_frame_bytes| FrameRequest {
            codec: DisplayCodec::Jpeg,
            mode: mode(),
            format: PixelFormat::Argb8888,
            fps: 30,
            hardware_video: false,
            max_frame_bytes,
        };
        let stop = AtomicBool::new(false);
        let mut encoder = Encoder::default();
        let full = encoder.encode(&mut capture, request(None), &stop).unwrap();
        let limit = full.len() - 1;
        let fitted = encoder
            .encode(&mut capture, request(Some(limit as u32)), &stop)
            .unwrap();
        assert!(fitted.len() <= limit);
        assert_eq!(
            encoder
                .encode(&mut capture, request(Some(full.len() as u32)), &stop)
                .unwrap(),
            fitted
        );
        encoder.reset(&mut capture).unwrap();
        assert_eq!(
            encoder.encode(&mut capture, request(None), &stop).unwrap(),
            full
        );
    }
}

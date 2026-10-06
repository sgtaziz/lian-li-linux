pub mod asset_access;
pub mod common;
pub mod custom;
mod fonts;
pub mod image;
pub mod pixel_cleaner;
pub mod preparation;
mod resource_budget;
mod text_raster;
mod text_validation;
mod text_work;
pub use resource_budget::Retained;
pub mod rgb;
pub mod sensor;
pub mod startup_image;
mod temporary_media;
pub use temporary_media::TemporaryMedia;
pub mod validation;
pub mod video;

pub use common::MediaError;
pub use custom::CustomAsset;
use lianli_shared::sensors::SensorInfo;
pub use preparation::PreparationControl;
pub use sensor::SensorAsset;

use lianli_shared::config::{ConfigKey, LcdConfig};
use lianli_shared::media::MediaType;
use lianli_shared::screen::ScreenInfo;
use lianli_shared::template::LcdTemplate;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct MediaAsset {
    pub config_key: ConfigKey,
    pub kind: MediaAssetKind,
    pub stream_fps: f32,
    pub hardware_video: bool,
}

#[derive(Debug, Clone)]
pub enum MediaAssetKind {
    Static {
        frame: Arc<Retained<Vec<u8>>>,
    },
    Video {
        frames: Arc<Retained<Vec<Vec<u8>>>>,
        frame_durations: Arc<Vec<Duration>>,
    },
    Sensor {
        asset: Arc<SensorAsset>,
    },
    H264Stream {
        encoder: Option<lianli_shared::ipc::MediaEncoderStatus>,
        path: PathBuf,
        looping: bool,
        fps: f32,
        _temp: Arc<TemporaryMedia>,
    },
    Custom {
        asset: Arc<CustomAsset>,
    },
}

impl PartialEq for MediaAsset {
    fn eq(&self, other: &Self) -> bool {
        self.config_key == other.config_key
    }
}

impl Eq for MediaAsset {}

pub fn prepare_media_asset(
    cfg: &LcdConfig,
    default_fps: f32,
    screen: &ScreenInfo,
    h264: bool,
    all_sensors: &[SensorInfo],
    user_templates: &[LcdTemplate],
    control: impl Into<PreparationControl>,
) -> Result<MediaAssetKind, MediaError> {
    let control = control.into();
    control.check()?;
    if cfg.media_type != MediaType::Custom {
        let dependencies = lianli_shared::media_dependencies::lcd_dependencies(cfg, user_templates)
            .map_err(MediaError::InvalidConfig)?;
        asset_access::validate_dependencies(&dependencies, &control)?;
    }
    let fps_cap = default_fps.min(screen.max_fps as f32).max(1.0);
    match cfg.media_type {
        MediaType::Image => {
            let path = cfg.path.as_ref().ok_or(MediaError::InvalidConfig(
                "image entry requires a 'path' field".into(),
            ))?;
            let frame = image::load_image_frame(path, cfg.orientation, &cfg.framing, screen)?;
            Ok(MediaAssetKind::Static {
                frame: Retained::frame(frame)?,
            })
        }
        MediaType::Color => {
            let rgb = cfg.rgb.ok_or(MediaError::InvalidConfig(
                "color entry requires an 'rgb' field".into(),
            ))?;
            let frame = image::build_color_frame(rgb, screen);
            Ok(MediaAssetKind::Static {
                frame: Retained::frame(frame)?,
            })
        }
        MediaType::Video | MediaType::Gif if h264 => {
            let path = cfg.path.as_ref().ok_or(MediaError::InvalidConfig(
                "video/gif entry requires a 'path' field".into(),
            ))?;
            let fps = video::ffmpeg::cap_fps_cancellable(
                path,
                cfg.fps.unwrap_or(default_fps).min(fps_cap),
                &control,
            )?;
            let (h264_path, temp, encoded_fps, encoder) = video::encode_h264_with_status(
                path,
                fps,
                cfg.orientation,
                &cfg.framing,
                screen,
                &control,
            )?;
            Ok(MediaAssetKind::H264Stream {
                encoder: Some(encoder),
                path: h264_path,
                looping: true,
                fps: encoded_fps,
                _temp: Arc::new(temp),
            })
        }
        MediaType::Video => {
            let desired_fps = cfg.fps.unwrap_or(default_fps).min(fps_cap);
            if desired_fps <= 0.0 {
                return Err(MediaError::InvalidFps);
            }
            let path = cfg.path.as_ref().ok_or(MediaError::InvalidConfig(
                "video entry requires a 'path' field".into(),
            ))?;
            let desired_fps = video::ffmpeg::cap_fps_cancellable(path, desired_fps, &control)?;
            let (frames, durations) = video::build_video_frames(
                path,
                desired_fps,
                cfg.orientation,
                &cfg.framing,
                screen,
                &control,
            )?;
            Ok(MediaAssetKind::Video {
                frames: Retained::frames(frames)?,
                frame_durations: Arc::new(durations),
            })
        }
        MediaType::Gif => {
            let path = cfg.path.as_ref().ok_or(MediaError::InvalidConfig(
                "gif entry requires a 'path' field".into(),
            ))?;
            let capped_fps = Some(video::ffmpeg::cap_fps_cancellable(
                path,
                cfg.fps.unwrap_or(default_fps).min(fps_cap),
                &control,
            )?);
            let (frames, durations) = video::build_gif_frames_cancellable(
                path,
                cfg.orientation,
                screen,
                capped_fps,
                &cfg.framing,
                &control,
            )?;
            Ok(MediaAssetKind::Video {
                frames: Retained::frames(frames)?,
                frame_durations: Arc::new(durations),
            })
        }
        MediaType::Sensor => {
            let descriptor = cfg.sensor.as_ref().ok_or(MediaError::InvalidConfig(
                "sensor entry requires a 'sensor' field".into(),
            ))?;
            let bg_path = cfg.path.as_deref();
            let update_interval_ms = cfg.update_interval_ms.unwrap_or(1000);
            let asset = SensorAsset::new(
                descriptor,
                cfg.orientation,
                screen,
                all_sensors,
                bg_path,
                update_interval_ms,
            )?;
            Ok(MediaAssetKind::Sensor { asset })
        }
        MediaType::Doublegauge | MediaType::Cooler => Err(MediaError::InvalidConfig(
            "Doublegauge/Cooler are retired; this LCD should have been migrated to Custom on load"
                .into(),
        )),
        MediaType::Custom => {
            let template_id = cfg.template_id.as_deref().ok_or_else(|| {
                MediaError::InvalidConfig("custom entry requires a 'template_id' field".into())
            })?;
            let template = user_templates
                .iter()
                .find(|t| t.id == template_id)
                .cloned()
                .ok_or_else(|| {
                    MediaError::InvalidConfig(format!("unknown template id '{template_id}'"))
                })?;
            let widget_fps = template
                .widgets
                .iter()
                .filter_map(|w| w.fps.filter(|&f| f >= 1.0))
                .fold(0.0f32, f32::max);
            let custom_fps = if widget_fps >= 1.0 {
                widget_fps
            } else {
                cfg.fps.unwrap_or(default_fps)
            }
            .min(fps_cap)
            .max(1.0);
            let asset = CustomAsset::new(
                &template,
                cfg.orientation,
                screen,
                all_sensors,
                cfg.smooth_edges(),
                custom_fps,
                &control,
            )?;
            Ok(MediaAssetKind::Custom { asset })
        }
    }
}

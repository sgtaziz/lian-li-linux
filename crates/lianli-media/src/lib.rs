pub mod common;
pub mod custom;
pub mod image;
pub mod sensor;
pub mod video;

pub use common::MediaError;
pub use custom::CustomAsset;
use lianli_shared::sensors::SensorInfo;
pub use sensor::SensorAsset;

use lianli_shared::config::{ConfigKey, LcdConfig};
use lianli_shared::media::MediaType;
use lianli_shared::screen::ScreenInfo;
use lianli_shared::template::LcdTemplate;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

#[derive(Debug, Clone)]
pub struct MediaAsset {
    pub config_key: ConfigKey,
    pub kind: MediaAssetKind,
    pub stream_fps: f32,
}

#[derive(Debug, Clone)]
pub enum MediaAssetKind {
    Static {
        frame: Arc<Vec<u8>>,
    },
    Video {
        frames: Arc<Vec<Vec<u8>>>,
        frame_durations: Arc<Vec<Duration>>,
    },
    Sensor {
        asset: Arc<SensorAsset>,
    },
    H264Stream {
        path: PathBuf,
        looping: bool,
        fps: f32,
        _temp: Arc<TempDir>,
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
) -> Result<MediaAssetKind, MediaError> {
    let fps_cap = default_fps.min(screen.max_fps as f32).max(1.0);
    match cfg.media_type {
        MediaType::Image => {
            let path = cfg.path.as_ref().ok_or(MediaError::InvalidConfig(
                "image entry requires a 'path' field".into(),
            ))?;
            let frame = image::load_image_frame(path, cfg.orientation, screen)?;
            Ok(MediaAssetKind::Static {
                frame: Arc::new(frame),
            })
        }
        MediaType::Color => {
            let rgb = cfg.rgb.ok_or(MediaError::InvalidConfig(
                "color entry requires an 'rgb' field".into(),
            ))?;
            let frame = image::build_color_frame(rgb, screen);
            Ok(MediaAssetKind::Static {
                frame: Arc::new(frame),
            })
        }
        MediaType::Video | MediaType::Gif if h264 => {
            let path = cfg.path.as_ref().ok_or(MediaError::InvalidConfig(
                "video/gif entry requires a 'path' field".into(),
            ))?;
            let fps = video::cap_fps_to_source(path, cfg.fps.unwrap_or(default_fps).min(fps_cap));
            let (h264_path, temp, encoded_fps) =
                video::encode_h264(path, fps, cfg.orientation, screen)?;
            Ok(MediaAssetKind::H264Stream {
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
            let desired_fps = video::cap_fps_to_source(path, desired_fps);
            let (frames, durations) =
                video::build_video_frames(path, desired_fps, cfg.orientation, screen)?;
            Ok(MediaAssetKind::Video {
                frames: Arc::new(frames),
                frame_durations: Arc::new(durations),
            })
        }
        MediaType::Gif => {
            let path = cfg.path.as_ref().ok_or(MediaError::InvalidConfig(
                "gif entry requires a 'path' field".into(),
            ))?;
            let capped_fps = Some(video::cap_fps_to_source(
                path,
                cfg.fps.unwrap_or(default_fps).min(fps_cap),
            ));
            let (frames, durations) =
                video::build_gif_frames(path, cfg.orientation, screen, capped_fps)?;
            Ok(MediaAssetKind::Video {
                frames: Arc::new(frames),
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
            )?;
            Ok(MediaAssetKind::Custom { asset })
        }
    }
}

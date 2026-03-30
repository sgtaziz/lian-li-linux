pub mod common;
pub mod image;
pub mod sensor;
pub mod video;

pub use common::MediaError;
pub use sensor::SensorAsset;

use lianli_shared::config::{ConfigKey, LcdConfig};
use lianli_shared::media::MediaType;
use lianli_shared::screen::ScreenInfo;
use std::sync::Arc;
use std::time::Duration;


#[derive(Debug, Clone)]
pub struct MediaAsset {
    pub config_key: ConfigKey, // unique ID, using the config key
    pub kind: MediaAssetKind, // the contents (originally the enum MediaAsset, now in MediaAssetKind)
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
}

// Implementation for comparison for MediaAsset
impl PartialEq for MediaAsset {
    fn eq(&self, other: &Self) -> bool {
        self.config_key == other.config_key
    }
}

impl Eq for MediaAsset {}

/// Prepare a media asset for a given LCD config and screen info.
pub fn prepare_media_asset(
    cfg: &LcdConfig,
    default_fps: f32,
    screen: &ScreenInfo,
) -> Result<MediaAssetKind, MediaError> {
    match cfg.media_type {
        MediaType::Image => {
            let path = cfg.path.as_ref().ok_or(MediaError::InvalidConfig("image entry requires a 'path' field".into()))?;
            let frame = image::load_image_frame(path, cfg.orientation, screen)?;
            Ok(MediaAssetKind::Static {
                frame: Arc::new(frame),
            })
        }
        MediaType::Color => {
            let rgb = cfg.rgb.ok_or(MediaError::InvalidConfig("color entry requires an 'rgb' field".into()))?;
            let frame = image::build_color_frame(rgb, screen);
            Ok(MediaAssetKind::Static {
                frame: Arc::new(frame),
            })
        }
        MediaType::Video => {
            let desired_fps = cfg.fps.unwrap_or(default_fps);
            if desired_fps <= 0.0 {
                return Err(MediaError::InvalidFps);
            }
            let path = cfg.path.as_ref().ok_or(MediaError::InvalidConfig("video entry requires a 'path' field".into()))?;
            let (frames, durations) =
                video::build_video_frames(path, desired_fps, cfg.orientation, screen)?;
            Ok(MediaAssetKind::Video {
                frames: Arc::new(frames),
                frame_durations: Arc::new(durations),
            })
        }
        MediaType::Gif => {
            let path = cfg.path.as_ref().ok_or(MediaError::InvalidConfig("gif entry requires a 'path' field".into()))?;
            let (frames, durations) = video::build_gif_frames(path, cfg.orientation, screen)?;
            Ok(MediaAssetKind::Video {
                frames: Arc::new(frames),
                frame_durations: Arc::new(durations),
            })
        }
        MediaType::Sensor => {
            let descriptor = cfg.sensor.as_ref().ok_or(MediaError::InvalidConfig("sensor entry requires a 'sensor' field".into()))?;
            let asset = SensorAsset::new(descriptor, cfg.orientation, screen)?;
            Ok(MediaAssetKind::Sensor { asset })
        }
    }
}

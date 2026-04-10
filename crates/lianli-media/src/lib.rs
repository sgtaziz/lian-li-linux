pub mod common;
pub mod cooler;
pub mod custom;
pub mod doublegauge;
pub mod image;
pub mod sensor;
pub mod video;

pub use common::MediaError;
pub use cooler::CoolerAsset;
pub use custom::CustomAsset;
pub use doublegauge::DoublegaugeAsset;
use lianli_shared::sensors::SensorInfo;
pub use sensor::SensorAsset;

use lianli_shared::config::{ConfigKey, LcdConfig};
use lianli_shared::media::{MediaType, SensorSourceConfig};
use lianli_shared::screen::ScreenInfo;
use lianli_shared::template::LcdTemplate;
use lianli_shared::template_defaults::builtin_template_resolved;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

#[derive(Debug, Clone)]
pub struct MediaAsset {
    pub config_key: ConfigKey,
    pub kind: MediaAssetKind,
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
        _temp: Arc<TempDir>,
    },
    Doublegauge {
        asset: Arc<DoublegaugeAsset>,
    },
    Cooler {
        asset: Arc<CoolerAsset>,
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
            let fps = cfg.fps.unwrap_or(default_fps).max(1.0);
            let (h264_path, temp) = video::encode_h264(path, fps, cfg.orientation, screen)?;
            Ok(MediaAssetKind::H264Stream {
                path: h264_path,
                looping: true,
                _temp: Arc::new(temp),
            })
        }
        MediaType::Video => {
            let desired_fps = cfg.fps.unwrap_or(default_fps);
            if desired_fps <= 0.0 {
                return Err(MediaError::InvalidFps);
            }
            let path = cfg.path.as_ref().ok_or(MediaError::InvalidConfig(
                "video entry requires a 'path' field".into(),
            ))?;
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
            let (frames, durations) = video::build_gif_frames(path, cfg.orientation, screen)?;
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
        MediaType::Doublegauge => {
            let descriptor = cfg.doublegauge.as_ref().ok_or_else(|| {
                MediaError::InvalidConfig(
                    "doublegauge entry requires a 'doublegauge' section".into(),
                )
            })?;
            let source_1 = resolve_sensor_config(&cfg.sensor_source_1, all_sensors)?;
            let source_2 = resolve_sensor_config(&cfg.sensor_source_2, all_sensors)?;
            let update_interval_ms = cfg.update_interval_ms.unwrap_or(1000);
            let asset = DoublegaugeAsset::new(
                descriptor,
                cfg.orientation,
                screen,
                source_1,
                source_2,
                update_interval_ms,
            )?;
            Ok(MediaAssetKind::Doublegauge { asset })
        }
        MediaType::Cooler => {
            let descriptor = cfg.doublegauge.as_ref().ok_or_else(|| {
                MediaError::InvalidConfig("cooler entry requires a 'doublegauge' section".into())
            })?;
            let source_1 = resolve_sensor_config(&cfg.sensor_source_1, all_sensors)?;
            let source_2 = resolve_sensor_config(&cfg.sensor_source_2, all_sensors)?;
            let update_interval_ms = cfg.update_interval_ms.unwrap_or(1000);
            let asset = CoolerAsset::new(
                descriptor,
                cfg.orientation,
                screen,
                source_1,
                source_2,
                update_interval_ms,
            )?;
            Ok(MediaAssetKind::Cooler { asset })
        }
        MediaType::Custom => {
            let template_id = cfg.template_id.as_deref().ok_or_else(|| {
                MediaError::InvalidConfig("custom entry requires a 'template_id' field".into())
            })?;
            let template = builtin_template_resolved(template_id, all_sensors)
                .or_else(|| user_templates.iter().find(|t| t.id == template_id).cloned())
                .ok_or_else(|| {
                    MediaError::InvalidConfig(format!("unknown template id '{template_id}'"))
                })?;
            let asset = CustomAsset::new(&template, cfg.orientation, screen, all_sensors)?;
            Ok(MediaAssetKind::Custom { asset })
        }
    }
}

fn resolve_sensor_config(
    cfg_source: &SensorSourceConfig,
    all_sensors: &[SensorInfo],
) -> Result<lianli_shared::sensors::ResolvedSensor, MediaError> {
    match cfg_source {
        SensorSourceConfig::Constant { value } => {
            Ok(lianli_shared::sensors::ResolvedSensor::Constant(*value))
        }
        _ => {
            let sensor_source = cfg_source.to_sensor_source();
            let sensor_info = all_sensors.iter().find(|s| s.source == sensor_source);
            let divider = sensor_info.map_or(1, |s| s.divider);

            lianli_shared::sensors::resolve_sensor(&sensor_source, divider)
                .ok_or_else(|| MediaError::Sensor("sensor not found on system".into()))
        }
    }
}

//! Shared application state, updated by the backend thread and read by UI callbacks.

use lianli_shared::config::AppConfig;
use lianli_shared::ipc::DeviceInfo;
use lianli_shared::rgb::RgbDeviceCapabilities;
use lianli_shared::sensors::SensorInfo;
use lianli_shared::template::LcdTemplate;

#[derive(Debug, Default)]
pub struct SharedState {
    pub config: Option<AppConfig>,
    pub rgb_caps: Vec<RgbDeviceCapabilities>,
    pub devices: Vec<DeviceInfo>,
    pub available_sensors: Vec<SensorInfo>,
    /// Combined built-in + user LCD templates, fetched via `GetLcdTemplates`.
    pub lcd_templates: Vec<LcdTemplate>,
}

//! Conversions between lianli-shared Rust types and Slint-generated structs.

mod aio;
mod device;
mod fan;
mod lcd;
mod rgb;

pub use aio::{
    aio_rotation_options_model, aio_sensor_options_model, aio_theme_options_model, aios_to_model,
};
pub use device::devices_to_model;
pub use fan::{
    build_clamp_segments, build_curve_segments, curve_names_to_model, fan_curves_to_model,
    fan_groups_to_model, font_options_model, sensor_options_model, speed_options_model,
};
pub use lcd::{
    lcd_device_options, lcd_entries_to_model, lcd_label_to_serial, template_id_for_label,
    template_labels_model,
};
pub use rgb::rgb_devices_to_model;

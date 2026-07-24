mod aio;
mod fan;
mod lcd;
mod parsing;
mod rgb;
mod template;

pub(crate) use aio::wire_aio_callbacks;
pub(crate) use fan::wire_fan_callbacks;
pub(crate) use lcd::wire_lcd_callbacks;
pub(crate) use rgb::wire_rgb_callbacks;
pub(crate) use template::strip_copy_suffix;

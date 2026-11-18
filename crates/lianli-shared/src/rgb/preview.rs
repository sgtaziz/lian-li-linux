use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RgbPreviewShape {
    Fan,
    Pump,
    Screen,
    Strip,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RgbPreviewElement {
    pub shape: RgbPreviewShape,
    /// Width divided by height.
    pub aspect: f32,
}

/// One physical LED, placed in its element's unit box.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RgbPreviewLed {
    pub element: u8,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RgbPreviewFidelity {
    Exact,
    Approximate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RgbPreview {
    pub elements: Vec<RgbPreviewElement>,
    pub leds: Vec<RgbPreviewLed>,
    /// Base64 of frame-major RGB bytes, `leds.len() * 3` bytes per frame.
    pub frames: String,
    pub frame_count: u32,
    pub interval_ms: u32,
    pub fidelity: RgbPreviewFidelity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Animation {
    pub frames: Vec<Vec<[u8; 3]>>,
    pub interval_hundredths: u32,
    pub secondary: Option<SecondaryTiming>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecondaryTiming {
    pub interval_ticks: u16,
    pub frame_count: u16,
    pub outer_longest: bool,
}

impl Animation {
    pub fn timing(&self) -> lianli_shared::rgb::RgbPlaybackTiming {
        let secondary = self.secondary;
        lianli_shared::rgb::RgbPlaybackTiming {
            interval_hundredths: self.interval_hundredths,
            secondary_interval_ticks: secondary.map_or(0, |s| s.interval_ticks),
            secondary_frame_count: secondary.map_or(0, |s| s.frame_count),
            outer_longest: secondary.is_some_and(|s| s.outer_longest),
        }
    }
}

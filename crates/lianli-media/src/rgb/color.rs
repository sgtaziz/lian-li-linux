pub(crate) type Color = [u8; 3];

pub(crate) fn scale(color: Color, factor: u16) -> Color {
    color.map(|channel| ((u16::from(channel) * factor) >> 8) as u8)
}

pub(crate) fn scale_byte(color: Color, factor: u8) -> Color {
    scale(color, u16::from(factor))
}

pub(crate) fn limit_current(mut color: Color, channel_sum_limit: u16) -> Color {
    while color.iter().map(|&channel| u16::from(channel)).sum::<u16>() > channel_sum_limit {
        color = color.map(|channel| (f64::from(channel) * 0.95) as u8);
    }
    color
}

pub(crate) fn screen_palette(effect: &lianli_shared::rgb::RgbEffect) -> [Color; 6] {
    // Missing legacy slots inherit the last color; explicitly configured black remains black.
    let mut colors = [effect.colors.last().copied().unwrap_or([0; 3]); 6];
    for (target, source) in colors.iter_mut().zip(&effect.colors) {
        *target = *source;
    }
    colors
}

pub(crate) fn active_palette(effect: &lianli_shared::rgb::RgbEffect, limit: usize) -> &[Color] {
    const BLACK: [Color; 1] = [[0; 3]];
    if effect.colors.is_empty() {
        &BLACK
    } else {
        &effect.colors[..effect.colors.len().min(limit)]
    }
}

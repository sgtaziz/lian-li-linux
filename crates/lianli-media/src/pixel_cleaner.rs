use image::{Rgb, RgbImage};

pub const FPS: u32 = 20;
pub const FRAME_COUNT: u64 = 100;

/// Fill a reusable, native-size buffer with one frame of the five-second loop.
pub fn render_frame(frame: &mut RgbImage, frame_index: u64) {
    let frame_index = frame_index % FRAME_COUNT;
    let color = match frame_index {
        0..30 => Some(if frame_index.is_multiple_of(2) {
            [0, 0, 0]
        } else {
            [255, 255, 255]
        }),
        30..60 => None,
        60..68 => Some([255, 0, 0]),
        68..76 => Some([0, 128, 0]),
        76..84 => Some([0, 0, 255]),
        84..92 => Some([255, 255, 255]),
        _ => Some([128, 128, 128]),
    };

    if let Some(color) = color {
        for pixel in frame.pixels_mut() {
            *pixel = Rgb(color);
        }
        return;
    }

    let mut random = 0x9e37_79b9_u32 ^ frame_index as u32;
    for pixel in frame.pixels_mut() {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        let luma = (random >> 24) as i32;
        // Match the reference noise's contrast after limited-range luma expansion.
        let gray = ((luma - 16) * 255 / 219).clamp(0, 255) as u8;
        *pixel = Rgb([gray; 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_phases_cover_the_frame_at_the_reference_times() {
        let mut frame = RgbImage::new(13, 29);
        for index in 0..FRAME_COUNT {
            let expected = match index {
                0..30 if index % 2 == 0 => [0, 0, 0],
                0..30 => [255, 255, 255],
                30..60 => continue,
                60..68 => [255, 0, 0],
                68..76 => [0, 128, 0],
                76..84 => [0, 0, 255],
                84..92 => [255, 255, 255],
                _ => [128, 128, 128],
            };
            render_frame(&mut frame, index);
            assert!(frame.pixels().all(|pixel| pixel.0 == expected));
        }
        assert_eq!(FRAME_COUNT * 1000 / u64::from(FPS), 5000);
    }

    #[test]
    fn noise_changes_between_frames_and_repeats_after_one_loop() {
        let mut first = RgbImage::new(97, 53);
        let mut next = first.clone();
        render_frame(&mut first, 30);
        render_frame(&mut next, 31);
        assert_ne!(first, next);
        render_frame(&mut next, 130);
        assert_eq!(first, next);
        assert!(first.pixels().all(|p| p[0] == p[1] && p[1] == p[2]));
        assert!(first.pixels().any(|p| p[0] == 0));
        assert!(first.pixels().any(|p| p[0] == 255));
        let mean = first.pixels().map(|p| f64::from(p[0])).sum::<f64>()
            / f64::from(first.width() * first.height());
        assert!((120.0..136.0).contains(&mean));
    }

    #[test]
    fn rendering_reuses_the_buffer_and_wraps_at_the_loop_boundary() {
        let mut frame = RgbImage::new(23, 11);
        let pixels = frame.as_ptr();
        render_frame(&mut frame, 99);
        assert!(frame.pixels().all(|p| p.0 == [128; 3]));
        render_frame(&mut frame, 100);
        assert!(frame.pixels().all(|p| p.0 == [0; 3]));
        assert_eq!(frame.dimensions(), (23, 11));
        assert_eq!(frame.as_ptr(), pixels);
    }
}

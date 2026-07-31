//! Rust FFI bindings for the vendored tinyuz compression library.
//!
//! tinyuz is an LZ77 variant designed for embedded systems.
//! The wireless fan firmware uses it to decompress RGB frame data.

use std::os::raw::c_uchar;

extern "C" {
    fn tuz_compress_mem(
        input: *const c_uchar,
        input_len: usize,
        output: *mut c_uchar,
        output_capacity: usize,
        dict_size: usize,
    ) -> usize;

    fn tuz_max_compressed_size(input_len: usize) -> usize;
}

#[cfg(test)]
extern "C" {
    fn tuz_decompress_mem(
        in_code: *const c_uchar,
        code_size: usize,
        out_data: *mut c_uchar,
        data_size: *mut usize,
    ) -> u32;
}

/// Default dictionary size (4KB).
const DICT_SIZE_4K: usize = 4096;

/// Compress data using tinyuz with a 4KB dictionary.
///
/// Returns the compressed bytes, or an error if compression fails.
pub fn compress(input: &[u8]) -> anyhow::Result<Vec<u8>> {
    if input.is_empty() {
        anyhow::bail!("tinyuz: cannot compress empty input");
    }

    let max_size = unsafe { tuz_max_compressed_size(input.len()) };
    let mut output = vec![0u8; max_size];

    let compressed_len = unsafe {
        tuz_compress_mem(
            input.as_ptr(),
            input.len(),
            output.as_mut_ptr(),
            output.len(),
            DICT_SIZE_4K,
        )
    };

    if compressed_len == 0 {
        anyhow::bail!("tinyuz: compression failed (returned 0)");
    }

    output.truncate(compressed_len);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decompress(compressed: &[u8], original_len: usize) -> Vec<u8> {
        let mut out = vec![0u8; original_len];
        let mut out_size = original_len;
        let result = unsafe {
            tuz_decompress_mem(
                compressed.as_ptr(),
                compressed.len(),
                out.as_mut_ptr(),
                &mut out_size,
            )
        };
        assert_eq!(
            result, 1,
            "tuz_decompress_mem failed with code {result} (expected STREAM_END=1)"
        );
        assert_eq!(out_size, original_len, "decompressed size mismatch");
        out.truncate(out_size);
        out
    }

    fn assert_round_trip(input: &[u8]) {
        let compressed = compress(input).expect("compression should succeed");
        assert!(!compressed.is_empty(), "compressed output is empty");
        let decompressed = decompress(&compressed, input.len());
        assert_eq!(
            decompressed,
            input,
            "round-trip mismatch (compressed {} -> {} bytes)",
            input.len(),
            compressed.len()
        );
    }

    #[test]
    fn compress_solid_color() {
        // 20 LEDs, all red — should compress well
        let mut rgb_data = Vec::new();
        for _ in 0..20 {
            rgb_data.extend_from_slice(&[255, 0, 0]);
        }

        let compressed = compress(&rgb_data).expect("compression should succeed");
        assert!(!compressed.is_empty());
        // Solid color should compress smaller than original
        assert!(compressed.len() < rgb_data.len());
    }

    #[test]
    fn compress_gradient() {
        // Gradient data — less compressible
        let mut rgb_data = Vec::new();
        for i in 0..80u8 {
            rgb_data.extend_from_slice(&[i, i, i]);
        }

        let compressed = compress(&rgb_data).expect("compression should succeed");
        assert!(!compressed.is_empty());
    }

    #[test]
    fn round_trip_static_color_frame() {
        // Single-color frame: 16 LEDs × 3 bytes (matches typical fan LED count)
        let mut frame = Vec::new();
        for _ in 0..16 {
            frame.extend_from_slice(&[255, 128, 0]);
        }
        assert_round_trip(&frame);
    }

    #[test]
    fn round_trip_two_color_breathing_frame() {
        // Two alternating colors across 40 LEDs (SLV3 LED count per fan)
        let mut frame = Vec::new();
        for i in 0..40 {
            let c = if i % 2 == 0 { [255, 0, 0] } else { [0, 0, 255] };
            frame.extend_from_slice(&c);
        }
        assert_round_trip(&frame);
    }

    #[test]
    fn round_trip_full_palette_frame() {
        // 16 distinct colors across one fan's LEDs
        let palette: [[u8; 3]; 16] = [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 0],
            [255, 0, 255],
            [0, 255, 255],
            [128, 0, 0],
            [0, 128, 0],
            [0, 0, 128],
            [128, 128, 0],
            [128, 0, 128],
            [0, 128, 128],
            [64, 64, 64],
            [192, 192, 192],
            [255, 255, 255],
            [0, 0, 0],
        ];
        let mut frame = Vec::new();
        for c in palette.iter() {
            frame.extend_from_slice(c);
        }
        assert_round_trip(&frame);
    }

    #[test]
    fn round_trip_multi_fan_frame() {
        // Full SLV3 group: 6 fans × 40 LEDs × 3 bytes = 720 bytes
        let mut frame = Vec::new();
        for fan in 0..6u8 {
            for _ in 0..40 {
                frame.extend_from_slice(&[fan * 40, 128, 255 - fan * 40]);
            }
        }
        assert_round_trip(&frame);
    }

    #[test]
    fn compress_is_deterministic() {
        let input: Vec<u8> = (0..120u8)
            .flat_map(|i| [i, i.wrapping_mul(2), 255 - i])
            .collect();
        let a = compress(&input).unwrap();
        let b = compress(&input).unwrap();
        assert_eq!(a, b, "compress should be deterministic for same input");
    }
}

use anyhow::{ensure, Result};

pub(super) fn combine(
    top: Vec<Vec<[u8; 3]>>,
    bottom: Vec<Vec<[u8; 3]>>,
    interval: u32,
) -> Result<Vec<Vec<[u8; 3]>>> {
    ensure!(
        !top.is_empty() && !bottom.is_empty(),
        "empty TL region animation"
    );
    let leds = top[0].len();
    ensure!(leds > 0 && leds.is_multiple_of(26), "invalid TL layout");
    ensure!(
        top.iter().chain(&bottom).all(|frame| frame.len() == leds),
        "inconsistent TL region layout"
    );
    ensure!(interval > 0, "invalid TL interval");
    let (mut longer, shorter, replace_top) = if bottom.len() > top.len() {
        (bottom, top, true)
    } else {
        (top, bottom, false)
    };
    let repeated_len = shorter.len() * (longer.len() / shorter.len());
    let shorter_interval = interval as usize * longer.len() / repeated_len;
    for (index, frame) in longer.iter_mut().enumerate() {
        let source =
            (index * interval as usize / shorter_interval).min(repeated_len - 1) % shorter.len();
        for fan in 0..leds / 26 {
            let start = fan * 26 + if replace_top { 0 } else { 13 };
            frame[start..start + 13].copy_from_slice(&shorter[source][start..start + 13]);
        }
    }
    Ok(longer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_shorter_region_before_mapping_frames() {
        let top = vec![vec![[100; 3]; 26]; 7];
        let bottom = (0..3).map(|i| vec![[i; 3]; 26]).collect();
        let combined = combine(top, bottom, 55).unwrap();
        let indices: Vec<_> = combined.iter().map(|f| f[13][0]).collect();
        assert_eq!(indices, [0, 0, 1, 2, 0, 1, 2]);
        assert!(combined.iter().all(|f| f[..13] == [[100; 3]; 13]));
    }
}

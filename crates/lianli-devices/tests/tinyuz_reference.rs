use lianli_devices::tinyuz;

fn raw_static_16led() -> Vec<u8> {
    (0..16).flat_map(|_| [255u8, 128, 0]).collect()
}

fn raw_static_40led() -> Vec<u8> {
    (0..40).flat_map(|_| [0u8, 255, 0]).collect()
}

fn raw_breathing_40led() -> Vec<u8> {
    (0..40)
        .flat_map(|i| if i % 2 == 0 { [255, 0, 0] } else { [0, 0, 255] })
        .collect()
}

fn raw_palette_16() -> Vec<u8> {
    [
        [255u8, 0, 0],
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
    ]
    .iter()
    .flat_map(|c| c.iter().copied())
    .collect()
}

fn raw_slv3_group() -> Vec<u8> {
    (0..6u8)
        .flat_map(|fan| {
            (0..40).flat_map(move |_| [fan.wrapping_mul(40), 128, 255 - fan.wrapping_mul(40)])
        })
        .collect()
}

fn raw_slinf_group() -> Vec<u8> {
    (0..5u8)
        .flat_map(|fan| (0..44).flat_map(move |_| [fan.wrapping_mul(50), 200, 55]))
        .collect()
}

fn raw_gradient_80() -> Vec<u8> {
    (0..80u8)
        .flat_map(|i| [i, i.wrapping_mul(2), 255 - i])
        .collect()
}

fn raw_black() -> Vec<u8> {
    vec![0u8; 240]
}

fn raw_white() -> Vec<u8> {
    vec![255u8; 240]
}

fn raw_tl_flex() -> Vec<u8> {
    (0..4u8)
        .flat_map(|fan| {
            (0..26).flat_map(move |led| [(fan * 60 + led) as u8, 100, (fan * 30 + led) as u8])
        })
        .collect()
}

fn assert_matches_fixture(name: &str, raw: Vec<u8>) {
    let fixture_path = format!("tests/fixtures/{name}.yuz");
    let expected = std::fs::read(&fixture_path)
        .unwrap_or_else(|e| panic!("failed reading fixture {fixture_path}: {e}"));
    let actual =
        tinyuz::compress(&raw).unwrap_or_else(|e| panic!("compress failed for {name}: {e}"));
    assert_eq!(
        actual,
        expected.as_slice(),
        "{name}: compressed output differs from reference (got {} bytes, expected {})",
        actual.len(),
        expected.len(),
    );
}

#[test]
fn matches_reference_static_16led() {
    assert_matches_fixture("01_static_16led", raw_static_16led());
}

#[test]
fn matches_reference_static_40led() {
    assert_matches_fixture("02_static_40led", raw_static_40led());
}

#[test]
fn matches_reference_breathing_40led() {
    assert_matches_fixture("03_breathing_40led", raw_breathing_40led());
}

#[test]
fn matches_reference_palette_16() {
    assert_matches_fixture("04_palette_16", raw_palette_16());
}

#[test]
fn matches_reference_slv3_group() {
    assert_matches_fixture("05_slv3_group", raw_slv3_group());
}

#[test]
fn matches_reference_slinf_group() {
    assert_matches_fixture("06_slinf_group", raw_slinf_group());
}

#[test]
fn matches_reference_gradient_80() {
    assert_matches_fixture("07_gradient_80", raw_gradient_80());
}

#[test]
fn matches_reference_black() {
    assert_matches_fixture("08_black", raw_black());
}

#[test]
fn matches_reference_white() {
    assert_matches_fixture("09_white", raw_white());
}

#[test]
fn matches_reference_tl_flex() {
    assert_matches_fixture("10_tl_flex", raw_tl_flex());
}

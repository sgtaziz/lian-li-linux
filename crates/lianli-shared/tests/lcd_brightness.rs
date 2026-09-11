use lianli_shared::config::LcdConfig;
use serde_json::json;

#[test]
fn legacy_lcd_brightness_restores_full_brightness_after_restart() {
    let legacy = json!({ "index": 0, "type": "color", "rgb": [0, 0, 0] });
    let config: LcdConfig = serde_json::from_value(legacy).unwrap();
    assert_eq!(config.brightness(), 100);
    let restarted: LcdConfig =
        serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert_eq!(restarted.brightness(), 100);
}

#[test]
fn null_defaults_to_full_brightness_and_explicit_values_are_preserved() {
    for (value, expected) in [
        (None, 100),
        (Some(0), 0),
        (Some(37), 37),
        (Some(100), 100),
        (Some(255), 100),
    ] {
        let config: LcdConfig = serde_json::from_value(json!({
            "index": 0, "type": "color", "rgb": [0, 0, 0], "brightness": value
        }))
        .unwrap();
        assert_eq!(config.brightness(), expected);
    }
}

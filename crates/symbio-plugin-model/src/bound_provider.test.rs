use super::*;

#[test]
fn bumps_top_level_and_nested_temperature_capped_at_one() {
    let body = br#"{"temperature":0.2,"nested":{"temperature":0.9},"name":"x"}"#;
    let out = retry_body_with_bumped_temperature(body, 2);
    let value: Value = serde_json::from_slice(&out).unwrap();
    assert!((value["temperature"].as_f64().unwrap() - 0.6).abs() < 1e-9);
    assert_eq!(value["nested"]["temperature"].as_f64(), Some(1.0));
    assert_eq!(value["name"], "x", "非温度字段不得改动");
}

#[test]
fn leaves_body_untouched_without_temperature() {
    let body = br#"{"messages":[]}"#;
    assert_eq!(retry_body_with_bumped_temperature(body, 1), body.to_vec());
    assert_eq!(
        retry_body_with_bumped_temperature(b"not json", 1),
        b"not json".to_vec()
    );
}

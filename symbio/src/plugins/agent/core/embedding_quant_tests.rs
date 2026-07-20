//! embedding_quant.rs 单元测试
//!
//! 对应源文件: `embedding_quant.rs`

use super::*;

#[test]
fn test_quantize_q8_basic_roundtrip() {
    let v = vec![0.0_f32, 0.5, 1.0, -0.25, 0.75];
    let q = quantize_q8(&v);
    let recovered = dequantize_q8(&q);
    assert_eq!(q.data.len(), 5);
    // 误差 ≤ 1 LSB ≈ scale
    for (i, (&orig, &rec)) in v.iter().zip(recovered.iter()).enumerate() {
        let diff = (orig - rec).abs();
        assert!(
            diff <= q.scale + 1e-6,
            "i={} diff={} scale={}",
            i,
            diff,
            q.scale
        );
    }
}

#[test]
fn test_quantize_q8_constant_vector() {
    // 全相等 → 不除零
    let v = vec![1.5_f32; 8];
    let q = quantize_q8(&v);
    let recovered = dequantize_q8(&q);
    // scale=1.0 zero_point=0 → q 全 0（round(1.5/1+0) = 2）
    // 实际：range < EPSILON 时 (1.5, 1.5)，scale=1.0 zp=0，每值 = round(1.5 + 0) = 2
    // 反量化: (2 - 0) * 1.0 = 2
    for &r in &recovered {
        assert!((r - 2.0).abs() < 1.0, "got {}", r);
    }
}

#[test]
fn test_quantize_q8_empty() {
    let q = quantize_q8(&[]);
    assert_eq!(q.data.len(), 0);
    assert_eq!(q.scale, 1.0);
    assert_eq!(q.zero_point, 0.0);
    let rec = dequantize_q8(&q);
    assert!(rec.is_empty());
}

#[test]
fn test_quantize_q8_nan_inf() {
    let v = vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0, 0.0];
    let q = quantize_q8(&v);
    let rec = dequantize_q8(&q);
    assert_eq!(rec.len(), 5);
    // 替换为 0 后范围 [0,1]，[0,0,0,1,0] → 都应在 0~1 附近
    for &r in &rec {
        assert!(r.is_finite());
    }
}

#[test]
fn test_q8_to_value_and_back() {
    let v: Vec<f32> = (0..32).map(|i| i as f32 * 0.1).collect();
    let q = quantize_q8(&v);
    let val = q8_to_value(&q);
    // 验证 JSON 结构（v19：含 _format_version 字段）
    let obj = val.as_object().unwrap();
    assert!(obj.contains_key("q8"));
    assert!(obj.contains_key("scale"));
    assert!(obj.contains_key("zero_point"));
    assert!(obj.contains_key("_format_version"));
    assert_eq!(
        obj.get("_format_version").and_then(|x| x.as_u64()),
        Some(Q8_FORMAT_VERSION_V2 as u64)
    );
    // 反序列化
    let q2 = value_to_q8(&val).unwrap();
    assert_eq!(q2.data.len(), q.data.len());
    assert!((q2.scale - q.scale).abs() < 1e-6);
    assert!((q2.zero_point - q.zero_point).abs() < 1e-6);
}

#[test]
fn test_q8_v1_compat_no_format_version() {
    // v19 兼容测试：v16 时代的 Q8 数据（无 _format_version 字段）应能正常解码
    let v1_obj = serde_json::json!({
        "q8": [0, 64, 128, 192, 255],
        "scale": 0.01,
        "zero_point": 0.0
        // 注意：没有 _format_version 字段
    });
    let q = value_to_q8(&v1_obj).expect("v1 data should decode");
    assert_eq!(q.data, vec![0, 64, 128, 192, 255]);
    assert!((q.scale - 0.01).abs() < 1e-6);
}

#[test]
fn test_q8_unknown_format_version_rejected() {
    // v19：未知 _format_version 拒绝（不静默降级）
    let future = serde_json::json!({
        "q8": [0, 1, 2],
        "scale": 1.0,
        "zero_point": 0.0,
        "_format_version": 999
    });
    assert!(value_to_q8(&future).is_none());
}

#[test]
fn test_value_to_q8_strict_legacy_rejected() {
    // v19 修复：value_to_q8 不再兜底 legacy 格式
    let legacy = Value::Array(
        vec![1.0_f32, 2.0, 3.0]
            .into_iter()
            .map(|f| {
                serde_json::Number::from_f64(f as f64)
                    .map(Value::Number)
                    .unwrap()
            })
            .collect(),
    );
    // 改用 dequantize_from_value 才能读 legacy
    assert!(value_to_q8(&legacy).is_none());
    let rec = dequantize_from_value(&legacy).unwrap();
    assert_eq!(rec, vec![1.0_f32, 2.0, 3.0]);
}

#[test]
fn test_dequantize_from_value_legacy_negative_values() {
    // v19 关键回归测试：legacy 格式带负值必须完整保留
    let legacy = Value::Array(
        vec![-0.3_f32, 0.5, 1.2, -1.5, 0.001]
            .into_iter()
            .map(|f| {
                serde_json::Number::from_f64(f as f64)
                    .map(Value::Number)
                    .unwrap()
            })
            .collect(),
    );
    let rec = dequantize_from_value(&legacy).expect("legacy must decode");
    assert_eq!(rec.len(), 5);
    // 关键断言：负值必须保留
    assert_eq!(rec[0], -0.3, "v18 静默丢失负值（clamp 到 0）— v19 已修复");
    assert!(
        (rec[1] - 0.5).abs() < 1e-6,
        "v18 把 0.5 round 到 1 — v19 已修复"
    );
    assert!(
        (rec[2] - 1.2).abs() < 1e-6,
        "v18 把 1.2 round 到 1 — v19 已修复"
    );
    assert_eq!(rec[3], -1.5);
    assert!((rec[4] - 0.001).abs() < 1e-5);
}

#[test]
fn test_dequantize_from_value_legacy_integer_values() {
    // 旧测试的语义纠正：legacy 直接读为 f32，**不走 Q8 round 转换**
    let legacy = Value::Array(
        vec![0.0_f32, 0.5, 1.0]
            .into_iter()
            .map(|f| {
                serde_json::Number::from_f64(f as f64)
                    .map(Value::Number)
                    .unwrap()
            })
            .collect(),
    );
    let rec = dequantize_from_value(&legacy).expect("legacy must decode");
    // 关键：原值是 0.5，恢复后还是 0.5（v18 是 1.0，v19 已修）
    assert_eq!(rec[0], 0.0);
    assert_eq!(rec[1], 0.5, "v18 把 0.5 当 u8 round 到 1 — v19 已修复");
    assert_eq!(rec[2], 1.0);
}

#[test]
fn test_dequantize_from_value_rejects_non_q8_object() {
    // v19：Value::Object 但不含 q8 字段 → 视为未知格式
    let bogus = serde_json::json!({
        "scale": 1.0,
        "zero_point": 0.0
    });
    assert!(dequantize_from_value(&bogus).is_none());
}

#[test]
fn test_dequantize_from_value_nan_in_scale_returns_none() {
    // v19：scale 是 NaN/Inf 时返 None（不污染搜索结果）
    let bad = serde_json::json!({
        "q8": [0, 128, 255],
        "scale": f64::NAN,
        "zero_point": 0.0,
        "_format_version": 2
    });
    assert!(dequantize_from_value(&bad).is_none());
}

#[test]
fn test_q8_to_value_sanitizes_nan_scale() {
    // v19：写入侧 NaN scale 应被替换为 0 + warn
    let bad = Q8Embedding {
        data: vec![0, 128, 255],
        scale: f32::NAN,
        zero_point: 0.0,
    };
    let val = q8_to_value(&bad);
    let obj = val.as_object().unwrap();
    // sanitize 后 scale 应该是 0
    assert_eq!(obj.get("scale").and_then(|x| x.as_f64()), Some(0.0));
}

#[test]
fn test_q8_to_value_rejects_out_of_range_bytes() {
    // v19：data 中 > 255 的字节应被标记为可疑（不会 panic，但 warn）
    let bad = serde_json::json!({
        "q8": [0, 128, 300, 255],  // 300 > 255
        "scale": 0.01,
        "zero_point": 0.0,
        "_format_version": 2
    });
    // value_to_q8 会过滤掉 300（返回 3 字节），并 warn
    let q = value_to_q8(&bad).expect("should still decode with warn");
    assert_eq!(q.data.len(), 3);
    assert_eq!(q.data, vec![0, 128, 255]);
}

#[test]
fn test_dequantize_from_value_full_pipeline() {
    let v: Vec<f32> = (0..16).map(|i| (i as f32 - 8.0) * 0.05).collect();
    let q = quantize_q8(&v);
    let val = q8_to_value(&q);
    let rec = dequantize_from_value(&val).unwrap();
    assert_eq!(rec.len(), v.len());
    for (a, b) in v.iter().zip(rec.iter()) {
        assert!((a - b).abs() < q.scale + 1e-5);
    }
}

#[test]
fn test_memory_savings() {
    // 512 维 embedding：原 2048B，Q8 后 520B
    let v: Vec<f32> = (0..512).map(|i| (i as f32) * 0.01).collect();
    let q = quantize_q8(&v);
    assert!(q.approx_bytes() < v.len() * 4);
    // 节省 ≥ 70%
    assert!(q.approx_bytes() * 10 < v.len() * 4 * 10 / 3);
}

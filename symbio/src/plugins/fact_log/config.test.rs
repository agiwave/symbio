//! fact_log 配置自测。

use super::*;

#[test]
fn defaults_are_sane() {
    let c = FactLogConfig::default();
    assert!(c.enabled, "出厂应启用（可选不等于默认关闭）");
    assert_eq!(c.effective_max_facts(), 4096);
}

/// 反向用例：配成 0 时必须收敛到 1，否则"一切查询都空"会看不出原因。
#[test]
fn zero_is_floored_to_one() {
    let c = FactLogConfig {
        enabled: true,
        max_facts: 0,
    };
    assert_eq!(c.effective_max_facts(), 1, "0 应被下界修正为 1");
}

/// 反序列化缺字段时取默认值（老配置兼容）。
#[test]
fn missing_fields_default() {
    let c: FactLogConfig = serde_json::from_str("{}").unwrap();
    assert!(c.enabled);
    assert_eq!(c.max_facts, 4096);
}

//! `crates/symbio-plugin-gateway/src/config.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[test]
fn default_inbound_off_and_native() {
    let c = GatewayConfig::default();
    assert!(!c.inbound_enabled);
    assert_eq!(c.inbound_protocol, "native");
    assert_eq!(c.inbound_port, 9231);
    assert_eq!(c.inbound_bind, "127.0.0.1");
    // 配置仅描述入站，不含出站字段
    let v = serde_json::to_value(&c).unwrap();
    assert!(v.get("outbound_protocol").is_none());
    assert!(v.get("outbound_endpoint").is_none());
    assert!(v.get("outbound_token").is_none());
}

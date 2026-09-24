//! `symbio/src/plugins/gateway/config.rs` 的单元测试 —— 拆自源码末尾的测试模块。
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

#[test]
fn readonly_allowlist() {
    let none = serde_json::json!({});
    // 合成根：只读判定**与根名无关**——它认的是「操作名」与「路径末段是不是
    // 配置文件」，因此测试不该、也不需要知道真实挂载名。
    let at = |rel: &str| serde_json::json!({ "path": format!("@vfs/{rel}") });

    // 放行：查询类 / VDFS 读操作
    assert!(is_readonly_allowed("vdfs/list", &none));
    assert!(is_readonly_allowed("vdfs/root", &none));
    assert!(is_readonly_allowed("vdfs/read", &at("session/abc")));
    assert!(is_readonly_allowed("vdfs/search", &at("model")));
    assert!(is_readonly_allowed("home/get_homedir", &none));
    assert!(is_readonly_allowed("work/get_workspace", &none));

    // 拒绝：写操作 / 命令执行 / 未知路径
    assert!(!is_readonly_allowed("vdfs/write", &none));
    assert!(!is_readonly_allowed("vdfs/delete", &none));
    assert!(!is_readonly_allowed("vdfs/action", &none));
    assert!(!is_readonly_allowed("session/chat/send", &none));
    assert!(!is_readonly_allowed("bogus/path", &none));

    // 拒绝：读**插件配置文件**——配置可能含凭据（网关访问令牌、
    // 搜索服务 API Key），放行等于只读模式下就能把它们读走
    assert!(!is_readonly_allowed("vdfs/read", &at("gateway/PLUGIN.yml")));
    assert!(!is_readonly_allowed("vdfs/read", &at("web/PLUGIN.yml")));
    // 配置文件的**节点**仍可 stat（节点只有 schema，无正文）
    assert!(is_readonly_allowed("vdfs/stat", &at("web/PLUGIN.yml")));
    // 与配置文件无关的读不受影响
    assert!(is_readonly_allowed("vdfs/read", &at("web")));
}

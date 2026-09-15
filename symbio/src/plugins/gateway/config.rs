//! 网关配置：仅保留「入站」（本应用对外提供服务的 HTTP/WebSocket 网关）。
//!
//! 字段刻意**扁平化**（`inbound_*`），以直接契合配置文档的 flat key 绑定：
//! 一次 `vdfs/write` 整体替换，无需嵌套路径解析。
//!
//! **出站（前端连向何处）不由本插件持有**：连接目标由前端「系统目录」切换器
//! 统一管理（localStorage 为权威），经 `initGatewayTransport` 决定 native / http 出站。
//! 因此本配置只描述「本实例如何被调用（入站）」。

use serde::{Deserialize, Serialize};

/// 网关配置（整体作为插件配置持久化于 `symbio.plugins.gateway`）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayConfig {
    // ---- 入站：本应用对外提供服务 ----
    /// 是否开放入站服务
    pub inbound_enabled: bool,
    /// 入站协议：`native`（仅 Tauri IPC，不监听）| `http`
    pub inbound_protocol: String,
    /// 监听地址（默认 `127.0.0.1`；`0.0.0.0` 对外暴露需谨慎）
    pub inbound_bind: String,
    /// 监听端口
    pub inbound_port: u16,
    /// 访问令牌（Bearer Token）；为空表示不校验（仅限回环地址时允许）
    pub inbound_token: String,
    /// 只读模式：仅放行查询类路径
    pub inbound_readonly: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            inbound_enabled: false,
            inbound_protocol: "native".to_string(),
            inbound_bind: "127.0.0.1".to_string(),
            inbound_port: 9231,
            inbound_token: String::new(),
            inbound_readonly: false,
        }
    }
}

/// 只读模式放行的路径白名单
///
/// 只读并非完整安全边界，而是**兜底**：即便令牌泄露到可信内网，也只能读取
/// 而无法触发写操作与命令执行。
///
/// **资源一律经 VDFS**（`vdfs/<操作>`）：只读放行其**读操作**——
/// `list`（`.vdfs` 即资源类别清单）/ `tree` / `stat` / `read` / `search`；
/// 写操作（`write` / `delete` / `mkdir` / `move` / `edit`）与节点动作
/// （`action`）不在其列。
///
/// **配置文件是例外**：插件配置可能含凭据（网关访问令牌、搜索服务 API Key），
/// 因此 `vdfs/read` 落到任一插件的配置文件（`<挂载点>/PLUGIN.yml`）时一律拒绝——
/// 否则只读模式下就能把令牌读走。（`tree` / `search` 只回地址与节点描述、不回正文，
/// 故不在此列。）
pub fn is_readonly_allowed(path: &str, payload: &serde_json::Value) -> bool {
    let p = path.trim_start_matches('/');
    if p == "vdfs/read" && reads_config_document(payload) {
        return false;
    }
    matches!(
        p,
        "vdfs/list"
            | "vdfs/tree"
            | "vdfs/stat"
            | "vdfs/read"
            | "vdfs/search"
            | "session/get_messages"
            | "home/get_homedir"
            | "work/get_workspace"
    )
}

/// `vdfs/read` 的地址是否正好落在某个插件的配置文件上
fn reads_config_document(payload: &serde_json::Value) -> bool {
    payload
        .get("path")
        .and_then(serde_json::Value::as_str)
        .map(|addr| {
            addr.trim_end_matches('/')
                .ends_with(&format!("/{}", crate::symbio_core::PLUGIN_FILE))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
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
        let at = |addr: &str| serde_json::json!({ "path": addr });

        // 放行：查询类 / VDFS 读操作
        assert!(is_readonly_allowed("vdfs/list", &none));
        assert!(is_readonly_allowed("vdfs/read", &at(".vdfs/session/abc")));
        assert!(is_readonly_allowed("vdfs/search", &at(".vdfs/model")));
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
        assert!(!is_readonly_allowed(
            "vdfs/read",
            &at(".vdfs/gateway/PLUGIN.yml")
        ));
        assert!(!is_readonly_allowed(
            "vdfs/read",
            &at(".vdfs/web/PLUGIN.yml")
        ));
        // 配置文件的**节点**仍可 stat（节点只有 schema，无正文）
        assert!(is_readonly_allowed(
            "vdfs/stat",
            &at(".vdfs/web/PLUGIN.yml")
        ));
        // 与配置文件无关的读不受影响
        assert!(is_readonly_allowed("vdfs/read", &at(".vdfs/web")));
    }
}

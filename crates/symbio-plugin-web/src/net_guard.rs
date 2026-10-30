//! 出站地址守卫：`web_fetch` 与 `http_request` 共用的主机判定。
//!
//! 两个工具都是「模型给 URL，我们发请求」，SSRF 判据必须**同一套**：
//! 一处按 IP 语义拦回环/内网、另一处只校验 scheme，用户看到的就是
//! 「同一个内网地址，一个工具能取、另一个不能」——没有可解释的差异。
//!
//! ## 三道闸，缺一不可
//!
//! | 闸 | 判据 | 出处 |
//! |---|---|---|
//! | ① scheme | 只允许 http / https | 各工具 |
//! | ② SSRF | 回环 / 内网 / 链路本地按 **IP 语义**拒 | [`is_private_or_local_host`] |
//! | ③ Agent 声明 | 出站主机须在**当前 Agent** 的 `permissions.network.domains` 里 | [`agent_network_allowed`] |
//!
//! ③ 是 [D-153](../../../docs/003-design/011-Agent目录规范.md#d-153-权限四字段的执行面同批落地空清单即拒glob-只管自己的包 "DE-011-Agent目录规范")
//! 的落地：没有选中 Agent 时它恒过（宿主态，无 manifest 可声明）；选中了而清单为空
//! ⇒ 一律拒（grant 语义，空清单即没有授权）。

use symbio_plugin_api::AgentScope;
use symbio_plugin_api::PluginError;

/// 从 URL 提取主机
pub(crate) fn extract_host(url: &str) -> Result<String, PluginError> {
    let url = url::Url::parse(url)
        .map_err(|e| PluginError::ValidationError(format!("无效的 URL: {e}")))?;

    url.host_str()
        .map(|h| h.to_string())
        .ok_or_else(|| PluginError::ValidationError("URL 缺少主机".to_string()))
}

/// 检查是否为私有 / 本地 / 不可路由的主机。
///
/// 判据走 **IP 语义**而不是字符串前缀：`127.0.0.1` 是前缀比较唯一认得出的回环地址，
/// 但 `127.0.0.2`、`0.0.0.0`、`::1`、`::ffff:127.0.0.1` 全都不是它，前缀比较会逐个
/// 放行——而这些地址指向的正是本机与内网（`169.254.169.254` 是云元数据端点）。
pub(crate) fn is_private_or_local_host(host: &str) -> bool {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
            }
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local() // fc00::/7
                    || v6.is_unicast_link_local() // fe80::/10
                    // IPv4-mapped（`::ffff:127.0.0.1`）：按内层 IPv4 判
                    || v6.to_ipv4_mapped()
                        .is_some_and(|v4| v4.is_loopback() || v4.is_private() || v4.is_link_local())
            }
        },
        // 不是 IP：内网域名的常见后缀
        Err(_) => host.ends_with(".local") || host.ends_with(".internal"),
    }
}

/// 第三道闸：出站主机须在**当前 Agent 声明的域名白名单**里（D-153）。
///
/// 三个出站工具（`web_fetch` / `http_request` / `web_search`）共用它，所以
/// 「同一个主机，一个工具能去、另一个不能」不会出现——`web_search` 的上游虽由
/// 宿主配置决定，数据照样经那个主机出境，一并判。
///
/// `scope = None` ⇒ 本会话没有选中 Agent（宿主态，`agent:main`）：恒过。
/// 选中了而清单为空 ⇒ 拒（**空清单即没有授权**，与授权表同一条 fail-closed 纪律）。
pub(crate) fn agent_network_allowed(
    host: &str,
    scope: Option<&AgentScope>,
) -> Result<(), PluginError> {
    let Some(scope) = scope else {
        return Ok(());
    };
    if scope.permissions.host_allowed(host) {
        return Ok(());
    }
    Err(PluginError::ValidationError(format!(
        "主机 `{host}` 不在智能体 `{}` 声明的 permissions.network.domains 里（空清单 = 无出站授权）",
        scope.id
    )))
}

#[cfg(test)]
#[path = "net_guard.test.rs"]
mod tests;

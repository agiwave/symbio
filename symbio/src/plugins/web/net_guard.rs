//! 出站地址守卫：`web_fetch` 与 `http_request` 共用的主机判定。
//!
//! 两个工具都是「模型给 URL，我们发请求」，SSRF 判据必须**同一套**：
//! 一处按 IP 语义拦回环/内网、另一处只校验 scheme，用户看到的就是
//! 「同一个内网地址，一个工具能取、另一个不能」——没有可解释的差异。

use crate::symbio_core::PluginError;

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

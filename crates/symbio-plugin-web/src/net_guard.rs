//! 出站地址守卫：`web_fetch` 与 `http_request` 共用的 URL、DNS 与重定向判定。
//!
//! 两个工具都是「模型给 URL，我们发请求」，SSRF 判据必须**同一套**：
//! 一处按 IP 语义拦回环/内网、另一处只校验 scheme，用户看到的就是
//! 「同一个内网地址，一个工具能取、另一个不能」——没有可解释的差异。
//!
//! ## 三道闸，缺一不可
//!
//! | 闸 | 判据 | 出处 |
//! |---|---|---|
//! | ① scheme | 只允许 http / https | [`validate_http_url`] |
//! | ② SSRF | 字面主机与 **DNS 返回的每一个地址**都按 IP 语义拒 | [`resolve_public_addresses`] |
//! | ③ Agent 声明 | 每一跳的出站主机都须在**当前 Agent** 的 `permissions.network.domains` 里 | [`agent_network_allowed`] |
//!
//! DNS 解析后不是把结果交回 reqwest 再解析一次，而是用 `ClientBuilder::resolve`
//! 钉住已校验的公网地址；URL 仍保留原主机名，Host 与 TLS SNI 都不会变成 IP。
//! 自动重定向必须关闭：reqwest 默认替调用方跟随 Location，不会经过这里的逐跳判据。

use reqwest::{Client, Method, Response, StatusCode};
use serde_json::Value;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use symbio_plugin_api::{AgentScope, CapabilityRiskLevel, PluginError, SandboxFace, SandboxFact};
use url::Url;

/// 手动跟随的最大跳数；初始请求不计入其中。
pub(crate) const MAX_REDIRECTS: usize = 5;

/// 边读边限制响应大小；返回解码正文、是否截断和实际读到的字节数。
pub(crate) async fn read_response_text(
    mut response: Response,
    limit: usize,
) -> Result<(String, bool, usize), PluginError> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let encoding = content_type
        .split(';')
        .skip(1)
        .filter_map(|part| part.trim().split_once('='))
        .find(|(name, _)| name.eq_ignore_ascii_case("charset"))
        .map(|(_, value)| value.trim().trim_matches('"'))
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);

    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    let mut truncated = false;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| PluginError::InternalError(format!("读取响应失败: {error}")))?
    {
        let remaining = limit.saturating_add(1).saturating_sub(bytes.len());
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    let size = bytes.len();
    let decoded = encoding.decode(&bytes).0;
    let mut content = decoded.to_string();
    if content.len() > limit {
        let end = symbio_plugin_api::text_floor_char_boundary(&content, limit);
        content.truncate(end);
        truncated = true;
    }
    Ok((content, truncated, size))
}

/// URL 意图事实的唯一归一化形状：注册收口点与逐跳裁决共用，均去掉 fragment。
pub(crate) fn normalized_url_fact(raw: &str) -> String {
    url::Url::parse(raw)
        .map(|mut value| {
            value.set_fragment(None);
            let _ = value.set_username("");
            let _ = value.set_password(None);
            value.to_string()
        })
        .unwrap_or_else(|_| raw.trim().to_string())
}

/// 一次出站调用固定使用的沙箱上下文；随重定向复用句柄与原始参数，但逐跳重算事实。
pub(crate) struct SandboxPolicy<'a> {
    sandbox: Option<&'a dyn SandboxFace>,
    tool_name: &'a str,
    args: &'a Value,
    tool_risk: CapabilityRiskLevel,
}

impl<'a> SandboxPolicy<'a> {
    pub(crate) fn new(
        sandbox: Option<&'a dyn SandboxFace>,
        tool_name: &'a str,
        args: &'a Value,
        tool_risk: CapabilityRiskLevel,
    ) -> Self {
        Self {
            sandbox,
            tool_name,
            args,
            tool_risk,
        }
    }

    /// 初始 URL 的审批由工具注册闸门承接，这里只复核许可与拦截。
    pub(crate) fn authorize(&self, url: &Url) -> Result<(), PluginError> {
        self.authorize_inner(url, false)
    }

    /// Location 不在模型已审批的原始参数里；高风险跳转必须在工具内单独拒绝。
    pub(crate) fn authorize_redirect(&self, url: &Url) -> Result<(), PluginError> {
        self.authorize_inner(url, true)
    }

    fn authorize_inner(&self, url: &Url, redirected: bool) -> Result<(), PluginError> {
        let Some(sandbox) = self.sandbox else {
            return Ok(());
        };
        let facts = [SandboxFact::new("url", normalized_url_fact(url.as_str()))];
        let decision = sandbox.decide(&facts, self.tool_name, self.args);
        if let Some(intercept) = decision.intercept {
            return Err(PluginError::Forbidden(format!(
                "沙箱拦截 {} 的出站 URL {url}: {intercept}",
                self.tool_name
            )));
        }
        if !decision.permitted {
            let reason = decision
                .reason
                .unwrap_or_else(|| "sandbox denied the call".to_string());
            return Err(PluginError::Forbidden(format!(
                "沙箱未放行 {} 的出站 URL {url}: {reason}",
                self.tool_name
            )));
        }
        if redirected {
            if let Some(threshold) = &decision.risk_threshold {
                let rule_risk = decision
                    .risk
                    .as_deref()
                    .map(CapabilityRiskLevel::from_name)
                    .unwrap_or(CapabilityRiskLevel::Low);
                let effective_risk = self.tool_risk.max(rule_risk);
                let threshold = CapabilityRiskLevel::from_name(threshold);
                if effective_risk > threshold {
                    return Err(PluginError::Forbidden(format!(
                        "沙箱风险阈值阻止 {} 的出站 URL {url}: 有效风险 {} 高于自动放行阈值 {}，重定向目标需要单独审批",
                        self.tool_name,
                        effective_risk.as_str(),
                        threshold.as_str(),
                    )));
                }
            }
        }
        Ok(())
    }
}

/// 从 URL 提取主机
pub(crate) fn extract_host(url: &str) -> Result<String, PluginError> {
    let url =
        Url::parse(url).map_err(|e| PluginError::ValidationError(format!("无效的 URL: {e}")))?;

    url.host_str()
        .map(|h| h.to_string())
        .ok_or_else(|| PluginError::ValidationError("URL 缺少主机".to_string()))
}

/// 校验模型可控 HTTP URL 的字面主机；DNS 结果由 [`resolve_public_addresses`] 另行校验。
pub(crate) fn validate_http_url(raw_url: &str) -> Result<Url, PluginError> {
    parse_http_url(raw_url, false)
}

/// 校验管理员显式配置的搜索上游。允许内网地址，但 scheme 与凭据约束不变。
pub(crate) fn validate_upstream_http_url(raw_url: &str) -> Result<Url, PluginError> {
    parse_http_url(raw_url, true)
}

fn parse_http_url(raw_url: &str, allow_private: bool) -> Result<Url, PluginError> {
    let raw_url = raw_url.trim();
    if raw_url.is_empty() {
        return Err(PluginError::ValidationError("URL 不能为空".to_string()));
    }
    if raw_url.chars().any(char::is_whitespace) {
        return Err(PluginError::ValidationError("URL 不能包含空格".to_string()));
    }

    let url = Url::parse(raw_url)
        .map_err(|e| PluginError::ValidationError(format!("无效的 URL: {e}")))?;
    validate_url_shape(&url)?;
    if !allow_private {
        validate_public_host(&url)?;
    }
    Ok(url)
}

/// 对已解析 URL 做 scheme / 主机 / 字面 IP 校验。
pub(crate) fn validate_public_url(url: &Url) -> Result<(), PluginError> {
    validate_url_shape(url)?;
    validate_public_host(url)
}

fn validate_url_shape(url: &Url) -> Result<(), PluginError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(PluginError::ValidationError(
            "不支持的 URL scheme；只允许 HTTP/HTTPS URL".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PluginError::ValidationError(
            "URL 不得包含用户名或密码凭据".to_string(),
        ));
    }
    url.host_str()
        .map(|_| ())
        .ok_or_else(|| PluginError::ValidationError("URL 缺少主机".to_string()))
}

fn validate_public_host(url: &Url) -> Result<(), PluginError> {
    let host = url
        .host_str()
        .ok_or_else(|| PluginError::ValidationError("URL 缺少主机".to_string()))?;
    if is_private_or_local_host(host) {
        return Err(PluginError::ValidationError(format!(
            "阻止本地/私有主机: {host}"
        )));
    }
    Ok(())
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
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(ip) => is_private_or_local_ip(ip),
        // 不是 IP：内网域名的常见后缀
        Err(_) => host.ends_with(".local") || host.ends_with(".internal"),
    }
}

/// DNS 答案与字面 IP 共用的地址判据；任何一个答案不可公网路由就整体拒绝。
fn is_private_or_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_or_local_ipv4(v4),
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_multicast()
                // fec0::/10 是已弃用的站点本地地址。
                || (segments[0] & 0xffc0 == 0xfec0)
                || v6.to_ipv4_mapped().is_some_and(is_private_or_local_ipv4)
                // ::a.b.c.d 形式的 IPv4-compatible 地址同样不能绕过内网判据。
                || v6.to_ipv4().is_some_and(is_private_or_local_ipv4)
        }
    }
}

fn is_private_or_local_ipv4(v4: Ipv4Addr) -> bool {
    let octets = v4.octets();
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_multicast()
        || v4.is_broadcast()
        // 100.64/10 是运营商级 NAT，不是公网地址。
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        // 198.18/15 为基准测试保留，240/4 为将来使用保留；都不能作为公网目标。
        || (octets[0] == 198 && (18..=19).contains(&octets[1]))
        || octets[0] >= 240
        // 文档示例网段不是真实公网服务； fail-closed 避免规则误配后发往保留地址。
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
}

/// 校验完整 DNS 答案集，并去掉重复项。混合公网/私网答案必须 fail-closed。
pub(crate) fn validate_resolved_addresses(
    host: &str,
    addresses: &[SocketAddr],
) -> Result<Vec<SocketAddr>, PluginError> {
    if addresses.is_empty() {
        return Err(PluginError::InternalError(format!(
            "DNS 解析失败: 主机 {host} 未返回任何地址"
        )));
    }

    let mut public_addresses = Vec::with_capacity(addresses.len());
    for address in addresses {
        if is_private_or_local_ip(address.ip()) {
            return Err(PluginError::ValidationError(format!(
                "阻止本地/私有地址: {}（主机 {host}）",
                address.ip()
            )));
        }
        if !public_addresses.contains(address) {
            public_addresses.push(*address);
        }
    }
    Ok(public_addresses)
}

/// 用 Tokio 的系统解析器解析主机，并校验**每一个**返回地址。
///
/// 只检查字面主机名不足以防 DNS rebinding；这里解析一次、校验一次，随后由
/// [`send_with_safe_redirects`] 把结果钉进 reqwest，连接阶段不会再解析。
pub(crate) async fn resolve_public_addresses(
    host: &str,
    port: u16,
) -> Result<Vec<SocketAddr>, PluginError> {
    let addresses = resolve_addresses(host, port).await?;
    validate_resolved_addresses(host, &addresses)
}

async fn resolve_addresses(host: &str, port: u16) -> Result<Vec<SocketAddr>, PluginError> {
    let lookup_host = if host.starts_with('[') {
        if host.ends_with(']') {
            format!("{host}:{port}")
        } else {
            format!("[{host}]:{port}")
        }
    } else if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let addresses = tokio::net::lookup_host(lookup_host)
        .await
        .map_err(|e| PluginError::InternalError(format!("DNS 解析失败: 无法解析主机 {host}: {e}")))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(PluginError::InternalError(format!(
            "DNS 解析失败: 主机 {host} 未返回任何地址"
        )));
    }
    Ok(addresses)
}

#[derive(Clone)]
pub(crate) enum OutboundBody {
    Text(String),
    Json(Value),
}

/// 一次出站请求的可重放模板；307/308 需要原样复用 body。
#[derive(Clone)]
pub(crate) struct OutboundRequest {
    pub(crate) method: Method,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<OutboundBody>,
}

impl OutboundRequest {
    /// 按状态码应用标准重定向方法/body 变更，并避免跨源泄露凭证头。
    pub(crate) fn redirected(
        mut self,
        status: StatusCode,
        from: &Url,
        to: &Url,
    ) -> Result<Self, PluginError> {
        let post_becomes_get = matches!(status, StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND)
            && self.method == Method::POST;
        let see_other_becomes_get = status == StatusCode::SEE_OTHER && self.method != Method::HEAD;
        if post_becomes_get || see_other_becomes_get {
            self.method = Method::GET;
            self.body = None;
            self.headers.retain(|(name, _)| {
                !matches!(
                    name.to_ascii_lowercase().as_str(),
                    "content-length" | "content-type" | "transfer-encoding"
                )
            });
        }

        if from.origin() != to.origin() {
            if matches!(
                status,
                StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT
            ) && self.body.is_some()
            {
                return Err(PluginError::Forbidden(
                    "跨源 307/308 重定向不得重放请求体".to_string(),
                ));
            }
            self.headers.retain(|(name, _)| {
                matches!(
                    name.to_ascii_lowercase().as_str(),
                    "accept" | "accept-language" | "content-type" | "content-length"
                )
            });
        }
        Ok(self)
    }
}

fn is_redirect_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

/// 解析下一跳并在发出请求前重新过 scheme / 字面主机 / 域名与 Agent 闸。
///
/// `validate_host` 由调用方提供宿主域名白名单与 Agent scope 的交集判据。
pub(crate) fn next_redirect<F>(
    current_url: &Url,
    status: StatusCode,
    location: Option<&str>,
    redirects_used: usize,
    mut validate_host: F,
) -> Result<Option<Url>, PluginError>
where
    F: FnMut(&str) -> Result<(), PluginError>,
{
    if !is_redirect_status(status) {
        return Ok(None);
    }
    // 没有 Location 的重定向响应不是可跟随跳，按普通响应返回。
    let Some(location) = location else {
        return Ok(None);
    };
    if redirects_used >= MAX_REDIRECTS {
        return Err(PluginError::InternalError(format!(
            "请求失败: 重定向次数超过限制（最多 {MAX_REDIRECTS} 次）"
        )));
    }

    let target = current_url
        .join(location)
        .map_err(|e| PluginError::ValidationError(format!("无效的重定向 Location: {e}")))?;
    validate_public_url(&target)?;
    let host = target
        .host_str()
        .ok_or_else(|| PluginError::ValidationError("重定向 URL 缺少主机".to_string()))?;
    validate_host(host)?;
    Ok(Some(target))
}

/// 共用安全发送循环：逐跳校验、逐次 DNS 解析并钉住地址、手动跟随重定向。
///
/// `trusted_origin` 只豁免管理员显式配置的搜索上游初始地址；模型 URL 与所有重定向
/// 仍必须通过公网地址校验。
pub(crate) async fn send_with_safe_redirects<F>(
    initial_url: Url,
    mut request: OutboundRequest,
    timeout: Duration,
    user_agent: Option<&str>,
    sandbox: SandboxPolicy<'_>,
    trusted_origin: Option<url::Origin>,
    mut validate_host: F,
) -> Result<reqwest::Response, PluginError>
where
    F: FnMut(&str) -> Result<(), PluginError>,
{
    let operation = async {
        let mut current_url = initial_url;
        let mut redirects_used = 0;

        loop {
            let trusted_initial = redirects_used == 0
                && trusted_origin
                    .as_ref()
                    .is_some_and(|origin| &current_url.origin() == origin);
            if trusted_initial {
                validate_url_shape(&current_url)?;
            } else {
                validate_public_url(&current_url)?;
            }
            // Location 是模型无法在初始参数里声明的数据；每一跳都必须重新交用户沙箱裁决，
            // 且拒绝发生在 DNS 解析与建连之前。无沙箱句柄时保持原出站行为。
            if redirects_used == 0 {
                sandbox.authorize(&current_url)?;
            } else {
                sandbox.authorize_redirect(&current_url)?;
            }
            let host = current_url
                .host_str()
                .ok_or_else(|| PluginError::ValidationError("URL 缺少主机".to_string()))?;
            validate_host(host)?;

            let port = current_url
                .port_or_known_default()
                .ok_or_else(|| PluginError::ValidationError("URL 缺少端口".to_string()))?;
            let addresses = if trusted_initial {
                resolve_addresses(host, port).await?
            } else {
                resolve_public_addresses(host, port).await?
            };

            // reqwest 0.13 的 resolve(domain, addr) 是单地址覆盖；先钉住一个已校验
            // 地址，保证连接阶段不会重新解析。URL 不改写为 IP，因此 Host 与 SNI
            // 仍使用原主机名。
            let mut builder = Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                // 代理会使目标主机在代理侧另行解析，绕过上述 DNS 钉住。
                .no_proxy();
            if let Some(user_agent) = user_agent {
                builder = builder.user_agent(user_agent);
            }
            let client = builder
                .resolve(host, addresses[0])
                .build()
                .map_err(|e| PluginError::InternalError(format!("创建 HTTP 客户端失败: {e}")))?;

            let mut outgoing = client.request(request.method.clone(), current_url.clone());
            for (name, value) in &request.headers {
                outgoing = outgoing.header(name, value);
            }
            match &request.body {
                Some(OutboundBody::Text(body)) => outgoing = outgoing.body(body.clone()),
                Some(OutboundBody::Json(body)) => outgoing = outgoing.json(body),
                None => {}
            }

            let response = outgoing.send().await.map_err(|e| {
                if e.is_timeout() {
                    PluginError::InternalError(format!(
                        "请求超时：{}s 内未完成（可调大 web_timeout 后重试）",
                        timeout.as_secs()
                    ))
                } else if e.is_connect() {
                    PluginError::InternalError(format!(
                        "连接失败：无法连到 {current_url}（请检查地址与网络后重试）"
                    ))
                } else {
                    PluginError::InternalError(format!("请求失败: {e}"))
                }
            })?;

            let status = response.status();
            if !is_redirect_status(status) {
                return Ok(response);
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .map(|value| {
                    value.to_str().map(str::to_string).map_err(|_| {
                        PluginError::ValidationError("无效的重定向 Location".to_string())
                    })
                })
                .transpose()?;
            let Some(target) = next_redirect(
                &current_url,
                status,
                location.as_deref(),
                redirects_used,
                &mut validate_host,
            )?
            else {
                return Ok(response);
            };

            request = request.redirected(status, &current_url, &target)?;
            current_url = target;
            redirects_used += 1;
        }
    };

    tokio::time::timeout(timeout, operation)
        .await
        .map_err(|_| {
            PluginError::InternalError(format!(
                "请求超时：{}s 内未完成（含重定向链路，可调大 web_timeout 后重试）",
                timeout.as_secs()
            ))
        })?
}

/// 第三道闸：出站主机须在**当前 Agent 声明的域名白名单**里。
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

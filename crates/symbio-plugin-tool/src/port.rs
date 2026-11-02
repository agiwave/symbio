//! `port.tool` 的执行面：工具调用的提供者（D-303）。
//!
//! 环只按端口名问一笔债；线上名怎么解析（`mcp__fs__read` → 能力名）、挂死怎么判
//! （空闲而不是总时长）、结果怎么压成一段文本，都是本插件的私事。工具世界由能力表
//! 承载：本类型经 `Registry::get` + `capability_invoke` 调各工具插件的 `Capability`，
//! 自己不做执行。
//!
//! ## 载荷契约（环 → 本插件）
//!
//! `PortInput.payload` 是一个 JSON 对象：
//!
//! | 键 | 含义 |
//! |---|---|
//! | `tool_name` | 模型给的**线上名**（解析在本插件） |
//! | `args` | 调用参数（模型发的原样 JSON） |
//! | `tool_call_id` | 本次调用的 id（幂等与节点挂靠） |
//! | `result_msg_id` | 结果占位节点 id（流式增量按它广播） |
//!
//! 环侧的宿主插件（`parent`）经 `ctx.parent()` 取得：解析失败时按原样走到它，由它给出
//! 诚实的 NotFound。
//!
//! ## 产出契约（本插件 → 环）
//!
//! `PortOutcome.facts` 只有一项，键为 `result_text` / `success` / `pending`。
//! 工具失败**不**映射成 `PortOutcome.error`——失败是信息性事实（模型据此自我纠正），
//! 与「这一笔协议失败」是两件事。

use std::sync::Arc;

use symbio_plugin_api::{
    capability_invoke, capability_to_wire, events::EVENT_AUTHZ_DENIED, events::EVENT_AUTHZ_GRANTED,
    failure_kind, family, plugin_error, plugin_info, plugin_warn, ports, CapabilityRiskLevel,
    Entity, ExecAbortSignal, FactDraft, PluginError, PluginInvokeRequestExt, PluginPayload,
    PortErrorClass, PortExecContext, PortExecutor, PortInput, PortOutcome, PortProvider, Verb,
    ABORT_SIGNAL, AGENT_ID, CAPABILITY_VISITOR, EVENT_SINK, PATH, PLUGIN_ID_TOOL, RESULT_MSG_ID,
    SESSION_ID, TOOL_CALL_ID, WORKDIR,
};

/// 端口名（唯一来源：本插件）。
pub(crate) const TOOL: &str = ports::TOOL;

/// 载荷键（环与提供者之间的私有约定，随 [`PortInput`] 走）。
const KEY_TOOL_NAME: &str = "tool_name";
const KEY_ARGS: &str = "args";
const KEY_TOOL_CALL_ID: &str = "tool_call_id";
const KEY_RESULT_MSG_ID: &str = "result_msg_id";

/// 产出键（与环的通用受理入口共用同一份形状）。
const FACT_RESULT_TEXT: &str = "result_text";
const FACT_SUCCESS: &str = "success";
const FACT_PENDING: &str = "pending";

/// 授权输入键（`open-tools` 在开账时写进请求载荷，`D-306` 的「载荷装配留在行」）。
///
/// 与执行键（`tool_name` / `tool_call_id`）分开：授权只看判定输入，执行才解析线上名。
const KEY_TOOL: &str = "tool";
const KEY_RISK: &str = "risk";
const KEY_THRESHOLD: &str = "threshold";
const KEY_APPROVED: &str = "approved";
const KEY_GRANT: &str = "grant";
const KEY_DEFERRED: &str = "deferred";
const KEY_INVALID: &str = "invalid";
/// 债键字段（与环的 `request` 事实同形，授权事实按它可被行认领）。
const KEY_ID: &str = "id";
const KEY_ATTEMPT: &str = "attempt";

/// 一次授权判定（`D-306` 裁决一：由端口执行体判，不再由环的动作判）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Authz {
    /// 放行；`basis` / `grant` 只用于记账，不参与执行。
    Granted {
        risk: &'static str,
        basis: Option<&'static str>,
        grant: Option<&'static str>,
    },
    /// route-only 工具（注册表里没有可判的风险）：放行延迟给下游路由闸门。
    Deferred,
    /// 拒绝：`why` 是模型与转写看得到的原因。
    Denied { why: &'static str },
}

/// 从请求载荷判一次授权；载荷里没有 `threshold` 就是**不由本执行体判**（`None`）。
///
/// `None` 是兼容位，不是放行位：旧路径的授权事实由环自己的动作写，这里的执行体
/// 只负责执行（P1 的双路径到期点见 `P4`）。
fn authorize(payload: &serde_json::Value) -> Option<Authz> {
    let threshold = payload
        .get(KEY_THRESHOLD)
        .and_then(|v| v.as_str())
        .map(CapabilityRiskLevel::from_name)?;
    if payload
        .get(KEY_INVALID)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Some(Authz::Denied {
            why: "invalid_call",
        });
    }
    let risk = payload
        .get(KEY_RISK)
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let deferred = payload
        .get(KEY_DEFERRED)
        .and_then(|v| v.as_bool())
        .unwrap_or(risk == "unknown");
    if risk == "unknown" || deferred {
        return Some(Authz::Deferred);
    }
    // 载荷里的风险档由开账行按注册表算定；认不出就是装配漂移，不能静默认成某一档。
    let Some(risk) = CapabilityRiskLevel::parse(risk) else {
        return Some(Authz::Denied {
            why: "unknown_risk",
        });
    };
    let approved = payload
        .get(KEY_APPROVED)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let grant = payload
        .get(KEY_GRANT)
        .and_then(|v| v.as_str())
        .and_then(CapabilityRiskLevel::parse);
    match risk.approval_basis(threshold, approved, grant) {
        Some(basis) => Some(Authz::Granted {
            risk: risk.as_str(),
            basis: Some(basis.as_str()),
            grant: grant.map(CapabilityRiskLevel::as_str),
        }),
        None => Some(Authz::Denied {
            why: "approval_required",
        }),
    }
}

/// 授权结论 → 一条授权事实草稿（`authz.granted` / `authz.denied`）。
///
/// 幂等键从这一笔债的 `idempotency_key` 派生：同一笔债重放两次只落一条授权，
/// 与宿主写口的重复吸收同一条口径。
fn authz_draft(idempotency_key: &str, payload: &serde_json::Value, authz: Authz) -> FactDraft {
    let tool = payload
        .get(KEY_TOOL)
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let id = payload
        .get(KEY_ID)
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let attempt = payload
        .get(KEY_ATTEMPT)
        .and_then(|v| v.as_u64())
        .unwrap_or(1);
    let (kind, verb, payload) = match authz {
        Authz::Granted { risk, basis, grant } => (
            EVENT_AUTHZ_GRANTED,
            Verb::Asserted,
            serde_json::json!({
                "port": TOOL,
                "id": id,
                "attempt": attempt,
                "tool": tool,
                "risk": risk,
                "basis": basis,
                "grant": grant,
                "deferred": false,
            }),
        ),
        Authz::Deferred => (
            EVENT_AUTHZ_GRANTED,
            Verb::Asserted,
            serde_json::json!({
                "port": TOOL,
                "id": id,
                "attempt": attempt,
                "tool": tool,
                "risk": "unknown",
                "basis": serde_json::Value::Null,
                "grant": serde_json::Value::Null,
                "deferred": true,
            }),
        ),
        Authz::Denied { why } => (
            EVENT_AUTHZ_DENIED,
            Verb::Asserted,
            serde_json::json!({
                "port": TOOL,
                "id": id,
                "attempt": attempt,
                "tool": tool,
                "why": why,
            }),
        ),
    };
    FactDraft {
        key: format!("{idempotency_key}.authz"),
        kind: kind.to_string(),
        entity: Entity::Turn,
        verb,
        payload,
        produced_by: None,
    }
}

/// `port.tool` 的提供者：无状态前端（执行期从调用方 ctx 读能力表）。
pub(crate) struct ToolPort;

impl PortProvider for ToolPort {
    fn ports(&self) -> &'static [&'static str] {
        &[TOOL]
    }

    fn provider_id(&self) -> &str {
        PLUGIN_ID_TOOL
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(ToolPort))
    }
}

/// 执行一次工具调用：解析线上名、经能力表调用、空闲超时看护、按结果契约解码。
#[async_trait::async_trait]
impl PortExecutor for ToolPort {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let payload = input.payload;
        // 授权判定先行（`D-306` 裁决一）：载荷带判定输入时由本执行体判，结论经写口落事实；
        // 拒绝即判死这一笔，不再进入能力调用。旧路径的载荷不带 `threshold`，这里保持旁路。
        if let Some(authz) = authorize(&payload) {
            exec.emit
                .emit(vec![authz_draft(&exec.idempotency_key, &payload, authz)])
                .await;
            if let Authz::Denied { why } = authz {
                return PortOutcome::failed(
                    PortErrorClass::NonRetryable,
                    format!("Error: authorization denied ({why})"),
                );
            }
        }
        let tool_name = match payload.get(KEY_TOOL_NAME).and_then(|v| v.as_str()) {
            Some(name) => name.to_string(),
            None => return error_outcome("Error: Missing tool_name"),
        };
        let args = payload
            .get(KEY_ARGS)
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let tool_call_id = payload
            .get(KEY_TOOL_CALL_ID)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let result_msg_id = payload
            .get(KEY_RESULT_MSG_ID)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let Some(ctx) = exec.ctx.clone() else {
            return error_outcome("Error: No request context");
        };
        let started_at = std::time::Instant::now();
        plugin_info!(
            "tool",
            "[Tool] 请求发起: {} args={}",
            tool_name,
            args_summary(&args, 200)
        );

        let parent = match ctx.parent().and_then(|w| w.upgrade()) {
            Some(p) => p,
            None => return error_outcome("Error: No parent plugin"),
        };

        // 入方向：模型给的**线上名** → 能力名。
        //
        // 判据是「注册表里谁映射到这个名字」，**不是**「把名字反演回去」：
        // `mcp__fs__read` 既可能是 `mcp.fs.read` 的线上形态，也可能本身就是
        // `mcp__fs__read`——字符串分不出这两种，集合可以。
        let tool_visitor = ctx.get(CAPABILITY_VISITOR);
        let known_names: Vec<String> = match tool_visitor.as_ref() {
            Some(visitor) => visitor
                .list(family::TOOL)
                .await
                .iter()
                .map(|row| row.name().to_string())
                .collect(),
            None => Vec::new(),
        };
        let resolved_name = resolve(&tool_name, known_names.iter().map(String::as_str));
        // 解析失败**不猜**：按原样交给路由，由它给出诚实的 NotFound。
        // 反演猜错会调起**另一个工具**，那比报错坏得多。
        let invoke_name = resolved_name.clone().unwrap_or_else(|| tool_name.clone());
        if resolved_name.is_none() && tool_visitor.is_some() {
            plugin_warn!(
                "tool",
                "[Tool] 名字 {} 不在能力注册表里（线上名解析失败），按原样路由",
                tool_name
            );
        }

        let session_id = ctx.get(SESSION_ID).unwrap_or_default();
        let agent_id = ctx.get(AGENT_ID).unwrap_or_default();
        let workdir = ctx.get(WORKDIR).unwrap_or_default();

        let tool_ctx = ctx.fork();
        tool_ctx.set(WORKDIR, workdir);
        tool_ctx.set(AGENT_ID, agent_id);
        tool_ctx.set(SESSION_ID, session_id);
        tool_ctx.set(TOOL_CALL_ID, tool_call_id.clone());
        // 流式工具（如 shell）据此 id 广播增量帧：与 result_msg_id 占位节点同 id。
        tool_ctx.set(RESULT_MSG_ID, result_msg_id.clone());
        // 执行期双原语注入：工具「把事件写到哪」与「如何感知中止」的唯一来源。
        tool_ctx.set(EVENT_SINK, exec.sink.clone());
        tool_ctx.set(ABORT_SIGNAL, exec.abort.clone());

        // 工具调用的发起（三条形态：能力命中 / 未命中回落 route / 无能力表直接 route）。
        // 包成 `Box<dyn Future>` 是为了下面那个「轮询 + 空闲判定」的循环能重复借用。
        let mut route_fut: std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<PluginPayload, PluginError>> + Send>,
        > = if let Some(tool_visitor) = tool_visitor {
            if resolved_name.is_some() {
                plugin_info!(
                    "tool",
                    "[Tool] 能力表命中: {} (能力名 {})",
                    tool_name,
                    invoke_name
                );
                let _ = tool_ctx.set_payload(args.clone());
                let visitor = tool_visitor.clone();
                let tool_ctx2 = tool_ctx.clone();
                let canonical = invoke_name.clone();
                Box::pin(async move {
                    match visitor.get(&canonical).await.and_then(|r| r.cap().cloned()) {
                        Some(cap) => capability_invoke(cap.as_ref(), tool_ctx2).await,
                        None => Err(PluginError::NotFound(format!(
                            "Tool not found: {canonical}"
                        ))),
                    }
                })
            } else {
                plugin_info!("tool", "[Tool] 能力表未命中: {}，回落到宿主路由", tool_name);
                tool_ctx.set(PATH, invoke_name.clone());
                let _ = tool_ctx.set_payload(args.clone());
                let p2 = parent.clone();
                let tool_ctx2 = tool_ctx.clone();
                Box::pin(async move { p2.route(tool_ctx2).await })
            }
        } else {
            tool_ctx.set(PATH, invoke_name.clone());
            let _ = tool_ctx.set_payload(args.clone());
            let p2 = parent.clone();
            let tool_ctx2 = tool_ctx.clone();
            Box::pin(async move { p2.route(tool_ctx2).await })
        };

        // 工具执行兜底超时：**空闲**上限（不是总时长上限）——「有进展就不算挂死」。
        //
        // 进展的判据是**出口的发射计数**：工具唯一的出方向动作就是 `emit`，因此
        // 「计数在涨」等价于「它还在干活」。单次 await、从不发事件的工具读数恒为 0，
        // 于是退化为「总时长上限」——与历史口径一致。
        const TOOL_EXEC_IDLE_TIMEOUT_SECS: u64 = 600; // 10 分钟无任何进展
        let progress = exec.sink.progress();
        let mut seen_emits = progress.emitted();

        // 三个出口：工具返回 / 用户中止 / 空闲超时。优先级用 `biased` 固定：工具已返回
        // ⇒ 立刻采纳（哪怕同时到点）；其次是中止；最后才是空闲判定。
        let route_result = loop {
            tokio::select! {
                biased;
                r = &mut route_fut => break r,
                _ = wait_tool_abort(&exec.abort) => {
                    plugin_warn!(
                        "tool",
                        "[Tool] 执行期间被用户中止: {} (耗时 {}ms, call_id={})",
                        tool_name,
                        started_at.elapsed().as_millis(),
                        tool_call_id
                    );
                    return error_outcome(format!(
                        "Error: 工具 {} 被用户中止，未返回结果",
                        tool_name
                    ));
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(TOOL_EXEC_IDLE_TIMEOUT_SECS)) => {
                    let now = progress.emitted();
                    if now != seen_emits {
                        seen_emits = now;
                        continue;
                    }
                    plugin_error!(
                        "tool",
                        format!(
                            "[Tool] 执行空闲超时 ({}s 无任何进展): {}，已中断。call_id={}",
                            TOOL_EXEC_IDLE_TIMEOUT_SECS, tool_name, tool_call_id
                        )
                    );
                    return error_outcome(format!(
                        "Error: 工具 {} 已 {} 秒无任何进展，已强制中断（疑似挂死）",
                        tool_name, TOOL_EXEC_IDLE_TIMEOUT_SECS
                    ));
                }
            }
        };

        match route_result {
            Ok(resp) => match resp {
                PluginPayload::Data(_) => {
                    let data = match resp.get::<serde_json::Value>() {
                        Ok(d) => d,
                        Err(_) => return error_outcome("Error: Failed to deserialize payload"),
                    };
                    let pending = pending_prompt_from(&data);
                    let res = extract_result(&data);
                    plugin_info!(
                        "tool",
                        "[Tool] 正常结束: {} (耗时 {}ms, 结果长度 {}, pending={})",
                        tool_name,
                        started_at.elapsed().as_millis(),
                        res.len(),
                        pending
                            .as_ref()
                            .and_then(|p| p.get("failure_kind"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("-")
                    );
                    outcome(res, true, pending)
                }
                _ => error_outcome("Error: Unexpected payload type"),
            },
            Err(e) => {
                plugin_error!(
                    "tool",
                    format!(
                        "[Tool] ROUTE Error: {} (耗时 {}ms)",
                        e,
                        started_at.elapsed().as_millis()
                    )
                );
                error_outcome(format!("Error: {e}"))
            }
        }
    }
}

/// 一次结果是**协议上的成功**（工具失败也是信息性事实，见模块文档）。
fn outcome(result_text: String, success: bool, pending: Option<serde_json::Value>) -> PortOutcome {
    PortOutcome {
        facts: vec![serde_json::json!({
            FACT_RESULT_TEXT: result_text,
            FACT_SUCCESS: success,
            FACT_PENDING: pending,
        })],
        error: None,
    }
}

fn error_outcome(result_text: impl Into<String>) -> PortOutcome {
    outcome(result_text.into(), false, None)
}

/// 线上名 → 能力名（精确命中优先，其后按投影函数比较）。
fn resolve<'a>(wire: &str, known: impl Iterator<Item = &'a str>) -> Option<String> {
    let names: Vec<&str> = known.collect();
    if let Some(exact) = names.iter().find(|name| **name == wire) {
        return Some((*exact).to_string());
    }
    names
        .iter()
        .find(|name| capability_to_wire(name) == wire)
        .map(|name| (*name).to_string())
}

/// 参数摘要：截断到 `max_bytes`，落在字符边界上（多字节字符安全）。
fn args_summary(args: &serde_json::Value, max_bytes: usize) -> String {
    let s = args.to_string();
    if s.len() <= max_bytes {
        s
    } else {
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…(len={})", &s[..end], s.len())
    }
}

/// 工具结果里「给模型看的正文」字段名（[`extract_result`] 的第 1 条判据）。
const RESULT_CONTENT: &str = "content";
/// 命令类工具的结果正文字段名（`shell` 等；[`extract_result`] 的第 2 条判据）。
const RESULT_OUTPUT: &str = "output";

/// 从工具返回的 JSON 数据中提取可读的文本结果（先命中者胜）。
fn extract_result(data: &serde_json::Value) -> String {
    if let Some(content) = data.get(RESULT_CONTENT).and_then(|v| v.as_str()) {
        return content.to_string();
    }
    if let Some(output) = data.get(RESULT_OUTPUT).and_then(|v| v.as_str()) {
        return output.to_string();
    }
    if let Some(success) = data.get("success").and_then(|s| s.as_bool()) {
        if success {
            data.to_string()
        } else {
            format!(
                "Error: {}",
                data.get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("unknown error")
            )
        }
    } else {
        data.as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| data.to_string())
    }
}

/// 等待「用户中止」信号（工具执行期间）。
async fn wait_tool_abort(abort: &ExecAbortSignal) {
    abort.cancelled().await;
}

/// 从工具返回的数据里读出「需要用户动作」的意图（`failure_kind` 约定字段）。
fn pending_prompt_from(data: &serde_json::Value) -> Option<serde_json::Value> {
    let kind = data.get("failure_kind").and_then(|v| v.as_str())?;
    if !failure_kind::is_pending(kind) {
        return None;
    }
    Some(serde_json::json!({
        "text": data.get("content").and_then(|v| v.as_str()).unwrap_or("需要用户操作"),
        "prompt": data.get("prompt").cloned().unwrap_or(serde_json::Value::Null),
        "failure_kind": kind,
    }))
}

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;

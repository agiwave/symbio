//! 审批闸门——所有能力进入 ToolManager 的**唯一判据**。
//!
//! ## 为什么住在这里
//!
//! 闸门判的是「工具自声明的风险 vs 会话阈值 / 工具授权上限」，两个量都**在能力自己
//! 身上与请求信封上**：[`Capability::risk_for`] 由工具回答，`ctx[RISK_LEVEL]` /
//! `ctx[APPROVAL_GRANTS]` / `ctx[MODE]` 由信封给出。判据是纯比较
//! （[`CapabilityRiskLevel::approval_basis`]），不吃任何插件的私有状态，因此**能**也
//! **该**住到收集器这一层——它是所有能力注册的收口点，把闸门搬到这里，`vdfs` / `web` /
//! `mcp` / `skill` 的裸注册也一并过闸。此前闸门长在 `local` 插件自己的注册循环上
//! （`SecureToolWrapper`），结果就是「风险是宣传语、执行无人读」：`vdfs_delete` 声明
//! `risk=High`，实测直接删（见 处理过程）。
//!
//! ## 两处入口，判定一份
//!
//! 分发有两条真实路径：按名执行（`Registry::get` + `capability_invoke`，注册表命中）与
//! `Plugin::route`
//! 兜底（未命中）。前者经过本模块的 [`ApprovalGate`] 装饰器，后者经
//! [`approval_gate`] 这个自由函数——**判定只有 [`approval_gate`] 一份**，
//! 装饰器只是它的注册面投影。

use crate::symbio_core::{
    ApprovalGrants, Capability, CapabilityMeta, CapabilityRiskLevel, ExecEnv, PluginError,
    PluginInvokeRequest, PluginInvokeRequestExt, APPROVAL_GRANTS, MODE, RISK_LEVEL,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// 从 `ctx[RISK_LEVEL]` 读 per-session 阈值；缺席 = `Medium`（新会话的安全默认）。
///
/// 字面量→档位的映射不在这里：[`CapabilityRiskLevel::from_name`] 是唯一解析点
/// （参数级风险判定 `local::shell` 问同一个方法）。
pub(crate) fn threshold_from_ctx(ctx: &Arc<dyn PluginInvokeRequest>) -> CapabilityRiskLevel {
    ctx.get(RISK_LEVEL)
        .map(|s| CapabilityRiskLevel::from_name(&s))
        .unwrap_or_default()
}

/// 从 `ctx[APPROVAL_GRANTS]` 读跨会话的永久放行清单；缺席 = 空清单。
///
/// 与会话阈值同一份配置解析点落进 ctx（D-170 决策 2）：清单与阈值是同一件策略的
/// 两级（按工具 / 按会话），各从一处读才不会出现「阈值取这边、清单取那边」。
pub(crate) fn grants_from_ctx(ctx: &Arc<dyn PluginInvokeRequest>) -> ApprovalGrants {
    ctx.get(APPROVAL_GRANTS)
        .map(|s| ApprovalGrants::parse(&s))
        .unwrap_or_default()
}

// 审批判定的**唯一判据**住在 `CapabilityRiskLevel::approval_basis`：返回放行依据
// （本次批准 / 会话阈值 / 工具上限），`None` 才需要审批。
//
// 为什么比较不写在这里：授权级联的记录方（`tool_dispatch` 的工具端口）与真正
// 执行闸门（`approval_gate`）必须用同一份判据；两处各写一遍「risk > threshold」，
// 改一处漏一处时两边都合法、没有任何东西会红。**判定位置不变**：执行闸门仍是
// 唯一放行点，记录方只决定「这笔账是否已经可以派发」。

/// 审批闸门的**唯一**判定：需要审批 ⇒ `Ok(Some(载荷))`；放行 ⇒ `Ok(None)`。
///
/// 三条放行条件，任一成立即过（[`CapabilityRiskLevel::approval_basis`]）：
/// - 参数带 `approved: true`（用户已在审批卡上批过）；
/// - `risk_for(args) ≤ threshold`（工具本次不比会话更危险）；
/// - `risk_for(args) ≤ grant`（该工具有永久放行上限，且本次风险不超过它）。
///
/// 需要审批时，`mode=auto` ⇒ `PERMISSION_DENIED` 友好错误（无人值守，不产卡），
/// 其余（含 `interactive` 与未设置）⇒ `NEEDS_APPROVAL` 载荷交编排层建 user_prompt 节点。
pub fn approval_gate(
    ctx: &Arc<dyn PluginInvokeRequest>,
    tool: &dyn Capability,
    args: &Value,
) -> Result<Option<Value>, PluginError> {
    // 风险由**工具自己**回答：静态档来自 `CapabilityMeta.risk`，随参数变化档（shell）
    // 覆写它。闸门不认工具名——新增/改名工具不必回来改这里。
    let meta = tool.meta();
    let tool_risk = tool.risk_for(args);
    let is_approved = args
        .get("approved")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let threshold = threshold_from_ctx(ctx);
    // 清单按**能力名**（canonical name）记：`meta.name` 就是注册名，线上名只是它
    // 的协议投影（D-170 决策 4）。
    let grant = grants_from_ctx(ctx).ceiling(&meta.name);
    let needs = tool_risk
        .approval_basis(threshold, is_approved, grant)
        .is_none();

    if !needs {
        return Ok(None);
    }

    let mode = ctx.get(MODE).unwrap_or_default();
    Ok(Some(confirm_prompt_payload(
        &meta.name,
        &meta.description,
        args,
        tool_risk,
        &mode,
    )))
}

/// 构造 confirm 类型 prompt 的**返回值**（不构造节点、不发事件）。
///
/// 职责边界：工具只回答「需要用户确认什么」——`prompt` 载荷 + `failure_kind`。
/// **user_prompt 节点由编排层构造**（`session/tools/tool_executor.rs`）：它才拥有
/// `result_msg_id`（节点身份）与父 ToolCall 的终态。
fn confirm_prompt_payload(
    tool_name: &str,
    tool_description: &str,
    args: &Value,
    risk_level: CapabilityRiskLevel,
    mode: &str,
) -> Value {
    let risk_str = risk_level.as_str();
    if mode == "auto" {
        return json!({
            "error": format!(
                "权限不足：工具 {} 需要用户审批（风险等级 {}），但当前为自动模式，无人可授权。请勿反复重试——请改用手动方式完成，或提示用户切换到交互模式以授权后重试。",
                tool_name, risk_str
            ),
            "success": false,
            "failure_kind": crate::symbio_core::failure_kind::PERMISSION_DENIED,
        });
    }
    json!({
        "content": format!("需要确认：{tool_description}"),
        "success": false,
        "failure_kind": crate::symbio_core::failure_kind::NEEDS_APPROVAL,
        "prompt": {
            "kind": "confirm",
            "tool_name": tool_name,
            "args": args.clone(),
            "risk_level": risk_str,
            "description": tool_description,
        },
    })
}

/// 注册面投影：`DefaultToolVisitor::put` 用它把每条能力裹一层闸门。
///
/// `meta` / `name` / `risk_for` 原样透传——装饰器**不重判风险**，否则「谁的风险」
/// 就成了包装层的事：闸门拿到 wrapper 的默认档，真实档（shell 的按命令分级）就丢了。
pub struct ApprovalGate {
    inner: Arc<dyn Capability>,
}

impl ApprovalGate {
    pub fn new(inner: Arc<dyn Capability>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl Capability for ApprovalGate {
    fn meta(&self) -> CapabilityMeta {
        self.inner.meta()
    }

    fn name(&self) -> String {
        self.inner.name()
    }

    fn risk_for(&self, args: &Value) -> CapabilityRiskLevel {
        self.inner.risk_for(args)
    }

    async fn execute(
        &self,
        args: Value,
        env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        if let Some(payload) = approval_gate(&ctx, &*self.inner, &args)? {
            return Ok(payload);
        }
        // 装饰器只加一道审批闸门，执行期环境与信封原样透传。
        self.inner.execute(args, env, ctx).await
    }
}

#[cfg(test)]
#[path = "approval.test.rs"]
mod tests;

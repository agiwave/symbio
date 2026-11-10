//! 审批闸门——所有工具调用进入执行的**唯一判据**，住在端口执行体的第一段。
//!
//! ## 为什么住在这里
//!
//! 闸门判的是「工具自声明的风险 vs 会话阈值 / 工具授权上限」，两个量都**在工具自己
//! 身上与请求信封上**：风险函数由工具回答（`Capability::risk_for`，`D-440` ③ 收口前
//! 的过渡形状），`ctx[RISK_LEVEL]` / `ctx[APPROVAL_GRANTS]` / `ctx[MODE]` 由信封给出。
//! 判据是纯比较（[`CapabilityRiskLevel::approval_basis`]），不吃任何插件的私有状态，
//! 因此**能**也**该**住到收集器这一层——它是所有能力注册的收口点，把闸门搬到这里，
//! `vdfs` / `web` / `mcp` / `skill` 的裸注册也一并过闸。此前闸门长在 `local` 插件自己
//! 的注册循环上（`SecureToolWrapper`），结果就是「风险是宣传语、执行无人读」：
//! `vdfs_delete` 声明 `risk=High`，实测直接删。
//!
//! ## 判定与执行同体（`D-385` / `D-440` ③-A）
//!
//! 闸门不再是一个裹在能力外面的装饰器，而是**端口执行体的第一段**：[`GatedExecutor`]
//! 先判 [`approval_gate`]，拦下就交回停等载荷（普通结果形态，派发侧的 pending 机制
//! 照常工作），放行才拆信封落到内层。判定只有 [`approval_gate`] 一份——装饰器、
//! 自由函数、端口桥三层合一后，再没有第二处「值得复制的判定」。
//!
//! `D-440` ③-B（工具直接 `impl PortExecutor`）之后，风险函数与内层执行体都由工具
//! 自己给出，本模块只保留判定与执行体形状。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use symbio_kernel::capability::{Capability, CapabilityMeta, CapabilityRiskLevel};
use symbio_kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};
use symbio_plugin_api::{
    capability_invoke, AuthzVerdict, PortErrorClass, PortExecContext, PortExecutor, PortInput,
    PortOutcome, RESULT_KEY,
};
use symbio_sdk::capability::ApprovalGrants;
use symbio_sdk::keys::{APPROVAL_GRANTS, MODE, RISK_LEVEL};

/// 风险回答函数：给定本次调用参数，回答「这一笔有多危险」。
///
/// 默认实现 = `CapabilityMeta::effective_risk`（静态档）；**风险随参数变化**的工具
/// （shell 是唯一已知的例子）给出自己的覆写——`rm -rf ./build` 与 `git status`
/// 同属一个工具，风险却差两档。
///
/// ## 为什么不写成「闸门按工具名特判」
///
/// 闸门里出现 `if name == "cmd"` 这类判定，等于把「有哪些工具」这份清单复制到审批层：
/// 工具改名/新增/改名都要回来改闸门，而漏改的表现是**静默放行**（新工具落到默认档，
/// 免审批）。让工具自己回答「我这次有多危险」，闸门只负责拿这个数去比阈值——
/// 判定与清单就都留在工具这一侧了。
pub type RiskOf = Arc<dyn Fn(&Value) -> CapabilityRiskLevel + Send + Sync>;

/// 从 `ctx[RISK_LEVEL]` 读 per-session 阈值；缺席 = `Medium`（新会话的安全默认）。
///
/// 字面量→档位的映射不在这里：[`CapabilityRiskLevel::from_name`] 是唯一解析点
/// （参数级风险判定 `local::shell` 问同一个方法）。
pub fn threshold_from_ctx(ctx: &Arc<dyn PluginInvokeRequest>) -> CapabilityRiskLevel {
    ctx.get(RISK_LEVEL)
        .map(|s| CapabilityRiskLevel::from_name(&s))
        .unwrap_or_default()
}

/// 从 `ctx[APPROVAL_GRANTS]` 读跨会话的永久放行清单；缺席 = 空清单。
///
/// 与会话阈值同一份配置解析点落进 ctx（D-170 决策 2）：清单与阈值是同一件策略的
/// 两级（按工具 / 按会话），各从一处读才不会出现「阈值取这边、清单取那边」。
pub fn grants_from_ctx(ctx: &Arc<dyn PluginInvokeRequest>) -> ApprovalGrants {
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

/// 审批闸门的**唯一**判定：需要审批 ⇒ `Some(停等载荷)`；放行 ⇒ `None`。
///
/// 三条放行条件，任一成立即过（[`CapabilityRiskLevel::approval_basis`]）：
/// - 参数带 `approved: true`（用户已在审批卡上批过）；
/// - `risk(args) ≤ threshold`（工具本次不比会话更危险）；
/// - `risk(args) ≤ grant`（该工具有永久放行上限，且本次风险不超过它）。
///
/// 需要审批时，`mode=auto` ⇒ `PERMISSION_DENIED` 友好错误（无人值守，不产卡），
/// 其余（含 `interactive` 与未设置）⇒ `NEEDS_APPROVAL` 载荷交编排层建 user_prompt 节点。
pub fn approval_gate(
    ctx: &Arc<dyn PluginInvokeRequest>,
    meta: &CapabilityMeta,
    risk: &dyn Fn(&Value) -> CapabilityRiskLevel,
    args: &Value,
) -> Option<Value> {
    // 风险由**工具自己**回答：静态档来自 `CapabilityMeta.risk`，随参数变化档（shell）
    // 覆写它。闸门不认工具名——新增/改名工具不必回来改这里。
    let tool_risk = risk(args);
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
        return None;
    }

    let mode = ctx.get(MODE).unwrap_or_default();
    Some(confirm_prompt_payload(
        &meta.name,
        &meta.description,
        args,
        tool_risk,
        &mode,
    ))
}

/// 构造 confirm 类型 prompt 的**返回值**（不构造节点、不发事件）。
///
/// 职责边界：工具只回答「需要用户确认什么」——`prompt` 载荷 + `failure_kind`。
/// **user_prompt 节点由编排层构造**（`session/tools/tool_executor.rs`）：它拥有
/// 结果节点 id（由调用 id 派生）与父 ToolCall 的终态。
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
            "failure_kind": symbio_kernel::capability::failure_kind::PERMISSION_DENIED,
        });
    }
    json!({
        "content": format!("需要确认：{tool_description}"),
        "success": false,
        "failure_kind": symbio_kernel::capability::failure_kind::NEEDS_APPROVAL,
        "prompt": {
            "kind": "confirm",
            "tool_name": tool_name,
            "args": args.clone(),
            "risk_level": risk_str,
            "description": tool_description,
        },
    })
}

/// 端口执行体形态的审批闸门（`D-440` ③-A）：**判定是执行的第一段**。
///
/// `meta` 与 `risk` 在构造时从能力取好（注册收口点一次性快照），`cap` 是被裹的
/// 内层能力。`execute` 先判 [`approval_gate`]：拦下 ⇒ 停等载荷按 [`RESULT_KEY`]
/// 交回（普通结果形态，派发侧 pending 机制不变，`authz` 维持放行——停等不是
/// 拒绝，环不落它交来的事实之外的东西）；放行 ⇒ 拆信封执行，出口同样按
/// [`RESULT_KEY`] 装载。
pub struct GatedExecutor {
    meta: CapabilityMeta,
    risk: RiskOf,
    cap: Arc<dyn Capability>,
}

impl GatedExecutor {
    /// 由一条能力造执行体：`meta` 是入表时的自述快照，`risk` 是工具的风险回答
    /// （动态覆写在闭包里原样生效），`cap` 是内层执行对象。
    pub fn new(meta: CapabilityMeta, risk: RiskOf, cap: Arc<dyn Capability>) -> Self {
        Self { meta, risk, cap }
    }
}

#[async_trait]
impl PortExecutor for GatedExecutor {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(request) = exec.ctx else {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                "工具端口执行体没有调用方上下文，无法拆信封",
            );
        };
        if let Some(payload) =
            approval_gate(&request, &self.meta, self.risk.as_ref(), &input.payload)
        {
            // 闸门拦下：停等载荷是普通结果形态（`failure_kind` 在载荷里），
            // `authz` 保持放行——这不是授权拒绝，是「等用户点头」。
            return PortOutcome {
                authz: AuthzVerdict::Authorized,
                facts: vec![json!({ RESULT_KEY: payload })],
                error: None,
                reply: Default::default(),
            };
        }
        // 放行：信封 ↔ 结果的换算收口在 `capability_invoke`，本处只喂参数。
        let _ = request.set_payload(input.payload);
        match capability_invoke(self.cap.as_ref(), request).await {
            Ok(payload) => {
                let value = payload.get::<Value>().unwrap_or(Value::Null);
                PortOutcome {
                    authz: AuthzVerdict::Authorized,
                    facts: vec![json!({ RESULT_KEY: value })],
                    error: None,
                    reply: Default::default(),
                }
            }
            Err(e) => PortOutcome::failed(PortErrorClass::NonRetryable, e.to_string()),
        }
    }
}

#[cfg(test)]
#[path = "approval.test.rs"]
mod tests;

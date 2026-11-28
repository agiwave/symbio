//! 审批闸门——所有工具调用进入执行的**唯一判据**，住在端口执行体的第一段。
//!
//! 工具提供意图事实、动态风险与审批摘要；sandbox 通过 [`ApprovalPolicy`] 决定放行、
//! 拒绝、拦截或需审批。本模块只验证并原子消费可信 [`ApprovalGrant`]，把策略结果转换为
//! 既有 prompt / 执行信封，再调用内层执行体。策略缺席时失败关闭。
//!
//! [`GatedExecutor`] 与端口执行体第一段合一，`vdfs` / `web` / `mcp` / `skill` 等所有
//! 注册入口都经 `register_tool` 绑定同一闸门。模型参数中的 `approved` 永远不是授权；
//! 只有凭据消费成功后，平台才向内层执行体发布可信标记。

use std::sync::Arc;

use crate::kernel::capability::{CapabilityMeta, CapabilityRiskLevel};
use crate::kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};
use crate::sdk::capability::port::{
    AuthzVerdict, PortErrorClass, PortExecContext, PortExecutor, PortInput, PortOutcome, RESULT_KEY,
};
use crate::sdk::capability::{ApprovalGrant, ApprovalGrants, PolicyBinding};
use crate::sdk::collectors::approval_policy::{approval_policy_of, ApprovalEvaluation};
use crate::sdk::keys::{APPROVAL_GRANTS, MODE, RISK_LEVEL, SANDBOX_ID, SESSION_ID, TOOL_CALL_ID};
use crate::sdk::sandbox::{sandbox_of, SandboxFact};
use async_trait::async_trait;
use serde_json::{json, Value};

/// collectors 在一次性授权凭据消费成功后写给内层执行体的可信标记。
///
/// 这不是模型参数，也不是可持久化授权：只有本闸门验证并消费 [`ApprovalGrant`] 后才能写
/// `"true"`。shell 等内层执行体只认这个 ctx 标记，绝不读 `args.approved`。
const TRUSTED_APPROVED: &str = "approved";

/// 工具自声明的审批摘要；属于注册机制，不进入内核能力数据面。
#[derive(Debug, Clone)]
pub struct ApprovalSummary(ApprovalSummarySpec);

#[derive(Debug, Clone)]
struct ApprovalSummarySpec {
    action: &'static str,
    effect: ApprovalEffect,
    target_arg: Option<&'static str>,
    target_label: Option<&'static str>,
}

impl ApprovalSummary {
    pub const fn targeted(
        action: &'static str,
        effect: ApprovalEffect,
        arg: &'static str,
        label: &'static str,
    ) -> Self {
        Self(ApprovalSummarySpec {
            action,
            effect,
            target_arg: Some(arg),
            target_label: Some(label),
        })
    }

    pub fn summary(&self, args: &Value) -> Value {
        let spec = &self.0;
        let mut summary = json!({
            "action": spec.action,
            "effect": spec.effect.as_str(),
        });
        if let Some(arg) = spec.target_arg {
            summary["target_label"] = json!(spec.target_label);
            summary["target"] = args.get(arg).cloned().unwrap_or(Value::Null);
        }
        summary
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalEffect(&'static str);

impl ApprovalEffect {
    pub const FILE_MODIFY: Self = Self("file_modify");
    pub const FILE_DELETE: Self = Self("file_delete");
    pub const NETWORK_SEND: Self = Self("network_send");
    pub const SYSTEM_EXECUTE: Self = Self("system_execute");
    pub const OTHER: Self = Self("other");

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// 工具自声明的**意图事实提取器**：把本次调用参数翻译成 `[SandboxFact]`。
///
/// 这是工具侧的能力（只有工具知道 `command` / `path` 这些参数怎么解释），闸门与沙箱
/// 都不认识工具名：闸门只拿它产出的开放 kind 事实去查沙箱，沙箱按 kind 配规则。
/// `ctx` 一并给出，供事实依赖会话级环境（如工作目录）的工具使用。
pub type FactsOf =
    Arc<dyn Fn(&Value, &Arc<dyn PluginInvokeRequest>) -> Vec<SandboxFact> + Send + Sync>;

pub type AsyncFactsOf = Arc<
    dyn Fn(
            &Value,
            &Arc<dyn PluginInvokeRequest>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Vec<SandboxFact>, String>> + Send>,
        > + Send
        + Sync,
>;

pub(crate) enum FactSource {
    Sync(FactsOf),
    Async(AsyncFactsOf),
}

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
/// 与会话阈值同一份配置解析点落进 ctx（ 决策 2）：清单与阈值是同一件策略的
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
/// - 本次调用绑定的 [`ApprovalGrant`] 已原子消费；
/// - `risk(args) ≤ threshold`（工具本次不比会话更危险）；
/// - `risk(args) ≤ grant`（该工具有永久放行上限，且本次风险不超过它）。
///
/// 模型参数 `args.approved` 永远不是授权。沙箱存在时只调用一次
/// [`crate::sdk::sandbox::SandboxFace::decide`]，
/// 许可、拦截、风险下限、审批模式与阈值都取自同一份快照。
///
/// 需要审批时，`mode=auto` ⇒ `PERMISSION_DENIED` 友好错误（无人值守，不产卡），
/// 其余（含 `interactive` 与未设置）⇒ `NEEDS_APPROVAL` 载荷交编排层建 user_prompt 节点。
pub fn approval_gate(
    ctx: &Arc<dyn PluginInvokeRequest>,
    meta: &CapabilityMeta,
    risk: &dyn Fn(&Value) -> CapabilityRiskLevel,
    args: &Value,
) -> Option<Value> {
    approval_gate_inner(ctx, meta, risk, args, None, None)
}

/// 带**工具声明事实**的闸门入口（ 能力约束接入）。
///
/// `facts_of` 由工具在注册时交出（只有工具知道自己的参数语义）；闸门把它产出的
/// 事实交给沙箱做两件事：命中 deny 规则直接拒绝；命中带风险档的 allow 规则则
/// 把该档作为本次工具风险参与阈值比较。沙箱缺席 / 工具未声明事实 ⇒ 完全回落原判定。
pub fn approval_gate_with_facts(
    ctx: &Arc<dyn PluginInvokeRequest>,
    meta: &CapabilityMeta,
    risk: &dyn Fn(&Value) -> CapabilityRiskLevel,
    facts_of: Option<&FactsOf>,
    args: &Value,
    approval: Option<&ApprovalSummary>,
) -> Option<Value> {
    let facts = facts_of.map(|f| f(args, ctx)).unwrap_or_default();
    approval_gate_inner(ctx, meta, risk, args, Some(&facts), approval)
}

fn approval_gate_inner(
    ctx: &Arc<dyn PluginInvokeRequest>,
    meta: &CapabilityMeta,
    risk: &dyn Fn(&Value) -> CapabilityRiskLevel,
    args: &Value,
    facts: Option<&[SandboxFact]>,
    approval: Option<&ApprovalSummary>,
) -> Option<Value> {
    // 风险由**工具自己**回答：静态档来自 `CapabilityMeta.risk`，随参数变化档（shell）
    // 覆写它。闸门不认工具名——新增/改名工具不必回来改这里。
    let mut tool_risk = risk(args);
    let mut threshold = threshold_from_ctx(ctx);
    let mut mode = ctx.get(MODE).unwrap_or_default();
    let mut profile_version = 0;

    // 沙箱裁决是一次原子快照读取：不再分别问 permits / risk_of，也不把装配期写进
    // ctx 的旧 MODE / RISK_LEVEL 与新裁决混用。规则风险只能抬高工具风险，不能调低。
    if let Some(face) = sandbox_of(ctx.as_ref()) {
        let decision = face.decide(facts.unwrap_or_default(), &meta.name, args);
        if let Some(intercept) = decision.intercept {
            return Some(intercept);
        }
        if !decision.permitted {
            let reason = decision
                .reason
                .unwrap_or_else(|| "sandbox denied the call".to_string());
            return Some(json!({
                "error": format!("沙箱未放行工具 {} 的本次调用：{reason}", meta.name),
                "success": false,
                "failure_kind": crate::kernel::capability::failure_kind::PERMISSION_DENIED,
            }));
        }
        if let Some(level) = decision
            .risk
            .as_deref()
            .and_then(CapabilityRiskLevel::parse)
        {
            tool_risk = tool_risk.max(level);
        }
        if let Some(level) = decision.risk_threshold.as_deref() {
            threshold = CapabilityRiskLevel::from_name(level);
        }
        if let Some(value) = decision.approval_mode {
            mode = value;
        }
        profile_version = decision.profile_version;
    } else if ctx.get(SANDBOX_ID).is_some() {
        return Some(json!({
            "error": format!("沙箱未放行工具 {} 的本次调用：已选择沙箱但沙箱句柄不可用", meta.name),
            "success": false,
            "failure_kind": crate::kernel::capability::failure_kind::PERMISSION_DENIED,
        }));
    }

    // 一次性凭据只在此 mandatory pre-execution gate 消费一次。call/tool/args/session/
    // profile version/expiry 全部由 SDK 原子校验；顶层 args.approved 在绑定比较中被忽略。
    let call_id = ctx.get(TOOL_CALL_ID).unwrap_or_default();
    let session_id = ctx.get(SESSION_ID).filter(|value| !value.trim().is_empty());
    let policy_binding = (profile_version > 0).then(|| {
        PolicyBinding::new(
            ctx.get(SANDBOX_ID).unwrap_or_else(|| "legacy".to_string()),
            profile_version,
        )
    });
    let is_approved = ApprovalGrant::take(
        ctx.as_ref(),
        &call_id,
        &meta.name,
        args,
        session_id.as_deref(),
        policy_binding.as_ref(),
    );
    // 先覆盖掉任何继承来的旧标记，再只在本轮 grant 消费成功时发布 "true"。
    // 这样内层执行体不会把上一次调用（或调用方元数据）的字符串当成本轮授权。
    ctx.set_raw(TRUSTED_APPROVED, Arc::new(is_approved.to_string()));

    // 清单按**能力名**（canonical name）记：`meta.name` 就是注册名，线上名只是它
    // 的协议投影（ 决策 4）。
    let grant = grants_from_ctx(ctx).ceiling(&meta.name);
    let needs = tool_risk
        .approval_basis(threshold, is_approved, grant)
        .is_none();

    if !needs {
        return None;
    }

    Some(confirm_prompt_payload(
        meta, args, tool_risk, &mode, approval,
    ))
}

/// 新策略路径：sandbox 决定结果，平台只做凭据消费与执行信封转换。
fn approval_policy_gate(
    ctx: &Arc<dyn PluginInvokeRequest>,
    meta: &CapabilityMeta,
    tool_risk: CapabilityRiskLevel,
    facts: &[SandboxFact],
    args: &Value,
    approval: Option<&ApprovalSummary>,
) -> Option<Value> {
    let policy = approval_policy_of(ctx.as_ref())?;
    let evaluation = policy.evaluate(facts, &meta.name, args, tool_risk, threshold_from_ctx(ctx));
    if let Some(intercept) = evaluation.intercept() {
        return Some(intercept.clone());
    }
    if evaluation.is_denied() {
        let reason = evaluation.reason().unwrap_or("sandbox denied the call");
        return Some(json!({
            "error": format!("沙箱未放行工具 {} 的本次调用：{reason}", meta.name),
            "success": false,
            "failure_kind": crate::kernel::capability::failure_kind::PERMISSION_DENIED,
        }));
    }
    if evaluation.is_allowed() {
        ctx.set_raw(TRUSTED_APPROVED, Arc::new("false".to_string()));
        return None;
    }

    let call_id = ctx.get(TOOL_CALL_ID).unwrap_or_default();
    let session_id = ctx.get(SESSION_ID).filter(|value| !value.trim().is_empty());
    let current_binding = policy.binding();
    let binding = evaluation
        .binding()
        .cloned()
        .or_else(|| current_binding.clone());
    let is_approved = ApprovalGrant::take(
        ctx.as_ref(),
        &call_id,
        &meta.name,
        args,
        session_id.as_deref(),
        binding.as_ref(),
    );
    ctx.set_raw(TRUSTED_APPROVED, Arc::new(is_approved.to_string()));
    if is_approved {
        return None;
    }
    Some(confirm_prompt_payload_from_evaluation(
        ctx.as_ref(),
        meta,
        args,
        approval,
        &evaluation,
        binding.as_ref(),
    ))
}

/// 构造 confirm 类型 prompt 的**返回值**（不构造节点、不发事件）。
///
/// 职责边界：工具只回答「需要用户确认什么」——`prompt` 载荷 + `failure_kind`。
/// **user_prompt 节点由编排层构造**（`session/tools/tool_executor.rs`）：它拥有
/// 结果节点 id（由调用 id 派生）与父 ToolCall 的终态。
fn confirm_prompt_payload(
    meta: &CapabilityMeta,
    args: &Value,
    risk_level: CapabilityRiskLevel,
    mode: &str,
    approval: Option<&ApprovalSummary>,
) -> Value {
    let tool_name = &meta.name;
    let tool_description = &meta.description;
    let risk_str = risk_level.as_str();
    if mode == "auto" {
        return json!({
            "error": format!(
                "权限不足：工具 {} 需要用户审批（风险等级 {}），但当前为自动模式，无人可授权。请勿反复重试——请改用手动方式完成，或提示用户切换到交互模式以授权后重试。",
                tool_name, risk_str
            ),
            "success": false,
            "failure_kind": crate::kernel::capability::failure_kind::PERMISSION_DENIED,
        });
    }
    let mut prompt = json!({
        "kind": "confirm",
        "tool_name": tool_name,
        "args": args.clone(),
        "risk_level": risk_str,
        "description": tool_description,
    });
    if let Some(approval) = approval {
        prompt["approval"] = approval.summary(args);
    }
    json!({
        "content": format!("需要确认：{tool_description}"),
        "success": false,
        "failure_kind": crate::kernel::capability::failure_kind::NEEDS_APPROVAL,
        "prompt": prompt,
    })
}

fn confirm_prompt_payload_from_evaluation(
    ctx: &dyn PluginInvokeRequest,
    meta: &CapabilityMeta,
    args: &Value,
    approval: Option<&ApprovalSummary>,
    evaluation: &ApprovalEvaluation,
    binding: Option<&PolicyBinding>,
) -> Value {
    let risk_level = evaluation.risk().unwrap_or_default();
    // 档案可以声明交互审批，但会话一旦是无人值守，就没有能批准的人：产卡只会把会话
    // 钉死。运行模式是本次调用的硬约束，优先于档案的审批方式。
    let mode = if ctx.get(MODE).as_deref() == Some("auto") {
        "auto"
    } else {
        evaluation.mode().unwrap_or("interactive")
    };
    if mode == "auto" {
        return json!({
            "error": format!(
                "权限不足：工具 {} 需要用户审批（风险等级 {}），但当前为自动模式，无人可授权。请勿反复重试——请改用手动方式完成，或提示用户切换到交互模式以授权后重试。",
                meta.name, risk_level.as_str()
            ),
            "success": false,
            "failure_kind": crate::kernel::capability::failure_kind::PERMISSION_DENIED,
        });
    }
    let mut prompt = json!({
        "kind": "confirm",
        "tool_name": meta.name,
        "args": args.clone(),
        "risk_level": risk_level.as_str(),
        "description": meta.description,
        "approval_scopes": evaluation.scopes().names(),
    });
    if let Some(approval) = approval {
        prompt["approval"] = approval.summary(args);
    }
    if let Some(binding) = binding {
        prompt["policy"] = json!({
            "profile_id": binding.profile_id(),
            "revision": binding.revision(),
        });
    }
    if let Some(reason) = evaluation.reason() {
        prompt["approval_reason"] = json!(reason);
    }
    json!({
        "content": format!("需要确认：{}", meta.description),
        "success": false,
        "failure_kind": crate::kernel::capability::failure_kind::NEEDS_APPROVAL,
        "prompt": prompt,
    })
}

/// 端口执行体形态的审批闸门（ ③）：**判定是执行的第一段**。
///
/// `meta` 与 `risk` 在注册收口处取好快照，`inner` 是被裹的内层执行体
/// （工具自己的 `PortExecutor`）。`execute` 先判 [`approval_gate`]：拦下 ⇒
/// 停等载荷按 [`RESULT_KEY`] 交回（普通结果形态，派发侧 pending 机制不变，
/// `authz` 维持放行——停等不是拒绝）；放行 ⇒ 原样透传给内层。
pub(crate) struct GatedExecutor {
    meta: CapabilityMeta,
    risk: RiskOf,
    facts: Option<FactSource>,
    approval: Option<ApprovalSummary>,
    inner: Arc<dyn PortExecutor>,
}

impl GatedExecutor {
    /// 由一条内层执行体造闸门执行体：`meta` 是注册时的自述快照，`risk` 是工具的
    /// 风险回答（动态覆写在闭包里原样生效）。
    pub fn new(meta: CapabilityMeta, risk: RiskOf, inner: Arc<dyn PortExecutor>) -> Self {
        Self {
            meta,
            risk,
            facts: None,
            approval: None,
            inner,
        }
    }

    /// 带事实提取器的构造器：工具声明自己的参数如何转成沙箱事实，闸门据此查沙箱。
    pub fn new_with_facts(
        meta: CapabilityMeta,
        risk: RiskOf,
        facts: FactsOf,
        inner: Arc<dyn PortExecutor>,
    ) -> Self {
        Self {
            meta,
            risk,
            facts: Some(FactSource::Sync(facts)),
            approval: None,
            inner,
        }
    }

    pub fn new_with_async_facts(
        meta: CapabilityMeta,
        risk: RiskOf,
        facts: AsyncFactsOf,
        inner: Arc<dyn PortExecutor>,
    ) -> Self {
        Self {
            meta,
            risk,
            facts: Some(FactSource::Async(facts)),
            approval: None,
            inner,
        }
    }

    pub fn with_approval(mut self, approval: ApprovalSummary) -> Self {
        self.approval = Some(approval);
        self
    }
}

#[async_trait]
impl PortExecutor for GatedExecutor {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(request) = exec.ctx.clone() else {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                "工具端口执行体没有调用方上下文，无法判闸",
            );
        };
        let intercepted = if approval_policy_of(request.as_ref()).is_some() {
            let tool_risk = (self.risk)(&input.payload);
            let facts = match &self.facts {
                Some(FactSource::Sync(facts)) => facts(&input.payload, &request),
                Some(FactSource::Async(facts)) => match facts(&input.payload, &request).await {
                    Ok(facts) => facts,
                    Err(error) => {
                        return PortOutcome {
                            authz: AuthzVerdict::Authorized,
                            facts: vec![json!({ RESULT_KEY: json!({
                                "error": format!("工具意图事实解析失败：{error}"),
                                "success": false,
                                "failure_kind": crate::kernel::capability::failure_kind::PERMISSION_DENIED,
                            }) })],
                            error: None,
                            reply: Default::default(),
                        };
                    }
                },
                None => Vec::new(),
            };
            approval_policy_gate(
                &request,
                &self.meta,
                tool_risk,
                &facts,
                &input.payload,
                self.approval.as_ref(),
            )
        } else if request.get(SANDBOX_ID).is_some() || sandbox_of(request.as_ref()).is_some() {
            match &self.facts {
                Some(FactSource::Sync(facts)) => approval_gate_with_facts(
                    &request,
                    &self.meta,
                    self.risk.as_ref(),
                    Some(facts),
                    &input.payload,
                    self.approval.as_ref(),
                ),
                Some(FactSource::Async(facts)) => match facts(&input.payload, &request).await {
                    Ok(facts) => approval_gate_inner(
                        &request,
                        &self.meta,
                        self.risk.as_ref(),
                        &input.payload,
                        Some(&facts),
                        self.approval.as_ref(),
                    ),
                    Err(error) => Some(json!({
                        "error": format!("工具意图事实解析失败：{error}"),
                        "success": false,
                        "failure_kind": crate::kernel::capability::failure_kind::PERMISSION_DENIED,
                    })),
                },
                None if self.approval.is_none() => {
                    approval_gate(&request, &self.meta, self.risk.as_ref(), &input.payload)
                }
                None => approval_gate_inner(
                    &request,
                    &self.meta,
                    self.risk.as_ref(),
                    &input.payload,
                    None,
                    self.approval.as_ref(),
                ),
            }
        } else {
            Some(json!({
                "error": "审批策略不可用：未发布沙箱审批策略，拒绝执行工具",
                "success": false,
                "failure_kind": crate::kernel::capability::failure_kind::PERMISSION_DENIED,
            }))
        };
        if let Some(payload) = intercepted {
            // 闸门拦下：停等载荷是普通结果形态（`failure_kind` 在载荷里），
            // `authz` 保持放行——这不是授权拒绝，是「等用户点头」。
            return PortOutcome {
                authz: AuthzVerdict::Authorized,
                facts: vec![json!({ RESULT_KEY: payload })],
                error: None,
                reply: Default::default(),
            };
        }
        // 放行：判定不截断执行——上下文与输入原样到达内层。
        self.inner.execute(exec, input).await
    }
}

#[cfg(test)]
#[path = "approval.test.rs"]
mod tests;

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
//! | `name` | 模型给的**线上名**（解析在本插件；环只当不具名纯值搬运） |
//! | `args` | 调用参数（模型发的原样 JSON） |
//! | `call_id` | 本次调用的 id（幂等与节点挂靠；结果节点 id 由它派生） |
//! | `node_id` | 工具卡在实时面上的节点 id（执行窗口的状态帧写到它上面；可选） |
//! | `scratch_dir` | 宿主为本轮准备的可写目录（结果超预算时在此写全文；子布局归本插件） |
//! | `artifact` | `{ key, produced_by, allowed, denied_reason }`：落产物格的上下文，`allowed` 是环侧产物闸的结论 |
//! | `land_only` | `{ text, success, needs_user_action, prompt, failure_kind }`：只落产物格、不再执行（唤醒结果与没跑到的结果走这一形态） |
//! | `gate` | `{ authorized, spent_ms, requested_ms, budget_ms }`：外部执行闸门的中性输入；快慢与开窗由执行体判 |
//!
//! 环侧的宿主插件（`parent`）经 `ctx.parent()` 取得：解析失败时按原样走到它，由它给出
//! 诚实的 NotFound。
//!
//! ## 产出契约（本插件 → 环）
//!
//! `PortOutcome.facts` 只有一项，键为 `result_text`（已守卫正文）/ `raw_text`
//! （未裁剪原文，环侧记账与钩子用）/ `success` / `pending` / `guard`
//! （`{ truncated, archive_path, origin_tokens }`）。产物格（`artifact.added`）由
//! 本执行体在 `land_only` 出口经写口直落（P1b / DE-048 D-308）；环只交回结果纯值，
//! 不装配产物载荷、不写产物格。
//! 工具失败**不**映射成 `PortOutcome.error`——失败是信息性事实（模型据此自我纠正），
//! 与「这一笔协议失败」是两件事。

use std::path::Path;
use std::sync::Arc;

use serde_json::json;

use crate::breaker::{CircuitBreaker, GateDecision};
use crate::result_guard::{guard_result, DEFAULT_TOKEN_CAP};
use symbio_plugin_api::hooks::{fire_hook, HookEvent};
use symbio_plugin_api::{
    capability_invoke, capability_to_wire,
    chat_node::{ChatNode, NodeStatus},
    effects::annotations,
    effects::apply,
    effects::expects_facts,
    effects::EffectEnv,
    effects::EffectInput,
    events::EVENT_ARTIFACT_ADDED,
    events::EVENT_AUTHZ_DENIED,
    events::EVENT_AUTHZ_GRANTED,
    events::EVENT_TOOL_REQUESTED,
    failure_kind, family, plugin_error, plugin_info, plugin_warn, CapabilityRiskLevel, Entity,
    ExecAbortSignal, FactDraft, LatencyTier, Plugin, PluginError, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginPayload, PortErrorClass, PortExecContext, PortExecutor,
    PortInput, PortOutcome, PortProvider, PromptMessage, Verb, ABORT_SIGNAL, AGENT_ID,
    CAPABILITY_VISITOR, EVENT_REQUEST, EVENT_SINK, PATH, PLUGIN_ID_TOOL, SESSION_ID, TOOL_CALL_ID,
    WORKDIR,
};

/// 端口名：名字住 `plugin-api` 的词汇面（D-364），提供者与消费方 import 同一常量。
pub(crate) use symbio_plugin_api::vocab::PORT_TOOL as TOOL;

/// 载荷键（环与提供者之间的私有约定，随 [`PortInput`] 走）。环不认识世界：
/// 线上名与调用 id 到这里只是两个**不具名的纯值**，语义由本插件给。
const KEY_TOOL_NAME: &str = "name";
const KEY_ARGS: &str = "args";
const KEY_TOOL_CALL_ID: &str = "call_id";
/// 工具卡在实时面上的节点身份（`tool_calls[].id`）。执行窗口的状态帧要落到
/// **那张卡**上，而 `call_id`（wire id）是产物配对键，两套身份不能互换。
const KEY_NODE_ID: &str = "node_id";
/// 应答配对键（环从模型应答承载里搬来的纯值，`D-319`）：工具结果按它挂回调用。
const KEY_REPLY_KEY: &str = "reply_key";
/// 结果正文末尾的附注（环算好的同参重复提示；执行体只追加，不解释来源）。
const KEY_REPLY_NOTE: &str = "reply_note";
/// 只落格、不执行：唤醒结果与没跑到的占位结果用这一形态交回执行体。
const KEY_LAND_ONLY: &str = "land_only";
/// 外部执行闸门的中性输入：`{ authorized, spent_ms, requested_ms, budget_ms }`。
///
/// 环的授权矩阵与成本台账只把结论交给执行体；快慢、开窗与熔断事实都由本执行体判
/// （DE-039 §6，DE-048 D-311）。缺省 = 不判（通用端口调用不带这一格）。
const KEY_GATE: &str = "gate";
/// 调用信封的协议读数（环只搬原始值，判在本次执行体）：
/// `{ id_missing, name_missing, parse_error }`。
const KEY_PROTOCOL: &str = "protocol";
/// 没成功的原因（环只把它交给能力包的效果端口，不解释）。
const FACT_WHY: &str = "why";

/// 产出键（与环的通用受理入口共用同一份形状）。
const FACT_RESULT_TEXT: &str = "result_text";
const FACT_SUCCESS: &str = "success";
const FACT_PENDING: &str = "pending";
/// 未裁剪的原始结果（环侧记账 / 钩子用它，节点正文用已守卫的 `result_text`）。
const FACT_RAW_TEXT: &str = "raw_text";
/// 守卫元数据：`{ truncated, archive_path, origin_tokens }`。
const FACT_GUARD: &str = "guard";

/// 授权输入键（`open-tools` 在开账时写进请求载荷，`D-306` 的「载荷装配留在行」）。
///
/// 与执行键（`tool_name` / `tool_call_id`）分开：授权只看判定输入，执行才解析线上名。
const KEY_TOOL: &str = "tool";
const KEY_RISK: &str = "risk";
const KEY_THRESHOLD: &str = "threshold";
const KEY_APPROVED: &str = "approved";
const KEY_GRANT: &str = "grant";
const KEY_DEFERRED: &str = "deferred";
/// 不可派发的成因词（缺 id / 解析失败 → `invalid_tool_call`；超限丢弃 →
/// `overflow_dropped`）。一个布尔会把这三种压扁成一个「是/否」，收据就写不出成因。
const KEY_FAILURE_KIND: &str = "failure_kind";
/// 授权专用输入（环的 gate 调用）：只判、只落授权事实，不执行也不写 `sent`。
const KEY_AUTHORIZE_ONLY: &str = "authorize_only";
/// 建账专用输入（环的 open 调用）：把模型应答里的调用清单展开成 `request` 草稿。
/// 环只交纯值（应答原文 / 调用清单 / 策略读数字串），线上名、风险档与
/// 永久放行上限全在本执行体算定（DE-048 D-311）。
const KEY_OPEN_ONLY: &str = "open_only";
/// 幕锚（id 兜底段）：环的 `drive_seq` 纯值。
const KEY_EPOCH: &str = "epoch";
/// 触发这笔债的事实标识（`tool.requested` 的事实行身份）。
const KEY_REQUEST_ID: &str = "request_id";
const KEY_POLICY: &str = "policy";
const KEY_RESPONSE: &str = "response";
const KEY_TOOL_CALLS: &str = "tool_calls";
const KEY_GRANTS: &str = "grants";
/// 会话的授权模式（`auto` = 无人可问 ⇒ 拒绝措辞 `permission_denied`，其余
/// ⇒ `needs_approval`）。措辞在线上契约里，不能由模式之外的东西决定。
const KEY_MODE: &str = "mode";
/// 唤醒动作（`approve` / `retry` / `supply` / `answer` / `reject`）：授权由用户动作裁决。
const KEY_RESUME: &str = "resume";
/// 债键字段（与环的 `request` 事实同形，授权事实按它可被行认领）。
const KEY_ID: &str = "id";
const KEY_ATTEMPT: &str = "attempt";

/// 恢复产物键的后缀：同一调用的**第二次落格**。
///
/// 首派键 `artifact:{call_id}` 已被那张「待用户动作」的产物占住；事实源按
/// `(kind, key)` 吸收重复，恢复结果若不换键会被直接吞掉。事件名仍是
/// `artifact.added`、幂等维度仍是 `tool_call_id`，投影按 `{wire}-result` 覆盖
/// 同一张卡——所以它是**同一次调用的新状态**，不是新事实（词表见
/// `declarations.rs` 的 `FACTS`）。
const RESUME_ARTIFACT_SUFFIX: &str = ":resume";

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
    /// 拒绝。`detail` 是**线上契约**（`authz.denied` 载荷的 `detail`：
    /// `needs_approval` / `permission_denied` / `user_rejected` / `invalid_tool_call`），
    /// `why` 是内部原因码（转写与日志用），两者并存：措辞由契约固定，原因由判定给出。
    Denied {
        detail: &'static str,
        why: &'static str,
    },
}

/// 从请求载荷判一次授权。
///
/// 优先看唤醒动作：批准 / 重试 / 补充 / 回答是用户给的放行，拒绝是用户给的判死——
/// 都不参与风险比较。普通调用按载荷里的风险输入判（缺 `threshold` 时按 route-only
/// 放行延迟给下游路由闸门）。
fn authorize(payload: &serde_json::Value) -> Authz {
    if let Some(resume) = payload.get(KEY_RESUME).and_then(|v| v.as_str()) {
        return if resume == "reject" {
            Authz::Denied {
                detail: "user_rejected",
                why: "user_rejected",
            }
        } else {
            Authz::Granted {
                risk: "unknown",
                basis: Some("approved"),
                grant: None,
            }
        };
    }
    let Some(threshold) = payload
        .get(KEY_THRESHOLD)
        .and_then(|v| v.as_str())
        .map(CapabilityRiskLevel::from_name)
    else {
        return Authz::Deferred;
    };
    if let Some(cause) = payload.get(KEY_FAILURE_KIND).and_then(|v| v.as_str()) {
        return Authz::Denied {
            detail: "invalid_tool_call",
            // 内部原因码带上成因（超限丢弃不是「调用本身非法」）：`why` 只用于
            // 转写与日志，不参与线上措辞，故可保留分辨率。
            why: if cause == "overflow_dropped" {
                "overflow_dropped"
            } else {
                "invalid_call"
            },
        };
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
        return Authz::Deferred;
    }
    // 载荷里的风险档由开账行按注册表算定；认不出就是装配漂移，不能静默认成某一档。
    let Some(risk) = CapabilityRiskLevel::parse(risk) else {
        return Authz::Denied {
            detail: "invalid_tool_call",
            why: "unknown_risk",
        };
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
        Some(basis) => Authz::Granted {
            risk: risk.as_str(),
            basis: Some(basis.as_str()),
            grant: grant.map(CapabilityRiskLevel::as_str),
        },
        None => {
            // 自动模式下没有人能被问「批不批」，拒绝的措辞就是契约里的 `permission_denied`。
            let auto = payload
                .get(KEY_MODE)
                .and_then(|v| v.as_str())
                .is_some_and(|m| m == "auto");
            Authz::Denied {
                detail: if auto {
                    "permission_denied"
                } else {
                    "needs_approval"
                },
                why: "approval_required",
            }
        }
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
                // `unknown` = route-only（唤醒放行也落这一格）：下游路由闸门据此接管。
                "deferred": risk == "unknown",
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
        Authz::Denied { detail, why } => (
            EVENT_AUTHZ_DENIED,
            Verb::Asserted,
            serde_json::json!({
                "port": TOOL,
                "id": id,
                "attempt": attempt,
                "tool": tool,
                "detail": detail,
                "why": why,
            }),
        ),
    };
    FactDraft {
        key: format!("{idempotency_key}.authz"),
        kind: kind.to_string(),
        entity: Entity::Scope,
        verb,
        payload,
        produced_by: None,
    }
}

/// 审批闸门拦下的一笔的留痕草稿（t78）。
///
/// 活闸门住在能力调用里（`ApprovalGate` 装饰器）：它把「需要用户批准」作为**结果值**
/// 交回。端口此前只把它折成停等载荷，事实面上没有一条授权结论——`request` 开过账，
/// `authz.denied` 却是空白。这里在同一个出口补上那条结论；id 与该笔 `request` 同源
///（开账行用的就是调用 id）。**不**写 `sent`：没放行就没有派发。
///
/// `detail` 是线上措辞：交互档 `needs_approval`（停在卡上等批）、自动档
/// `permission_denied`（无人可批，直接判死）——两者都是「闸门拒绝」，只是能不能等
/// 人不一样（`approval.rs::confirm_prompt_payload`）。
fn approval_denied_draft(
    exec: &PortExecContext,
    tool: &str,
    call_id: &str,
    detail: &'static str,
) -> FactDraft {
    let payload = serde_json::json!({
        KEY_TOOL: tool,
        KEY_ID: call_id,
    });
    // 批派里同一笔债含多条调用：按调用 id 派生幂等键，一条调用一份授权结论。
    authz_draft(
        &format!("{}-{call_id}", exec.idempotency_key),
        &payload,
        Authz::Denied {
            detail,
            why: "approval_required",
        },
    )
}

/// 批准结论的留痕草稿（D-368）。
///
/// 批准是**一次用户动作**，但此前的授权结论只在内存里（`resume` 动作串）：
/// `authz.denied{needs_approval}` 之后没有对应的放行结论。这里在批准执行前补上
/// `authz.granted`，风险档**原样取闸门已交回的产物载荷**（`prompt.risk_level`，
/// 唯一解析点在 `ApprovalGate`）——消费端不许再算一遍（D-350）。
///
/// 幂等键与首派的 `authz` 分开：同一笔债的第二次结论，不是重复；重放会被事实源
/// 按同键吸收。
fn approval_granted_draft(
    idempotency_key: &str,
    artifact: &serde_json::Value,
    call_id: &str,
    tool: &str,
) -> Option<FactDraft> {
    // 没有风险档的卡（非审批路径）不造授权结论：宁可不写，也不写一个假的 `unknown`。
    let risk = artifact
        .get("prompt")
        .and_then(|prompt| prompt.get("risk_level"))
        .and_then(|v| v.as_str())?;
    Some(FactDraft {
        key: format!("{idempotency_key}-{call_id}.authz-approved"),
        kind: EVENT_AUTHZ_GRANTED.to_string(),
        entity: Entity::Scope,
        verb: Verb::Asserted,
        payload: serde_json::json!({
            "port": TOOL,
            "id": call_id,
            "attempt": 1,
            "tool": tool,
            "risk": risk,
            "basis": "approved",
            "grant": serde_json::Value::Null,
            "deferred": risk == "unknown",
        }),
        produced_by: None,
    })
}

/// 建账：把模型应答里的调用清单展开成 `request` 草稿（D-118 决策 5 的第一笔债）。
///
/// 环只交纯值：应答原文（取 `id` / `calls`）、调用清单、阈值 / 放行清单 / 模式的
/// 读数字串。线上名解析、风险档、永久放行上限与「不可派发」判定全在这里算定——
/// 这是 DE-048 D-311 的唯一判定式：**事实面答不了「这个名对应哪个能力」，所以判在执行体**。
async fn open_drafts(exec: &PortExecContext, payload: &serde_json::Value) -> Vec<FactDraft> {
    let tool_calls: Vec<serde_json::Value> = payload
        .get(KEY_TOOL_CALLS)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let response = payload
        .get(KEY_RESPONSE)
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let root_id = response
        .get(KEY_ID)
        .and_then(|v| v.as_str())
        .unwrap_or("r0");
    let created: &[serde_json::Value] = response
        .get("calls")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let epoch = payload.get(KEY_EPOCH).and_then(|v| v.as_u64()).unwrap_or(0);
    let policy = payload
        .get(KEY_POLICY)
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let threshold = policy
        .get(KEY_THRESHOLD)
        .and_then(|v| v.as_str())
        .map(CapabilityRiskLevel::from_name)
        .unwrap_or_default();
    let grants: std::collections::BTreeMap<String, CapabilityRiskLevel> = policy
        .get(KEY_GRANTS)
        .and_then(|v| v.as_str())
        .map(|s| serde_json::from_str(s).unwrap_or_default())
        .unwrap_or_default();
    let mode = policy
        .get(KEY_MODE)
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let visitor = exec.ctx.as_ref().and_then(|c| c.get(CAPABILITY_VISITOR));
    let rows = match visitor.as_ref() {
        Some(visitor) => visitor.list(family::TOOL).await,
        None => Vec::new(),
    };
    let known: Vec<String> = rows.iter().map(|row| row.name().to_string()).collect();
    let mut out = Vec::with_capacity(tool_calls.len());
    for (idx, tc) in tool_calls.iter().enumerate() {
        let model_id = tc
            .get(KEY_ID)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty());
        let name = tc
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty());
        let args = tc
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let id = created
            .get(idx)
            .and_then(|c| c.get(KEY_ID))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| fallback_key(&exec.subject, epoch, idx, model_id, root_id));
        // 不可派发的调用**也开账**：创建行的集合里有这个元素，跳过它那一笔集合债
        // 就永远欠着。它的终局是「授权拒绝 + 批尾 failed」，不是「不存在」。
        // 超限丢弃的标记是**字符串正文**（`mark_overflow_dropped` 写整句提示）；
        // 旧读法 `.as_bool()` 恒为 false，于是「不可派发」的判定从未生效。
        let overflow = match tc.get("overflow_notice") {
            Some(serde_json::Value::Bool(true)) => true,
            Some(serde_json::Value::String(text)) => !text.trim().is_empty(),
            _ => false,
        };
        // 成因词取代布尔：缺 id / 解析失败 → `invalid_tool_call`，超限丢弃 →
        // `overflow_dropped`。授权门按「有没有成因」判不可派发，收据按同一个词写
        // 失败种类——一处判定，两处读同一个值，不再各自把三种成因压成一个 bool。
        let invalid_cause = if overflow {
            Some("overflow_dropped")
        } else if model_id.is_none()
            || name.is_none()
            || tc
                .get("parse_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        {
            Some("invalid_tool_call")
        } else {
            None
        };
        // 注册表里没有可判的能力（route-only 工具）⇒ 放行是**交给下游路由闸门**的
        // 延迟判定；不能凭一个猜出来的风险档拒绝它。
        let resolved = match (name, visitor.as_ref()) {
            (Some(name), Some(visitor)) => match resolve(name, known.iter().map(String::as_str)) {
                Some(canonical) => visitor
                    .get(&canonical)
                    .await
                    .and_then(|row| row.cap().cloned())
                    .map(|cap| (canonical, cap.risk_for(&args))),
                None => None,
            },
            _ => None,
        };
        let risk = resolved.as_ref().map(|(_, risk)| *risk);
        let risk_name = risk.map(CapabilityRiskLevel::as_str).unwrap_or("unknown");
        // 永久放行上限按 canonical name 算（D-170）：线上名只是协议字符集的投影。
        let grant = resolved
            .as_ref()
            .and_then(|(canonical, _)| grants.get(canonical))
            .map(|level| level.as_str());
        let approved = args
            .get(KEY_APPROVED)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        out.push(FactDraft {
            key: format!("{EVENT_REQUEST}-{TOOL}-{id}-1"),
            kind: EVENT_REQUEST.to_string(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: serde_json::json!({
                "port": TOOL,
                "id": id,
                "attempt": 1,
                "name": name.unwrap_or_default(),
                "args": args,
                "risk": risk_name,
                "threshold": threshold.as_str(),
                "approved": approved,
                // 缺注册面 / 风险未知 = route-only：授权按 `deferred` 放行。
                "deferred": risk.is_none(),
                "grant": grant,
                "failure_kind": invalid_cause,
                "mode": mode,
            }),
            produced_by: exec.origin,
        });
    }
    out
}

/// 调用 id 的兜底段（模型应答没有 `calls` / 缺 id 时用）：只由应答自身的顺序决定。
fn fallback_key(
    subject: &str,
    epoch: u64,
    idx: usize,
    model_id: Option<&str>,
    root_id: &str,
) -> String {
    match model_id {
        Some(id) => format!("{TOOL}-{subject}-d{epoch}-{root_id}-c{idx:03}-{id}"),
        None => format!("{TOOL}-{subject}-d{epoch}-{root_id}-c{idx:03}"),
    }
}

/// 一次调用在落产物格时需要的结构身份（都是环侧传来的纯值，不含判定）。
struct CallMeta<'a> {
    tool_name: &'a str,
    tool_call_id: &'a str,
    args: &'a serde_json::Value,
    /// 本工具声明的行为名（从注册表元数据抄来；`annotations` 按它选实现）。
    declared: &'static [&'static str],
}

/// 产物格上下文（宿主在 [`PortExecContext`] 里给出，字段一律中性）。
struct ArtifactCtx {
    key: String,
    produced_by: Option<u64>,
    allowed: bool,
    denied_reason: String,
}

/// 产物格的授权结论由**本执行体自判**（D-385）：外部执行闸门的授权读数在载荷里，
/// 有读数按读数，没有读数就是「无待判事项」（通用端口调用不带这一格）。
fn artifact_ctx(exec: &PortExecContext, gate: Option<GateInput>) -> Option<ArtifactCtx> {
    let allowed = gate.map(|g| g.authorized).unwrap_or(true);
    Some(ArtifactCtx {
        // 键按**这一笔债务**（幂等键）生成：批派之外的出口（唤醒、续接）没有调用清单
        // 形状的 `artifact:{call_id}` 键可用；按债键生成让同一笔债重放只落一个产物。
        key: format!("a-{}", exec.idempotency_key),
        produced_by: exec.origin,
        allowed,
        denied_reason: if allowed {
            String::new()
        } else {
            "外部执行闸门未授权".to_string()
        },
    })
}

/// 外部执行闸门的中性输入（环读授权矩阵与成本台账后交过来的四个值）。
#[derive(Debug, Clone, Copy)]
struct GateInput {
    authorized: bool,
    spent_ms: u64,
    requested_ms: u64,
    budget_ms: u64,
}

/// 载荷里有没有闸门输入：**没有 = 不判**（通用端口调用不带这一格）。
fn gate_inputs(payload: &serde_json::Value) -> Option<GateInput> {
    let gate = payload.get(KEY_GATE)?;
    Some(GateInput {
        authorized: gate
            .get("authorized")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        spent_ms: gate
            .get("spent_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        requested_ms: gate
            .get("requested_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        budget_ms: gate
            .get("budget_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(u64::MAX),
    })
}

/// 前置闸拒绝的出口：结果事实带上中性的 `failure_kind`，环据此渲染、不解释原因。
fn refused(text: &str, failure_kind: &'static str) -> PortOutcome {
    PortOutcome {
        facts: vec![serde_json::json!({
            FACT_RESULT_TEXT: text,
            FACT_RAW_TEXT: text,
            FACT_SUCCESS: false,
            FACT_PENDING: serde_json::Value::Null,
            FACT_GUARD: serde_json::Value::Null,
            "failure_kind": failure_kind,
        })],
        error: None,
        ..PortOutcome::ok()
    }
}

/// `artifact.added` 载荷的**唯一构造点**（`land_only` 与环派发都经 [`land_artifact`] 到这里）。
///
/// 载荷字段（`tool` / `text` / `tool_call_id` / `args` / `success` /
/// `needs_user_action` / `prompt` / `failure_kind` + 能力交出的富化字段）只在这一处算定。
/// 停等信息从结果事实**顶层**取（`needs_user_action` / `prompt` / `failure_kind`）；
/// 嵌套 `pending` 只是详情，不是唯一来源。
/// 结果节点 id 由 `tool_call_id` 派生，不作为载荷键（DE-048 D-312）。
fn artifact_payload(meta: &CallMeta<'_>, result: &serde_json::Value) -> serde_json::Value {
    let pending = result.get(FACT_PENDING).filter(|v| !v.is_null());
    let needs_user_action = result
        .get("needs_user_action")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(pending.is_some());
    let prompt = result
        .get("prompt")
        .cloned()
        .filter(|v| !v.is_null())
        .or_else(|| pending.and_then(|p| p.get("prompt")).cloned())
        .unwrap_or(serde_json::Value::Null);
    let failure_kind = result
        .get("failure_kind")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            pending
                .and_then(|p| p.get("failure_kind"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or_default()
        .to_string();
    serde_json::json!({
        "tool": meta.tool_name,
        "text": result.get(FACT_RESULT_TEXT).cloned().unwrap_or(serde_json::Value::Null),
        "tool_call_id": meta.tool_call_id,
        // 定稿参数随产物落格：变更账按 `args.path` 取路径（读侧不反推结果文本）。
        "args": meta.args.to_string(),
        "success": result.get("success").cloned().unwrap_or(serde_json::Value::Bool(true)),
        // 等待用户：结果节点是一张**待用户动作的卡**（`user_prompt` +
        // `waiting_user_action`），不是普通结果文本。
        "needs_user_action": needs_user_action,
        "prompt": prompt,
        "failure_kind": failure_kind,
    })
}

/// 落一条 `artifact.added`：产物是**这次结果的产物**，所以由执行体写（P1b / D-308）。
///
/// **唯一写点**：批派与单次出口都不许自己装配载荷，一律经 [`artifact_payload`]。
/// `allowed` 由环侧产物闸给出，执行体不自判权限；拒绝时按 `deny-{key}-artifact`
/// 落一条幂等否定事实（同一笔债的多条结果说的是同一件事）。
async fn land_artifact(
    exec: &PortExecContext,
    art: &ArtifactCtx,
    meta: &CallMeta<'_>,
    result: &serde_json::Value,
) {
    if !art.allowed {
        // 产物闸拒绝：只落一条否定事实；幂等键让它对同一笔债的多条结果只存在一条。
        // `reason` 的取值与环侧 `ClosureReason::Denied::as_str()` 同字面。
        exec.emit
            .emit(vec![FactDraft {
                key: format!("deny-{}-artifact", art.key),
                kind: EVENT_AUTHZ_DENIED.to_string(),
                entity: Entity::Scope,
                verb: Verb::Asserted,
                payload: serde_json::json!({
                    "key": EVENT_ARTIFACT_ADDED,
                    "kind": "artifact",
                    "reason": "denied",
                    "detail": art.denied_reason,
                }),
                produced_by: art.produced_by,
            }])
            .await;
        return;
    }
    let mut artifact_payload = artifact_payload(meta, result);
    // 能力交出的富化字段（DE-025 D-207）：执行体算，环侧只落（读侧认字段不认工具名）。
    // 待用户动作的调用不记账——「审批没到可判时刻」，与环侧记账同一条口径。
    let pending = result.get(FACT_PENDING).filter(|v| !v.is_null());
    if pending.is_none() && !meta.declared.is_empty() {
        let input = EffectInput {
            tool: meta.tool_name.to_string(),
            call_id: meta.tool_call_id.to_string(),
            args: meta.args.clone(),
            success: result
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            result: result
                .get(FACT_RAW_TEXT)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            why: String::new(),
            declared: meta.declared,
        };
        // 声明的账：与环派发走**同一个**承接函数（单一实现，不复制）。
        settle_declared(exec, &input, &mut artifact_payload).await;
    }
    exec.emit
        .emit(vec![FactDraft {
            key: art.key.clone(),
            kind: EVENT_ARTIFACT_ADDED.to_string(),
            entity: Entity::Artifact,
            verb: Verb::Asserted,
            payload: artifact_payload,
            produced_by: art.produced_by,
        }])
        .await;
}

/// 一次调用的「声明的账」：按 `declared` 求行为草稿，交宿主唯一写口。
///
/// **两套承接出口（单次调用 / 环派发）都走这里**：同一承接体只能有一份实现。
/// 复制一份的后果是——接错了不报错，只是静默地什么都不做（t93 就是这样卡住的）。
/// 记账判决：声明的账落没落。判定只在这里合成一次（两个出口都读它）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EffectsLanded {
    drafts: usize,
    bookkeeping_missing: bool,
}

/// 声明了**造格型**行为、执行也成功，却一条草稿都没有 ⇒ 违约（fail-closed）。
///
/// 判据只该管造格型（[`symbio_plugin_api::behaviors::Kind::LandsFacts`]）：富化型
/// 只给既有事实补字段（如 `file_op`），纯前置判断型只判不写——两者 0 草稿是正常
/// 形态，不能判成「有账不落」。
fn bookkeeping_missing(expects_facts: bool, success: bool, drafts: usize) -> bool {
    success && expects_facts && drafts == 0
}

/// 把「声明的账没落」写进产物格：不报成功，并给出可判定词。
///
/// `failure_kind` 目前是字符串词表（`needs_interaction` / `not_executed` /
/// `tool_unavailable` 同一层），本词新增 `bookkeeping_missing`。
///
/// ⚠️ 次序：本函数改写 `success`，而 `annotations()` 已在它**之前**把富化字段
/// （如 `file_op`）写进产物格。任何改 `success` / 判失败的写点都**不许连带抹掉
/// 已写的富化字段**——失败要说明哪里失败，不是把痕迹擦干净。
fn mark_bookkeeping_missing(artifact: &mut serde_json::Value) {
    if let Some(obj) = artifact.as_object_mut() {
        obj.insert("success".to_string(), serde_json::Value::Bool(false));
        obj.insert(
            "failure_kind".to_string(),
            serde_json::json!("bookkeeping_missing"),
        );
        obj.insert(
            "why".to_string(),
            serde_json::json!("声明了行为却没有产出任何事实"),
        );
    }
}

/// 一次调用收口时的「声明的账」：**先**补产物格的富化字段，**再**落事实草稿并判违约。
///
/// 两个承接出口（单次调用 / 环派发）都走这里：次序只能有一份实现——富化字段先写，
/// 判失败后写。任何改 `success` 的判断都**不许**抹掉已写的富化字段（失败要说明
/// 哪里失败，不是把痕迹擦干净）。
async fn settle_declared(
    exec: &PortExecContext,
    input: &EffectInput,
    artifact: &mut serde_json::Value,
) {
    if let Some(obj) = artifact.as_object_mut() {
        for (key, value) in annotations(input) {
            obj.insert(key.to_string(), value);
        }
    }
    if land_effects(exec, input).await.bookkeeping_missing {
        mark_bookkeeping_missing(artifact);
    }
}

async fn land_effects(exec: &PortExecContext, input: &EffectInput) -> EffectsLanded {
    if input.declared.is_empty() {
        return EffectsLanded {
            drafts: 0,
            bookkeeping_missing: false,
        };
    }
    // 行为只读「发生了什么」与宿主面；端口手里没有内核 `Event` 切片，
    // 两片历史交空片是诚实的缺省（行为不得据此假设历史非空）。
    //
    // 主体 = **发起这次调用的主体**（`ctx[AGENT_ID]` 的纯派生，与会话写侧同一取值口径），
    // 不是会话 id：立约格的 `from` 落在「谁许的诺」头上、声誉按主体聚合（DE-038），
    // 两者混用会把账记到会话名下。
    let agent_id = exec.ctx.as_deref().and_then(|ctx| ctx.get(AGENT_ID));
    let principal = symbio_plugin_api::principal_of(agent_id.as_deref());
    // 承诺对象 = **这条消息的来源主体**（D-067：`to` 是数据，不是写死的常量）。
    // 宿主已在句柄束里解析好（`PortExecHandles.source`）；空 = 本机用户发起，
    // 回落会话身份（旧口径的唯一保留处，不是第二套默认）。
    let source = if exec.source.trim().is_empty() {
        exec.session_id.clone()
    } else {
        exec.source.clone()
    };
    let env = EffectEnv {
        anchor: &exec.idempotency_key,
        user_seq: exec.origin.unwrap_or(0),
        principal: &principal,
        source: &source,
        snapshot: &[],
        applied: &[],
    };
    let drafts: Vec<FactDraft> = apply(&input.call(&env))
        .into_iter()
        .map(|mut d| {
            d.produced_by = exec.origin;
            d
        })
        .collect();
    let n = drafts.len();
    if n > 0 {
        exec.emit.emit(drafts).await;
    }
    EffectsLanded {
        drafts: n,
        bookkeeping_missing: bookkeeping_missing(expects_facts(input.declared), input.success, n),
    }
}

/// 本工具声明的行为名：与环侧 `ToolDecls::behaviors_of` 同一匹配口径
/// （注册表键先等值、再取线上名的末段投影）。表只作查找，返回值拷贝切片本体。
fn declared_of(table: &[(String, &'static [&'static str])], wire: &str) -> &'static [&'static str] {
    let wire = wire.trim();
    if let Some((_, declared)) = table.iter().find(|(name, _)| name == wire) {
        return declared;
    }
    let short = wire.rsplit('/').next().unwrap_or_default();
    table
        .iter()
        .find(|(name, _)| name == short)
        .map(|(_, declared)| *declared)
        .unwrap_or(&[])
}

/// 行为声明表：三个出口（批派 / 只落格 / 恢复）都只查这一张表。
///
/// 复制一份的后果与承接体是同一个教训——接错了不报错，只是静默地不做富化。
async fn declared_table_of(exec: &PortExecContext) -> Vec<(String, &'static [&'static str])> {
    match exec.ctx.as_ref() {
        Some(ctx) => match ctx.get(CAPABILITY_VISITOR) {
            Some(visitor) => visitor
                .list(family::TOOL)
                .await
                .iter()
                .map(|row| {
                    let m = row.meta();
                    (m.name.to_string(), m.behaviors)
                })
                .collect(),
            None => Vec::new(),
        },
        None => Vec::new(),
    }
}

/// 产物里的 `args` 是**定稿参数文本**（[`artifact_payload`] 按 `to_string` 落格）；
/// 恢复要重跑同一调用时必须先解析回值，才能把 `approved` 合并进去。
fn artifact_args(payload: &serde_json::Value) -> serde_json::Value {
    match payload.get("args") {
        Some(serde_json::Value::String(text)) => {
            serde_json::from_str(text).unwrap_or(serde_json::Value::Null)
        }
        Some(other) => other.clone(),
        None => serde_json::Value::Null,
    }
}

/// 答复原文：字符串按原文用（不再套一层 JSON 引号），其余按 JSON 文本。
fn answer_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// 统一出口：L0 守卫 → 交回事实（产物格由 [`land_only`] 出口另落）。
///
/// 守卫在**执行体**里做（P1b / D-308）：环只消费「已守卫文本 + 守卫元数据」，不再
/// 认识 token 预算或存档布局。待用户动作的结果不裁剪（它是卡片正文，不是工具结果）。
async fn finish(
    raw: String,
    success: bool,
    pending: Option<serde_json::Value>,
    archive_dir: &Path,
) -> PortOutcome {
    let (text, guard) = match &pending {
        Some(_) => (
            raw.clone(),
            serde_json::json!({
                "truncated": false,
                "archive_path": serde_json::Value::Null,
                "origin_tokens": 0,
            }),
        ),
        None => {
            let guarded = guard_result(&raw, DEFAULT_TOKEN_CAP, archive_dir);
            (
                guarded.text,
                serde_json::json!({
                    "truncated": guarded.truncated,
                    "archive_path": guarded.archive_path,
                    "origin_tokens": guarded.original_tokens,
                }),
            )
        }
    };
    let needs_user_action = pending.is_some();
    let prompt = pending
        .as_ref()
        .and_then(|p| p.get("prompt"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let pending_kind = pending
        .as_ref()
        .and_then(|p| p.get("failure_kind"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let mut fact = serde_json::json!({
        FACT_RESULT_TEXT: text,
        FACT_RAW_TEXT: raw,
        FACT_SUCCESS: success,
        FACT_PENDING: pending,
        // 停等信息在**结果事实顶层**也可见：`pending` 只是详情，不是唯一来源。
        // 下游（产物格 / 重建 / 编排层）读顶层，不必钻进嵌套对象。
        "needs_user_action": needs_user_action,
        "prompt": prompt,
        FACT_GUARD: guard,
    });
    if !pending_kind.is_empty() {
        fact["failure_kind"] = serde_json::json!(pending_kind);
    }
    PortOutcome {
        facts: vec![fact],
        error: None,
        ..PortOutcome::ok()
    }
}

/// [`finish`] 的失败变体：额外带上失败种类与「为什么没履约」。
///
/// 两者都是**事实值**，环只读；`failure_kind` 进父节点元数据，`why` 交给能力包的
/// 效果端口（DE-025 D-207）。
async fn finish_with(
    raw: String,
    success: bool,
    pending: Option<serde_json::Value>,
    archive_dir: &Path,
    failure_kind: Option<&str>,
    why: &str,
) -> PortOutcome {
    let mut out = finish(raw, success, pending, archive_dir).await;
    if let Some(fact) = out.facts.first_mut().and_then(|f| f.as_object_mut()) {
        if let Some(kind) = failure_kind {
            fact.insert("failure_kind".into(), serde_json::json!(kind));
        }
        if !why.is_empty() {
            fact.insert(FACT_WHY.into(), serde_json::json!(why));
        }
    }
    out
}

/// 读面里派发这一笔债的 `tool.requested`：返回该请求的调用清单。
///
/// 清单只在执行体这里展开，债表不看（它只认逐笔标记）。
/// 载荷里的 `request_id` 是**触发这笔债的事实标识**（债行按它组 id）：按它精确取，
/// 不拿「最近一条」顶替——两条工具请求同时在场时，「最近」不是配对。
fn read_requested(
    exec: &PortExecContext,
    payload: &serde_json::Value,
) -> Result<Vec<RequestedCall>, (PortErrorClass, String)> {
    let Some(reader) = exec.reader.clone() else {
        return Err((
            PortErrorClass::NonRetryable,
            "tool.requested 无读面".to_string(),
        ));
    };
    let facts = reader.facts();
    let Some(request_id) = payload
        .get(KEY_REQUEST_ID)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    else {
        return Err((
            PortErrorClass::NonRetryable,
            "工具债载荷缺 request_id".to_string(),
        ));
    };
    let Some(fact) = facts
        .iter()
        .rev()
        .find(|fact| fact.kind == EVENT_TOOL_REQUESTED && fact.id == request_id)
    else {
        // 事实与派发之间可能有半步时差：可重试，不是协议失败。
        return Err((
            PortErrorClass::Retryable,
            "tool.requested 尚未可见".to_string(),
        ));
    };
    let calls = fact
        .payload
        .get("calls")
        .and_then(|calls| calls.as_array())
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let id = call.get("id")?.as_str()?.to_string();
                    let name = call.get("name")?.as_str()?.to_string();
                    // 丢弃标记的正文是**字符串**（`mark_overflow_dropped` 写整句提示），
                    // 不是布尔：读成布尔会恒为 false，调用就照样被执行。
                    let failure = call
                        .get("overflow_notice")
                        .and_then(|v| v.as_str())
                        .filter(|text| !text.trim().is_empty())
                        .map(|text| RequestedFailure {
                            kind: "overflow_dropped",
                            text: text.to_string(),
                        });
                    Some(RequestedCall {
                        id,
                        name,
                        node_id: call
                            .get("node_id")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .map(str::to_string),
                        arguments: call
                            .get("arguments")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                        failure,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(calls)
}

/// 补齐应答承载（`D-319`）：工具结果的 `tool` 消息**在端口里组装**。
///
/// 环只搬来配对键与结果正文附注（纯值），消息文本、角色、配对全在这里定；
/// 没有配对键（唤醒 / 无模型往返的路径）就不产出——环侧按旧口径兜底。
fn attach_reply(mut out: PortOutcome, payload: &serde_json::Value) -> PortOutcome {
    if out.error.is_some() {
        return out;
    }
    let Some(key) = payload
        .get(KEY_REPLY_KEY)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    else {
        return out;
    };
    let Some(fact) = out.facts.first() else {
        return out;
    };
    let Some(text) = fact.get(FACT_RESULT_TEXT).and_then(|v| v.as_str()) else {
        return out;
    };
    let name = payload
        .get(KEY_TOOL_NAME)
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let mut body = text.to_string();
    let is_pending = fact
        .get(FACT_PENDING)
        .map(|v| !v.is_null())
        .unwrap_or(false);
    // 待用户动作的卡片不挂结果附注（它不是结果正文），与环侧同一条口径。
    if !is_pending {
        if let Some(note) = payload
            .get(KEY_REPLY_NOTE)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            body.push('\n');
            body.push_str(note);
        }
    }
    out.reply.messages.push(PromptMessage {
        role: "tool".into(),
        text: body,
        tool_call_id: Some(key.to_string()),
        tool: Some(name.to_string()),
        tool_calls: None,
    });
    out
}

/// 调用信封的协议读数。缺这一格 = 不是协议级调用（通用端口调用）。
struct ProtocolInput<'a> {
    id_missing: bool,
    name_missing: bool,
    parse_error: Option<&'a str>,
}

fn protocol_input(payload: &serde_json::Value) -> Option<ProtocolInput<'_>> {
    let p = payload.get(KEY_PROTOCOL)?;
    Some(ProtocolInput {
        id_missing: p
            .get("id_missing")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        name_missing: p
            .get("name_missing")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        parse_error: p.get("parse_error").and_then(serde_json::Value::as_str),
    })
}

/// 协议失败的结果：判定与措辞都在执行体（DE-048 D-312）。
///
/// 环只把原始协议读数搬进来，并给一个兜底 id 以便结果有落点；它不再判断
/// 「缺 id / 缺 name / 参数解析失败」。
async fn refuse_protocol(protocol: ProtocolInput<'_>, archive_dir: &Path) -> Option<PortOutcome> {
    if protocol.id_missing {
        plugin_error!("tool", "[Tool] 协议失败：模型未返回工具调用 id");
        return Some(
            finish_with(
                "Error: 模型未返回有效的工具调用 ID（协议错误）".to_string(),
                false,
                None,
                archive_dir,
                Some(failure_kind::ERROR),
                "",
            )
            .await,
        );
    }
    if protocol.name_missing {
        plugin_error!("tool", "[Tool] 协议失败：模型未返回工具名称");
        return Some(
            finish_with(
                "Error: 模型未返回有效的工具名称（协议错误）".to_string(),
                false,
                None,
                archive_dir,
                Some(failure_kind::ERROR),
                "",
            )
            .await,
        );
    }
    let raw = protocol.parse_error?;
    plugin_error!("tool", "[Tool] 协议失败：工具参数 JSON 解析失败");
    let preview: String = raw.chars().take(400).collect();
    let truncated = raw.chars().count() > 400;
    let text = format!(
        "Error: 工具参数 JSON 解析失败（可能被长度上限截断），本次调用未执行。\
         只有完整且可解析的 JSON 参数才会被执行，请勿以相同内容重试。\n\
         参数原文（{} 字符{}）：{preview}",
        raw.chars().count(),
        if truncated { "，已截断展示" } else { "" }
    );
    Some(
        finish_with(
            text,
            false,
            None,
            archive_dir,
            Some(failure_kind::ERROR),
            "参数 JSON 解析失败，未执行",
        )
        .await,
    )
}

/// 工具生命周期钩子（Pre/PostToolUse）由**执行体**触发：环不按事件名调钩子，
/// 只搬工具名与参数（DE-048 D-312）。
async fn fire_post_hook(
    parent: &Arc<dyn Plugin>,
    tool_name: &str,
    tool_input: &serde_json::Value,
    tool_output: serde_json::Value,
    ctx: &Arc<dyn PluginInvokeRequest>,
) {
    let _ = fire_hook(
        &Some(parent.clone()),
        HookEvent::PostToolUse {
            tool_name: tool_name.to_string(),
            tool_input: tool_input.clone(),
            tool_output,
        },
        ctx.clone(),
    )
    .await;
}

/// 只落产物格、不执行：唤醒结果与没跑到的占位结果走这一形态。
///
/// 载荷里的 `land_only` 是**结果纯值**（`text` / `success` / `needs_user_action` /
/// `prompt` / `failure_kind`）；结构身份（`tool_name` / `args` / `tool_call_id`）与
/// 执行形态共用同一组键。行为声明仍从能力表取——富化字段由执行体算，环不复制各
/// 工具的行为名（DE-025 D-207）。
async fn land_only(exec: &PortExecContext, payload: &serde_json::Value) {
    let Some(lo) = payload.get(KEY_LAND_ONLY) else {
        return;
    };
    let tool_name = payload
        .get(KEY_TOOL_NAME)
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let tool_call_id = payload
        .get(KEY_TOOL_CALL_ID)
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let args = payload
        .get(KEY_ARGS)
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let pending = if lo
        .get("needs_user_action")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        Some(serde_json::json!({
            "text": lo.get("text").cloned().unwrap_or(serde_json::Value::Null),
            "prompt": lo.get("prompt").cloned().unwrap_or(serde_json::Value::Null),
            "failure_kind": lo
                .get("failure_kind")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
        }))
    } else {
        None
    };
    let text = pending
        .as_ref()
        .and_then(|p| p.get("text"))
        .and_then(|v| v.as_str())
        .or_else(|| lo.get("text").and_then(|v| v.as_str()))
        .unwrap_or_default()
        .to_string();
    let success = lo.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
    // 行为声明表：与执行形态同一匹配口径（注册表显示名）。
    let declared_table = declared_table_of(exec).await;
    let meta = CallMeta {
        tool_name,
        tool_call_id,
        args: &args,
        declared: declared_of(&declared_table, tool_name),
    };
    let Some(art) = artifact_ctx(exec, gate_inputs(lo)) else {
        return;
    };
    // 结果按**结果事实**的形状交回唯一构造点：停等信息由 `pending` 上提到顶层
    // （见 [`artifact_payload`]）。
    let result = serde_json::json!({
        FACT_RESULT_TEXT: text.clone(),
        FACT_RAW_TEXT: text,
        FACT_SUCCESS: success,
        FACT_PENDING: pending.unwrap_or(serde_json::Value::Null),
    });
    land_artifact(exec, &art, &meta, &result).await;
}

/// 端口声明的同批在飞上限（`PortDecl.concurrency`）——**执行体自己的**并发策略。
///
/// 环不再做风险档 → 并行/串行映射，也不持上限（DE-048 D-311）：它只把每笔债交过来；
/// 限流在这里，因为「同批最多几笔在飞」是这个端口的能力，不是环的判定。
fn tool_concurrency() -> usize {
    crate::declarations::PORTS
        .iter()
        .find(|p| p.port == TOOL)
        .map_or(1, |p| usize::from(p.concurrency).max(1))
}

/// 一个会话的执行窗口：写侧档独占，只读档共享，只读还受端口声明的在飞上界约束。
struct ExecWindow {
    gate: tokio::sync::RwLock<()>,
    slots: tokio::sync::Semaphore,
}

/// 执行窗口按**会话**分格（DE-048 D-311）。
///
/// 不分格会在委派时死锁：父会话的 `agent_run` 持写侧窗口、等子会话跑完，而子会话的
/// 工具调用又来抢同一把锁。子会话有独立的 `SESSION_ID`，格子因此天然不互相等待；
/// 同会话内部的写侧独占则保持不变。
static WINDOWS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, Arc<ExecWindow>>>,
> = std::sync::OnceLock::new();

fn window_for(session_id: &str) -> Arc<ExecWindow> {
    let map = WINDOWS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .entry(session_id.to_string())
        .or_insert_with(|| {
            Arc::new(ExecWindow {
                gate: tokio::sync::RwLock::new(()),
                slots: tokio::sync::Semaphore::new(tool_concurrency()),
            })
        })
        .clone()
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
        // 授权判定专用入口优先：同一载荷即使带着唤醒动作，也只判、不执行、不恢复。
        if input.payload.get(KEY_AUTHORIZE_ONLY).is_some() {
            let payload = input.payload.clone();
            let out = self.run(exec, input).await;
            return attach_reply(out, &payload);
        }
        // 恢复（答复）：动作已落成 `user.answer` 事实，环按行表派发到这里。
        // 该分支**不执行**挂起的调用——答复本身就是那次调用的结果，走 `land_only`
        // 同一出口（`tool_call_id` 不变）。恢复只能从事实进环（D-121）。
        if input.payload.get(KEY_RESUME).is_some() {
            return self.run_resume(exec, input.payload).await;
        }
        // 新表达：债行载荷只带**触发事实身份**（`request_id` + 这一笔的 `call_id`），
        // 调用清单在 `tool.requested` 这条世界事实里 —— 由执行体自己去读面精确取，
        // **不是**协议错误（债表永不读载荷）。
        if input.payload.get(KEY_REQUEST_ID).is_some() {
            return self.run_requested(exec, input.payload).await;
        }
        let payload = input.payload.clone();
        let out = self.run(exec, input).await;
        attach_reply(out, &payload)
    }
}

/// 一次模型往返里被请求的一次工具调用（清单由模型写进 `tool.requested` 的载荷）。
struct RequestedCall {
    id: String,
    name: String,
    /// 工具卡的节点身份（与 `id` 的 wire 身份分工见 [`KEY_NODE_ID`]）。
    node_id: Option<String>,
    arguments: serde_json::Value,
    /// 模型侧已判定的「本次调用不执行」及成因（当前只有超限丢弃）。
    failure: Option<RequestedFailure>,
}

/// 一次「不执行」的判定：成因词 + 模型写下的那句正文。
///
/// 正文与收据同源：正文就是 `overflow_notice`，收据由同一条结果事实派生——
/// 不许一处写「未执行」、另一处写「成功」再让渲染层挑。
struct RequestedFailure {
    kind: &'static str,
    text: String,
}

impl ToolPort {
    /// 答复恢复：`user.answer` 已经落了事实；按事实里的 `kind` 分岔——
    /// `answer` 把答复原文作为那次调用的结果、`approve` 合并 `approved:true` 走
    /// **正常执行路径**、`deny` 不执行并以 `user_rejected` 收尾。三者都不新开
    /// 端口往返：这是同一笔调用的第二次落格（产物键带 [`RESUME_ARTIFACT_SUFFIX`]）。
    async fn run_resume(&self, exec: PortExecContext, payload: serde_json::Value) -> PortOutcome {
        let Some(reader) = exec.reader.clone() else {
            return PortOutcome {
                facts: Vec::new(),
                error: Some((
                    PortErrorClass::NonRetryable,
                    "user.answer 无读面".to_string(),
                )),
                ..PortOutcome::ok()
            };
        };
        let facts = reader.facts();
        let answer_id = payload
            .get("answer_event")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        // 恢复失败判 **NonRetryable**：读面里没有答复 / 产物不是「还没到」的时机问题，
        // 重试只会把同一笔债抬到新 attempt 上重跑（首派跟着重来）。这是**当前选择、
        // 非能力上限**：将来要做真正的重试归属，再单立一条裁决。
        let Some(answer) = facts.iter().rev().find(|fact| {
            fact.kind == symbio_plugin_api::events::EVENT_USER_ANSWER
                && answer_id.is_none_or(|id| fact.id == id)
        }) else {
            return PortOutcome {
                facts: Vec::new(),
                error: Some((
                    PortErrorClass::NonRetryable,
                    "恢复目标缺失：user.answer 不在读面".to_string(),
                )),
                ..PortOutcome::ok()
            };
        };
        let kind = answer
            .payload
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let call_id = answer
            .payload
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let answer_value = answer
            .payload
            .get("answer")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        // 从已落的产物复原那次调用：产物的 `tool` / `args` / `tool_call_id` 就是权威。
        let Some(artifact) = facts.iter().rev().find(|fact| {
            fact.kind == symbio_plugin_api::events::EVENT_ARTIFACT_ADDED
                && fact.payload.get("tool_call_id").and_then(|v| v.as_str())
                    == Some(call_id.as_str())
        }) else {
            return PortOutcome {
                facts: Vec::new(),
                error: Some((
                    PortErrorClass::NonRetryable,
                    format!("恢复目标缺失：找不到 {call_id} 的产物"),
                )),
                ..PortOutcome::ok()
            };
        };
        let tool_name = artifact
            .payload
            .get("tool")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let original_args = artifact_args(&artifact.payload);
        let archive_dir = exec.scratch_dir.clone().unwrap_or_default();
        // 分岔的判据是事实里的 `kind`（动作闭集 `answer | approve | deny`），不是
        // 债行载荷：债表只问存在性、不读载荷，动作语义由写方落成具体事实。
        let (outcome, run_args) = match kind.as_str() {
            // 答复原文即那次调用的结果：字符串就是原文，不再套一层 JSON 引号。
            "answer" => (
                finish(answer_text(&answer_value), true, None, &archive_dir).await,
                original_args,
            ),
            // 批准：批准位**只进 args**（活闸门读的就是 `args.approved`），不另开
            // 端口载荷键——载荷键是机制形状，批准是这一次调用的参数值。
            "approve" => {
                let Some(mut args) = original_args.as_object().cloned() else {
                    return PortOutcome {
                        facts: Vec::new(),
                        error: Some((
                            PortErrorClass::NonRetryable,
                            "恢复批准失败：产物里的 args 不是 JSON 对象".to_string(),
                        )),
                        ..PortOutcome::ok()
                    };
                };
                args.insert(KEY_APPROVED.to_string(), serde_json::Value::Bool(true));
                let args = serde_json::Value::Object(args);
                // 批准留痕（D-368）：批准结论先落事实，再执行；风险档从产物载荷原样搬。
                if let Some(draft) = approval_granted_draft(
                    &exec.idempotency_key,
                    &artifact.payload,
                    &call_id,
                    &tool_name,
                ) {
                    exec.emit.emit(vec![draft]).await;
                }
                let outcome = self
                    .run(
                        exec.clone(),
                        PortInput {
                            port: TOOL,
                            payload: json!({
                                KEY_TOOL_NAME: tool_name.clone(),
                                KEY_ARGS: args.clone(),
                                KEY_TOOL_CALL_ID: call_id.clone(),
                            }),
                        },
                    )
                    .await;
                (outcome, args)
            }
            // 拒绝：不执行，按失败收尾（`user_rejected` 是判决词，不是「等用户」）。
            "deny" => {
                let text = match &answer_value {
                    serde_json::Value::Null => "用户拒绝了本次调用，未执行。".to_string(),
                    other => answer_text(other),
                };
                (
                    finish_with(
                        text,
                        false,
                        None,
                        &archive_dir,
                        // 判决词（线上契约，与 `Authz::Denied{detail:"user_rejected"}` 同一词）。
                        Some("user_rejected"),
                        "",
                    )
                    .await,
                    original_args,
                )
            }
            other => {
                return PortOutcome {
                    facts: Vec::new(),
                    error: Some((
                        PortErrorClass::NonRetryable,
                        format!("user.answer 的 kind 不可恢复：{other:?}"),
                    )),
                    ..PortOutcome::ok()
                };
            }
        };
        // 结果按**结果事实**的形状落进产物格：与批派同一个构造点（`land_artifact`），
        // 只是键换成「第二次落格」的那个。
        let Some(result) = outcome.facts.first().cloned() else {
            return outcome;
        };
        let declared = declared_table_of(&exec).await;
        let art = ArtifactCtx {
            key: format!("artifact:{call_id}{RESUME_ARTIFACT_SUFFIX}"),
            produced_by: exec.origin,
            allowed: gate_inputs(&payload).map(|g| g.authorized).unwrap_or(true),
            denied_reason: String::new(),
        };
        let meta = CallMeta {
            tool_name: &tool_name,
            tool_call_id: &call_id,
            args: &run_args,
            declared: declared_of(&declared, &tool_name),
        };
        land_artifact(&exec, &art, &meta, &result).await;
        outcome
    }

    /// 一次只执行**一笔**（D-378）：载荷里的 `call_id` 从 `tool.requested.calls[]` 精确取。
    ///
    /// 不再按批循环；`tool.calls.done` 也不在这里写——收尾是**投影**，由工具插件的
    /// 簿记写方（`round_done`）在该请求每一笔债都有终局事实后重算，执行体中途死掉
    /// 不会让这批永远停在「未完成」。
    async fn run_requested(
        &self,
        exec: PortExecContext,
        payload: serde_json::Value,
    ) -> PortOutcome {
        let Some(call_id) = payload
            .get(KEY_TOOL_CALL_ID)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        else {
            return PortOutcome {
                facts: Vec::new(),
                error: Some((
                    PortErrorClass::NonRetryable,
                    "工具债载荷缺 call_id".to_string(),
                )),
                ..PortOutcome::ok()
            };
        };
        let calls = match read_requested(&exec, &payload) {
            Ok(found) => found,
            Err(error) => {
                return PortOutcome {
                    facts: Vec::new(),
                    error: Some(error),
                    ..PortOutcome::ok()
                }
            }
        };
        let Some(call) = calls.iter().find(|call| call.id == call_id) else {
            return PortOutcome {
                facts: Vec::new(),
                error: Some((
                    PortErrorClass::NonRetryable,
                    format!("tool.requested 清单里没有 call_id={call_id}"),
                )),
                ..PortOutcome::ok()
            };
        };
        let inner = serde_json::json!({
            KEY_TOOL_NAME: call.name.clone(),
            KEY_ARGS: call.arguments.clone(),
            KEY_TOOL_CALL_ID: call.id.clone(),
            KEY_NODE_ID: call.node_id.clone(),
            KEY_REPLY_KEY: call.id.clone(),
        });
        // 模型已判定「不执行」的调用（超限丢弃）：**不派发**，直接以失败收尾。
        // 正文 = 模型写进 `overflow_notice` 的那一句，收据由同一条结果事实派生
        // ⇒ 留痕但不冒充成功；这里就是派发处本身，不在它外面另补一个 if。
        let outcome = match &call.failure {
            Some(failure) => {
                let raw = finish_with(
                    failure.text.clone(),
                    false,
                    None,
                    &exec.scratch_dir.clone().unwrap_or_default(),
                    Some(failure.kind),
                    "",
                )
                .await;
                attach_reply(raw, &inner)
            }
            None => {
                self.run(
                    exec.clone(),
                    PortInput {
                        port: TOOL,
                        payload: inner,
                    },
                )
                .await
            }
        };
        if let Some(error) = outcome.error {
            return PortOutcome {
                authz: outcome.authz.clone(),
                facts: outcome.facts,
                error: Some(error),
                reply: outcome.reply,
            };
        }
        let Some(result) = outcome.facts.first().cloned() else {
            return PortOutcome {
                authz: outcome.authz.clone(),
                facts: Vec::new(),
                error: Some((
                    PortErrorClass::NonRetryable,
                    "工具调用没有产出结果事实".to_string(),
                )),
                reply: outcome.reply,
            };
        };
        // 停等：这一笔未完成，产物卡照落（正文声明「需要人」），但交回第三态「挂起」；
        // 环落 `sent{deadline_ms: null}`，收束只由配对的答复事实（`tool-resume`）来。
        let awaiting_user = result
            .get("needs_user_action")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        // 产物载荷只有一个构造点：逐笔派发也必须走 `land_artifact`（它再调
        // `artifact_payload`），不在这里另装一份。产物键按调用 id（同一请求内唯一）。
        let declared_table = declared_table_of(&exec).await;
        let allowed = gate_inputs(&payload).map(|g| g.authorized).unwrap_or(true);
        let art = ArtifactCtx {
            key: format!("artifact:{}", call.id),
            produced_by: exec.origin,
            allowed,
            denied_reason: if allowed {
                String::new()
            } else {
                "外部执行闸门未授权".to_string()
            },
        };
        let meta = CallMeta {
            tool_name: &call.name,
            tool_call_id: &call.id,
            args: &call.arguments,
            declared: declared_of(&declared_table, &call.name),
        };
        land_artifact(&exec, &art, &meta, &result).await;
        let error = awaiting_user.then(|| {
            (
                PortErrorClass::NeedsHuman,
                "awaiting_user_answer".to_string(),
            )
        });
        PortOutcome {
            authz: outcome.authz.clone(),
            facts: outcome.facts,
            error,
            reply: outcome.reply,
        }
    }

    /// 执行主体：各出口只交回事实，应答承载由 [`attach_reply`] 在唯一出口补齐。
    async fn run(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        // 受理即起算：闸门的 `elapsed` 量的是**本执行体**从拿到这笔债到判定的时长。
        let started_at = std::time::Instant::now();
        let payload = input.payload;
        // 建账专用入口（环的 open 调用）：把调用清单展开成 `request` 草稿后即终局，
        // 不执行、不写 `sent`（下一笔债由环的授权行另开）。
        if payload.get(KEY_OPEN_ONLY).is_some() {
            let drafts = open_drafts(&exec, &payload).await;
            if !drafts.is_empty() {
                exec.emit.emit(drafts).await;
            }
            return PortOutcome {
                facts: Vec::new(),
                error: None,
                ..PortOutcome::ok()
            };
        }
        let tool_name = payload
            .get(KEY_TOOL_NAME)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let tool_call_id = payload
            .get(KEY_TOOL_CALL_ID)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let args = payload
            .get(KEY_ARGS)
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let archive_dir = exec.scratch_dir.clone().unwrap_or_default();
        // 授权专用入口（环的 gate 调用）：只判一次、只落一条授权事实，**不执行、
        // 不写 `sent`**。这是环上唯一的下判点——拒绝的债因此没有 `sent` 可等，
        // 由 `deny-tools` 直接以 `failed{denied}` 关账。
        if payload.get(KEY_AUTHORIZE_ONLY).is_some() {
            let authz = authorize(&payload);
            exec.emit
                .emit(vec![authz_draft(&exec.idempotency_key, &payload, authz)])
                .await;
            return PortOutcome {
                facts: Vec::new(),
                error: None,
                ..PortOutcome::ok()
            };
        }
        // 只落格、不执行：唤醒结果与没跑到的占位结果。
        if payload.get(KEY_LAND_ONLY).is_some() {
            land_only(&exec, &payload).await;
            return PortOutcome {
                facts: Vec::new(),
                error: None,
                ..PortOutcome::ok()
            };
        }
        // 协议失败：判定与措辞都在执行体（DE-048 D-312）。环只搬原始读数与兜底 id。
        if let Some(protocol) = protocol_input(&payload) {
            if let Some(out) = refuse_protocol(protocol, &archive_dir).await {
                return out;
            }
        }

        // 上下文在旧端口单测里可能缺席；授权 / 闸门的拒绝事实不该因缺上下文而丢失，
        // 所以先把上下文解析成 Option，钩子只在有上下文时触发；走到能力调用前再
        // 按原口径响亮失败。
        let request = exec.ctx.clone();
        let hook_parent = request
            .as_ref()
            .and_then(|ctx| ctx.parent())
            .and_then(|w| w.upgrade());
        if let (Some(ctx), Some(parent)) = (request.as_ref(), hook_parent.as_ref()) {
            plugin_info!(
                "tool",
                "[Tool] 请求发起: {} args={}",
                tool_name,
                args_summary(&args, 200)
            );
            // 工具生命周期钩子的入方向：由执行体触发，环不按事件名调钩子（DE-048 D-312）。
            let pre = fire_hook(
                &Some(parent.clone()),
                HookEvent::PreToolUse {
                    tool_name: tool_name.clone(),
                    tool_input: args.clone(),
                },
                ctx.clone(),
            )
            .await;
            if !pre.should_proceed {
                let reason = pre
                    .block_reason
                    .unwrap_or_else(|| "Blocked by pre hook".to_string());
                let text = format!("Blocked: {reason}");
                let why = format!("被 PreToolUse 钩子拦下：{reason}");
                return finish_with(text, false, None, &archive_dir, Some("blocked"), &why).await;
            }
        }

        // 授权只走上面那条 gate 入口（单一判定点）：执行这一笔时不再重判。
        // 通用端口调用可能不带协议读数；名字仍为空时按协议失败拒绝。
        if tool_name.trim().is_empty() {
            return finish_with(
                "Error: 模型未返回有效的工具名称（协议错误）".to_string(),
                false,
                None,
                &archive_dir,
                Some(failure_kind::ERROR),
                "",
            )
            .await;
        }

        // 外部执行闸门：环**不判快慢、不据此拒发**，只把中性输入随债交过来。
        // 「熔断中不发」在这里落地——本执行体自己不开窗，并把拒绝事实写进事实源；
        // 环只认结果事实，不解释原因（DE-039 §6，DE-048 D-311）。
        if let Some(gate) = gate_inputs(&payload) {
            match CircuitBreaker.gate(
                gate.authorized,
                gate.spent_ms,
                gate.requested_ms,
                gate.budget_ms,
                started_at.elapsed().as_millis() as u64,
                LatencyTier::Reflex.budget_ms(),
            ) {
                GateDecision::Allow => {}
                GateDecision::Refuse => {
                    // 验收 1：拒绝**零事件**——只回结果，不落事实。
                    let text = "Refused: 未持外部执行能力（produce.artifact），本次调用未执行";
                    if let (Some(ctx), Some(parent)) = (request.as_ref(), hook_parent.as_ref()) {
                        fire_post_hook(
                            parent,
                            &tool_name,
                            &args,
                            serde_json::json!({ "error": text }),
                            ctx,
                        )
                        .await;
                    }
                    return refused(text, failure_kind::PERMISSION_DENIED);
                }
                GateDecision::Break { reason } => {
                    // 验收 2：熔断必须入格，不允许静默继续。
                    exec.emit
                        .emit(vec![
                            CircuitBreaker.break_fact(&exec.idempotency_key, reason)
                        ])
                        .await;
                    let text = format!("Refused: 外部执行闸门熔断（{reason}），本次调用未执行");
                    if let (Some(ctx), Some(parent)) = (request.as_ref(), hook_parent.as_ref()) {
                        fire_post_hook(
                            parent,
                            &tool_name,
                            &args,
                            serde_json::json!({ "error": text }),
                            ctx,
                        )
                        .await;
                    }
                    return refused(&text, failure_kind::TOOL_UNAVAILABLE);
                }
            }
        }

        let Some(ctx) = request else {
            return finish(
                "Error: No request context".to_string(),
                false,
                None,
                &archive_dir,
            )
            .await;
        };
        let parent = match hook_parent {
            Some(p) => p,
            None => {
                return finish(
                    "Error: No parent plugin".to_string(),
                    false,
                    None,
                    &archive_dir,
                )
                .await
            }
        };

        // 入方向：模型给的**线上名** → 能力名。
        //
        // 判据是「注册表里谁映射到这个名字」，**不是**「把名字反演回去」：
        // `mcp__fs__read` 既可能是 `mcp.fs.read` 的线上形态，也可能本身就是
        // `mcp__fs__read`——字符串分不出这两种，集合可以。
        let tool_visitor = ctx.get(CAPABILITY_VISITOR);
        // 线上名解析用的名字集合（产物富化的行为声明表在 `land_only` 出口另取）。
        let tool_rows = match tool_visitor.as_ref() {
            Some(visitor) => visitor.list(family::TOOL).await,
            None => Vec::new(),
        };
        let known_names: Vec<String> = tool_rows.iter().map(|row| row.name().to_string()).collect();
        let resolved_name = resolve(&tool_name, known_names.iter().map(String::as_str));
        // 并发策略在本执行体（DE-048 D-311）：只读档（`Low`）可同时在途，受端口声明的
        // 上界约束；写侧档（`Medium` / `High`）与查不到风险档的调用独占**本会话**的
        // 执行窗口——写文件、改配置、发消息这类并行会引入顺序不确定。环不再替这里
        // 做任何判定；窗口按会话分格，委派链不会父子互等。
        let risk = resolved_name.as_deref().and_then(|canon| {
            tool_rows
                .iter()
                .find(|r| r.name() == canon)
                .map(|r| r.meta().effective_risk())
        });
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

        // 声明闸（DE-025 D-219）：宿主闸之后、执行窗口之前，按 owner 序问这个工具
        // 声明过的前置判断，先拒绝者胜。判据只认工具自己的 `CapabilityMeta.behaviors`
        // ——本执行体不认识任何工具名。历史交 `reader.events()`（与行执行体同一份）。
        let declared: &'static [&'static str] = resolved_name
            .as_deref()
            .and_then(|canon| {
                tool_rows
                    .iter()
                    .find(|r| r.name() == canon)
                    .map(|r| r.meta().behaviors)
            })
            .unwrap_or(&[]);
        if !declared.is_empty() {
            let facts = exec
                .reader
                .as_ref()
                .map(|reader| reader.events())
                .unwrap_or_default();
            let principal = symbio_plugin_api::principal_of(ctx.get(AGENT_ID).as_deref());
            // 读侧：闸要的结论从**本会话事实切片**折出来（只读 `reader.events()`）。
            // 判的是调用方主体的 `assign.work`，不解析工具参数里的目标——闸与目标是否存在无关。
            let readings =
                symbio_plugin_api::effects::BeforeReadings::from_facts(&facts, &principal);
            if let Some(judged) = symbio_plugin_api::effects::apply_before(
                &symbio_plugin_api::effects::BeforeRequest {
                    tool: &tool_name,
                    call_id: &tool_call_id,
                    args: &args,
                    principal: &principal,
                    declared,
                    facts: &facts,
                    readings,
                },
            ) {
                let text = format!("Refused: {}", judged.reason);
                fire_post_hook(
                    &parent,
                    &tool_name,
                    &args,
                    serde_json::json!({ "error": text }),
                    &ctx,
                )
                .await;
                return refused(&text, judged.kind);
            }
        }

        let session_id = ctx.get(SESSION_ID).unwrap_or_default();
        let window = window_for(&session_id);
        let read_only = matches!(risk, Some(CapabilityRiskLevel::Low));
        let _shared = if read_only {
            Some(window.gate.read().await)
        } else {
            None
        };
        let _exclusive = if read_only {
            None
        } else {
            Some(window.gate.write().await)
        };
        let _slot = if read_only {
            window.slots.acquire().await.ok()
        } else {
            None
        };
        // 执行窗口开始：把 `meta.started_at` 写到**工具卡**上（前端据它显示
        // 「运行中 · Ns」）。放到窗口之后、路由之前——排队等窗口的时间不算执行时长。
        if let Some(node_id) = payload
            .get(KEY_NODE_ID)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            exec.sink
                .emit(ChatNode {
                    id: node_id.to_string(),
                    status: Some(NodeStatus::Streaming),
                    meta: Some(serde_json::json!({
                        "started_at": symbio_plugin_api::clock_now_ms(),
                        "tool_name": tool_name,
                    })),
                    ..Default::default()
                })
                .await;
        }
        let agent_id = ctx.get(AGENT_ID).unwrap_or_default();
        let workdir = ctx.get(WORKDIR).unwrap_or_default();

        let tool_ctx = ctx.fork();
        tool_ctx.set(WORKDIR, workdir);
        tool_ctx.set(AGENT_ID, agent_id);
        tool_ctx.set(SESSION_ID, session_id);
        tool_ctx.set(TOOL_CALL_ID, tool_call_id.clone());
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
                    let text = format!("Error: 工具 {} 被用户中止，未返回结果", tool_name);
                    fire_post_hook(
                        &parent,
                        &tool_name,
                        &args,
                        serde_json::json!({ "error": text }),
                        &ctx,
                    )
                    .await;
                    return finish(text, false, None, &archive_dir).await;
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
                    let text = format!(
                        "Error: 工具 {} 已 {} 秒无任何进展，已强制中断（疑似挂死）",
                        tool_name, TOOL_EXEC_IDLE_TIMEOUT_SECS
                    );
                    fire_post_hook(
                        &parent,
                        &tool_name,
                        &args,
                        serde_json::json!({ "error": text }),
                        &ctx,
                    )
                    .await;
                    return finish(text, false, None, &archive_dir).await;
                }
            }
        };

        match route_result {
            Ok(resp) => match resp {
                PluginPayload::Data(_) => {
                    let data = match resp.get::<serde_json::Value>() {
                        Ok(d) => d,
                        Err(_) => {
                            let text = "Error: Failed to deserialize payload".to_string();
                            fire_post_hook(
                                &parent,
                                &tool_name,
                                &args,
                                serde_json::json!({ "error": text }),
                                &ctx,
                            )
                            .await;
                            return finish(text, false, None, &archive_dir).await;
                        }
                    };
                    let pending = pending_prompt_from(&data);
                    // 活闸门（`approval_gate`）拦下的一笔必须留痕：停等载荷只是结果形态，
                    // 事实面还要有一条与原 `request` 同 id 的授权拒绝（t78）。两类措辞
                    // 都要落：交互档停在卡上（`needs_approval`）、自动档无人可批
                    // （`permission_denied`，它没有 pending 形态，判据在结果 `failure_kind`）。
                    let denial = if pending
                        .as_ref()
                        .and_then(|p| p.get(KEY_FAILURE_KIND))
                        .and_then(|v| v.as_str())
                        == Some(failure_kind::NEEDS_APPROVAL)
                    {
                        Some("needs_approval")
                    } else if data.get(KEY_FAILURE_KIND).and_then(|v| v.as_str())
                        == Some(failure_kind::PERMISSION_DENIED)
                    {
                        Some("permission_denied")
                    } else {
                        None
                    };
                    if let Some(detail) = denial {
                        exec.emit
                            .emit(vec![approval_denied_draft(
                                &exec,
                                &tool_name,
                                &tool_call_id,
                                detail,
                            )])
                            .await;
                    }
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
                    fire_post_hook(
                        &parent,
                        &tool_name,
                        &args,
                        serde_json::json!({ "content": res.clone() }),
                        &ctx,
                    )
                    .await;
                    finish(res, true, pending, &archive_dir).await
                }
                _ => {
                    let text = "Error: Unexpected payload type".to_string();
                    fire_post_hook(
                        &parent,
                        &tool_name,
                        &args,
                        serde_json::json!({ "error": text }),
                        &ctx,
                    )
                    .await;
                    finish(text, false, None, &archive_dir).await
                }
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
                let text = format!("Error: {e}");
                fire_post_hook(
                    &parent,
                    &tool_name,
                    &args,
                    serde_json::json!({ "error": text }),
                    &ctx,
                )
                .await;
                finish(text, false, None, &archive_dir).await
            }
        }
    }
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

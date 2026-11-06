//! model 的债表（数据）：模型接缝欠哪一轮、什么时候欠，全由本插件自报。
//!
//! 两行是**两个输入身份**：
//! - `model-reply`：用户输入事实的 id = 这一轮的身份（`when` 有输入、`unless_id` 该轮尚无终答）；
//! - `model-after-tools`：最近一条工具产物事实的 id = 工具轮之后那一轮的身份
//!   （`when` 有工具请求、`when_no_open` 工具端口无未还债）。
//!
//! ## 输入依赖写进 `when`（不是屏障）
//!
//! 模型执行体要读 `session.config` 的**工具表与系统提示词**；请求缺了它只能发半成品
//! （空工具、空提示词）。所以两行的可派发条件里都带 `when_id(session.config, …)`：
//! 必须是**这一轮** `user.message` 名下的那条 config，上一轮留下的不算。
//!
//! 这不是「等配置就绪」的时序补丁，而是**依赖声明**：行说清楚自己读哪条输入，
//! 环按输入的存在性选债。写方（session 开轮单元）保证 config 一定会落——
//! 空表也落，见 `consume_inbox_turn` 的兜底。
//!
//! `unless_id` 认**种类 + 身份**：终答是独立事实种类（`chat.assistant.final`），
//! 环的 `request` / `sent` / `retry` 因种类不同不误命中——可重试失败后的重试轮
//! 不会被提前跳过。债表永不读载荷。
//!
//! ## 判决时序：`after` 等判决行收束（不是执行体复判）
//!
//! 环的计划是**一次算全、按批派发**：同一计划里分类行与模型行都会被选中，而计划用的是
//! 派发前的事实。**时间差必须由派发顺序解决**——`model-reply` 用 `after` 指向判决行的
//! 债 id（`classify.decide:id=<user.message id>`）：前置债未收束时本行**根本不进计划**
//! （`drive.rs` 的 `SupportSource::rows` 解析 `after`，未收束即跳过），收束后的下一次
//! 计划才轮得到它。
//!
//! 与之并存的语义门是上面那道 `answered_this_turn`（已判 `Answered` ⇒ 不欠）。两者分工：
//! `after` 管「什么时候算账」，门管「该不该欠」。**都不许下沉到 `port.rs` 执行体**：
//! 执行体在发送前再查一次事实，等于给环的记账开后门——请求发不发由它临时决定，而环
//! 以为已派发；两层判据一旦分叉就是最难查的一类错。
//!
//! `after` 只在**本轮分类开关的有效值为真**时声明。会话把 `classify_enabled` 写成
//! 生效值（调用方开关 ∧ 分类插件本轮挂载）：卸载分类插件（t21-E）时该值为 false，
//! 本行不挂 `after`——否则它会等一笔不会出现的债，整轮一个请求都不发。
//!
//! 首响开启时（`opening_enabled` 同样由会话写成生效值），`after` 指向**首响行**
//! 的债 id（`session.opening:id=<user.message id>`）而不是判决行：首响行自己等
//! `classify.escalate`，链是「判决 → 首响 → 生成」，模型不会在「我先看一下」之前抢跑。

use std::sync::Arc;

use serde_json::json;
use symbio_plugin_api::debt::{session_config_id, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{
    EVENT_ARTIFACT_ADDED, EVENT_CHAT_ASSISTANT_FINAL, EVENT_CLASSIFY_ANSWERED,
    EVENT_SESSION_CONFIG, EVENT_TOOL_REQUESTED, EVENT_USER_MESSAGE,
};

use crate::port::MODEL_PORT;

/// 工具端口名（数据，不是类型）：本行只问它「还有没有未还债」，不解释它的语义。
const TOOL_PORT: &str = "tool";

/// 终答事实的身份前缀：与执行体落的那条事实**逐字一致**（`chat.assistant.final:<root_id>`）。
const FINAL_FACT_PREFIX: &str = EVENT_CHAT_ASSISTANT_FINAL;

/// 判决行名（数据）：与 `crates/symbio-plugin-classify/src/decl.rs` 的
/// `CLASSIFY_DECIDE` 同一字面量。跨插件的时序参照只能按字符串约定——本行不 import
/// 兄弟插件的类型，只等它那笔债收束。
const CLASSIFY_DECIDE_ROW: &str = "classify.decide";

/// 首响行名（数据）：与 `crates/symbio-plugin-session/src/opening.rs` 的
/// `SESSION_OPENING` 同一字面量。首响由会话的行经 `compose/wording` 取措辞后落格。
const SESSION_OPENING_ROW: &str = "session.opening";

/// model 的债表（无状态：每轮从请求与事实视图现算）。
pub(crate) struct ModelDebts;

impl DebtSource for ModelDebts {
    fn rows(&self, request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(user_id) = facts.latest_fact_id(EVENT_USER_MESSAGE) else {
            return Vec::new();
        };
        // W2c-1a 门：本轮已落 `classify.answered` 且**有人能说话**（compose 有效）⇒
        // 本轮**不欠**模型往返（终答由会话经 compose 的 `model.assist` 行债生成后落格，
        // 见 D-341）。只认**本轮身份**（事件 id = `classify.answered:<user_id>`，按种类 +
        // 身份比较、不读载荷）：不扫历史轮，也不把别的轮的判决借到这一轮；
        // `classify.escalate` 不触发本门。
        //
        // `compose_enabled` 是会话写下的**有效值**（调用方开关 ∧ compose 债表在场）：
        // 关掉 / 未挂载时为 false——判决说「能直接答」却没人能说话，这一轮必须降级进
        // 工具循环，而不是两个产线都不出手、整轮沉默。
        let config_id = session_config_id(user_id);
        let compose_on = facts
            .fact_payload(EVENT_SESSION_CONFIG, &config_id)
            .and_then(|c| c.get("compose_enabled"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let answered = answered_this_turn(facts, user_id);
        if compose_on && answered {
            return Vec::new();
        }
        // 这一轮的 config 身份 = 由这条 user.message 的 id 派生（写方同源）。
        // 判决时序：本轮分类生效 ⇒ 本行等判决行收束后才可派发（见模块文档）。
        let classify_on = facts
            .fact_payload(EVENT_SESSION_CONFIG, &config_id)
            .and_then(|c| c.get("classify_enabled"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        // 首响开关：只影响 `after` 指向哪一笔前置债（见模块文档）。分类关闭时它不参与，
        // 否则会等一笔永远不会出现的首响债。
        let opening_on = facts
            .fact_payload(EVENT_SESSION_CONFIG, &config_id)
            .and_then(|c| c.get("opening_enabled"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let payload = json!({
            "provider": request.provider,
            "root_id": user_id,
            // 会话主体随驱动请求而来（开面时从 `agent_id` 解析）；不再写死 `"user"`，
            // 与信封归属（`@principal` → `session/*`）同源（D-339）。
            "principal": request.principal,
        });
        let mut reply = DebtSpec::new(
            "model-reply",
            MODEL_PORT,
            format!("model:{user_id}"),
            payload.clone(),
        )
        .when(EVENT_USER_MESSAGE)
        .when_id(EVENT_SESSION_CONFIG, config_id.clone())
        .unless_id(FINAL_FACT_PREFIX, format!("{FINAL_FACT_PREFIX}:{user_id}"));
        if classify_on {
            // 首响开启 ⇒ 等首响行收束（它自己等 `classify.escalate`）；否则直接等判决行。
            // **answered 且没人能说话**（compose 关掉 / 未挂载）时不等首响：这一轮不会
            // 进入 `escalate`，首响行根本不会触发，等它就是整轮停摆——直接等已收束的
            // 判决行，让模型接管（t22-C/D 的「降级进工具循环」）。
            // 前置债 id 两种判决路径共用同一个 id（见 classify/debt.rs）。
            let prerequisite = if opening_on && !answered {
                format!("{SESSION_OPENING_ROW}:id={user_id}")
            } else {
                format!("{CLASSIFY_DECIDE_ROW}:id={user_id}")
            };
            reply = reply.after(prerequisite);
        }
        let mut rows = vec![reply];
        // 工具轮结束后的续接：身份来自最近一条工具产物，没有产物就不欠。
        // `when` 认 `tool.requested`：没有这一轮的工具请求，`when_no_open` 会在
        // 「根本没有工具债」时恒成立，每条响应都想再派一次。
        // 续接请求同样读本轮 config，因此依赖同一条身份。
        if let Some(tool_id) = facts.latest_fact_id(EVENT_ARTIFACT_ADDED) {
            rows.push(
                DebtSpec::new(
                    "model-after-tools",
                    MODEL_PORT,
                    format!("model:after:{tool_id}"),
                    payload,
                )
                .when(EVENT_TOOL_REQUESTED)
                .when_id(EVENT_SESSION_CONFIG, config_id)
                .when_no_open(TOOL_PORT),
            );
        }
        rows
    }
}

/// 本轮是否已有「可直接回答」的判决。
///
/// 只比较**事件身份**（`classify.answered:<user_id>`），不读载荷——判决事实的身份
/// 就是本轮 `user.message` 的 id，和本行 `unless_id` 的认法同源。
fn answered_this_turn(facts: &DebtFacts, user_id: &str) -> bool {
    facts
        .facts_of(EVENT_CLASSIFY_ANSWERED)
        .iter()
        .any(|(id, _)| id == &format!("{EVENT_CLASSIFY_ANSWERED}:{user_id}"))
}

/// 把两行债挂进 `debt` 族（构建 / 能力收集机制，不是新注册面）。
pub(crate) fn model_debt_rows() -> Vec<symbio_plugin_api::CapabilityRow> {
    vec![symbio_plugin_api::debt::debt_row(
        "model",
        Arc::new(ModelDebts) as Arc<dyn DebtSource>,
    )]
}

#[cfg(test)]
#[path = "debt.test.rs"]
mod tests;

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
//! ## 判决时序：`after_port(plugin_row)`（只认端口，不认行名）
//!
//! 环的计划是**一次算全、按批派发**：同一计划里分类行、首响行、终答行与模型行都会
//! 被选中，而计划用的是**派发前**的事实。时间差只能由**债状态**解决——`model-reply`
//! 声明 `after_port(plugin_row)`：该端口这一圈还有人认领（分类判决 / 首响 / 终答 /
//! 其他行债）时本行**不进计划**；该端口没人认领时，前提**已经了**，本行照常可还。
//!
//! 「有人能说话」不再由本 crate 判断：终答是会话自己申报的一笔 `plugin_row` 债
//! （D2）。判决说能答且有人能说话 ⇒ 会话认领端口，本行等它，终答落格后
//! `unless_id(final)` 放行；说话方缺席 / 关闭 ⇒ 会话不申报，端口干净，
//! **缺席即收束**，本行直接接管（t21-E / t22-C：不会整轮沉默）。
//!
//! 依赖方只认端口：本 crate 里没有兄弟插件的行名、不拼债 id、不读任何开关 bool。
//!
//! 这与 `when_no_open` 是同一条读面（债状态），方向相反：那个问「端口干净才欠」，
//! 这个问「端口干净才可还」。门本身有租约（`plugin_row` 行 60s / 1 次）：认领了却
//! 收不了束的，租约到期会被判死；环也绝不在门没开时把这一轮报成 `NoDebt`。
//!
//! 执行体**不许**在发送前再查一次事实：请求发不发由执行体临时决定而环以为已派发，
//! 两层判据一旦分叉就是最难查的一类错。

use std::sync::Arc;

use serde_json::json;
use symbio_plugin_api::debt::{session_config_id, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{
    EVENT_ARTIFACT_ADDED, EVENT_CHAT_ASSISTANT_FINAL, EVENT_SESSION_CONFIG, EVENT_TOOL_REQUESTED,
    EVENT_USER_MESSAGE,
};
use symbio_plugin_api::rows::PLUGIN_ROW;

use crate::port::MODEL_PORT;

/// 工具端口名（数据，不是类型）：名字住 `plugin-api` 的词汇面（D-364），
/// 提供者 `tool` 与本行共用同一常量。
use symbio_plugin_api::vocab::PORT_TOOL as TOOL_PORT;

/// 终答事实的身份前缀：与执行体落的那条事实**逐字一致**（`chat.assistant.final:<root_id>`）。
const FINAL_FACT_PREFIX: &str = EVENT_CHAT_ASSISTANT_FINAL;

/// model 的债表（无状态：每轮从请求与事实视图现算）。
pub(crate) struct ModelDebts;

impl DebtSource for ModelDebts {
    fn rows(&self, request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(user_id) = facts.latest_fact_id(EVENT_USER_MESSAGE) else {
            return Vec::new();
        };
        // 这一轮的 config 身份 = 由这条 user.message 的 id 派生（写方同源）。
        // 判决 / 首响 / 终答这类行债都在 `plugin_row` 端口：本行只等**端口**这一圈的
        // 认领，不认行名（见模块文档）。没人认领 ⇒ 前提已了，本行直接可还。
        let config_id = session_config_id(user_id);
        let payload = json!({
            "provider": request.provider,
            "root_id": user_id,
            // 会话主体随驱动请求而来（开面时从 `agent_id` 解析）；不再写死 `"user"`，
            // 与信封归属（`@principal` → `session/*`）同源（D-339）。
            "principal": request.principal,
        });
        let reply = DebtSpec::new(
            "model-reply",
            MODEL_PORT,
            format!("model:{user_id}"),
            payload.clone(),
        )
        .when(EVENT_USER_MESSAGE)
        .when_id(EVENT_SESSION_CONFIG, config_id.clone())
        .unless_id(FINAL_FACT_PREFIX, format!("{FINAL_FACT_PREFIX}:{user_id}"))
        .after_port(PLUGIN_ROW);
        let mut rows = vec![reply];
        // 工具轮结束后的续接：身份来自最近一条工具产物，没有产物就不欠。
        // `when` 认 `tool.requested`：没有这一轮的工具请求，`when_no_open` 会在
        // 「根本没有工具债」时恒成立，每条响应都想再派一次。
        // 续接请求同样读本轮 config，因此依赖同一条身份。
        //
        // 两个门并用，读的是债状态的两面（谓词语义都不变）：
        // - `when_no_open(tool)`：已落账的调用债（在飞 / 挂起）未收束就不欠；
        // - `after_port(tool)`：本圈**有人认领**也不欠。工具债改逐笔后，同一请求
        //   的调用按 `calls[]` 串成派发链，后一笔在前一笔落 `sent` 的那一圈才首次
        //   可派——此时账上还没有它的任何事实，`unresolved()` 看不见它，只靠
        //   `when_no_open` 会让本行与它同圈派发（模型在调用未回时续跑）。
        //   `after_port` 的认领集合含本圈候选，正好补上这个窗口。
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
                .when_no_open(TOOL_PORT)
                .after_port(TOOL_PORT),
            );
        }
        rows
    }
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

//! session 侧的 Actor 登记：把现行 `chat_loop` **显式声明为一行**。
//!
//! ## 这一行就是现行行为，不是新行为
//!
//! ```text
//! ActorSpec { name: "session.reasoner", principal: "", pattern: Reasoner,
//!             budget_ms: 0, scope: Root }
//! ```
//!
//! 四项取值逐条对应现行代码（见 `symbio_core::actor` 模块文档的表）：
//!
//! | 字段 | 现行出处 |
//! |---|---|
//! | `principal = ""` | 占位——登记发生在任何会话存在之前，运行期以 `session_id` 覆盖 |
//! | `pattern = Reasoner` | `chat_loop` 的形态（多轮 LLM + 工具循环） |
//! | `budget_ms = 0` | 现行无墙钟上限（`max_tool_rounds = 0` 即无限轮次） |
//! | `scope = Root` | root 会话（子智能体树里的行由子树各自登记） |
//!
//! ## 登记时机：`run_chat_loop` 入口，幂等
//!
//! 为什么不放在装配期：装配期没有任何会话，"哪一棵树的哪一层"只能由
//! `ctx[PLUGIN_DIR]` 反推——那是**装配器**的知识，不该让 session 反查。
//! 放在 `run_chat_loop` 入口则一切都在手里：`ctx[AGENT_ID]` 决定 [`ActorScope`]，
//! 同 `name` 覆盖保证幂等（第二次进入同一个会话时只是重写同一行）。
//!
//! ## 为什么本模块不改 `chat_loop` 的行为
//!
//! 登记只是"让表里出现一行"；执行器读到它时，取到的
//! `budget_ms` / `scope` 与现行代码自算的值**逐字节相同**（见 [`super::chat_loop`]
//! 入口那两行注释）。因此本模块是**零行为变更**的显式化。

use crate::symbio_core::{
    ActorPattern, ActorScope, ActorSpec, PluginInvokeRequest, PluginInvokeRequestExt, AGENT_ID,
};

/// 现行 `chat_loop` 的名字 —— 表里的稳定标识，`actor/list` 会列出它。
pub const ACTOR_NAME_SESSION_REASONER: &str = "session.reasoner";

/// 现行 `chat_loop` 的预算：`0` = 无墙钟上限（与 `max_tool_rounds = 0` 同构）。
pub const REASONER_BUDGET_MS: u64 = 0;

/// 从请求上下文推断**本层**（断言 A5 的输入）。
///
/// `ctx[AGENT_ID]` 存在 ⇒ 子智能体树；否则 root。
/// 这与现行 `chat_loop` / `entry.rs` 判定 agent 边界的**同一判据**——
/// 不新增概念，只是把既有判定复用一次。
pub fn layer_of(ctx: &dyn PluginInvokeRequest) -> ActorScope {
    match ctx.get(AGENT_ID) {
        Some(id) if !id.trim().is_empty() => ActorScope::SubAgent(id),
        _ => ActorScope::Root,
    }
}

/// 内置模板行 —— 名字 + 本层，`principal` 留空。
///
/// 为什么**不**经 `ActorSource`：那条路给的是"装配期就能定下来的登记方"
/// （如 `actor` 插件在 `build` 期登记判定者）。session 的 reasoner 行不同——
/// 它要**每轮**以真实会话 id 写进表，`ActorSource::register_all` 没有"运行期上下文"
/// 这个参数。硬套 trait 只会把 `principal` 塞进结构体字段再当场改写，
/// 是给抽象让路而不是用它。
fn builtin_template(layer: ActorScope) -> ActorSpec {
    ActorSpec::new(
        ACTOR_NAME_SESSION_REASONER,
        "", // 占位：登记后立刻由 `register_reasoner` 以真实会话 id 覆盖
        ActorPattern::Reasoner,
        REASONER_BUDGET_MS,
        layer,
    )
}

/// 在 `run_chat_loop` 入口登记本层 reasoner 行（幂等），并**以真实会话 id 解析**。
///
/// ## 为什么登记时就解析 `principal`，而不是留给执行器
///
/// `principal` 表的是"**谁在跑**"。表若只存占位符，`actor/list` 看到的就不是
/// 实际在跑的那个 Actor——那张表就成了"登记了什么"而不是"**在跑什么**"，
/// 前者对排障几乎无用（占位符是常量，看不出是哪一路会话）。
///
/// 因此登记与解析在同一处完成：`register` 写进表的就是**本轮生效的行**。
/// 每轮进入都覆盖一次（同 name），于是表里那一行始终是"最近一次运行的 Actor"。
///
/// 失败只记日志：登记失败意味着"表里少一行"，而执行器在表为空时回落到
/// **内置默认行**（取值与本行相同）——会话行为不受影响。把它升级为致命错误
/// 会让"登记表"变成新的单点故障，与 J2（可选即退化）相反。
pub fn register_reasoner(ctx: &dyn PluginInvokeRequest, session_id: &str) {
    let layer = layer_of(ctx);
    let spec = builtin_template(layer.clone()).resolve(session_id);
    if let Err(e) = crate::symbio_core::actor_register(&layer, spec) {
        crate::plugin_warn!("session", "Actor 行登记失败（执行器将回落到内置默认）: {e}");
    }
}

/// 取本轮生效的 reasoner 行。
///
/// 表里没有 ⇒ 内置默认行 —— 这正是 J2：**移除登记行，系统退化为现行形态**。
pub fn resolve_reasoner(ctx: &dyn PluginInvokeRequest, session_id: &str) -> ActorSpec {
    let layer = layer_of(ctx);
    crate::symbio_core::actor_get(ACTOR_NAME_SESSION_REASONER)
        .unwrap_or_else(|| builtin_template(layer.clone()))
        .resolve(session_id)
        .tap_scope(&layer)
}

/// 把登记行的 `scope` 校准到本层。
///
/// 为什么需要：`actor_get` 返回的是**表里的行**（可能是别的层登记的），而执行器
/// 必须按**本层**运行。用 `layer` 覆盖而非信任表里的值——A5 已在登记期拦过，
/// 这里是第二道（运行期）保证，且让"scope 从哪来"只有一处。
trait TapScope {
    fn tap_scope(self, layer: &ActorScope) -> Self;
}

impl TapScope for ActorSpec {
    fn tap_scope(mut self, layer: &ActorScope) -> Self {
        self.scope = layer.clone();
        self
    }
}

#[cfg(test)]
#[path = "actors.test.rs"]
mod tests;

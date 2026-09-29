//! actor 域 —— **主体规格**：一个执行者"以什么身份、按什么模式、在什么预算与范围内"运行。
//!
//! ## 为什么需要它（v2 桥接 B3）
//!
//! 现行 `chat_loop` 里已经隐含了**一个** Actor：它就是
//! `{ pattern: reasoner, budget_ms: 0, scope: root }`——只不过这几项**散落在代码里**：
//! 模式（reasoner）由函数结构暗示、预算由 `SessionConfig` 的 `max_tool_rounds` 承载、
//! 范围由 `ctx[AGENT_ID]` 的存在性决定。于是"再加一个执行者"
//! （检索者 / 判定者 / 验证者）的唯一做法是**往 `chat_loop` 里塞分支**——
//! 每加一个能力就改一次主循环。
//!
//! 本域把那个隐含的 Actor **命名出来**，并允许**登记更多**：
//!
//! ```text
//! 现行：  chat_loop  = 一个大函数（行为写死在里面）
//! B3：    chat_loop  = 一张表 + 一个执行器（行为登记在表里）
//! ```
//!
//! 注意这**不是**重写 `chat_loop`：主循环骨架（闸门 / 单轮收尾 / 出口）本来就是
//! "执行器"，不改；改的只是"参数从哪来"——从散落的配置改为**一行 [`ActorSpec`]**。
//!
//! ## 三条设计边界（与 [`projection`](crate::projection) 域同构）
//!
//! | 边界 | 为什么 |
//! |---|---|
//! | **只命名、不重写**：`chat_loop` 的骨架不动，只把散落参数收进结构 | 一次性重写主循环 = 大 diff 且无行为收益；把"已有的"显式化即可 |
//! | **表住 core、行住插件**：契约在 core，具体 Actor 由各插件登记 | 消费方按名字取用，不必认识产生方（插件独立原则） |
//! | **表为空 = 内置默认**：执行器不依赖表非空 | 这是 J2 的落点——移除全部登记行，系统退化为现行形态 |
//!
//! ## 四字段从哪来（**反推自现行代码，不是凭空设计**）
//!
//! | 字段 | 现行出处 |
//! |---|---|
//! | [`ActorSpec::principal`] | `ctx[SESSION_ID]`（root 会话即 session_id） |
//! | [`ActorSpec::pattern`] | `chat_loop` 实际就是 [`ActorPattern::Reasoner`] |
//! | [`ActorSpec::budget_ms`] | `SessionConfig` 的 `max_tool_rounds` 语义；`0 = 不限` |
//! | [`ActorSpec::scope`] | `ctx[AGENT_ID]` 存在 ⇒ [`ActorScope::SubAgent`] |
//!
//! v2 的第五字段 `capabilities`（7 个权威动作）**第一版不引入**——那些动作在现行
//! 代码里不存在，权限先靠视图层过滤。理由见 `docs/plan/06-落地桥接方案.md` §2.3。
//!
//! ## 与 v2 的对应
//!
//! - 四字段规格 ← v2 [01 §4]（本方案取子集，见 §2.3 的"诚实备注"）
//! - `scope` 与装配层一致（A5）← v2 [01 §7]"权限第零天"

mod registry;
mod spec;

pub use registry::{
    actor_clear, actor_get, actor_list, actor_register, ActorError, ActorSource, ActorSpecSubmit,
};
pub use spec::{ActorPattern, ActorScope, ActorSpec};

/// 测试专用：取全局串行闸并清表，得到互斥的干净状态。
///
/// 进程级表 + 并行测试 = 必然串味（见 [`registry::test_lock`] 的说明）。
/// 所有涉及 Actor 表的测试**必须**以此为起点，否则失败会随机出现。
#[cfg(test)]
pub fn actor_test_guard() -> std::sync::MutexGuard<'static, ()> {
    let g = registry::test_lock();
    actor_clear();
    g
}

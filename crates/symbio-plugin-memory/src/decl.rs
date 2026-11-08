//! 本插件交出的**全部声明**（DE-025 D-241 / D-250 / DE-026 D-263 / D-264 / D-268）。
//!
//! ## 一个 owner，六张表，一处提交
//!
//! 声明表按 owner 分组提交（`assemble_declarations` 按 owner 排序后逐份 `declare`）。
//! 本插件是一个所有者（D-267：记忆一个所有者），因此只有**一次** `submit_declarations!`
//! ——三个能力模块各交自己那几张，装配点在这里：
//!
//! | 模块 | 交什么 | 为什么在它那里 |
//! |---|---|---|
//! | 本模块 | `memory.*` 四格的 `EventDecl` | 词汇一个 owner 一处提交（`EVENTS`） |
//! | [`crate::episodic::decl`] | `memory.encoded` 的事实形状 + 归宿行 | 编码是这条格子的家 |
//! | [`crate::recall::decl`] | 声明视图 `memory.live` | 活集折叠的 owner 是检索这一侧 |
//! | [`crate::semantic::decl`] | `memory.recalled` 的名字 | 写方是语义记忆的收束贡献，它没有第二个读方 ⇒ 格位随写方走（D-264 第 4 条） |
//!
//! 分成两次提交会让同一插件在表里出现两个 owner 身份：`HS-001` 的插件面、`E-009` 的
//! 事件名 owner、`EX-006` 的落点列都按 owner 说话，一处插件两条身份就没有任何一处
//! 说得出「这份声明归谁」。
//!
//! ## 名字与形状分开
//!
//! `memory.encoded` 与 `memory.consolidated` 的**名字与格位**由本模块的 `EVENTS` 交出，
//! 它们都有跨插件生产读者，名字的消费方从清单生成物 `symbio_plugin_api::events` 取；
//! `memory.forgotten` 的名字同样在本模块声明，但除 owner 外没有生产读者（CS-007），
//! 故以本地常量交给本 crate 的写方 / 读方，不进生成物——**事实形状与归宿行**仍在
//! [`crate::episodic::decl`]（编码 / 巩固 / 遗忘是那些格子的家）。写方向借用格位
//! ——本包的巩固执行体（`crate::consolidate`）与技能 / 情景记忆的收束贡献——但名字不因此
//! 换 owner；本 crate 不认识任何兄弟插件。
//!
//! `memory.recalled` 今天没有跨插件读方 ⇒ 不登记 `shared_events`、不进生成物。真有
//! 第二个读方时，动作是把它登记进清单再让读方从生成物取，而不是让读方抄一份字面量
//! （DE-024 D-204）。

use symbio_plugin_api::events::{EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED};
use symbio_plugin_api::{Entity, EventDecl, PluginDeclarations, Verb, PRINCIPAL_USER};

use crate::episodic::decl as episodic;
use crate::recall::decl as recall;
use crate::semantic::decl as semantic;

/// owner —— 与插件目录同形（`crates/symbio-plugin-memory`，D-267：记忆一个所有者），
/// 装配表与贡献次序按它排序。
pub(crate) const OWNER: &str = "crates/symbio-plugin-memory";

/// `memory.forgotten` 的线格式名字。
///
/// 除 owner 外没有生产读者（CS-007 普查）⇒ 不进 `PLUGIN.yml` 的 `shared_events`、
/// 不进生成物；名字与格位仍由本模块的 `EVENTS` 一处声明，消费方从本模块取。
pub(crate) const EVENT_MEMORY_FORGOTTEN: &str = "memory.forgotten";

/// 记忆词汇四格（`memory.*`）。
///
/// 名字与格位都归本插件：`encoded` / `consolidated` / `forgotten` 的写方向借用
/// 格位（本包的巩固执行体、技能与情景记忆的收束贡献），`recalled` 的写方就在本包。
/// 只有 `encoded` / `consolidated` 被 skill 幂等、recall 与 session 的召回可见切片
/// 按名字跨插件读，因此它们登记进本插件 `PLUGIN.yml` 的 `shared_events`；
/// `forgotten` 除 owner 外没有生产读者（CS-007），名字留在本模块的本地常量；
/// `recalled` 同样不进清单（DE-024 D-204）。
///
/// 记忆事实归**认知归属人**，不是跑轮主体：召回视图按 `actor == viewer` 过滤，
/// 记忆要按人看见它（D-339）。
const EVENTS: &[EventDecl] = &[
    EventDecl::new(
        semantic::EVENT_MEMORY_RECALLED,
        (Entity::Memory, Verb::Asserted),
    ),
    EventDecl::new(
        EVENT_MEMORY_CONSOLIDATED,
        (Entity::Memory, Verb::Progressed),
    )
    .owned(PRINCIPAL_USER),
    EventDecl::new(EVENT_MEMORY_ENCODED, (Entity::Memory, Verb::Opened)).owned(PRINCIPAL_USER),
    EventDecl::new(EVENT_MEMORY_FORGOTTEN, (Entity::Memory, Verb::Closed)).owned(PRINCIPAL_USER),
];

// 装配口的提交：宿主链接本 crate 即生效。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: episodic::FACTS,
        views: recall::VIEWS,
        ports: &[],
        actors: Vec::new(),
        rows: episodic::ROWS,
    }
}

//! 本插件交出的**全部声明**（DE-025 D-241 / D-250 / DE-026 D-263 / D-268）。
//!
//! ## 一个 owner，六张表，一处提交
//!
//! 声明表按 owner 分组提交（`assemble_declarations` 按 owner 排序后逐份 `declare`）。
//! 本插件是一个所有者（D-267：记忆一个所有者），因此只有**一次** `submit_declarations!`
//! ——两个能力模块各交自己那几张，装配点在这里：
//!
//! | 模块 | 交什么 | 为什么在它那里 |
//! |---|---|---|
//! | [`crate::episodic::decl`] | `memory.encoded` 的事实形状 + 归宿行 | 编码是这条格子的家 |
//! | [`crate::recall::decl`] | 声明视图 `memory.live` | 活集折叠的 owner 是检索这一侧 |
//!
//! 分成两次提交会让同一插件在表里出现两个 owner 身份：`HS-001` 的插件面、`E-009` 的
//! 事件名 owner、`EX-006` 的落点列都按 owner 说话，一处插件两条身份就没有任何一处
//! 说得出「这份声明归谁」。
//!
//! ## 名字在这里不在这里
//!
//! 事件名（`memory.encoded` / `memory.consolidated` / `memory.forgotten`）的**名字与
//! 格位**由 `loop` 交出（它同时是召回与巩固的读方，`session` 的 stats 也读
//! `memory.encoded`）；本插件是它们的**读方 / 写方**，交的是事实形状与视图。名字从
//! 清单生成物 `symbio_plugin_api::events` 取，本 crate 不认识任何兄弟插件。

use symbio_plugin_api::PluginDeclarations;

use crate::episodic::decl as episodic;
use crate::recall::decl as recall;

/// owner —— 与插件目录同形（`crates/symbio-plugin-memory`，D-267：记忆一个所有者），
/// 装配表与贡献次序按它排序。
pub(crate) const OWNER: &str = "crates/symbio-plugin-memory";

// 装配口的提交：宿主链接本 crate 即生效。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: &[],
        facts: episodic::FACTS,
        views: recall::VIEWS,
        ports: &[],
        actors: Vec::new(),
        rows: episodic::ROWS,
    }
}

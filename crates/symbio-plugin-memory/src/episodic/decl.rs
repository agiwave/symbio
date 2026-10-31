//! 本能力包拥有的事实形状与归宿行（DE-025 D-241 / D-250 / DE-026 D-263）。
//!
//! ## 名字在这里不在这里
//!
//! `memory.encoded` 的**名字与格位**由 loop 交出——它同时是召回投影与巩固的读方，
//! `session` 的 stats 也按这个名字读；本插件是它的**生产写方**，交的是事实形状与
//! 归宿行。名字从清单生成物 `symbio_plugin_api::events` 取，本 crate 不认识任何
//! 兄弟插件，也不依赖 core。
//!
//! `tag` 的取值「经验」「判断」是本能力自己的词汇（七类认知内容里的两类，
//! DE-030）。读侧的巩固域在 loop 按同样的两个字面量分组——两条拼写是否一致由
//! 端到端判据（e2e t29）钉住，与技能包对 `SKILL_TAG` 的处理同一条口径。

use symbio_plugin_api::events::EVENT_MEMORY_ENCODED;
use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{FactDecl, LatencyTier, PluginDeclarations};

/// owner —— 与插件目录同形（`crates/symbio-plugin-memory`，D-267：记忆一个所有者），
/// 装配表与贡献次序按它排序。
pub(crate) const OWNER: &str = "crates/symbio-plugin-memory";

/// 认知内容标签：本轮**用户发言**编成的情景记忆标「经验」（七类，DE-030）。
pub(crate) const MEMORY_TAG: &str = "经验";

/// 认知内容标签：本轮**智能体成功收束的正文**标「判断」。
///
/// `recall:tag=judgment` 的唯一生产写方（DE-036 §3.1）——兜底 / 中止的收束
/// 没有正文，一条都不编（固化失败套路等于把没走通的路写成记忆）。
pub(crate) const JUDGMENT_TAG: &str = "判断";

/// 单条记忆正文的字符上限（截断点）。
///
/// 转写（`messages.json`）已存全文，记忆存**提炼**——两处各存全文就是两份
/// 事实源。与技能编译的 `COMPILE_LIMIT` 同值：触发串与命中判定必须是同一对
/// 函数、同一个上限。
pub(crate) const ENCODE_MAX_CHARS: usize = 400;

pub(crate) const FACTS: &[FactDecl] = &[
    // 情景记忆的事实形状（写方是本能力的收束贡献；`skill_id` / `trigger` 两字段
    // 由技能能力的同格写入贡献，形状随格子走，归这条事实的写族共管——技能包交
    // `memory.encoded` 的形状给这里，是因为**编码**是这条格子的家）。
    FactDecl {
        name: EVENT_MEMORY_ENCODED,
        port: None,
        key: &[],
        expand: &[],
        fields: &[
            "content",
            "tag",
            "generation",
            "vis_scope",
            "skill_id",
            "trigger",
        ],
    },
];

pub(crate) const ROWS: &[Decl] = &[
    // 记忆事实的**归宿行**：`owes` 与 `until` 同源——有编码即闭合，没有则不欠。
    // 每条 `memory.encoded` 都要有一行接住它（`seal` 的 `OrphanFact`），而编码
    // 不是环上的债：它没有「必须发生」的时机，是收束这一刻的贡献。
    Decl {
        name: EVENT_MEMORY_ENCODED,
        prio: 200,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: EVENT_MEMORY_ENCODED,
            key: &[],
            eq: &[],
        },
        until: &[Pattern {
            source: EVENT_MEMORY_ENCODED,
            key: &[],
            eq: &[],
        }],
        unless: &[],
    },
];

// 装配口的提交：宿主链接本 crate 即生效。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: &[],
        facts: FACTS,
        views: &[],
        ports: &[],
        actors: Vec::new(),
        rows: ROWS,
    }
}

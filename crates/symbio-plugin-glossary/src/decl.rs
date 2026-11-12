//! 本插件拥有的名字与格位。
//!
//! 领域事实落在 `Entity::Domain` 扩展位：`core` / `kernel` 不认识「术语表」这个词，
//! 写闸按本插件自己的 `EventDecl` 放行，读侧一律不解释（D-209）。
//!
//! 事实名、工具名、行为名都**由本 crate 自报**：宿主只做收集，不按名字写分支。

use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{Entity, EventDecl, FactDecl, PluginDeclarations, Verb};

/// LLM 可见工具名。
pub(crate) const GLOSSARY_TOOL: &str = "glossary_define";
/// 一条术语定义（`Domain × Opened`）。
pub(crate) const GLOSSARY_DEFINED: &str = "glossary.defined";
/// 归属本插件的领域事实：`glossary.defined`。
pub(crate) const OWNER: &str = "glossary";

/// 工具与行为共用的入参上界（口径只写这一处，两侧不各写一份数字）。
pub(crate) const MAX_TERM_CHARS: usize = 64;
pub(crate) const MAX_DEFINITION_CHARS: usize = 500;

pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::new(
    GLOSSARY_DEFINED,
    (Entity::Domain, Verb::Opened),
)];

pub(crate) const FACTS: &[FactDecl] = &[FactDecl {
    name: GLOSSARY_DEFINED,
    port: None,
    key: &[],
    expand: &[],
    fields: &["term", "definition"],
}];

/// 归宿行：事实的终点就是它自己（与 `notes.created` 同一条惯例）。
///
/// 没有这条声明，`seal` 会把 `glossary.defined` 读成孤儿事实；本行不产生新的派生，
/// 只把「这条事实由本能力负责」写进声明表。
pub(crate) const ROWS: &[Decl] = &[Decl {
    name: GLOSSARY_DEFINED,
    reads: &[],
    expand: None,
    owes: Pattern {
        source: GLOSSARY_DEFINED,
        key: &[],
        eq: &[],
    },
    until: &[Pattern {
        source: GLOSSARY_DEFINED,
        key: &[],
        eq: &[],
    }],
    unless: &[],
}];

/// 本插件的声明面。视图默认私有，故这里不交 `ViewDecl`：术语表的读面是上下文段。
fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: FACTS,
        views: &[],
        ports: &[],
        actors: Vec::new(),
        rows: ROWS,
    }
}

// 装配口的提交：宿主链接本 crate 即生效。本插件不认识也不依赖任何兄弟插件。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

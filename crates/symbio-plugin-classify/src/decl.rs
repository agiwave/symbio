//! 本插件拥有的事件名、事实形状与事实归宿行（W2c-1a）。
//!
//! 判决**落成事实**：规则可判的结论不再只活在执行体的返回值里，而是落一格
//! `classify.answered` / `classify.escalate`。环只认事实，后续的模型债行抑制
//! （下一轮）与收束后的第二问都从这一格读，不需要认识本插件的内部函数。
//!
//! ## 为什么归宿行不派发
//!
//! 实际的债由 [`super::debt`] 的 `DebtSource` 申报（**挂载后才生效**，且带
//! `classify_enabled` 与规则命中的收窄）。`Decl` 是链接期声明，不随挂载状态变化；
//! 若把它写成会开账的行（`owes` 与 `until` 不同源，交给 `submit_row!` 的实现），
//! 环会对**每一条** `user.message` 开账，
//! 停用插件或 `classify_enabled=false` 都拦不住。
//!
//! 所以本行只登记「这两格事实归谁看护」：`owes` 与 `until` 同键（本轮用户输入
//! 的 `id`），满足装载期的 `OrphanFact` / `KeyMismatch` 检查，但不派发。
//!
//! `owes` 指**本插件自己声明的** `classify.answered`，不指 `user.message`：装载期
//! 的存在性检查只认事实 / 视图（`user.message` 是共享事件，没有 `FactDecl`），
//! 指过去会以 `UnknownSource` 拒绝整张声明表。归宿行不派发，`owes` 只表达
//! 「这一格的归宿」，不需要再从别人的事实起账。
use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{Entity, EventDecl, FactDecl, PluginDeclarations, Verb};

/// 判决：可直接回答。载荷 `{ id, reason }`，`id` = 本轮 `user.message` 的事实 id。
pub(crate) const CLASSIFY_ANSWERED: &str = "classify.answered";

/// 判决：交给工具循环。载荷同 [`CLASSIFY_ANSWERED`]。
pub(crate) const CLASSIFY_ESCALATE: &str = "classify.escalate";

/// 行名：规则可判 ⇒ 直接落一条判决事实（`by = plugin_row`，W2c-1a 的唯一产线）。
pub(crate) const CLASSIFY_DECIDE: &str = "classify.decide";

/// owner —— 与目录名同形（`crates/symbio-plugin-classify`）。
pub(crate) const OWNER: &str = "crates/symbio-plugin-classify";

pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::new(CLASSIFY_ANSWERED, (Entity::Verdict, Verb::Asserted)),
    EventDecl::new(CLASSIFY_ESCALATE, (Entity::Verdict, Verb::Asserted)),
];

pub(crate) const FACTS: &[FactDecl] = &[
    FactDecl {
        name: CLASSIFY_ANSWERED,
        port: None,
        key: &[],
        expand: &[],
        fields: &["id", "reason"],
    },
    FactDecl {
        name: CLASSIFY_ESCALATE,
        port: None,
        key: &[],
        expand: &[],
        fields: &["id", "reason"],
    },
];

pub(crate) const ROWS: &[Decl] = &[Decl {
    // 归宿行的名字不参与派发（开不出账，`D-306`），取一个不与行名冲突的名字。
    name: "classify.verdict",
    reads: &[],
    expand: None,
    owes: Pattern {
        source: CLASSIFY_ANSWERED,
        key: &["id"],
        eq: &[],
    },
    until: &[
        Pattern {
            source: CLASSIFY_ANSWERED,
            key: &["id"],
            eq: &[],
        },
        Pattern {
            source: CLASSIFY_ESCALATE,
            key: &["id"],
            eq: &[],
        },
    ],
    unless: &[],
}];

// 装配口的提交：宿主链接本 crate 即生效。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

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

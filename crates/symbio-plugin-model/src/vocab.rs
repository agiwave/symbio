//! 本插件写、别的插件按名字消费的事实词汇。
//!
//! 只登记**跨插件消费**的名字（DE-024 D-197 / D-201）：`tool.requested` 由模型执行体
//! 写（一次一轮一条，载荷 = 本轮调用清单），`plugin-tool` 的债表按这个名字问「有没有
//! 这条事实」，不读载荷。`tool.call.requested` 是逐笔债的标记：同一出口按 `calls[]`
//! 落 N 条，`plugin-tool` 的债表按名字逐条取（D-378）。单消费方的 `tool.calls.done`
//! 归工具插件自己声明，不进这里。
//!
//! 例外是**私有名**：本插件的语音键拒因 `model.speech.denied`（D-390）也只在本表
//! 登记，但它不进 `PLUGIN.yml` 的 `shared_events`——没有第二个插件按名字读它，
//! 会话按格位 + 载荷的语音键认停（`ChatVocab::denies_speech`）。
//!
//! 格位是 `(Scope, Asserted)`：对外是一轮里的普通世界事实，收束与否由读者按自己的
//! 判据决定（债表只问存在性）。语音键拒因同样落在这一格，但带 `StopsTurn` 角色——
//! 它是"这一轮为什么停"。

use symbio_plugin_api::{Entity, EventDecl, FactRole, Verb};

/// 本插件的 owner 标识（声明表用；与清单里的目录同源）。
pub(crate) const OWNER: &str = "crates/symbio-plugin-model";

/// 模型侧的语音键拒因：**本插件私有**（D-390）。
///
/// 拆名前它与工具侧的授权拒因共用 `authz.denied`。拆开后它只描述「本轮终答被
/// 写侧闸拒」——这**是一次停轮**（不落 `final`，轮照样了结），因此带 `StopsTurn`
/// 与 `reason`；会话投影按格位 + `key = chat.assistant.final` 认它。
pub(crate) const EVENT_MODEL_SPEECH_DENIED: &str = "model.speech.denied";

/// 本插件导出的世界事实（跨插件消费的那几条 + 私有语音键拒因）。
pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::host("tool.requested", (Entity::Scope, Verb::Asserted)),
    EventDecl::host("tool.call.requested", (Entity::Scope, Verb::Asserted)),
    EventDecl::with_roles(
        EVENT_MODEL_SPEECH_DENIED,
        (Entity::Scope, Verb::Asserted),
        &[FactRole::StopsTurn],
    ),
];

symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> symbio_plugin_api::PluginDeclarations<'static> {
    symbio_plugin_api::PluginDeclarations {
        events: EVENTS,
        facts: &[],
        views: &[],
        // 端口声明仍在环侧（步二搬走）；本插件先只交事实名。
        ports: &[],
        actors: Vec::new(),
        rows: &[],
    }
}

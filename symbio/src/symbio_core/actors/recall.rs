//! 检索 Translator（`pattern = Translator`）：读视图 → 一格 `memory.recalled` 事实（[plan/01 §9.2](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。

use crate::symbio_core::Event;

/// 检索 Translator（`pattern = Translator` 的 S5 形态，[plan/01 §9.2](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
///
/// **Translator 不创造信息**：召回内容原样来自 `RecallView`（③ 的投影产出），
/// 本体的职责只是把「读视图」落成「一条事实」——`memory.recalled` 事件
/// （`memory × asserted` 格子，带溯源指向触发它的事件）。检索是
/// 「读视图 → 产出事实」，这正是 Actor 定义对 Translator 的要求。
///
/// 载荷三键 `found` / `top` / `degraded` **全是视图自带的**数据（降级也要有
/// 生产读数口——只活在投影外壳或提示词里，就没有一条事件说得出「这轮记忆只显示了一部分」）。
pub struct RecallTranslator;

impl RecallTranslator {
    /// 把召回结果固化为一条 `memory.recalled` 事件。
    ///
    /// `trigger_seq`：触发本次检索的事件 seq（溯源锚——I2：断言类必带溯源）。
    pub fn recalled_event(
        &self,
        view: &crate::symbio_core::view::RecallView,
        trigger_seq: u64,
        // 谁在断言「我召回了这些」——会话主体（`memory.recalled` 的 `actor`）。
        actor: &str,
    ) -> Event {
        let top = view
            .entries
            .first()
            .map(|e| e.content.as_str())
            .unwrap_or("");
        Event::pending(
            format!("recalled-{trigger_seq}"),
            crate::symbio_core::event::EVENT_MEMORY_RECALLED,
            crate::symbio_core::event::Entity::Memory,
            crate::symbio_core::event::Verb::Asserted,
            0,
            actor,
        )
        .with_produced_by(trigger_seq)
        .with_payload(serde_json::json!({
            "found": view.entries.len(),
            "top": top,
            // 本次召回是否只给出**部分结果**。
            // 观测落进事实才有**生产读数口**：降级只活在投影外壳 / 提示词里，就没有
            // 一条事件说得出「这一轮的记忆只显示了一部分」（静默的两种形态之一）。
            "degraded": view.degraded,
        }))
    }
}

/// `memory.recalled` 的**运行时入口**：读视图 → 一格事实（`RecallTranslator` 的一跳封装）。
///
/// 生产侧只准调本函数，**不得在定义域之外提及主体类型**——那是
/// `scripts/no-direct-call-audit.mjs` 的 NDC-001：提及即可持有，持有即可绕过事实源
/// 直连（协作只走事件是 I1 的直接推论）。分工因此清楚：「什么时候把检索事实落格」
/// 是写方（`plugins/session` 收束时）的决定，「怎么把视图变成一格事实」是本域的
/// 决定，两者的缝就是本函数——与生产侧驱动 [`TurnRunner`] 而不逐个持有主体同一形态。
pub fn recalled_event(
    view: &crate::symbio_core::view::RecallView,
    trigger_seq: u64,
    actor: &str,
) -> Event {
    RecallTranslator.recalled_event(view, trigger_seq, actor)
}

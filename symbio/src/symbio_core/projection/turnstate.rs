//! `turnstate` 投影（S1，[plan/05 §4](../../../../docs/plan/05-模块架构.md)：③ 加 `turnstate`）。
//!
//! **projection 参数表的取值**（[plan/01 §8](../../../../docs/plan/01-核心架构.md)
//! `projection = turnstate`），平凡值链的终点：S01 说 `projection` 平凡值是
//! `turnstate`（"只看当前 turn"）——它是**最小可用视图**，后面的 `snapshot` /
//! `checkpoint` 都从这里长出。
//!
//! 纯净性由 [`Projection::new`](super::Projection::new) 的构造约束保证：
//! 形参只有 `&[Event]` / `Timestamp` / `Budget`，拿不到 `Store`（F2 一级强制）。

use super::super::event::{Event, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL};
use super::super::view::{Budget, View};
use super::Projection;

/// 当前 turn 的状态（[`turnstate`] 投影的产出）。
///
/// 「一个 turn 恰好一条收束」是 **N3 的形状**：`final` 与 `fallback` 各自至多一条，
/// 但二者**互斥**才是合法态——一个 turn 既 final 又 fallback = 用户收到两句话。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnState {
    /// 该 turn 是否已开（`turn × opened` 的 `user.message` 已见）。
    pub opened: bool,
    /// 最终答复文本（`chat.assistant.final` 的载荷）。
    pub final_text: Option<String>,
    /// 兜底话术（`chat.assistant.fallback` 的载荷）。
    pub fallback_text: Option<String>,
}

impl TurnState {
    /// 收束是否合法：恰好一条收束事件（final 或 fallback，二选一）。
    ///
    /// 这是 [`crate::symbio_core::invariants`] N3 的投影侧孪生——一个给 CI 报警，
    /// 一个给运行时判断「能不能把答复递给用户」。
    pub fn settled(&self) -> bool {
        self.final_text.is_some() ^ self.fallback_text.is_some()
    }
}

/// `turnstate` 投影：按**入参顺序**扫描事件，取**最后一次**开 turn 后的收束态。
///
/// 确定性（N1）：同一事件序列 ⇒ 逐字节相同的 [`TurnState`]——由纯函数扫描保证。
pub fn turnstate() -> Projection<TurnState> {
    Projection::new(|events: &[Event], _now, _b: Budget| {
        let mut state = TurnState {
            opened: false,
            final_text: None,
            fallback_text: None,
        };
        for e in events {
            match e.kind.as_str() {
                crate::symbio_core::event::EVENT_USER_MESSAGE => {
                    // 新 turn 开启：清空上一轮收束（投影的是**当前** turn，平凡值口径）。
                    state = TurnState {
                        opened: true,
                        final_text: None,
                        fallback_text: None,
                    };
                }
                EVENT_ASSISTANT_FINAL => {
                    state.final_text = text_of(e);
                }
                EVENT_ASSISTANT_FALLBACK => {
                    state.fallback_text = text_of(e);
                }
                _ => {}
            }
        }
        View::ok(state, Budget::new(0, 0))
    })
}

fn text_of(e: &Event) -> Option<String> {
    e.payload
        .get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
}

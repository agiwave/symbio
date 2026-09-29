//! 视图 —— 投影的输出，带「平凡值」语义。
//!
//! ## 为什么输出不是裸 `V`
//!
//! 投影的契约是"**永不失败**"：它是纯函数，输入决定输出，没有 IO 也就没有
//! `Result`。但消费方仍然需要区分一件事：**"折出来是空的"** 与
//! **"这条投影根本没生效"**（未登记 / 走了平凡值）。前者是正当结果，后者是
//! 能力缺失。把两者混成一个空 `Vec` 会让"停用"变成静默失效。
//!
//! [`View`] 因此带一个 `trivial` 标记：`trivial = true` 表示**这是平凡值兜底**，
//! 不是真实投影结果。这对应 v2 的"平凡值必须可区分"（D3：平凡值滥用会让
//! "启用"与"没启用"在外部无法分辨）。
//!
//! ## 双跑逐字节相同（A4）
//!
//! [`View::value`] 必须可 `Serialize`，且**同一份 [`ProjectionInput`](super::ProjectionInput)
//! 双跑得到逐字节相同的结果**。这条性质由投影的签名保证（拿不到时钟/存储），
//! 并由断言目录核对。

use serde::{Deserialize, Serialize};

/// 投影的输出视图。
///
/// `trivial = false` 是正常路径；`trivial = true` 是平凡值兜底
/// （未登记投影 / 显式退化），调用方据此区分"空结果"与"能力缺失"。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View<V> {
    /// 折叠出的值。
    pub value: V,
    /// 是否为平凡值兜底（`true` = 这条投影没生效，`value` 是退化默认）。
    pub trivial: bool,
}

impl<V> View<V> {
    /// 正常视图（`trivial = false`）。
    pub fn new(value: V) -> Self {
        Self {
            value,
            trivial: false,
        }
    }

    /// 平凡值兜底（`trivial = true`）—— 显式标注"这不是真实投影结果"。
    pub fn trivial(value: V) -> Self {
        Self {
            value,
            trivial: true,
        }
    }

    /// 把值映射为另一种类型，**保留 `trivial` 标记**。
    pub fn map<U>(self, f: impl FnOnce(V) -> U) -> View<U> {
        View {
            value: f(self.value),
            trivial: self.trivial,
        }
    }
}

#[cfg(test)]
#[path = "view.test.rs"]
mod tests;

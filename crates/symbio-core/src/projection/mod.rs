//! 投影函数（行为）住这里；`Projection` 构造器是机制，住 `symbio_kernel`
//! （DE-023 D-180 / D-188 C2a）。
//!
//! **实现住 `symbio-sdk`**（D-259）：投影的形状是 `&[Event] → 视图` 的纯折叠，
//! 构造约束（F2）让它拿不到 `Store`——它读的只有内核信封上的 `turn` / `kind` /
//! `entity × verb` 与构造期交进来的词表，不持有也不推进任何容器状态。
//!
//! 本模块只转发，`crate::projection::…` 这条稳定路径不变。域目录不删，是因为它是
//! `core-naming-audit` N-004 的登记面：搬的是住址，不是归属。

pub use symbio_sdk::projection::*;

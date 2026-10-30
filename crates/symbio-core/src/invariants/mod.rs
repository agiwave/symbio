//! 可执行不变量检查（F4）的实现在 `symbio-sdk`（D-259）。
//!
//! 判据的形状是「事件切片 ⇒ 违规清单」的纯函数：输入是数据、输出是数据，跑在导出的
//! 事件序列上也不需要任何运行时。它问的是**网格格子与判据角色**（`turn × opened` 上
//! 的每一条都是开轮），读的是内核信封上的 `turn` 字段——不是要被持有的容器。
//!
//! 本模块只转发，`crate::invariants::…` 与根别名 `crate::check_all` 都不变。
//! 域目录不删，是因为它是 `core-naming-audit` N-004 的登记面。

pub use symbio_sdk::invariants::*;

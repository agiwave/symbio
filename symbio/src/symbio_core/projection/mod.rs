//! projection 域 —— 受约束的**纯投影**：把事实序列折叠成一个视图。
//!
//! ## 为什么需要它（v2 桥接 B2）
//!
//! 现行体系里"把存储折成一个视图"的纯函数散落各处——滑动窗口住
//! `session/chat_session/read.rs`、内容淡化住 `session/context/view.rs`、压缩步骤住
//! `session/context/pipeline.rs`。它们**都能读任意东西**（拿 `&Store`、拿 `Clock`、
//! 甚至 `&mut`），于是：
//!
//! - 回放不确定（同一份存储可能折出不同结果，因为读了时钟/外部状态）；
//! - 改一处不知影响谁；
//! - **无法给新能力提供"平铺测试"的落点**——新能力一来就往大函数里塞代码。
//!
//! 本域给出的**唯一约束**是：投影只能看 [`ProjectionInput`]，只能返回 [`View`]。
//! 拿不到 `&Store` / `&mut` / 时钟 / 主体 —— **这条约束由签名表达，不靠约定**。
//!
//! ## 三条设计边界
//!
//! | 边界 | 为什么 |
//! |---|---|
//! | **只登记、不搬迁**：投影仍住各自插件，只是登记进一张进程级表 | 物理搬迁违反 ADR-039 的「域内落位」；登记表足够提供统一寻址 |
//! | **新增必须经 [`Projection::new`]；既有裸函数不被强制** | 一次性改写全部纯函数 = 大 diff 且无行为收益；新能力从第一天起受约束即可 |
//! | **契约住 core，实现住插件** | 消费方按名字取投影，不必认识产生方（插件独立原则） |
//!
//! ## 平凡值（J2）
//!
//! **没登记任何投影 = 空表，系统照常运行**。取不到某个投影名 = 该能力未接入，
//! 不是错误。这与 `fact` 域的 [`FactSource`](crate::FactSource) 同型。
//!
//! ## 与 v2 的对应
//!
//! - 私有构造签名约束 ← v2 [01 §1.4]（见 [`Projection::new`]）
//! - 双跑逐字节相同 ← v2 C6 / N1（见 [`View`]）

mod registry;
mod view;

pub use registry::{
    projection_has, projection_list, projection_run, Projection, ProjectionError, ProjectionFn,
    ProjectionInput, ProjectionSubmit,
};
pub use view::View;

#[cfg(test)]
mod tests;

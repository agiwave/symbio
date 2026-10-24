//! `present` —— 宿主呈现方言域：后端下发描述，前端零业务知识地渲染。
//!
//! 两个文件是同一条哲学的两种用途：
//!
//! | 文件 | 服务什么 | 形态 |
//! |---|---|---|
//! | [`detail`] | **可编辑**的详情页 / 表单 | `DetailDefinition` 描述字段、预设、动作 |
//! | [`reading`] | **只读**的指标读数 | `ReadingPanel` 描述分区、行、单位、口径 |
//!
//! ## 为什么它是一个域而不是「UI 工具」
//!
//! 这份形状里有**没有任何业务语义**：分区 / 字段 / 标签 / 值 / 口径提示——不含
//! 「会话」「档位」「兜底」任何一个词。所以换一个产者（成本、记忆、任何只读路由）
//! 前端零改动。判据是**消费方数量**（前端通用渲染器 + 全部产者插件），与「它像不像 UI」无关。
//!
//! ## 谁依赖本域（方向：只许别人指向这里）
//!
//! - `capability`（选项族的行载荷就是一份字段描述——表单字段本来就是呈现物）；
//! - 各插件的内容 provider（`vdfs` 的 `ext=form` 节点）。
//!
//! 反方向不存在：本域只依赖 `vdfs` 的错误载荷形状（字段级校验失败），
//! 不知道任何表、任何插件。

mod detail;
mod reading;

pub use detail::{
    DetailAction, DetailBadge, DetailCondition, DetailDefinition, DetailField, DetailOption,
    DetailPreset, DetailPresetSpec, DetailSection, DETAIL_PICK_DIRECTORY,
};
pub use reading::{ReadingBar, ReadingPanel, ReadingRow, ReadingSection, ReadingTone};

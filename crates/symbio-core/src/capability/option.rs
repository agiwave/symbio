//! 选项收集机制（跨插件共享设施）
//!
//! ## 与能力收集机制的关系
//!
//! 会话能力（工具 / 模型服务 / 系统提示词）经 `Registry` +
//! `Plugin::traverse(TRAVERSE_AVAILABLE_TOOLS)` 收集；选项本是**平行第二通道**
//! （经 `OPTION_VISITOR` 键 + `Plugin::traverse(TRAVERSE_AVAILABLE_OPTIONS)` 广播），
//! **写入落位到同一张表**（4e）：选项是 `CapabilityRow::option(order, 字段)` 一行，
//! 读侧是本文件的 [`capability_option_fields`]——广播通道仍各走各的，变的只是
//! 「行住哪张表」：两个专服单族的 trait（`OptionVisitor` 随 4e）退场，收集口只剩
//! `put` / `get` / `list` 三个方法（DE-025）。
//!
//! ```text
//! session ──collect_options(parent, ctx)──▶ parent.traverse(available_options)
//!                                             ├─ session : 工作目录 / 运行模式 / 风险等级 / 心跳
//!                                             ├─ agent   : 智能体选择
//!                                             └─ model   : Model 选择
//! ```
//!
//! ## 本模块只有**读协议**：收集器实现与收集管线都在别处
//!
//! 本域提供一个符号，且它是**多消费方**的：
//!
//! | 符号 | 消费方 | 为什么在这里 |
//! |---|---|---|
//! | [`capability_option_fields`] | session（读产物拼表单） | 选项行住全插件共用的能力表，读法是跨插件协议 |
//!
//! 写侧是 `put(CapabilityRow::option(order, 字段))`（写入者 = agent / model / session
//! 各自在 `traverse` 里注册字段），键 `OPTION_VISITOR` 的值类型是 `Registry`。
//!
//! 端点字面量
//! `TRAVERSE_AVAILABLE_OPTIONS`
//! **不在这里**：它属于 `Plugin::traverse` 的契约，与 `TRAVERSE_AVAILABLE_TOOLS` 同处
//! `plugin::traverse`——同一份协议的两个端点该住在一起。本域只是那条通道的**产出方之一**。
//!
//! 而**收集器实现**落在全表共用的 `DefaultToolVisitor`（`providers/collectors/`）——
//! 它的**写入者是全体插件**（在 `traverse` 里注册字段），安装它的宿主只是其中之一，
//! 故不隶属于任何插件。
//! **收集管线**（`collect_options` / `collect_capabilities`）只有 session 一个消费方，
//! 按「依赖方数量」判据（D-023）下沉到 `crates/symbio-plugin-session/src/capabilities.rs`。
//!
//! ```text
//! 在 core：行词汇 + 读协议（本文件）        ← 写入者、读写者都认
//! 在 providers：全表共用的内存收集器       ← 写入者是全体插件，无宿主归属
//! 在 session：收集管线（collect_options）   ← 只有宿主认
//! ```
//!
//! ## 产物是 `DetailField`，不是自成一体的节点类型
//!
//! 选项（会话输入区下方的可修改项）**就是会话配置表单的字段**，因此产物直接
//! 复用 VDFS 详情方言的 [`DetailField`]
//! ——「候选」「条件显隐/禁用」「子表单」各只有一份实现与一个校验器：产物只有
//! [`DetailField`] 一种形状（独立的 `OptionNode` 节点类型与 `options/list` 端点已随
//! 「会话选项 schema 化」下线，2026-09-23）。
//!
//! ## 为什么不与工具同通道广播，但仍同表
//!
//! 广播通道（`TRAVERSE_AVAILABLE_OPTIONS`，候选来自运行期数据，每会话现算）与
//! 工具广播分开，产物语义也不同（可展示的**数据节点**，不是可调用对象）；但两者
//! 已经不需要两个 trait：表按族分列，`list(family::OPTION)` 只回选项行，
//! 工具协议（`list(Tool)`）根本看不见它们——混装的代价在**载荷类型**上就挡住了，
//! 不靠分 trait。这也是 DE-025 把两个专服单族 trait 收回同一张表的判据。
//!
//! ## 契约
//!
//! - 收集是**广播**：宿主对父插件调一次 `traverse`，所有插件在同一契约下
//!   按 `ctx[PATH] == TRAVERSE_AVAILABLE_OPTIONS` 判定是否贡献；
//! - 收集是**无状态**的：每次调用返回全新收集器（产物是「有哪些字段 + 候选
//!   有哪些」的**声明**，不含任何会话状态）；
//! - 单插件失败只记日志，不中断收集。

use crate::DetailField;
use crate::{family, Registry};

/// 列出已注册的选项字段：按 `order` 升序稳定排序（同号保持注册顺序），`order` 不下发。
///
/// - **同键覆盖**是表语义（行名 = `field.key`，后写者胜、保留先注册槽位）；
/// - 排序是**读侧**的事：`order` 不进产物数组——前端收到的数组序 = 展示序，
///   比「前端按 order 再排一次」是更强的保证。
pub async fn capability_option_fields(visitor: &dyn Registry) -> Vec<DetailField> {
    let mut rows = visitor.list(family::OPTION).await;
    // 稳定排序：`sort_by_key` 是稳定的，order 相同者保持注册顺序（表序 = 广播序）
    rows.sort_by_key(|row| {
        row.option_entry()
            .map(|(order, _)| order)
            .unwrap_or(i32::MAX)
    });
    rows.iter()
        .filter_map(|row| row.option_entry().map(|(_, field)| field.clone()))
        .collect()
}

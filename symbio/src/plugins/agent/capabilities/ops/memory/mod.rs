//! memory 域操作模块
//!
//! 包含 5 个操作：save, retrieve, graph_query, reflect, consolidate
//!
//! - save: 保存/更新 CU（**软删除也用它**：`confidence: 0` → 立即物理删除）
//! - retrieve: 统一检索（结构化过滤 + 语义搜索 + 计数）
//! - graph_query: 图关系搜索
//! - reflect: 反思——把对话经验提炼为持久化认知单元（I-066 v50）
//! - consolidate: 自动遗忘/合并/晋升（周期性后台任务）
//!
//! ## 为什么没有 view_prompt？
//!
//! 系统提示词就是 LLM 能看到的文本，单独一个 op 让 LLM "查看"自己已经看到的
//! 内容是冗余的。系统提示词过多时，应该**主动**在提示词末尾追加"预算告警"段，
//! 让 LLM 主动调用 `memory.save` (confidence:0 软删除) / `memory.consolidate`
//! 来优化，而非用一个 op 让 LLM "查询"。
//!
//! 每个操作通过 `submit_cognition_op!` 宏自注册，无需在此文件中手动注册。

mod consolidate;
mod graph_query;
mod reflect;
mod retrieve;
mod save;

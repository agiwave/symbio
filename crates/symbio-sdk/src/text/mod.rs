//! 字符串安全截断工具的**宿主机制余部**：token 计量与头尾切分（`D-404`）。
//!
//! 纯算法半边（`truncate_chars` / `text_truncate_bytes` / `text_floor_char_boundary`）
//! 已随 `D-486` 迁入 `symbio-contract`（`symbio_contract::merge_kernel` / `symbio_contract::text`）：≥2 个不同 owner 的
//! 生产消费者、纯 std、零 sdk 内部读者。本模块只留**有预算语义的宿主机制**——
//! tokenizer 是「预算的计量基础」，机制归服务层。
//!
//! 同族的**巩固合并算法**（分句 / 合并 + 保真度）只有 memory 的巩固写口一个消费者，
//! 按共享库准入的第一条（≥2 个不同 owner）**一律私有**，住
//! `symbio-plugin-memory/src/consolidate/merge.rs`。

/// Token 计量与头尾切分（预算的计量基础；`D-404`）。
pub mod tokenizer;

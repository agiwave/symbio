---
kind: task
id: T-162-ClassifyOnly快速档路径裁决
title: 令牌闸门的快速档（ClassifyOnly / issue_fast）零生产调用——接线，或显式退役并从 F3 论证划界
status: todo
created: 2026-10-17
track: plan
prio: next
parent: T-156-架构机制化与旧代码清理批
where: symbio/src/symbio_core/adapters/mod.rs · symbio/src/plugins/classify/ · symbio/src/plugins/session/v2_exec.rs
exit: `ClassifyOnly` 令牌有生产调用路径（classify 走令牌闸门）**或**显式退役（成文 ADR 说明为什么快速档不做成令牌闸门、classify 插件为何是正确落点）；两种结论都必须让「快速档不得生成」这条约束有强制力，而不是靠 classify 插件「结构上不注册 Capability」这个巧合；cargo test + clippy + 门禁全绿
signal: `ClassifyOnly` 与 `TokenIssuer::issue_fast()`（adapters/mod.rs:99-115,153-154）只在 adapters/mod.test.rs:47,55 出现，**生产代码零调用**；对照组 `issue_reflex`（RuleOnly）与 `issue_deep`（FullModel）在 v2_exec.rs:1360,1376 有真实调用。实际的分类由独立插件 classify 承担（chat_loop/decide.rs:55 经容器路由 classify/decide），未走令牌闸门——「快速档只允许分类不允许生成」这条约束（PL-001 §10）今天靠的是 classify 插件不注册任何 Capability 这个结构巧合，不是 ClassifyOnly 令牌。adapters 域 README 已诚实标注 CanClassify/CanGenerate 是 `#[cfg(test)]`、无生产接入位
---

## 内容

父任务：[T-156-架构机制化与旧代码清理批](156-架构机制化与旧代码清理批.md)
依据：[PL-001 §10](../004-plan/001-核心架构.md "PL-001-核心架构")（快通道做的是**分类**不是生成；
「只允许分类」由令牌类型强制，**不靠 prompt 里写一句「请不要生成」**）· 判据 J3

**问题**：设计说「快速档 = 只允许分类不允许生成，由 `ClassifyOnly` 令牌类型强制」。
现实是：那个令牌**从未被签发**（`issue_fast()` 零生产调用），分类实际由 `classify` 插件承担，
而它**绕过了令牌闸门**。

这条与 [T-157-ActorSpec-pattern死字段接线](157-ActorSpec-pattern死字段接线.md) 是**同一个病的两面**：
F3 冻结锚点里的两个机制（`pattern` 三模式、四档令牌）都**有数据无行为**。

**但这里有一个需要诚实划界的点**：`classify` 插件**结构上不注册任何 Capability**
（`classify/plugin.rs:18-20`），所以它的工具集里**不可能出现**生成类工具——
「不得生成」这条约束**今天是成立的**，只是它的强制力来自**插件切分**，不来自**令牌类型**。

这正是 PL-012 §0.1.2 说的「切分就是架构的切分线」的一个正面案例：
**用切分实现的约束，比事后过滤更硬。** 所以「接线」未必是唯一正确答案。

## 两条合法出路（必须二选一，不能悬着）

| 出路 | 适用条件 | 代价 |
|---|---|---|
| **A 接线**：让 classify 经 `issue_fast` 拿令牌 | 若将来分类不只在 classify 插件发生（如 session 内的就地分类） | 需要生产接入位，且要证明令牌真的拦住了什么 |
| **B 显式退役**：承认快速档不做成令牌闸门 | 若「分类 = classify 插件」是稳定结构 | 需成文 ADR 从 F3 论证里划界，并把 `ClassifyOnly` / `issue_fast` / `CanClassify` 从契约面收掉 |

**不能接受的现状**：一个冻结契约名躺在 core 里、有测试守着它的编译期形状、
却没有任何生产路径——它是**假的机制感**（读代码的人以为有四档闸门，实际只有两档）。

## 处理过程

- **无论选哪条，都要让约束有强制力**：A 走令牌类型，B 走插件切分（成文并加一条 e2e 断言
  「classify 的工具集恒空」，把这个巧合变成判据）。
- **不要两边都留一半**：既不接线也不退役 = 现状，正是本条要消除的。
- 走 A 时注意：`CanClassify` / `CanGenerate` 今天是 `#[cfg(test)]`，升为生产项会**变宽 core**
  （CS-001 的 defaults/methods 格），须走 CS-002（`core-shape-allow` + 存在的 ADR）。

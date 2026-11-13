---
kind: design
id: crates/symbio-plugin-compact/README
title: 上下文压缩能力包（compact）
status: active
created: 2026-11-13
impl: done
---

# 上下文压缩能力包（compact）

超水位时把本轮之前的上下文压成一份快照，落 `session.consolidated`。名册里它是「上下文
压缩」那一格，形态是**两行债 + 一个端口执行体**——它不认识 session，也不改环。

## 它交什么

| 面 | 交出物 | 谁用 |
|---|---|---|
| 端口族行 | `<port:compact>` → 执行体（读回摘要、落快照） | 环按端口名解析提供者 |
| 债 | `compact-summarize`（`model.assist` 执行）、`compact-write`（本包执行） | 环按行取债、按端口派发 |
| 事实 | `session.consolidated`（`message × progressed`） | 重建面把它折成折叠叶 |
| 工具 / 路由 / 配置 | **无** | — |

**为什么必须两行**：执行体归端口提供方。`model.assist` 的执行体不认识
`session.consolidated`，本包的执行体不调模型；一行做完等于让模型插件写会话世界的事实。

## 口径

- **欠不欠在债源判，不在执行体判**：水位 = `0.7 × provider 上限`（`debt.rs` 的
  `COMPACT_AT`），没超就不欠这两行，执行体压根不会被派发；
- **四谓词全部从本轮事实派生**（禁计数）：`when` 是本轮有终答这一种类、`when_id` 钉死
  本轮那条终答、`unless_id` 钉死「已压缩」、`after` 按债键等上游收束；
- **未超阈值不落事实**：那是「没做」，不是一条领域事实——执行体照常返回成功，环照常还债；
- **失败是产物**：拿不到可信摘要（`model.assist` 失败 / 空转写）⇒ 落一条带原因分类的
  `failed` 事实，读面折出 `Failed` 压缩叶，重试入口可达。不再返回成功、不再让压缩节点
  恒 `Completed`。

快照载荷是 `{ messages, keep_from, split }`：`keep_from` 是首条**未压**事实（切片只覆盖
最旧前缀），缺席时读面回落「快照下标 + 1」并在尾部补本轮终答；`split` 记录这一段被头尾
截断过。

## 已知项

`session.consolidated` 的**声明 owner 是 session、写方是本包**（E-009 / CS-007 待裁）。
因此 `PLUGIN.yml` 里没有 `shared_events`：名字不跨插件导出。

## 关联

- 债的形状与裁决：`docs/003-design/051-机制形状规格.md`（两行债）、`D-336` / `D-402`
- 快照的折叠读法：`crates/symbio-sdk/src/projection/rebuild.rs`
- 长会话的断点与预算：`docs/003-design/035-长会话与断点恢复.md`

---
kind: design
id: crates/symbio-plugin-glossary/README
title: Glossary 插件（项目术语表）
status: active
created: 2026-11-07
impl: done
---

# Glossary 插件（项目术语表）

把「这个项目里某个词到底指什么」变成一条可复用的事实：模型用工具登记术语，之后每一轮的提示词里都带着当前术语表。同一个词再次登记会覆盖旧释义；同样的释义重复登记在执行前被拒。

## 为什么它是能力面，不是玩具

- **模型缺的正是这一层**：项目私有词（如「债」「环」「行」）在公开语料里没有定义，模型只能从上下文猜；记忆召回是模糊的，便签是无键的自由文本——都答不了「这个词在本项目的**准确**释义是什么」。
- **语义由 owner 定**：术语是键，释义是最新值；重复定义有明确判定（同一键 + 同一释义 ⇒ 拒；同一键 + 新释义 ⇒ 覆盖）。这是可机械判定的能力语义，不是一段提示词。
- **读面是每轮注入**：不需要模型主动查表，术语表作为上下文段随提示词送达；上下文成本由段的上限（20 条）兜住。

## 扩展面

| 面 | 实现 | 接入方式 |
|---|---|---|
| 工具 | [`src/tool.rs`](src/tool.rs) 的 `glossary_define` | `Plugin::traverse` 注册 `Capability` |
| 领域事实 | [`src/decl.rs`](src/decl.rs) 的 `glossary.defined`（`Domain × Opened`） | `submit_declarations!` |
| 效果行为 | [`src/behavior.rs`](src/behavior.rs) 的 `glossary_record` | `submit_behavior!` |
| 前置判断 | [`src/behavior.rs`](src/behavior.rs) 的 `ToolBehavior::before`（同样的释义重复登记在执行前拒） | 同一个 `submit_behavior!` |
| 上下文段 | [`src/context.rs`](src/context.rs) 的 `glossary_context` | `submit_context_provider!` |

本插件**不导出跨插件视图**：读面就是上下文注入，没有别的插件要按名字读术语表。工具也**不声明 `tool_namespace`**——两者都会把生成物写进别的 crate，而本能力的边界是「只改自己与生成物」。

## 范围

术语落在**本会话**的事实源里，与便签、任务表同一条范围：换一个会话就是另一张表。跨会话共享术语需要一条持久化读面，本能力不声明它——需要时应该作为新的能力面讨论，而不是在这里塞一个旁路。

## 判据

单元测试钉住三件事：入参校验（空值 / 非字符串 / 超长）、效果（成功才落草稿、幂等键带轮锚）、前置判断（同键同释义拒、同键新释义放行、别的事件名不算数）。上下文段另钉「同键取最新」「按首次定义序」「上限截断」。

端到端见 `e2e/cases/t96-glossary-extension.mjs`：真实 CLI 三轮，读模型请求正文里的术语表段与拒因。

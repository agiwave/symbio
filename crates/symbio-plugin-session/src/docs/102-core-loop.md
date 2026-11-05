---
kind: design
id: DE-102-core-loop
title: 会话核心循环（LLM × 工具）
status: retired
created: 2026-09-16
impl: done
reason: 它写的实现（`turn_exec::drive` / `TurnState` / `chat_loop` / `orchestrator/` / `AbortGuard`）已随环收口退场（D-317 等）：主循环现在是 `symbio-plugin-loop` 的环（行数据由应用交来，环只按行还债）加宿主的驱动，一轮的装配与收尾在会话插件的 `plugin/turn.rs`。正文（主循环结构、幕与内环、节点状态推进）随退场删除，历史在 `git log`。
---

# 会话核心循环（LLM × 工具）

> **已退场。** 它写的实现——前步骤 + `TurnState` + `turn_exec::drive` + `chat_loop::run_chat_loop` +
> `AbortGuard`——已被环收口取代（D-317 起轮的确定身份是 `scope` 键，不再有轮态对象）。
> 本文原有的主循环结构、幕与内环、节点状态推进已删除（退场删除正文的约定见 D-145）——历史在 `git log`。

## 这些事实现在的 owner

| 曾在这里的事实 | 现在的 owner |
|---|---|
| 债的挑选与偿还（原来的幕 / 内环 / 派发） | `crates/symbio-plugin-loop/src/ring/engine.rs`（环只按行还债，行数据由应用交来）；驱动在 `crates/symbio/src/drive.rs` |
| 一轮的装配与收尾 | `crates/symbio-plugin-session/src/plugin/turn.rs` |
| 轮的身份 | 事实源打开时绑定的会话身份（取代 `TurnState`，D-317） |
| 节点状态与流式 | DE-105 |
| 模块拆分 | DE-104 |

---
kind: design
id: symbio/src/plugins/loop/README
title: Loop 插件（单环驱动器）
status: active
created: 2026-10-19
impl: done
---

# Loop 插件（单环驱动器）

一轮对话的**执行引擎**：上下文装配、轮首判决、工具循环、收束与派生事实。它是
该批 批次 6b
从 session 切出的驱动器件，形状由 [D-086](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")
裁决；跨墙通货闭族见 [D-083](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构") §4。

## 它不拥有什么

转写、事实源、压缩熔断、用户补充队列、收件箱、运行态都归 **session**（留守层）。
本插件没有配置文档、没有自有存储、没有常驻任务、没有能力行——它对前端与 LLM
不可见（`hidden`）。它贡献**恰好 1 条 Mount 行**：挂载点就是起始地址，session
经容器把 `run_turn` 派发到它。

## 唯一入口

`loop/<sid>` 上的 `Action("run_turn")`（[`provider.rs`](provider.rs)）。授权三条：

1. 宿主 ctx 上有 `TURN_DISPATCH`（`WIRE=false` 的进程内键，线上不可伪造）；
2. `dispatch.spec.session_id == <sid>`（交付包只对本轮会话有效）；
3. 动作名逐字等于 `run_turn`。

动作体就是**整轮 future**：跑完把结局序列化进 `TurnReport` 交回 session。本插件
不维护任务表、不记轮次、不开通道。

## 两条连接

| 方向 | 通道 | 用途 |
|---|---|---|
| session → loop | `loop/<sid>` `run_turn` + `TurnDispatch` | 触发一轮；`TurnSpec` 纯值 + `facts` / `sink` / `writer` / `abort` 等闭族对象 |
| loop → session | `session/<sid>/_run/<op>`（[`session_port.rs`](session_port.rs)） | 回读历史 / 写转写 / 压缩入格 / 补充取批 / worker 进展 / 恢复等会话侧动作 |

任何第三条连接都要先改 D-086。本插件的 import 根被
[`E-013`](../../../../scripts/plugin-entry-audit.mjs) 钉在闭族内：只准
`crate::symbio_core` + 通用库 + crate 根日志 / 工厂宏，不得引用 `crate::plugins::session`
或任何兄弟插件。

## 卸载 / 停用判据

停用本插件 ⇒ `loop/<sid>` 返回 `NotFound`、不再开新轮；**在飞的轮在有限时间内收敛**
（动作体已 spawned，持有交付包与端口的 `Arc`，存活期长于插件实例）。存储侧
`list` / `stat` / 读转写 / 写收件箱照常。守卫用例在
[`../composite/composite.test.rs`](../composite/composite.test.rs)（真实 `mount_all`
+ 停用 `loop`）。

## 模块分工

入口（`plugin.rs` / `provider.rs`）· 跨墙面（`session_port.rs` / `turn_session.rs`）·
主循环（`chat_loop*` / `loop_rows.rs` / `stage_events.rs` / `turn_runner.rs` / `turn_exec.rs` /
`round_hook.rs`）· 上下文与请求（`context*` / `message_build.rs` / `tokenizer.rs` /
`model_chat.rs`）· 工具与分派（`tools*` / `tool_dispatch.rs`）· 事实与记忆
（`derived_facts.rs` / `memory_events.rs` / `skill_events.rs` / `task_events.rs` /
`resume.rs`）。逐项职责见
[`../session/docs/104-module-layout.md`](../session/docs/104-module-layout.md "DE-104-module-layout")
§1.4，主环结构见 [`../session/docs/102-core-loop.md`](../session/docs/102-core-loop.md "DE-102-core-loop")。

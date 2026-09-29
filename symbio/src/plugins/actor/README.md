# actor 插件

**可选的执行者登记方与内省口**：登记一行 Actor（抢占判定者），并把 `symbio_core` 的 Actor 表暴露为只读查询路由。

## 它解决什么

B3 的 Actor 表（[`symbio_core::actor`](../../symbio_core/actor/mod.rs)）是**进程内**登记机制：行住各自插件，core 收口。但"表里有哪些行""某行长什么样"在**系统外部不可见**——审计者 / 测试 / 调试者无从观察。本插件做两件事：

1. **登记** `session.decider` 行（§2.3 的"抢占判定者"，`budget_ms = 80`）；
2. 给表加一个**只读窗口**（`list` 路由）。

## 为什么第 1 件是 B3 交付判据的证明

> 交付判据 #2：新注册的 Actor 可在**不修改** `chat_loop` 代码的前提下接入。

本插件登记判定者只调 `ActorSource::register_all`；`chat_loop.rs` 里**没有任何一行**提到 `decider` 或本插件名。可用 `git diff -- symbio/src/plugins/session/chat_loop.rs` 直接核对。

## 路由

| 路径 | 用途 |
|---|---|
| `actor/list` | 列出全部 Actor 行：`{ actors: [{ name, principal, pattern, budget_ms, scope }] }` |

> **只读内省口**，**不是** LLM 工具，不进 `available_tools`。

## 三条设计边界

| 边界 | 为什么 |
|---|---|
| **可选**：停用 / 未装配 = 系统照常运行（J2 平凡值） | 它提供的是**可观测性 + 一个演示行**，不是地基。不在 `ASSEMBLY_SUB_AGENT_PLUGINS` 里 |
| **只读**：只登记一行、只查表，不写任何域 | 登记不是"写业务数据"；本插件没有任何业务副作用 |
| **零插件依赖**：只经 `symbio_core` 的 actor API，不 import 任何兄弟插件 | 插件独立原则 |

## 为什么登记在 `build` 而不是 `route`

判定者行**没有请求**也要在表里（它是"这个构建有什么执行者"的陈述，不是"某次请求要干什么"）。放 `build` 则插件一装配就登记；放 `route` 会让"表里有没有这一行"取决于"有没有人调过 `actor/list`"——那是自欺。

装配顺序无关：`actor_register` 是幂等的同 `name` 覆盖。

## 关联

- actor 域：[`symbio_core/actor`](../../symbio_core/actor/mod.rs)
- session 侧 reasoner 行：[`symbio/src/plugins/session/actors.rs`](../session/actors.rs)
- 端到端回归：[`e2e/cases/t20-actor-spec.mjs`](../../../../e2e/cases/t20-actor-spec.mjs)

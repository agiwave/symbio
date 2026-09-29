# projection 插件

**可选的投影内省口**：把 `symbio_core` 里登记过的**纯投影**暴露为只读查询路由，供审计 / 调试 / 测试观察。

## 它解决什么

B2 的投影表（[`symbio_core::Projection`](../../symbio_core/projection/mod.rs)）是**进程内**登记机制：投影住各自插件，编译期登记进 core。但"表里有哪些投影""某个投影跑出什么"在**系统外部不可见**。本插件加一个只读窗口。

消费方（检索者 / 巩固者）**不**经本插件取投影——它们直接调 `symbio_core::projection_run`。本插件纯属**人看的口**，与机制面分离。

## 路由

| 路径 | 用途 |
|---|---|
| `projection/list` | 列出已登记的投影名（排序去重，JSON 数组） |
| `projection/run` | 运行一个投影：入参 `{ name, facts?, at_ms? }`，回 `{ name, view }`（`view` 即 `View<V>`） |

> 两条都是**只读内省口**，**不是** LLM 工具，不进 `available_tools`。

## 三条设计边界

| 边界 | 为什么 |
|---|---|
| **可选**：停用 / 未装配 = 系统照常运行（平凡值） | 它提供的是**可观测性**，不是地基。不在 `ASSEMBLY_SUB_AGENT_PLUGINS` 里 |
| **只读**：只查表 / 跑纯投影，不写任何域 | 投影本身无副作用；本插件只是它的出口 |
| **零插件依赖**：只经 `symbio_core` 投影 API，不 import 任何兄弟插件 | 插件独立原则 |

## 关联

- 投影域：[`symbio_core/projection`](../../symbio_core/projection/mod.rs)
- 投影登记（会话三函数）：[`symbio/src/plugins/session/projections.rs`](../session/projections.rs)
- 端到端回归：[`e2e/cases/t19-projection.mjs`](../../../../e2e/cases/t19-projection.mjs)

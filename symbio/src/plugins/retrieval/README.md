# retrieval 插件

**可选的检索者（S06）**：登记一行检索 Actor，并提供一条只读召回路由（跑 `memory.recall` 投影）。

## 它解决什么

S06「长期记忆与语义检索」要的能力是**跨会话记住东西并能找回来**。B4 用前三个桥头堡把它落地：

| 要什么 | 由谁提供 | 本插件做什么 |
|---|---|---|
| 事实原料 | B1 `FactKind` 的 `memory.*` 格子 | 从磁盘派生（含 `MEMORY.md`） |
| 折叠方式 | B2 投影表（`memory.recall`） | 按名取用，跑一次 |
| 执行者身份 | B3 Actor 表（`session.retrieval`） | 登记一行 |

**四个"否"就是 B4 想验证的**：它若成立，`docs/plan/06-落地桥接方案.md` §5 的"需要改的既有代码 = 0"就不是口号。

## 路由

| 路径 | 用途 |
|---|---|
| `retrieval/list` | 派生事实 → 跑 `memory.recall` → 回 `{ facts, recall, degraded, actor }` |

> **只读内省口**，**不是** LLM 工具，不进 `available_tools`。

`degraded: true` 表示 `memory.recall` 投影**未登记**（能力未接入）——不是错误。

## 三条设计边界

| 边界 | 为什么 |
|---|---|
| **可选**：停用 / 未装配 = 系统照常运行（J2 平凡值） | 它提供的是**召回能力**，不是地基。不在 `ASSEMBLY_SUB_AGENT_PLUGINS` 里 |
| **只读**：只派生事实、跑投影、登记一行，不写任何域 | 检索是**读视图 → 产出事实**；本轮交付读的那半 |
| **零插件依赖**：只经 `symbio_core` 的 fact / projection / actor API，不 import 任何兄弟插件 | 插件独立原则 |

## 为什么只依赖磁盘公开布局，而不复用 `fact_log` 的事实源

`fact_log` 把事实登记进 `CapabilityVisitor`，消费方经 `list_fact_sources` 取用。但那条路要求 `retrieval` **认识登记时序**，且**依赖兄弟插件的运行时产物 = 隐式插件依赖**。因此本插件沿用 `fact_log` 自己的先例——只依赖公开布局（`<root>/session/<id>/messages.json` 与 `MEMORY.md`，见 [`docs/design/vdfs.md`](../../../../docs/design/vdfs.md)），**就地**派生。两者是同一份约定的两个独立消费者。

## `kind` 的线上词形：`memory.encoded`，不是 `memory_encoded`

事实类型在线上**只有一种写法** = v2 网格的 `<实体>.<动词>` = `FactKind::wire()`：
`Serialize` / `Deserialize` 手写委托给它，**不是** `derive` + `snake_case`。

原因见 [`symbio_core/fact`](../../symbio_core/fact/mod.rs) 的 `kind.rs`：两者曾并存，
而**单测测不出来**（两侧都用 `wire()` 比较），只有跨进程的真实载荷才暴露——
本插件的 e2e 第一次跑就撞上了。消费方按 wire 词表匹配即可，不要另写一套映射。

## 平凡值（J2）

| 项 | 平凡值 | 平凡值下 |
|---|---|---|
| 插件 | 不在位 | 无路由；**会话照常** |
| `memory.recall` | 未登记 | `degraded: true`（可区分） |
| `MEMORY.md` | 不存在 / 空 | 不产生 `memory.*` 事实 ⇒ 投影折出平凡值 |

## 文件

| 文件 | 职责 |
|---|---|
| [`derive`](derive.rs) | 从磁盘派生事实（含 `memory.*` 格子）的**纯函数** |
| [`plugin`](plugin.rs) | 插件本体（`Plugin` 实现 + 检索行登记 + `list` 路由） |

## 关联

- 事实信封：[`symbio_core/fact`](../../symbio_core/fact/mod.rs)
- 投影域：[`symbio_core/projection`](../../symbio_core/projection/mod.rs)
- 投影登记（`memory.recall`）：[`symbio/src/plugins/session/projections.rs`](../session/projections.rs)
- 端到端回归：[`e2e/cases/t21-retrieval.mjs`](../../../../e2e/cases/t21-retrieval.mjs)

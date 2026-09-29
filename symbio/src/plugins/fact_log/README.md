# fact_log 插件

**可选的事实日志**：把各域的可观测事实聚合为一条**只读**序列，供新能力（检索、巩固、审计）按统一形状取用。

## 它解决什么

现行体系里「发生过什么」散在各域的私有存储里（会话在 `messages.json`、资源在 VDFS）。
新能力想知道「哪些事实发生过」只能去读别人的存储——那会产生**插件间依赖**，违反本仓的基本原则。
本插件把这件事变成**一条可选的服务**：它从磁盘派生事实、登记事实，消费方经
[`symbio_core::FactSource`](../../symbio_core/fact/envelope.rs) 取用，**谁也不必认识谁**。

## 路由

| 路径 | 用途 |
|---|---|
| `fact_log/list` | 返回当前派生出的事实序列（JSON 数组）。**只读内省口**，供审计 / 调试 / 测试观察 |

> 本插件**只此一条**路由。它的机制面是**事实源登记**（经 `CapabilityVisitor::register_fact_source`，
> 见 [`plugin.rs`](plugin.rs) 的 `traverse`）——那对模型不可见；`list` 是给系统外部的观察窗口。
> **不是** LLM 工具，不进 `available_tools`。

## 三条设计边界

| 边界 | 为什么 |
|---|---|
| **可选**：停用 / 未装配 = 系统照常运行（平凡值） | 它提供的是**增强**，不是地基。不在 `ASSEMBLY_SUB_AGENT_PLUGINS` 里，只有目录存在才挂载 |
| **只读**：只派生 [`Fact`](../../symbio_core/fact/envelope.rs)，不写任何域 | 杜绝「第二份真相」——写入仍走各域原有路径 |
| **零插件依赖**：只读磁盘上的**公开布局**，不 import 任何插件 | 插件独立原则 |

## 事实从哪来

本插件**不订阅**运行时事件，而是**从磁盘派生**：会话目录的布局是公开约定
（`<homedir>/session/<id>/messages.json`，形状 `{"messages":[…]}`，见
[`docs/design/vdfs.md`](../../../../docs/design/vdfs.md)），因此本插件只依赖**约定**，
不依赖 `session` 的代码。派生因此是**纯的**——同一份磁盘 → 同一串事实（`seq` 确定），
可双跑比对。

## 文件

| 文件 | 职责 |
|---|---|
| [`config`](config.rs) | 配置结构（能不能派生 / 派生多少） |
| [`derive`](derive.rs) | 从磁盘派生事实的**纯函数**（本插件的全部机制） |
| [`plugin`](plugin.rs) | 插件本体（`Plugin` 实现 + 事实源登记 + `list` 路由） |

## 关联

- 事实信封与契约：[`symbio_core/fact`](../../symbio_core/fact/mod.rs)
- 端到端回归：[`e2e/cases/t18-fact-log.mjs`](../../../../e2e/cases/t18-fact-log.mjs)

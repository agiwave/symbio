# Symbio 产品愿景与方向

> **文档类型：Explanation（阐述）** — 讲"为什么"与"做成什么"，非操作指引。

> 本文档描述 Symbio 的**产品愿景与演进方向**，基于对当前代码已实现能力的提炼。
> 它与 [README.md](../README.md)（"这是什么 / 能做什么"）和 [architecture/](../architecture/ARCHITECTURE.md)（"怎么实现"）互为补充：
> README 讲现状，本文讲**我们要把它做成什么**。
>
> 早期、未落地的产品设想（曾用名 "Symbiont" 的生信分析学习平台等）已统一归档在 [archive/ideas/](./archive/ideas/)，仅供历史参考，**不代表当前方向**。

---

## 1. 一句话愿景

**Symbio 是一个"能力可组合、认知可沉淀"的 AI Agent 平台**：用一套分形插件机制，把对话、长期记忆、工具与外部集成编排成同一棵可寻址的能力树，让多智能体像搭积木一样协作，并让每次交互沉淀为可复用的认知。

---

## 2. 核心信念（已落地为架构基石）

这些理念已经体现在当前代码中，是后续方向的"地基"：

- **能力即路径**：任何能力都用 `plugin/action` 路径寻址（`agent/chat`、`local/shell`、`model/chat`…）。新增能力不改核心，只加插件。
- **分形自相似**：容器与叶子插件接口完全一致，插件可无限嵌套。复杂系统由简单单元组合而成。
- **机制化而非硬编码**：Agent 的认知（关系、展示、记忆）由"属性认知单元（prop CU）"驱动，新增认知类型无需改核心代码。
- **本地优先 / 平台无关**：核心库 `symbio` 不依赖 UI，可被桌面端、命令行或后端服务复用；敏感数据留在本地。
- **LLM 原生**：递归收集插件树中的工具定义，深度支持 Function Calling，工具发现自动化。

---

## 3. 当前已具备的能力（基石）

| 能力 | 现状 |
| --- | --- |
| 多智能体对话 | `agent/chat` + 7 个种子角色（pm / architect / coder / reviewer / tester / documenter / devops） |
| 统一 LLM 接入 | `model/chat` 内置 4 套协议（OpenAI Chat / OpenAI Responses / Anthropic Messages / Gemini） |
| 长期认知记忆 | `agent` 认知单元存储 + 5 个记忆操作（save / retrieve / graph_query / reflect / consolidate） |
| 工具与集成 | 本地 shell / 文件、Web 请求与搜索、skill、MCP server、Telegram |
| 会话管理 | `session/` 长连接、消息持久化、历史裁剪与压缩 |
| 宿主形态 | Tauri 桌面端（Vue 3）+ `seed_agents` CLI |

---

## 4. 演进方向（规划中，未全部落地）

> 以下为方向性目标，按"与当前架构契合度"排序；具体路线图以 [CHANGELOG.md](../CHANGELOG.md) 与代码为准。

### 4.1 认知体系深化
- **更多认知域落地**：当前 `agent_cognition` 仅 `memory` 域 5 个操作；`reason / learn / plan / metacognition` 为路线图目标，需在保持"机制化"前提下逐步落地。
- **认知质量与去重**：信念衰减、冲突检测、语义去重已初具雏形，需强化为可信的长期记忆。

### 4.2 多智能体协作范式
- 从"单 Agent 对话"走向"可编排的多 Agent 工作流"（角色化 Agent 已就绪，调度与协作原语待完善）。
- 让 `composite` 容器成为真正的"团队"，支持子任务分发与结果汇聚。

### 4.3 能力生态
- **插件市场 / 外部加载**：当前插件与库同 crate 编译；规划支持外部动态加载（`abi_stable` / `wasmtime`），通过配置挂载。
- **MCP 与协议扩展**：MCP 工具已支持动态发现，需扩展更多传输与鉴权能力。

### 4.4 人机协作体验
- 桌面端承担"配置 + 展示"，后端承担逻辑——这一分层已确立，未来强化可视化（会话、认知图谱、工具流）与交互式调试。

### 4.5 可信与可控
- 延续"执行 + 验证"的思路：工具调用的结果可追溯、可验证、可回滚；错误与边界清晰可见。

---

## 5. 不在当前方向内（与早期设想的区别）

为避免与代码脱节，明确**不再**追求早期 "Symbiont" 设想中的：

- 以"生信分析（RNA-seq / Docker 执行 R/Python）"为垂直场景的产品定位；
- 内置特定领域流程模板与"结果三层验证"等产品形态。

这些设想反映了项目早期探索，其**通用理念**（执行 + 记录 + 复用、边做边学）仍被抽象为"工具调用 + 认知沉淀"融入当前架构，但具体形态已不同。完整内容见 [archive/ideas/](./archive/ideas/)。

---

## 6. 相关文档

- [README.md](../README.md) — 项目现状与快速开始
- [architecture/](../architecture/) — 分形架构、运作机制、API 设计
- [development/](../development/) — 开发、编译、插件开发指南
- [archive/](../archive/) — 早期设想、规划草案与历史评审（仅供参考）

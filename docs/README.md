# Symbio 文档中心

欢迎来到 Symbio 文档中心。本文档帮助你快速定位**与当前代码一致**的权威说明。

> **当前形态**：本仓库是 **Rust 核心库 + Tauri (Vue 3) 桌面前端 + CLI 工具**。`tauri/` 桌面端与 `symbio/` 核心库同步演进；历史形态说明统一归档在 `archive/`。

## 文档体系（Diátaxis）

本目录按业界通用的 **Diátaxis** 框架组织，四类文档各有明确目的：

| 类型 | 回答的问题 | 特点 |
| --- | --- | --- |
| **Tutorials（教程）** | 跟着做，学会跑通一个场景 | 以学习者为中心、循序渐进 |
| **How-to guides（操作指南）** | 我想达成 X，怎么做 | 以任务为中心、解决问题 |
| **Reference（参考）** | 某个接口/命令/配置的准确事实 | 客观、完整、结构化 |
| **Explanation（阐述）** | 为什么这样设计 | 讲背景、概念与决策 |

---

## 📘 Tutorials（教程）

* **[Getting Started](./getting-started.md)** — 从零跑通：编译核心库、灌入种子 Agent、启动桌面端、完成一次对话。

## 🛠️ How-to guides（操作指南）

* **[编译与排错](./development/BUILD_GUIDE.md)** — Rust 工具链、核心库与 CLI 编译、依赖要求。
* **[编码规范](./development/DEVELOPMENT_GUIDE.md)** — 错误处理、并发安全、序列化约定。
* **[插件开发实战](./development/PLUGIN_DEVELOPMENT_GUIDE.md)** — 从零编写一个业务插件并接入分形路由树。
* **[项目结构与文件组织](./development/STRUCTURE_GUIDE.md)** — 目录约定、可见性、资源/脚本放置规范。

## 📗 Reference（参考）

* **[API 设计规范](./architecture/API_DESIGN.md)** — `Plugin` Trait、`PluginFrame` / `PluginPayload`、`route()` / `traverse()` 协议、错误码。
* **[运作机制](./architecture/OPERATION_MECHANISM.md)** — 插件初始化、AI 对话工作流、工具发现与调用。
* **[更新日志](./CHANGELOG.md)** — 项目详细功能更新与修复记录。
* **插件自包含文档**（与代码同目录，保持高内聚）：

  | 插件 | 文档位置 |
  | --- | --- |
  | `agent` | [symbio/src/plugins/agent/docs/](./../symbio/src/plugins/agent/docs/)，含 `ARCHITECTURE.md` / `COGNITION.md` / `PRINCIPLES.md` / `TESTING.md` / `PLAN.md` / `ISSUES.md` / `CHANGELOG.md` |
  | `model` | [symbio/src/plugins/model/README.md](./../symbio/src/plugins/model/README.md) |
  | `mcp` | [symbio/src/plugins/mcp/README.md](./../symbio/src/plugins/mcp/README.md) |
  | `session` | [symbio/src/plugins/session/README.md](./../symbio/src/plugins/session/README.md) |
  | `telegram` | [symbio/src/plugins/telegram/README.md](./../symbio/src/plugins/telegram/README.md) |

## 💡 Explanation（阐述）

* **[产品愿景与方向](./VISION.md)** — 我们要把它做成什么（与代码一致，区别于归档区的早期设想）。
* **[核心架构设计](./architecture/ARCHITECTURE.md)** — 分形插件架构、自相似性、能力路由、扁平化实现。

## 🗄️ Archive（归档，仅供参考）

早期产品设想、规划草案与历史评审已统一移入 `archive/`，**不一定与当前代码同步**：

* `archive/ideas/` — 创意与产品向文档（`agi/*` 大模型设想库、`BUSINESS_PLAN.md` 等）。
* `archive/proj/` — 早期项目规划与改进方案。
* `archive/design_docs/HISTORY_AND_REVIEWS.md` — 关键版本里程碑回顾与设计讨论。

---

## 阅读建议

1. **第一次接触**：先读 [Getting Started](./getting-started.md) 跑通，再看 [运作机制](./architecture/OPERATION_MECHANISM.md) 与 [核心架构](./architecture/ARCHITECTURE.md)。
2. **开发新插件**：[插件开发实战](./development/PLUGIN_DEVELOPMENT_GUIDE.md) + [API 设计规范](./architecture/API_DESIGN.md)。
3. **理解 Agent 认知体系**：[agent/docs/COGNITION.md](./../symbio/src/plugins/agent/docs/COGNITION.md)（与代码同源）。
4. **追溯架构演进**：[archive/design_docs/HISTORY_AND_REVIEWS.md](./archive/design_docs/HISTORY_AND_REVIEWS.md) + [CHANGELOG.md](./CHANGELOG.md)。

# Symbio 架构总览

> **文档类型：阐述** — 解释「为什么这样设计」的**架构契约与分层**。
>
> **边界**：本文只画**逻辑分层与契约**（哲学、内核分工、`Plugin` trait、寻址规则）。
> **运行时拓扑**（谁挂在谁下面）以 [SYSTEM_MAP.md](../SYSTEM_MAP.md) 为准；
> **插件 × 挂载点 × 路由 × 工具**的权威清单以 [CURRENT.md](../CURRENT.md) §1 为准；
> 单条决策的**来龙去脉**以 [DECISIONS.md](../DECISIONS.md) 为准——本文不复述。

## 核心设计哲学

Symbio 的设计核心是**分形插件架构 (Fractal Plugin Architecture)**。

### 1. 自相似性 (Self-Similarity)

每个插件都可以作为容器包含子插件，对外暴露的接口完全一致。容器类插件（`home` / `composite`）和业务叶子插件（`local` / `web` / `model` / `agent` …）在接口上是对等的——`Plugin` Trait 只需要实现 `route()` 与 `traverse()` 两个方法。

### 2. 对称通信 (Symmetrical Communication)

通过统一的 `route()` 入口，抹平同步调用、异步流式输出和双向会话的差异；用一个枚举覆盖 3 种载荷：`Empty` / `Data(PluginSerializeData)` / `Session(PluginChannel)`。

### 3. 能力路由 (Capability Routing)

路径即路由。通过 `/` 分隔的字符串（如 `session/chat/send`、`local/content_search`）定位任何插件或具体能力。`traverse()` 与 `route()` 共享同一路径协议。

### 4. 扁平化实现 (Flattened Implementation)

物理代码平铺（`symbio/src/plugins/<name>/`），逻辑层级通过 `Composite` 容器动态维护。

## 核心目录分工 (`symbio/src`)

源码分三层，**权威清单在代码里**（各模块 `//!` 头注释写明自己是什么）：

- **`symbio_core/` — 内核契约层**：`Plugin` / `PluginInvokeRequest` / `PluginPayload`、能力系统
  （`capability*`）、跨端 schema（`schemas/`）、资源访问契约（`vdfs`）、模型契约
  （`model_provider`）、上下文键（`keys`）等。**准入判据是「依赖方数量」，不是「够不够底层」**
  （[ADR-023](../DECISIONS.md)）。
  模块清单见 [`symbio_core/mod.rs`](../../symbio/src/symbio_core/mod.rs)。
- **`plugins/` — 实现层**：所有插件平铺存放，容器与叶子实现同一 `Plugin` trait；
  职责与插件清单见 [`docs/README.md` §模块文档地图](../README.md#模块文档地图)与
  [CURRENT.md](../CURRENT.md) §1（**本文不复制插件表**——手抄一份必然漂移）。
- **`providers/` — 基础设施层**：嵌入推理与 `vdfs_service`（资源落盘的集中实现）。
  实现清单见 [`providers/mod.rs`](../../symbio/src/providers/mod.rs)。

## 核心 Trait 与路由

`Plugin` trait、帧 / 载荷 / 通道的**线上形状**以 [PROTOCOLS.md](./PROTOCOLS.md) 为准；
绝对地址 vs 相对臂的**地址规则**见 [design/plugin-route-address.md](../design/plugin-route-address.md)。
本节只保留一句契约：**容器与叶子实现同一 `Plugin` trait，寻址全凭路径字符串，
容器在 `route()` 里剥离当前层级后转发**；签名与规则表不在此复制。

## 关键设计决策 → 见 DECISIONS.md

「为什么用 `inventory` 静态注册」「为什么 Session 是唯一编排入口」「为什么 Model 支持多协议」
这类问题的答案**只在 [DECISIONS.md](../DECISIONS.md) 各一条 ADR 里**（ADR-007 / ADR-003 / ADR-004），
本文不复述——复述就会在下次改动时漏掉一处。

---

> **维护原则**：架构变更必须先更新本文档，再改代码。

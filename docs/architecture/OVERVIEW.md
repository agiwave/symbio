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

通过统一的 `route()` 入口，抹平同步调用、异步流式输出和双向会话的差异；用一个枚举覆盖 4 种载荷：`Empty` / `Data(SerializeData)` / `Native(Arc<dyn Any>)` / `Session(PluginChannel)`。

### 3. 能力路由 (Capability Routing)

路径即路由。通过 `/` 分隔的字符串（如 `session/chat/send`、`local/content_search`）定位任何插件或具体能力。`traverse()` 与 `route()` 共享同一路径协议。

### 4. 扁平化实现 (Flattened Implementation)

物理代码平铺（`symbio/src/plugins/<name>/`），逻辑层级通过 `Composite` 容器动态维护。

## 核心目录分工 (`symbio/src`)

源码分三层，**权威清单在代码里**（各模块 `//!` 头注释写明自己是什么）：

- **`symbio_core/` — 内核契约层**：`Plugin` / `InvokeRequest` / `PluginPayload`、能力系统
  （`capability*`）、跨端 schema（`schemas/`）、资源访问契约（`vdfs_provider`）、模型契约
  （`model_provider`）、上下文键（`keys`）等。**准入判据是「依赖方数量」，不是「够不够底层」**
  （[ADR-023](../DECISIONS.md)）。
  模块清单见 [`symbio_core/mod.rs`](../../symbio/src/symbio_core/mod.rs)。
- **`plugins/` — 实现层**：所有插件平铺存放，容器与叶子实现同一 `Plugin` trait；
  职责与插件清单见 [`docs/README.md` §模块文档地图](../README.md#模块文档地图)与
  [CURRENT.md](../CURRENT.md) §1（**本文不复制插件表**——手抄一份必然漂移）。
- **`providers/` — 基础设施层**：嵌入推理与 `vdfs_service`（资源落盘的集中实现）。
  实现清单见 [`providers/mod.rs`](../../symbio/src/providers/mod.rs)。

## 核心 Trait 与路由

### `Plugin` Trait（V3.0 上下文注入版）

```rust
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn meta(&self) -> PluginMeta;

    /// 分形路由入口
    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>)
        -> InvokeResponse<PluginPayload>;

    /// 分形遍历（用于工具发现 / 全树诊断）
    async fn traverse(self: Arc<Self>, path: String, ctx: Arc<dyn InvokeRequest>)
        -> InvokeResponse<PluginPayload>;
}
```

- `ctx: Arc<dyn InvokeRequest>` 是上下文对象，按需提取 `PATH` / `PAYLOAD` / `WORKDIR` / `SESSION_ID` 等
- 容器类插件在 `route()` 内按 `PATH` 剥离当前层级前缀，转发给子插件
- `_root` 等特殊路径可用于查询当前节点的拓扑

### 路由寻址逻辑

1. **检查路径**：容器插件收到 `route` 时，先判断 `PATH` 是否是自己的指令；若是则本地处理
2. **递归路由**：若包含子级前缀，剥离当前层级后转发给对应子插件
3. **叶子执行**：叶子插件在 `route("xxx", …)` 内完成业务
4. **内省**：`_root` 等特殊路径返回当前节点子插件拓扑

## 关键设计决策 → 见 DECISIONS.md

「为什么用 `inventory` 静态注册」「为什么 Session 是唯一编排入口」「为什么 Model 支持多协议」
这类问题的答案**只在 [DECISIONS.md](../DECISIONS.md) 各一条 ADR 里**（ADR-007 / ADR-003 / ADR-004），
本文不复述——复述就会在下次改动时漏掉一处。

## 文档体系约定（下沉原则）

见 [docs/README.md](../README.md) §文档下沉原则与 §文档职责边界——系统级文档只留跨模块内容与引用，
模块细节、单条决策、变更历史各有自己的 owner，本文件不复述。

---

> **维护原则**：架构变更必须先更新本文档，再改代码。

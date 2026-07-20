# Symbio 核心架构设计

> **文档类型：Explanation（阐述）** — 讲架构"为什么这样设计"。

> 本文档描述**当前代码**对应的架构。
> 早期"Tauri + Vue 前端"叙述的引用已统一迁移到 `docs/archive/design_docs/` 作为历史档案。

## 1. 核心设计理念

Symbio 的设计核心是**分形插件架构 (Fractal Plugin Architecture)**。

* **自相似性 (Self-Similarity)**：每个插件都可以作为容器包含子插件，对外暴露的接口完全一致。
  容器类插件（`home` / `composite`）和业务叶子插件（`local` / `web` / `model` / `agent` …）
  在接口上是对等的——`Plugin` Trait 只需要实现 `route()` 与 `traverse()` 两个方法。
* **对称通信 (Symmetrical Communication)**：通过统一的 `route()` 入口，抹平同步调用、
  异步流式输出和双向会话的差异；`PluginPayload` 用一个枚举覆盖 4 种载荷：
  `Empty` / `Data(SerializeData)` / `Native(Arc<dyn Any>)` / `Session(PluginChannel)`。
* **能力路由 (Capability Routing)**：路径即路由。通过 `/` 分隔的字符串
  （如 `agent/chat`、`local/shell`）定位任何插件或具体能力。
  `traverse()` 与 `route()` 共享同一路径协议。
* **扁平化实现 (Flattened Implementation)**：物理代码平铺
  （`symbio/src/plugins/<name>/`），逻辑层级通过 `Composite` 容器动态维护。
* **机制化 (Mechanismization)**：在 Agent 内部，关系类型与展示行为由 **`prop` CU 驱动**
  （v9 / v9.1），新增关系或认知类型无需改动核心代码（详见 [PRINCIPLES.md](./../symbio/src/plugins/agent/docs/PRINCIPLES.md)）。

## 2. 核心架构层级

```mermaid
graph TD
    subgraph "Host Layer (CLI / Tauri / Web)"
        H[App Entry] --> SR[create_root_plugin]
    end

    subgraph "Object Creator Registry (symbio_core::creator)"
        REG[ObjectCreatorRegistry] -->|submit_object_creator!| C1["create_object(\"home\")"]
        REG --> C2["create_object(\"composite\")"]
        REG --> C3["create_object(\"agent\")"]
        REG --> C4["create_object(\"session\")"]
        REG --> C5["create_object(\"model\")"]
        REG --> C6["create_object(\"local\" / \"web\" / ... )"]
    end

    subgraph "Plugin Tree (Logical Runtime)"
        P1[Home /] --> P2[worker / Composite]
        P1 --> P3[explorer]
        P1 --> P4[setting]
        P2 --> P5[agent]
        P2 --> P6[session]
        P2 --> P7[model]
        P2 --> P8[local / web / skill / mcp / telegram]
        P5 --> P9[MindscapeScaffold]
    end

    SR --> P1
```

## 3. 核心目录分工 (`symbio/src`)

### `symbio_core/` — 内核层

| 模块 | 职责 |
| --- | --- |
| `plugin.rs` | `Plugin` / `InvokeRequest` / `InvokeRequestExt` / `PluginMeta` / `SimpleRequest` 核心契约（V3.0 上下文注入版） |
| `transport.rs` | `PluginFrame` / `PluginPayload` / `PluginChannel` 传输协议 |
| `creator.rs` | 通用对象创建注册表（`submit_object_creator!` 宏、`create_object` / `has_creator`） |
| `error.rs` | 统一 `PluginError` 与稳定错误码 |
| `types.rs` | 流/事件类型（`BoxStream` / `EventResult` / `SystemEvent` / `ToolCall` 等） |
| `capability.rs` | `Capability` / `CapabilityManager` 能力系统 |
| `chat_session.rs` | `ChatSession` / `ChatSessionHandle` 会话抽象 |
| `schemas/` | 跨端数据结构 (Request/Response)，Rust 端定义 |
| `logger.rs` | `init` 日志系统初始化 |
| `keys.rs` | 上下文键（`PATH` / `WORKDIR` / `SESSION_ID` / `TRACE_ID` …） |
| `ids.rs` | 插件 id 常量（`PLUGIN_HOME` 等）与能力/路径常量 |
| `paths.rs` / `homedir.rs` / `system.rs` / `event_bus.rs` / `providers.rs` | 路径解析、主目录、系统门面、事件总线、服务 trait 等 |

### `plugins/` — 实现层

所有插件在 `plugins/` 目录下平铺存放。容器与叶子插件实现同一 `Plugin` Trait。

| 插件 | 角色 |
| --- | --- |
| `home` | **根容器**。持全局配置（`~/.symbio/config.yaml`），挂载 `worker` (Composite) 等顶级实例。 |
| `composite` | **动态容器**。按配置实例化任意子插件，是"分形"的关键——上层配置 `plugins.<name>: { plugin_provider: "..." }` 即可挂载。 |
| `agent` | **认知中心**。管理 Agent 人格、心智流形 (Mindscape)，执行提示词注入；内部以**机制化 (v9)** 方式组织关系与展示。 |
| `session` | **会话中心**。长连接、消息持久化、历史裁剪；是 `session/chat` 路由的入口。 |
| `model` | **Model 引擎**。多协议适配（`openai_chat` / `openai_responses` / `anthropic_messages` / `gemini_api`）、流式编排、工具调用循环。 |
| `local` | 本地工具集：`shell` / `file_read` / `file_write` / `file_edit` / `glob_search` / `content_search`。 |
| `web` | Web 工具集：`http_request` / `web_search` / `web_fetch`。 |
| `skill` | 技能加载与执行。 |
| `mcp` | MCP 桥接：server 注册与 tool 调用。 |
| `telegram` | Telegram 通道。 |
| `explorer` | 文件/目录浏览。 |
| `setting` | 系统级配置读写。 |
| `hook` | 钩子注册与触发。 |
| `event_bus` | 事件总线：进程内帧广播（连接级 SSE 风格推送）。 |

## 4. 核心 Trait 与路由

### 4.1 `Plugin` Trait（V3.0 上下文注入版）

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

* `ctx: Arc<dyn InvokeRequest>` 是上下文对象，按需提取 `PATH` / `PAYLOAD` / `WORKDIR` / `SESSION_ID` 等。
* 容器类插件在 `route()` 内按 `PATH` 剥离当前层级前缀，转发给子插件。
* `_root` 等特殊路径可用于查询当前节点的拓扑。

### 4.2 路由寻址逻辑

1. **检查路径**：容器插件收到 `route` 时，先判断 `PATH` 是否是自己的指令；若是则本地处理。
2. **递归路由**：若包含子级前缀，剥离当前层级后转发给对应子插件。
3. **叶子执行**：叶子插件（如 `local` 的 `shell`）在 `route("shell", …)` 内完成业务。
4. **内省**：`_root` 等特殊路径返回当前节点子插件拓扑。

## 5. 帧协议 `PluginFrame` 与载荷 `PluginPayload`

### 5.1 `PluginFrame`（沿通道发送的最小消息）

```rust
pub enum PluginFrame {
    Data(Value),                  // 业务数据
    Error(String, Option<Value>), // (msg, details)，details 约定含 { "code": "ERR_CODE" }
}
```

### 5.2 `PluginPayload`（`route()` 的响应载荷）

```rust
pub enum PluginPayload {
    Empty,
    Data(SerializeData),                 // 进程内零拷贝；跨进程自动 serde_json
    Native(Arc<dyn Any + Send + Sync>),  // 进程内原生接口
    Session(PluginChannel),              // 全双工 / 流式会话
}
```

`PluginPayload::new(&value)` / `get::<T>()` 是类型化访问入口，
`serialize()` 强制走 JSON（用于跨进程通信）。

## 6. 工具发现（`traverse`）

1. **触发发现**：调用根插件的 `traverse(TRAVERSE_AVAILABLE_TOOLS, …)`。
2. **递归下发**：容器插件将自己的 `path` 前缀下发给所有子插件。
3. **能力暴露**：叶子插件返回 `ToolDefinition` 列表，工具名自动带命名空间（如 `local/shell`）。
4. **精准路由回放**：LLM 调用的工具名（如 `local/shell`）通过 `route("local/shell", …)` 原路返回执行。

## 7. 错误处理契约

* **后端**：`PluginError` 通过 `code()` 提供稳定错误码。
* **通讯**：`PluginFrame::Error` 的 details 字段约定含 `code`。
* **稳定性**：`PluginError` 的 `code()` 字符串是 ABI 的一部分，跨版本禁止随意变更。

## 8. 机制化原则（Agent 子系统）

Agent 插件自 v9 起贯彻**机制化 (Mechanismization)** 原则：

* **关系机制化**：哪些属性名是"关系"由 `prop` CU 决定
  （`RelationPropRegistry::from_prop_cus`），不在核心代码中硬编码关系清单。
* **展示机制化**：`kind` 类型清单、索引优先级由 `prop` CU 的 `is_a` 与 `priority` 派生。
* **类型与展示单一事实来源**：同一份 `seed_cus.jsonl` 同时驱动
  "如何解析 CU" 与 "如何展示 CU"。

> 详见 [PRINCIPLES.md](./../symbio/src/plugins/agent/docs/PRINCIPLES.md) 与
> [COGNITION.md](./../symbio/src/plugins/agent/docs/COGNITION.md)。

## 9. 文档映射约定

* **后端**：`// Corresponding Host: <path>` 注释指向该数据结构在宿主层（CLI / 旧 Tauri 前端）的对应定义。
* **插件自包含**：业务细节文档直接放在 `symbio/src/plugins/<name>/docs/`，不在 `docs/` 集中维护。

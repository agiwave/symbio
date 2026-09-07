# Symbio 架构总览

> **文档类型：阐述** — 解释"为什么这样设计"。

## 核心设计哲学

Symbio 的设计核心是**分形插件架构 (Fractal Plugin Architecture)**。

### 1. 自相似性 (Self-Similarity)

每个插件都可以作为容器包含子插件，对外暴露的接口完全一致。容器类插件（`home` / `composite`）和业务叶子插件（`local` / `web` / `model` / `agent` …）在接口上是对等的——`Plugin` Trait 只需要实现 `route()` 与 `traverse()` 两个方法。

### 2. 对称通信 (Symmetrical Communication)

通过统一的 `route()` 入口，抹平同步调用、异步流式输出和双向会话的差异；用一个枚举覆盖 4 种载荷：`Empty` / `Data(SerializeData)` / `Native(Arc<dyn Any>)` / `Session(PluginChannel)`。

### 3. 能力路由 (Capability Routing)

路径即路由。通过 `/` 分隔的字符串（如 `agent/chat`、`local/shell`）定位任何插件或具体能力。`traverse()` 与 `route()` 共享同一路径协议。

### 4. 扁平化实现 (Flattened Implementation)

物理代码平铺（`symbio/src/plugins/<name>/`），逻辑层级通过 `Composite` 容器动态维护。

### 5. 机制化 (Mechanismization)

在 Agent 内部，关系类型与展示行为由 **`prop` CU 驱动**（v9 / v9.1），新增关系或认知类型无需改动核心代码。

## 核心架构层级

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
        P2 --> P3[gateway]
        P2 --> P4[setting]
        P2 --> P5[agent]
        P2 --> P6[session]
        P2 --> P7[model]
        P2 --> P8[local / web / skill / mcp / telegram]
        P2 --> P10[hook / event_bus]
        P5 --> P9[MindscapeScaffold]
    end

    SR --> P1
```

## 核心目录分工 (`symbio/src`)

### `symbio_core/` — 内核层

| 模块                                                                        | 职责                                                                                    |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| `plugin.rs`                                                               | `Plugin` / `InvokeRequest` / `InvokeRequestExt` / `PluginMeta` / `SimpleRequest` 核心契约 |
| `transport.rs`                                                            | `PluginFrame` / `PluginPayload` / `PluginChannel` 传输协议                                |
| `creator.rs`                                                              | 通用对象创建注册表（`submit_object_creator!` 宏、`create_object` / `has_creator`）                 |
| `error.rs`                                                                | 统一 `PluginError` 与稳定错误码                                                               |
| `types.rs`                                                                | 流/事件类型（`BoxStream` / `EventResult` / `SystemEvent` / `ToolCall` 等）                    |
| `capability.rs`                                                           | `Capability` / `CapabilityManager` 能力系统                                               |
| `chat_session.rs`                                                         | `ChatSession` / `ChatSessionHandle` 会话抽象                                              |
| `entities.rs`                                                             | 统一实体框架：`EntityProvider` trait + list/get/upload/delete/status 公共流程                    |
| `chat_pipeline.rs`                                                        | 会话能力收集管线（`traverse(available_tools)` 统一工具贡献机制）                                        |
| `tools.rs`                                                                | `DefaultToolManager` 默认能力管理器                                                          |
| `schemas/`                                                                | 跨端数据结构 (Request/Response)，Rust 端定义                                                    |
| `logger.rs`                                                               | 日志系统初始化                                                                               |
| `keys.rs`                                                                 | 上下文键（`PATH` / `WORKDIR` / `SESSION_ID` / `TRACE_ID` …）                                |
| `ids.rs`                                                                  | 插件 id 常量（`PLUGIN_HOME` 等）与能力/路径常量                                                     |
| `paths.rs` / `homedir.rs` / `system.rs` / `event_bus.rs` / `providers.rs` | 路径解析、主目录、系统门面、事件总线、服务 trait 等                                                         |

### `plugins/` — 实现层

所有插件在 `plugins/` 目录下平铺存放。容器与叶子插件实现同一 `Plugin` Trait。

| 插件          | 角色           | 关键能力                                                                                                              |
| ----------- | ------------ | ----------------------------------------------------------------------------------------------------------------- |
| `home`      | **根容器**      | 持全局配置（`<homedir>/config.yaml`）；仅挂载 `worker` (Composite)，自身终结 `home/*`、`work/*`、`entities/providers`、`save_config` |
| `composite` | **动态容器**     | 按配置实例化任意子插件，是"分形"的关键                                                                                              |
| `agent`     | **认知中心**     | 管理 Agent 人格；会话选定智能体时经 `traverse` 贡献工具与人格，不再独占会话编排                                                                 |
| `session`   | **会话中心**     | 长连接、消息持久化、历史裁剪；**会话编排的唯一入口**（收集工具、组装提示词、直连 `model/chat`）                                                          |
| `model`     | **Model 引擎** | 多协议适配、流式编排、工具调用循环                                                                                                 |
| `local`     | 本地工具         | shell / file_read / file_write / file_edit / glob_search / content_search                                         |
| `web`       | Web 工具       | http_request / web_search / web_fetch                                                                             |
| `skill`     | 技能           | 加载与执行技能定义                                                                                                         |
| `mcp`       | MCP 桥        | MCP server 注册（stdio / http）与工具调用                                                                                  |
| `telegram`  | Telegram 通道  | 消息收发与人机交互                                                                                                         |
| `gateway`   | **入站网关**     | HTTP/WebSocket 入站适配（与 route_v2 同构），外部客户端接入                                                                        |
| `setting`   | 配置           | 系统级配置读写                                                                                                           |
| `hook`      | 钩子           | 钩子注册与触发                                                                                                           |
| `event_bus` | 事件总线         | 进程内帧广播（连接级 SSE 风格推送）                                                                                              |

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

## 关键设计决策

### 为什么用 `inventory` 静态注册？

每个插件模块在其 `plugin.rs` 末尾调用 `submit_object_creator!(PLUGIN_X, XPlugin::build, dyn Plugin)`。该宏通过 `inventory` 收集构造函数到全局 `ObjectCreatorRegistry`。

**理由**：

1. **零配置**：新增插件无需修改注册代码
2. **编译期保证**：未注册的插件在链接期报错
3. **惰性初始化**：首次使用时才收集

### 为什么 Session 是编排入口？

**历史演进**：早期 `agent` 插件独占会话编排 → 重构后 `session` 成为唯一编排入口。

**理由**：

1. **关注点分离**：Agent 只负责"人格"，Session 负责"对话"
2. **可组合性**：同一 Session 可绑定不同 Agent，或无 Agent 纯工具模式
3. **可测试性**：Session 可独立于 Agent 测试

### 为什么 Model 支持多协议？

```rust
pub enum ModelProtocol {
    OpenAiChat,        // POST /v1/chat/completions
    OpenAiResponses,   // POST /v1/responses
    AnthropicMessages, // POST /v1/messages
    GeminiApi,         // generateContent
}
```

**理由**：供应商无关、协议演进、功能差异适配

## 机制化原则（Agent 子系统）

Agent 插件自 v9 起贯彻**机制化 (Mechanismization)** 原则：

- **关系机制化**：哪些属性名是"关系"由 `prop` CU 决定（`RelationPropRegistry::from_prop_cus`），不在核心代码中硬编码关系清单。
- **展示机制化**：`kind` 类型清单、索引优先级由 `prop` CU 的 `is_a` 与 `priority` 派生。
- **类型与展示单一事实来源**：同一份 `seed_cus.jsonl` 同时驱动 "如何解析 CU" 与 "如何展示 CU"。

## 文档映射约定

- **后端**：`// Corresponding Host: <path>` 注释指向该数据结构在宿主层的对应定义
- **文档集中**：插件不各自维护文档，全部统一在 `docs/`；复杂机制的实现细节见 `docs/design/`（如统一实体管理机制）

---

> **维护原则**：架构变更必须先更新本文档，再改代码。

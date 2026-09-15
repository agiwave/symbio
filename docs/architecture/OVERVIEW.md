# Symbio 架构总览

> **文档类型：阐述** — 解释"为什么这样设计"。

## 核心设计哲学

Symbio 的设计核心是**分形插件架构 (Fractal Plugin Architecture)**。

### 1. 自相似性 (Self-Similarity)

每个插件都可以作为容器包含子插件，对外暴露的接口完全一致。容器类插件（`home` / `composite`）和业务叶子插件（`local` / `web` / `model` / `agent` …）在接口上是对等的——`Plugin` Trait 只需要实现 `route()` 与 `traverse()` 两个方法。

### 2. 对称通信 (Symmetrical Communication)

通过统一的 `route()` 入口，抹平同步调用、异步流式输出和双向会话的差异；用一个枚举覆盖 4 种载荷：`Empty` / `Data(SerializeData)` / `Native(Arc<dyn Any>)` / `Session(PluginChannel)`。

### 3. 能力路由 (Capability Routing)

路径即路由。通过 `/` 分隔的字符串（如 `session/chat/send`、`local/shell`）定位任何插件或具体能力。`traverse()` 与 `route()` 共享同一路径协议。

### 4. 扁平化实现 (Flattened Implementation)

物理代码平铺（`symbio/src/plugins/<name>/`），逻辑层级通过 `Composite` 容器动态维护。

### 5. 机制化 (Mechanismization)

在 Agent 内部，关系类型与展示行为由数据（CU）驱动而非硬编码，新增关系或认知类型无需改动核心代码。详见 `symbio/src/plugins/agent/README.md`。

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
| `turn.rs`                                                                 | 单轮执行机器与流式类型（`execute_post_with_abort` / `parse_sse_stream` / `ToolCallInfo` / emit 辅助） |
| `capability.rs`                                                           | `Capability` / `CapabilityVisitor` 能力系统                                               |
| `capability_error.rs`                                                     | 能力收集期错误通道（写侧=任意 traverse 插件，读侧=session 编排方）                                     |
| `entities.rs`                                                             | 存储原语（自由函数，无 trait）：写盘 / 删除 / 导入 / 导出 + zip / base64 工具                  |
| `tools.rs`                                                                | `DefaultToolVisitor` 默认能力管理器                                                          |
| `schemas/`                                                                | 跨端数据结构 (Request/Response)，Rust 端定义                                                    |
| `logger.rs`                                                               | 日志系统初始化                                                                               |
| `keys.rs`                                                                 | 上下文键（`PATH` / `WORKDIR` / `SESSION_ID` / `TRACE_ID` …）                                |
| `ids.rs`                                                                  | 插件 id 常量（`PLUGIN_HOME` 等）与能力/路径常量                                                     |
| `paths.rs` / `homedir.rs` / `event_bus.rs` / `providers.rs` | 路径常量、主目录、事件总线、服务 trait 等                                                         |

### `plugins/` — 实现层

所有插件在 `plugins/` 目录下平铺存放。容器与叶子插件实现同一 `Plugin` Trait。

| 插件          | 角色           | 关键能力（详见各插件 `plugins/<name>/README.md`）                                                              |
| ----------- | ------------ | ----------------------------------------------------------------------------------------------------------------- |
| `home`      | **根容器**      | 持全局配置（`<homedir>/config.yaml`）；仅挂载 `worker` (Composite)，自身终结 `home/*`、`work/*`、`save_config` |
| `composite` | **动态容器**     | 按配置实例化任意子插件，是"分形"的关键                                                                                              |
| `agent`     | **认知中心**     | 管理 Agent 人格；会话选定智能体时经 `traverse` 贡献工具与人格 → `plugins/agent/README.md`                                     |
| `session`   | **会话中心**     | 长连接、消息持久化、历史裁剪；**会话编排的唯一入口**（收集工具、组装提示词、直连 model 单轮网关）→ `plugins/session/README.md`（含六大压缩策略）                       |
| `model`     | **单轮 LLM 网关** | 无状态单轮执行（`execute_turn`）；按上下文注册唯一生效 `ModelProvider`（自含参数与协议适配器）、4 协议适配、配置存取；不含工具执行与会话循环 |
| `local`     | 本地工具         | shell / file_read / file_write / file_edit / glob_search / content_search                                         |
| `web`       | Web 工具       | http_request / web_search / web_fetch                                                                             |
| `skill`     | 技能           | 加载与执行技能定义（含 `skill/search`）                                                                                       |
| `mcp`       | MCP 桥        | MCP server 注册（stdio / http）与工具调用（资源经 `.vdfs/mcp` 维护）                                                    |
| `telegram`  | Telegram 通道  | 长轮询收发与“继续会话”交互（`telegram/send`）                                                                                  |
| `gateway`   | **入站网关**     | HTTP/WS 入站适配（`/api/v1/invoke`、`/api/v1/ws`、`/api/v1/health`，与 route_v2 同构）                                              |
| `setting`   | 配置           | 系统级配置读写 + `.vdfs/setting` 子目录（`config/get` / `config/set`）                                                     |
| `hook`      | 钩子           | 钩子注册与触发（PreCompact 等生命周期点）                                                                                        |
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

协议适配契约 `ModelProtocol`（钩子：get_api_url / get_headers / prepare_request / parse_response_line / ping / query_context_limit）**内化在 model 插件内部**（`plugins/model/protocols/`），内置 4 个实现：

```text
openai_chat        // POST /v1/chat/completions
openai_responses   // POST /v1/responses
anthropic_messages // POST /v1/messages
gemini_api         // generateContent
```

协议差异被钩子吸收后，model 插件以 `BoundProvider`（配置 + 协议适配器绑定）实现 core 的纯 `ModelProvider` trait（`provider_id` / `api_protocol` / `rate_limit_ms` / `max_context_tokens` / `effective_context_tokens` / `execute_turn`）注册给 session——**session 只依赖这一个模型契约**，对协议体系零感知。

**理由**：供应商无关、协议演进、功能差异适配；核心契约保持 object-safe trait，协议细节可独立演进

## 机制化原则（Agent 子系统）

Agent 插件自 v9 起贯彻**机制化 (Mechanismization)** 原则：关系判定与展示规则由数据（CU）驱动而非硬编码。机制细节（`prop` CU、`RelationPropRegistry`、`seed_cus.jsonl` 单一事实源）见 `symbio/src/plugins/agent/README.md`。

## 文档体系约定（下沉原则）

- **模块文档下沉**：每个插件的职责、路由、内部机制写在插件目录内的 `README.md`（可选 `docs/` 子目录），如 `symbio/src/plugins/session/README.md`；前端同理见 `tauri/README.md` 与 `tauri/docs/`。
- **系统级文档只留跨模块内容**：`docs/` 只保留跨模块的架构、协议、导航与设计总览，单模块细节一律引用模块文档，不在系统级重复维护。
- **后端注释**：`// Corresponding Host: <path>` 注释指向该数据结构在宿主层的对应定义。

---

> **维护原则**：架构变更必须先更新本文档，再改代码。

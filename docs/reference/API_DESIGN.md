# Symbio API 设计规范 (V3.0+)

> **文档类型：Reference（参考）** — 接口/协议/错误码的准确事实，供查阅。

> 本文档基于**当前代码**（纯 Rust 核心库）。
> 早期"Tauri + Vue 前端"叙述中关于 `tauri/src/schemas/*.ts` 的引用已不再适用，
> host 侧 TS schema 维护随前端剥离同步取消；如有需要请按插件自包含原则就近维护。

## 1. 核心交互协议：分形路由

Symbio 统一采用**分形路由协议 (Fractal Routing)**。
每一个插件既是功能的执行者，也是请求的分发路由。

### 1.1 统一路由入口 (`route`)

所有交互均通过 `Plugin::route(ctx)` 发起，宿主侧通过 `InvokeRequest` 上下文注入参数。

**`InvokeRequest`** **提供的常用键**（定义于 `symbio_core::keys`）：

| 键                                                            | 说明                                  |
| ------------------------------------------------------------ | ----------------------------------- |
| `PATH`                                                       | 目标路径（如 `agent/chat`），容器插件按 `/` 逐级剥离 |
| `PAYLOAD`                                                    | 交互载荷数据（`serde_json::Value`）         |
| `WORKDIR`                                                    | 当前工作区根路径                            |
| `SESSION_ID`                                                 | 会话唯一标识                              |
| `TRACE_ID`                                                   | 调用链追踪 ID                            |
| `AGENT_ID`                                                   | 当前 Agent 标识（agent 插件内部使用）           |
| `CONTENT` / `KIND` / `ID` / `NAME` / `SCOPE` / `DESCRIPTION` | 通用字段                                |

**响应载荷** **`PluginPayload`**：

```rust
pub enum PluginPayload {
    Empty,                              // 空返回
    Data(SerializeData),                // 类型化数据，进程内零拷贝；跨进程自动 JSON
    Native(Arc<dyn Any + Send + Sync>), // 进程内原生接口
    Session(PluginChannel),             // 全双工 / 流式会话
}
```

> `SerializeData::serialize()` 在需要跨进程时统一走 JSON Value。
> 同进程优先走 `downcast_ref::<T>()` 零拷贝路径。

## 2. 帧协议 `PluginFrame`

定义于 `symbio_core/transport.rs`，是**沿** **`PluginChannel`** **发送的最小消息**：

```rust
pub enum PluginFrame {
    /// 业务数据载荷
    Data(Value),
    /// 错误（message, details）
    /// details 约定含 `{ "code": "ERR_CODE" }`
    Error(String, Option<Value>),
}
```

辅助方法：

* `into_value()` —— `Data` 帧转 `Value`，其他返回 `Value::Null`。

* `try_into_event::<T>()` —— 尝试把 `Data` 帧反序列化为指定事件模型。

## 3. 数据契约 `Schemas`

所有跨端数据结构集中定义在 `symbio/src/symbio_core/schemas/`，
按业务域拆分（`agent_config.rs` / `model_chat.rs` / `session_*.rs` / `mcp_*.rs` / `memory_*.rs` / …）。
所有结构都派生 `Serialize` / `Deserialize`，命名遵循 `snake_case`（Rust 默认）↔ `camelCase`（host 转换）约定。

### 3.1 常用路径示例

| 路径                                                     | 用途                                                                 |
| ------------------------------------------------------ | ------------------------------------------------------------------ |
| `_root`                                                | 查询当前节点子插件拓扑                                                        |
| `{plugin}/config`                                      | 统一配置管理（`get` / `set` action）                                       |
| `{plugin}/resources/list\|get\|upload\|delete\|status` | **统一资源管理**（五类资源，见 3.2）                                             |
| `session/chat`                                         | 发起 AI 长连接会话                                                        |
| `session/get_messages`                                 | 获取对话历史                                                             |
| `session/open` / `session/update` / `session/clear`    | 会话生命周期（会话内容操作）                                                     |
| `agent/chat`                                           | Agent 层对话入口（由 session 内部转发或外部直调）                                   |
| `model/chat`                                           | 底层 Model 引擎调用                                                      |
| `local/shell`                                          | 本地 shell 工具                                                        |
| `explorer/list`                                        | 文件列表                                                               |
| `web/http_request`                                     | Web 请求工具（`web_search` / `web_fetch` 为内部能力，经 `web/http_request` 暴露） |

### 3.2 统一资源协议（resources/\*，五类资源）

`model` / `mcp` / `agent` / `skill` / `session` 五种资源共享同一套 `resources/*`
操作集（契约定义于 `symbio/src/symbio_core/schemas/resources.rs`，
zip 工具函数位于 `symbio/src/symbio_core/resources.rs`）：

| 操作                 | 说明                                                            |
| ------------------ | ------------------------------------------------------------- |
| `resources/list`   | 列出全部资源，返回 `ResourcesListResponse`（能力开关 + `ResourceSummary[]`） |
| `resources/get`    | 读取单个资源详情                                                      |
| `resources/upload` | 创建/更新（zip 上传或 JSON manifest 表单）                               |
| `resources/delete` | 删除资源                                                          |
| `resources/status` | 查询实时/连接状态（可选能力）                                               |

* 资源差异仅由 **`ResourceCapabilities`** **能力开关**驱动（`zip_upload` / `independent_form` /
  `realtime_status` / `mutable` / `test_connection` / `read_only`），前后端据此统一实现。

* 统一路径在不同插件实例化：`worker/model/resources/*`、`mcp/resources/*`、
  `skill/resources/*`、`agent/resources/*`、`worker/session/resources/*`。

* **实时状态机制**：初始状态由 `resources/list` 携带；运行时状态变化由后端经事件总线 push `resource` kind 事件
  （`EventBus::publish_resource_status`），前端 `subscribeResourceStatus(resourceType)` 即时刷新，**不做前端轮询**。

* **五类操作集统一，后端渐进支持**：所有资源（含 session）共享同一套 `list/get/upload/delete/status` 契约，
  每种资源的上传/下载（如 session 导出/导入）均有意义；按进度渐进实现，后端逐类补齐即可，无需改协议。

* 前端由一份 `ResourceManagerView` 实例化多类；会话聊天主界面（`SessionView`）检索的是
  `resources/list` 统一契约，本身保留专属（会话为内存态交互面）。

## 4. 工具发现与 AI 集成

### 4.1 `traverse` 协议

* 常量 `TRAVERSE_AVAILABLE_TOOLS = "available_tools"`。

* 调用 `root.traverse(TRAVERSE_AVAILABLE_TOOLS, ctx)` 即可获得全树工具清单。

* 容器插件将自己的 `path` 前缀下发给子插件；
  叶子插件返回 `ToolDefinition` 列表，工具名自动带命名空间（如 `local/shell`）。

由于 `traverse` 与 `route` 共享 `InvokeRequest` / `PluginPayload`，
`traverse` 不仅能用于工具发现，还能承载流式响应与其他全树诊断。

### 4.2 `traverse` / `route` 协议一致性

* **同协议**：两者都接收 `Arc<dyn InvokeRequest>` 并返回 `InvokeResponse<PluginPayload>`。

* **同路径**：两者都按 `PATH` 工作。

* **不同点**：`traverse` 通常返回 `Data`（汇总），`route` 可返回 `Session`（流式）。

## 5. AI 会话流式规范

1. **建立会话**：发起 `route("session/chat", payload)`，后端返回 `PluginPayload::Session(channel)`。
2. **握手响应**：host 从 `channel.tx` 接收首批帧（典型为 `Data` 携带 `session_meta`）。
3. **流式推送**：后端持续推送 `PluginFrame::Data` 帧（典型 schema：`session_chat_response::StreamEvent`），
   包含文本增量、思考过程、工具调用进度。
4. **终止信号**：任务结束时发送最后一帧 `Data` 携带 `done: true`，
   或在错误时发送 `Error(msg, details)`。

## 6. 错误处理体系

`PluginError` 在 `symbio_core/error.rs` 定义，提供稳定的字符串错误码：

| 错误码                | 说明             |
| :----------------- | :------------- |
| `NOT_FOUND`        | 路由路径不存在或子插件未找到 |
| `VALIDATION_ERROR` | 输入参数格式或内容校验不通过 |
| `INTERNAL_ERROR`   | 内部执行异常         |
| `TIMEOUT`          | 请求超时           |
| `FORBIDDEN`        | 权限不足或触发安全策略    |

错误码是 ABI 的一部分，跨版本变更需谨慎。host 侧可以根据 `code` 做差异化处理（如引导设置 API Key）。

## 7. 注册与扩展

### 7.1 `submit_object_creator!` 宏

每个插件模块在其 `plugin.rs` 末尾调用：

```rust
submit_object_creator!(PLUGIN_X, XPlugin::build, dyn Plugin);
```

宏利用 [`inventory`](https://docs.rs/inventory) 在编译期把构造函数注册到全局 `ObjectCreatorRegistry`（定义于 `symbio_core::creator`）。
宿主首次调用 `create_object::<dyn Plugin>(id, ctx)` 时惰性收集完毕，**无需手动注册**。

### 7.2 容器动态挂载

`HomePlugin::init_worker_composite()` 按 `~/.symbio/config.yaml` 的 `plugins.<name>: { plugin_provider: "..." }`
从注册表取构造函数并实例化子插件。
新增插件只需把目录放到 `symbio/src/plugins/`，实现 `Plugin` 并调用 `submit_object_creator!`，再在配置里挂载即可。

## 8. 文档映射约定

* **后端**：`// Corresponding Host: <path>` 注释（如旧 Tauri 前端 schema 路径）保留为可选。

* **插件自包含**：业务细节文档放在 `symbio/src/plugins/<name>/docs/`，不集中在 `docs/`。


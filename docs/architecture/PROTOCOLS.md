# Symbio 协议规范

> **文档类型：参考** — 帧/载荷/通道的准确规格。

## 帧协议 (PluginFrame)

沿 `PluginChannel` 发送的最小消息单位。

```rust
pub enum PluginFrame {
    /// 业务数据载荷
    Data(serde_json::Value),
    /// 错误 (message, details)
    /// details 约定含 { "code": "ERR_CODE" }
    Error(String, Option<serde_json::Value>),
}
```

### 帧类型

| 类型 | 用途 | 示例 |
|------|------|------|
| `Data` | 正常业务数据 | `{"type": "text_delta", "content": "Hello"}` |
| `Error` | 执行异常 | `("API key missing", Some({"code": "CONFIG_ERROR"}))` |

### 辅助方法

```rust
// Data 帧转 Value，其他返回 Value::Null
frame.into_value()

// 尝试把 Data 帧反序列化为指定事件模型
frame.try_into_event::<T>()
```

---

## 载荷协议 (PluginPayload)

`route()` 和 `traverse()` 的返回类型。

```rust
pub enum PluginPayload {
    Empty,                              // 空返回
    Data(SerializeData),                // 类型化数据
    Native(Arc<dyn Any + Send + Sync>), // 进程内原生接口
    Session(PluginChannel),             // 全双工流式会话
}
```

### 载荷类型

| 类型 | 用途 | 序列化行为 |
|------|------|-----------|
| `Empty` | 操作成功但无返回值 | `null` |
| `Data` | 一次性返回的数据 | 跨进程 → JSON；进程内 → 零拷贝 |
| `Native` | 进程内原生接口 | 不序列化，直接 downcast |
| `Session` | 长连接流式会话 | 返回 channel，后续用帧通信 |

### 构造与访问

```rust
// 构造
PluginPayload::new(&value)           // 自动推断类型
PluginPayload::Session(channel)      // 显式创建会话

// 访问
payload.get::<T>()                   // 类型化访问 (进程内零拷贝)
payload.serialize()                  // 强制 JSON (跨进程)
```

---

## 通道协议 (PluginChannel)

全双工流式会话的底层通道。

```rust
pub struct PluginChannel {
    pub tx: mpsc::Sender<PluginFrame>,   // 发送端 (后端 → 前端)
    pub rx: mpsc::Receiver<PluginFrame>, // 接收端 (前端 → 后端)
}
```

### 通道生命周期

```
1. 创建: PluginChannel::pair(buffer_size)
2. 返回: PluginPayload::Session(peer_channel)  ← 给调用方
3. 使用: 后端通过 my_channel.tx 发送帧
4. 结束: channel drop → rx 收到 None (EOF)
```

### 缓冲区大小

```rust
// 典型用法
let (my_channel, peer_channel) = PluginChannel::pair(64);
// 64 = 缓冲区帧数，防止慢消费者阻塞生产者
```

---

## 路由上下文 (InvokeRequest)

请求的上下文注入接口。

```rust
#[async_trait]
pub trait InvokeRequest: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn config(&self) -> Option<serde_json::Value>;
    fn payload(&self) -> Result<serde_json::Value, PluginError>;
    fn fork(&self) -> Arc<dyn InvokeRequest>;
}
```

### 标准上下文键

| 键 | 类型 | 用途 |
|----|------|------|
| `PATH` | String | 目标路径 (如 `agent/chat`) |
| `PAYLOAD` | Value | 交互载荷数据 |
| `WORKDIR` | String | 当前工作区根路径 |
| `SESSION_ID` | String | 会话唯一标识 |
| `TRACE_ID` | String | 调用链追踪 ID |
| `AGENT_ID` | String | 当前 Agent 标识 |
| `CONTENT` | String | 通用内容字段 |
| `KIND` | String | 通用类型字段 |
| `ID` | String | 通用 ID 字段 |
| `NAME` | String | 通用名称字段 |
| `SCOPE` | String | 作用域 |
| `DESCRIPTION` | String | 描述 |

---

## 工具发现协议 (traverse)

### 触发常量

```rust
pub const TRAVERSE_AVAILABLE_TOOLS: &str = "available_tools";
```

### 发现流程

```
1. 调用 root.traverse("available_tools", ctx)
2. 容器插件将自身 path 前缀下发给子插件
3. 叶子插件返回 ToolDefinition 列表
4. 工具名自动带命名空间 (如 "local/shell")
```

### 工具定义

```rust
pub struct CapabilityMeta {
    pub name: String,           // 工具名称 (带路径前缀)
    pub description: String,    // 工具描述
    pub input_schema: Value,    // JSON Schema 参数定义
    pub keywords: Vec<String>,  // 意图识别关键词
    pub category: Option<CapabilityCategory>, // 能力分类
    pub examples: Option<Vec<String>>,        // 使用示例
}
```

---

## 跨进程通信 (Tauri IPC)

### 线格式 (Wire Format)

```rust
// 请求
pub struct PluginMessageWire {
    pub metadata: serde_json::Value,  // { path, trace_id, ... }
    pub payload: serde_json::Value,   // 业务数据
}

// 响应载荷
pub enum PluginPayloadWire {
    Data(serde_json::Value),
    Connection(String),  // 返回连接 ID
}
```

### Tauri Commands

| Command | 用途 | 输入 | 输出 |
|---------|------|------|------|
| `route_v2` | 发起路由请求 | `PluginMessageWire` | `PluginMessageWire` |
| `route_v2_send` | 向会话发送帧 | `connection_id, PluginFrame` | `()` |
| `route_v2_close` | 关闭连接 | `connection_id` | `()` |

### 流式会话建立

```
1. 前端调用 route_v2 (带 client_id)
2. 后端返回 Connection(conn_id)
3. 后端通过 Tauri emit 推送帧到 `route/{conn_id}` 事件
4. 前端通过 route_v2_send 发送后续帧
5. 前端调用 route_v2_close 关闭连接
```

---

## 错误码

| 错误码 | 含义 | 处理建议 |
|--------|------|----------|
| `NOT_FOUND` | 路由路径不存在 | 检查路径拼写 |
| `VALIDATION_ERROR` | 输入参数校验失败 | 检查 payload 格式 |
| `INTERNAL_ERROR` | 内部执行异常 | 查看后端日志 |
| `TIMEOUT` | 请求超时 | 检查网络或增加超时 |
| `FORBIDDEN` | 权限不足 | 检查安全策略配置 |


---

## 统一实体管理

> 机制详解（`EntityProvider` trait、`dispatch` 公共流程、能力开关实现）见 [design/entity-management-mechanism.md](../design/entity-management-mechanism.md)。

### 路径约定

```
{plugin}/entities/list|get|upload|delete|status
```

### 操作类型

| 操作 | 说明 |
|------|------|
| `entities/list` | 列出全部实体，返回 `EntitiesListResponse` |
| `entities/get` | 读取单个实体详情 |
| `entities/upload` | 创建/更新 (zip 上传或 JSON manifest) |
| `entities/delete` | 删除实体 |
| `entities/status` | 查询实时/连接状态 |

> **扩展操作**（由特定插件提供，不属通用 dispatch）：`entities/detail`（agent：实体详情页定义，info 绑定概览 + open-container 入口）、`entities/providers`（home：宿主级 provider 注册表，前端启动拉取生成导航）。

### 能力开关

实体差异仅由 **`EntityCapabilities`** 驱动：

- `zip_upload` - 支持 zip 上传
- `independent_form` - 支持独立表单
- `realtime_status` - 实时状态推送
- `mutable` - 可修改
- `test_connection` - 支持连接测试
- `read_only` - 只读

---

## AI 会话流式规范

1. **建立会话**：发起 `route("session/chat/send", payload)`，后端返回 `PluginPayload::Session(channel)`
2. **握手响应**：host 从 `channel.tx` 接收首批帧 (典型为 `Data` 携带 `session_meta`)
3. **流式推送**：后端持续推送 `PluginFrame::Data` 帧，包含增量文本、思考过程、工具调用进度
4. **终止信号**：任务结束时发送最后一帧 `Data` 携带 `done: true`，或在错误时发送 `Error(msg, details)`

---

## 数据契约 (Schemas)

所有跨端数据结构集中定义在 `symbio/src/symbio_core/schemas/`，按业务域拆分：

| 文件 | 内容 |
|------|------|
| `agent_config.rs` | Agent 配置结构 |
| `model_chat.rs` | Model 请求/响应 |
| `session_*.rs` | 会话消息结构 |
| `mcp_*.rs` | MCP 配置 |
| `memory_*.rs` | 记忆操作 |

所有结构都派生 `Serialize` / `Deserialize`，命名遵循 `snake_case`（Rust）↔ `camelCase`（host 转换）约定。

---

## 注册与扩展

### `submit_object_creator!` 宏

每个插件模块在其 `plugin.rs` 末尾调用：

```rust
submit_object_creator!(PLUGIN_X, XPlugin::build, dyn Plugin);
```

宏利用 [`inventory`](https://docs.rs/inventory) 在编译期把构造函数注册到全局 `ObjectCreatorRegistry`。宿主首次调用 `create_object::<dyn Plugin>(id, ctx)` 时惰性收集完毕，**无需手动注册**。

### 容器动态挂载

`HomePlugin::init_worker_composite()` 按 `~/.symbio/config.yaml` 的 `plugins.<name>: { plugin_provider: "..." }` 从注册表取构造函数并实例化子插件。

新增插件只需：
1. 把目录放到 `symbio/src/plugins/`
2. 实现 `Plugin` 并调用 `submit_object_creator!`
3. 在配置里挂载

---

## 文档映射约定

- **后端**：`// Corresponding Host: <path>` 注释指向该数据结构在宿主层的对应定义
- **文档集中**：插件不各自维护文档，全部统一在 `docs/`；复杂机制的实现细节见 `docs/design/`（如统一实体管理机制）

---

> **维护原则**：协议变更必须向后兼容，错误码字符串是 ABI 的一部分。

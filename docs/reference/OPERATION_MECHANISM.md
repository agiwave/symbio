# Symbio 项目整体运作机制

> **文档类型：Reference（参考）** — 系统运作的事实描述，供查阅。

> 本文档基于**当前代码**（纯 Rust 核心库 + E2E CLI）。
> 早期"Tauri + Vue 前端"叙述中关于 Vue 组件、EventHandler、useChatEventHandler 等前端细节已不再适用，相关引用统一在 `docs/archive/design_docs/` 中保留。

## 1. 核心架构概述

Symbio 是一个**纯 Rust 分形插件核心库**，由以下几个层次构成：

- **宿主层 (Host)**：任何 Rust 二进制（CLI / 桌面 / Web 服务），通过 `symbio::create_root_plugin()` 拿到 root plugin。
- **对象创建注册层 (Creator)**：各插件通过 `submit_object_creator!` 宏经 `inventory` 提交构造函数，由 `symbio_core::creator` 的全局 `ObjectCreatorRegistry` 收集；首次调用 `create_object::<T>(id, ctx)` 时惰性初始化。
- **逻辑运行时层 (Plugin Tree)**：以 `Home` 为根的分形插件树，物理代码平铺在 `symbio/src/plugins/`。

```text
host (CLI / Tauri / Web)
        │ create_root_plugin()
        ▼
    Home / 根容器
        │ init_worker_composite()
        ▼
   worker / Composite  (按 plugins.<name> 配置动态挂载)
        │
        ├─ agent ── MindscapeScaffold
        ├─ session
        ├─ model ── protocols/{openai, anthropic, gemini}
        ├─ local ── shell / file_read / file_write / file_edit / glob / content_search
        ├─ web ── http_request / web_search / web_fetch
        ├─ skill
        ├─ mcp
        ├─ telegram
        └─ …
```

---

## 2. 插件初始化场景

### 2.1 注册机制 (`submit_object_creator!`)

每个插件模块在其 `plugin.rs` 末尾调用 `submit_object_creator!(PLUGIN_X, XPlugin::build, dyn Plugin)`：
该宏通过 [`inventory`](https://docs.rs/inventory) 收集构造函数到全局 `ObjectCreatorRegistry`（定义在 `symbio_core::creator`）。
在库 `init` 时被一次性收集完毕，**无需在外部显式注册**。

### 2.2 树状挂载与实例化

1. **宿主启动**：调用 `symbio::initialize()` 初始化日志；调用 `create_root_plugin()` 拿到 root plugin（内部通过 `create_object::<dyn Plugin>(PLUGIN_HOME, ctx)` 构造 `HomePlugin`）。
2. **根节点创建**：`create_root_plugin()` 经注册表取 `home` 构造函数并实例化 `HomePlugin`。
3. **递归挂载**：`HomePlugin::init_worker_composite()` 从注册表取 `composite` 构造函数，
   读取配置 `symbio.plugins.<name>`，按 `plugin_provider` 字段从注册表再次取构造函数，
   把子插件挂载到 `Composite` 容器中。
4. **叶子挂载**：每个叶子插件的构造函数自包含构造逻辑（如 `agent` 还会进一步构造 `MindscapeScaffold`）。

最终在内存中形成一棵以 `Home` 为根节点的逻辑分形树。

---

## 3. AI 对话场景工作流

### 3.1 会话建立

1. **客户端调用**：通过任意 host（如 CLI：`symbio::cli`）发起 `route("session/chat", { messages, agent_id, … })`。
2. **路由分发**：`Home` 容器按 `PATH` 逐层剥离：→ `worker` (Composite) → `session`。
3. **建立连接**：`SessionPlugin` 返回 `PluginPayload::Session(PluginChannel)`，
   代表一个基于 mpsc 通道的双向长连接。

### 3.2 认知注入与流式推理

1. **历史与上下文**：`Session` 负责加载历史消息并进行裁剪（Trimming）。
2. **提示词增强 (Agent)**：消息流经 `Agent` 插件。Agent 按 v9 机制化原则
   从 `seed_cus.jsonl` 拉取 prop / 关系 / 类型清单，组装提示词：
   - 注入身份 CU（`id == "identity"` 的 fact）
   - 注入 `rule` 类型 CU（全量）
   - 注入认知索引（其余类型摘要）
3. **无状态执行 (Model)**：组装好的 messages 交给 `model` 插件。
   `model` 内置 4 套协议适配器，调用 Model API 并处理工具调用循环。
4. **流式返回**：`model` 通过 `PluginChannel` 持续推送 `PluginFrame::Data` 帧，
   包含增量文本、思考过程、工具调用进度、最终结果。

### 3.3 客户端渲染

host 侧解析 `PluginFrame::Data` 触发各自的渲染逻辑（CLI 串行打印、UI 打字机效果等）。

---

## 4. 工具发现与调用机制

### 4.1 分形发现 (`traverse`)

1. **触发发现**：系统需要 LLM 工具列表时，调用根插件的 `traverse(TRAVERSE_AVAILABLE_TOOLS, …)`。
2. **递归下发**：每个容器类插件（`Home` / `Composite`）将自己的 `path` 前缀下发给所有子插件。
3. **能力暴露**：叶子插件（`local` / `web` / `mcp` / `skill` …）验证 `request.path` 后
   返回 `ToolDefinition` 列表，工具名自动带命名空间（如 `local/shell`）。
4. **全局唯一**：路径前缀确保工具名全局唯一，并能通过 `route("local/shell", …)` 精准回放。

### 4.2 路由执行 (`route`)

1. **LLM 决策**：AI 推理后决定调用工具，输出工具名（如 `local/shell`）与参数。
2. **动态路由**：系统拦截工具调用，按普通请求发起 `route("local/shell", payload)`。
3. **精准命中**：`Home` 剥离前缀 → `worker` → `local` → `shell`。
4. **安全拦截**：若该工具包含高危操作（如系统命令），由 `local/policy` 中的 `ToolApprovalRequest` 流程要求用户授权。
5. **结果回传**：执行结果通过通道返回给 AI 引擎继续下一轮对话。

---

## 5. Agent 认知注入的关键路径

| 注入内容 | 识别方式 | 注入形式 |
| --- | --- | --- |
| 身份 (identity) | `id == "identity"` | 全量 |
| 系统规则 (rule) | `is_a` 含 `rule` | 全量 |
| 认知索引 | 其他 `kind` 类型 | 摘要（每类最多 N 条，按 `priority` 排序） |
| 记忆召回 | 语义检索结果 | 相关 top-K 条目 |

> 上述所有清单均由 `seed_cus.jsonl` 中的 prop CU 派生，无硬编码。
> 详见 [PRINCIPLES.md](./../symbio/src/plugins/agent/docs/PRINCIPLES.md)。

---

## 6. 项目开发指导总结

### 6.1 插件"平权"思想

不要把业务逻辑都塞进单一模块。新的能力（数据库、自研云服务等）应当作为独立目录放在
`symbio/src/plugins/` 下，实现自己的 `Plugin` 与 `Factory`，并通过配置挂载到 `Composite` 中。
**逻辑上，每个业务插件都拥有平等的路由地位。**

### 6.2 统一错误与契约

- 后端使用统一的 `PluginError` 返回错误，并提供稳定字符串错误码。
- 跨端数据结构集中在 `symbio/src/symbio_core/schemas/`。

### 6.3 避免状态污染

- 业务插件尽量保持**无状态 (Stateless)**。
- 需要状态时优先用 `Arc<AtomicXxx>` / `Arc<RwLock<T>>`，并防范锁中毒。

### 6.4 善用路由调试

所有能力基于路径（如 `agent/chat`、`local/shell`），
因此任何能调用 `route()` 的入口（CLI、单元测试、外部脚本）都能独立测试某个插件的能力。
这是"分形架构"带来的极致解耦优势。

> 详细 API 协议、架构图、插件开发实战请参阅
> [API_DESIGN.md](./API_DESIGN.md) / [ARCHITECTURE.md](./ARCHITECTURE.md) / [PLUGIN_DEVELOPMENT_GUIDE.md](../development/PLUGIN_DEVELOPMENT_GUIDE.md)。

# Symbio API 设计规范

## 概述

Symbio 采用**路径驱动**的插件架构，每个路径对应一个 Plugin 实例，通过三种请求模式（meta/invoke/stream）进行交互。本文档定义统一的 API 规范，确保接口简洁一致。

## 核心概念

### 1. 路径路由

```
路径格式: {plugin}/{sub_plugin}/{...}/{action}
         ↓         ↓           ↓
       一级      二级        三级
```

**示例**：
- `` → home 插件根
- `agent` → agent 插件
- `agent/openai` → agent 下的 openai 插件
- `agent/openai/config` → openai 插件的配置接口

### 2. 请求模式

| 模式 | 用途 | 返回类型 |
|------|------|----------|
| `meta` | 获取接口 Schema | `PluginMeta` |
| `invoke` | 同步调用 | `StreamChunk` |
| `stream` | 流式调用 | `Stream<StreamChunk>` |

### 3. 能力路由

使用 `@capability` 语法进行能力查找：

```
@llm      → 查找支持 LLM 能力的插件（如 openai）
@session  → 查找支持会话能力的插件
@memory   → 查找支持记忆能力的插件
@tools    → 查找支持工具能力的插件
```

**示例**：
- `agent/@llm` → 路由到 openai 插件
- `agent/@session` → 路由到 session 插件

### 4. 可用工具查询

所有插件支持 `available_tools` 路径，返回该插件及其子插件的所有可用工具：

```
POST /invoke
{
  "path": "agent/available_tools",
  "input": {}
}
```

**响应**：
```json
{
  "success": true,
  "tools": [
    {
      "name": "tools/read_file",
      "description": "读取文件内容",
      "input_schema": {...},
      "output_schema": {...}
    },
    {
      "name": "memory/store",
      "description": "存储记忆",
      "input_schema": {...},
      "output_schema": {...}
    }
  ]
}
```

## 插件层级结构

```
home (根插件)
├── work (工作区插件)
├── note (笔记插件)
├── explorer (文件浏览器插件)
├── agent (智能体插件)
│   ├── openai (LLM 插件，能力: @llm)
│   ├── session (会话插件，能力: @session)
│   ├── memory (记忆插件，能力: @memory)
│   ├── tools (工具插件，能力: @tools)
│   ├── chat (聊天插件)
│   └── telegram (Telegram 插件)
└── setting (设置插件)
```

---

## AI 对话流式 API

### 设计原则

1. **真正的流式传输**：使用 Tauri 事件系统实时推送每个 chunk
2. **工具调用支持**：LLM 可以调用工具，工具结果自动返回
3. **系统提示词**：从文件动态加载，支持项目特定配置
4. **分页加载历史**：避免一次性加载所有历史消息

### 流式对话流程

```
前端 → sendMessageStream() → streamPlugin()
  ↓
后端 → stream 命令 → app.emit(eventId, chunk)
  ↓
OpenAI 插件 → 流式 API (stream: true)
  ↓
解析 SSE 格式 → 实时 yield StreamChunk
  ↓
前端收到 chunk → 更新 UI（逐字显示）
```

### 系统提示词加载

Session 插件的系统提示词从文件拼接：

1. **优先加载** `<workspace>/.symbio/README.ai.md`，如果不存在则加载 `~/.symbio/README.ai.md`
2. **如果存在** `<workspace>/README.ai.md`，也加载
3. **将所有文件内容**用 `\n\n---\n\n` 分隔符拼接

**示例文件结构**：
```
~/.symbio/README.ai.md          # 全局 AI 行为配置
~/projects/myapp/.symbio/README.ai.md  # 项目特定配置
~/projects/myapp/README.ai.md   # 项目文档
```

### 会话历史分页

Session 插件支持分页获取消息：

```
POST /invoke
{
  "path": "agent/session",
  "input": {
    "action": "get_messages",
    "session_id": "note-ai-session",
    "limit": 10,
    "before": 1234567890  // 可选，获取此时间戳之前的消息
  }
}
```

**响应**：
```json
{
  "success": true,
  "messages": [...],
  "has_more": true,
  "total": 150
}
```

---

## 配置管理 API

### 设计原则

1. **统一接口**：所有插件通过 `config` path 暴露配置
2. **分层存储**：全局配置集中管理，会话数据独立存储
3. **自动同步**：配置变更自动保存
4. **Schema 驱动**：通过 meta 接口描述配置结构

### 配置分类

| 类型 | 存储位置 | 示例 |
|------|----------|------|
| **全局配置** | `~/.symbio/config.yaml` | API Key、模型选择、温度参数 |
| **会话数据** | `<workspace>/.symbio/agent/session/` | 对话历史、上下文 |
| **记忆数据** | `<workspace>/.symbio/agent/memory/` | 持久化记忆条目 |

---

### 统一配置接口

每个需要配置的插件必须实现 `config` path：

#### 获取配置

```http
POST /invoke
Content-Type: application/json

{
  "path": "{plugin}/config",
  "input": {
    "action": "get"
  }
}
```

**响应**：
```json
{
  "data": {
    "success": true,
    "config": {
      "api_base": "https://api.openai.com/v1",
      "api_key_set": true,
      "model": "gpt-4o",
      "temperature": 0.7,
      "max_tokens": 4096
    }
  },
  "done": true
}
```

#### 设置配置

```http
POST /invoke
Content-Type: application/json

{
  "path": "{plugin}/config",
  "input": {
    "action": "set",
    "config": {
      "model": "gpt-4-turbo",
      "temperature": 0.5
    }
  }
}
```

**响应**：
```json
{
  "data": {
    "success": true,
    "message": "配置已更新并保存"
  },
  "done": true
}
```

#### 获取配置 Schema

```http
POST /invoke
Content-Type: application/json

{
  "path": "{plugin}/config",
  "input": {
    "action": "schema"
  }
}
```

**响应**：
```json
{
  "data": {
    "success": true,
    "schema": {
      "api_base": {
        "type": "string",
        "title": "API Base URL",
        "description": "OpenAI 兼容 API 基础地址",
        "default": "https://api.openai.com/v1"
      },
      "api_key": {
        "type": "string",
        "title": "API Key",
        "description": "OpenAI API 密钥",
        "secret": true
      },
      "model": {
        "type": "string",
        "title": "Model",
        "description": "模型名称",
        "enum": ["gpt-4o", "gpt-4-turbo", "gpt-3.5-turbo"],
        "default": "gpt-4o"
      },
      "temperature": {
        "type": "number",
        "title": "Temperature",
        "description": "生成温度",
        "minimum": 0,
        "maximum": 2,
        "default": 0.7
      }
    }
  },
  "done": true
}
```

---

### 全局配置管理

Home 插件负责全局配置的持久化：

#### 获取所有配置

```http
POST /invoke
Content-Type: application/json

{
  "path": "config",
  "input": {
    "action": "get"
  }
}
```

**响应**：
```json
{
  "data": {
    "success": true,
    "config": {
      "work": { ... },
      "agent": {
        "openai": { ... },
        "session": { ... },
        "memory": { ... }
      },
      "setting": { ... }
    }
  },
  "done": true
}
```

---

### 配置文件格式

**全局配置文件** (`~/.symbio/config.yaml`)：

```yaml
# Symbio 全局配置
version: "1.0"

plugins:
  work:
    workspace_path: "~/projects"
    auto_save: true

  agent:
    openai:
      api_base: "https://api.openai.com/v1"
      model: "gpt-4o"
      temperature: 0.7
      max_tokens: 4096

    session:
      max_messages: 100
      storage_dir: "~/.local/share/symbio/sessions"

    memory:
      storage_dir: "~/.local/share/symbio/memory"

    tools:
      shell_enabled: true
      web_enabled: true

  setting:
    theme: "dark"
    language: "zh-CN"
    font_size: 14
```

---

## 各插件配置规范

### 1. OpenAI 插件配置

**路径**: `agent/openai/config`

**配置结构**：
```typescript
interface OpenAiConfig {
  // API 配置
  api_base: string;        // API 基础 URL
  api_key?: string;        // API 密钥（敏感）

  // 模型配置
  model: string;           // 模型名称
  temperature: number;     // 温度 (0-2)
  max_tokens?: number;     // 最大输出 tokens
  max_context_tokens: number; // 最大上下文 tokens

  // 行为配置
  system_prompt?: string;  // 系统提示词
  timeout?: number;        // 请求超时（秒）
}
```

**默认值**：
```json
{
  "api_base": "https://api.openai.com/v1",
  "model": "gpt-4o",
  "temperature": 0.7,
  "max_context_tokens": 128000
}
```

---

### 2. Session 插件配置

**路径**: `agent/session/config`

**配置结构**：
```typescript
interface SessionConfig {
  storage_dir: string;     // 存储目录
  max_messages: number;    // 最大消息数
  auto_compress: boolean;  // 自动压缩
  compress_threshold: number; // 压缩阈值（消息数）
}
```

**默认值**：
```json
{
  "storage_dir": "~/.symbio/agent/session",
  "max_messages": 100,
  "auto_compress": true,
  "compress_threshold": 50
}
```

---

### 3. Memory 插件配置

**路径**: `agent/memory/config`

**配置结构**：
```typescript
interface MemoryConfig {
  storage_dir: string;     // 存储目录
  max_entries: number;     // 最大条目数
  categories: string[];    // 预定义分类
}
```

**默认值**：
```json
{
  "storage_dir": "~/.symbio/agent/memory",
  "max_entries": 1000,
  "categories": ["preference", "fact", "instruction"]
}
```

---

### 4. Tools 插件配置

**路径**: `agent/tools/config`

**配置结构**：
```typescript
interface ToolsConfig {
  // 工具开关
  shell_enabled: boolean;
  file_enabled: boolean;
  web_enabled: boolean;

  // 安全配置
  allowed_paths: string[];  // 允许访问的路径
  blocked_commands: string[]; // 禁止执行的命令

  // 超时配置
  shell_timeout: number;    // Shell 超时（秒）
  web_timeout: number;      // Web 请求超时（秒）
}
```

**默认值**：
```json
{
  "shell_enabled": true,
  "file_enabled": true,
  "web_enabled": true,
  "allowed_paths": ["~"],
  "blocked_commands": ["rm -rf", "sudo"],
  "shell_timeout": 60,
  "web_timeout": 30
}
```

---

## 配置变更流程

### 前端修改配置流程

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   前端 UI   │────>│  Plugin     │────>│   Parent    │
│  (Settings) │     │  config/set │     │  save_config│
└─────────────┘     └─────────────┘     └─────────────┘
                                               │
                                               ▼
                                        ┌─────────────┐
                                        │  Home       │
                                        │  收集配置   │
                                        │  写入文件   │
                                        └─────────────┘
```

### 配置加载流程

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   启动时    │────>│   Home      │────>│  分发配置   │
│             │     │  加载配置   │     │  到子插件   │
└─────────────┘     └─────────────┘     └─────────────┘
```

---

## 实现指南

### 插件实现配置接口

```rust
// 在 Plugin 的 invoke 方法中处理 config path
fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
    if path == "config" {
        let action = input.get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("get");

        return Ok(InvokeStream::Single(tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match action {
                    "get" => self.get_config().await,
                    "set" => {
                        let result = self.set_config(input.get("config")).await;
                        // 通知父插件保存
                        if let Some(parent) = self.get_parent() {
                            let _ = parent.invoke("save_config", json!({}));
                        }
                        result
                    }
                    "schema" => self.get_config_schema(),
                    _ => StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(format!("未知操作: {}", action)),
                    }
                }
            })
        })));
    }
    // ... 其他路径处理
}
```

---

## API 完整参考

### Home 插件

| 路径 | Action | 描述 |
|------|--------|------|
| `` | - | 获取插件列表 |
| `config` | get | 获取所有配置 |
| `config` | set | 设置配置（批量） |
| `config` | save | 保存配置到文件 |
| `config` | load | 从文件加载配置 |
| `config` | collect | 收集所有子插件配置 |

### Agent 插件

| 路径 | Action | 描述 |
|------|--------|------|
| `agent` | - | 获取子插件列表和能力 |
| `agent/config` | get | 获取所有子插件配置 |
| `agent/config` | set | 分发配置到子插件 |
| `agent/available_tools` | - | 获取所有子插件的可用工具 |
| `agent/@llm` | * | 路由到 LLM 插件 |
| `agent/@session` | * | 路由到会话插件 |

### OpenAI 插件

| 路径 | Action | 描述 |
|------|--------|------|
| `agent/openai` | status | 获取状态 |
| `agent/openai` | list_models | 列出可用模型 |
| `agent/openai` | chat | 发送聊天请求（流式） |
| `agent/openai/config` | get | 获取配置 |
| `agent/openai/config` | set | 设置配置 |
| `agent/openai/config` | schema | 获取配置 Schema |

### Session 插件

| 路径 | Action | 描述 |
|------|--------|------|
| `agent/session` | get | 获取会话 |
| `agent/session` | get_messages | 分页获取消息 |
| `agent/session` | append | 追加消息 |
| `agent/session` | clear | 清除会话 |
| `agent/session` | list | 列出所有会话 |
| `agent/session` | get_context | 获取 LLM 上下文 |
| `agent/session/config` | get/set | 配置管理 |

### Memory 插件

| 路径 | Action | 描述 |
|------|--------|------|
| `agent/memory` | store | 存储记忆 |
| `agent/memory` | recall | 回忆记忆 |
| `agent/memory` | forget | 删除记忆 |
| `agent/memory` | list | 列出所有记忆 |
| `agent/memory` | search | 搜索记忆 |
| `agent/memory/config` | get/set | 配置管理 |
| `agent/memory/available_tools` | - | 获取记忆工具列表 |

### Tools 插件

| 路径 | Action | 描述 |
|------|--------|------|
| `agent/tools` | _list | 列出可用工具 |
| `agent/tools` | _search | 搜索工具 |
| `agent/tools/config` | get/set | 配置管理 |
| `agent/tools/available_tools` | - | 获取工具列表 |

---

## 最佳实践

### 1. 敏感信息处理

```typescript
// 配置返回时隐藏敏感字段
{
  "api_key_set": true,  // 不返回实际值
  "api_key": null       // 或返回 null
}

// Schema 标记敏感字段
{
  "api_key": {
    "type": "string",
    "secret": true  // 前端使用 password 输入
  }
}
```

### 2. 配置验证

```rust
// 在 set 时验证配置
fn validate_config(&self, config: &Value) -> Result<(), PluginError> {
    if let Some(temp) = config.get("temperature").and_then(|v| v.as_f64()) {
        if temp < 0.0 || temp > 2.0 {
            return Err(PluginError::ValidationError(
                "temperature 必须在 0-2 之间".into()
            ));
        }
    }
    Ok(())
}
```

### 3. 流式响应处理

```rust
// 使用 async_stream 实现真正的流式响应
let stream = async_stream::stream! {
    let mut content = String::new();
    let mut stream = response.bytes_stream();
    
    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk_bytes) => {
                // 解析 chunk 并 yield
                if let Some(content) = parse_content(&chunk_bytes) {
                    content.push_str(content);
                    yield StreamChunk {
                        data: json!({ "content": content.clone() }),
                        done: false,
                        error: None,
                    };
                }
            }
            Err(e) => {
                yield StreamChunk {
                    data: json!({}),
                    done: true,
                    error: Some(format!("读取流失败: {}", e)),
                };
                return;
            }
        }
    }
};

Ok(InvokeStream::Stream(Box::pin(stream)))
```

---

## 附录

### A. 能力标识常量

```rust
pub const CAPABILITY_LLM: &str = "llm";
pub const CAPABILITY_SESSION: &str = "session";
pub const CAPABILITY_MEMORY: &str = "memory";
pub const CAPABILITY_TOOLS: &str = "tools";
pub const CAPABILITY_TELEGRAM: &str = "telegram";
pub const CAPABILITY_DOCKER: &str = "docker";
```

### B. 错误码

| 错误类型 | 描述 |
|----------|------|
| `NotFound` | 插件或路径未找到 |
| `NotImplemented` | 功能未实现 |
| `ValidationError` | 输入验证失败 |
| `InternalError` | 内部错误 |
| `ParseError` | 解析错误 |

### C. 相关文件

- 核心类型定义: `src-tauri/src/core/types.rs`
- 插件 Trait: `src-tauri/src/core/traits.rs`
- 插件注册表: `src-tauri/src/core/registry.rs`
- Home 插件: `src-tauri/src/plugins/home/plugin.rs`
- Agent 插件: `src-tauri/src/plugins/agent/mod.rs`
- Session 插件: `src-tauri/src/plugins/agent/session/plugin.rs`
- OpenAI 插件: `src-tauri/src/plugins/agent/openai/plugin.rs`

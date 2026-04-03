# Symbio API 设计规范

## 概述

Symbio 采用**路径驱动**的插件架构，每个路径对应一个 Plugin 实例，通过三种请求模式（meta/invoke/stream）进行交互。本文档定义统一的 API 规范，确保接口简洁一致。

## 核心概念

### 1. 路径路由

```
路径格式: /{plugin}/{sub_plugin}/{...}/{action}
         ↓         ↓           ↓
       一级      二级        三级
```

**示例**：
- `/` → home 插件根
- `/agent` → agent 插件
- `/agent/openai` → agent 下的 openai 插件
- `/agent/openai/config` → openai 插件的配置接口

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
- `/agent/@llm` → 路由到 openai 插件
- `/agent/@session` → 路由到 session 插件

## 插件层级结构

```
home (根插件)
├── work (工作区插件)
├── agent (智能体插件)
│   ├── openai (LLM 插件，能力: @llm)
│   ├── session (会话插件，能力: @session)
│   ├── memory (记忆插件，能力: @memory)
│   ├── tools (工具插件，能力: @tools)
│   ├── chat (聊天插件)
│   └── docker (Docker 插件，能力: @docker)
└── setting (设置插件)
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
| **会话数据** | `~/.local/share/symbio/sessions/` | 对话历史、上下文 |
| **记忆数据** | `~/.local/share/symbio/memory/` | 持久化记忆条目 |

---

### 统一配置接口

每个需要配置的插件必须实现 `config` path：

#### 获取配置 Schema

```http
POST /meta
Content-Type: application/json

{
  "path": "config"
}
```

**响应**：
```json
{
  "name": "config",
  "description": "OpenAI 配置管理",
  "input_schema": {
    "type": "object",
    "properties": {
      "action": {
        "type": "string",
        "enum": ["get", "set", "schema"],
        "description": "操作类型"
      },
      "config": {
        "type": "object",
        "description": "配置数据（set 操作时使用）"
      }
    },
    "required": ["action"]
  },
  "output_schema": {
    "type": "object",
    "properties": {
      "success": { "type": "boolean" },
      "config": { "type": "object" },
      "schema": { "type": "object" },
      "error": { "type": "string" }
    }
  }
}
```

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

#### 保存配置到文件

```http
POST /invoke
Content-Type: application/json

{
  "path": "config",
  "input": {
    "action": "save"
  }
}
```

#### 从文件加载配置

```http
POST /invoke
Content-Type: application/json

{
  "path": "config",
  "input": {
    "action": "load"
  }
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

**路径**: `/agent/openai/config`

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

**路径**: `/agent/session/config`

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
  "storage_dir": "~/.local/share/symbio/sessions",
  "max_messages": 100,
  "auto_compress": true,
  "compress_threshold": 50
}
```

---

### 3. Memory 插件配置

**路径**: `/agent/memory/config`

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
  "storage_dir": "~/.local/share/symbio/memory",
  "max_entries": 1000,
  "categories": ["preference", "fact", "instruction"]
}
```

---

### 4. Tools 插件配置

**路径**: `/agent/tools/config`

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

### 5. Work 插件配置

**路径**: `/work/config`

**配置结构**：
```typescript
interface WorkConfig {
  workspace_path: string;   // 工作区路径
  auto_save: boolean;       // 自动保存
  auto_save_interval: number; // 自动保存间隔（毫秒）
  recent_files: string[];   // 最近文件列表
}
```

---

### 6. Setting 插件配置

**路径**: `/setting/config`

**配置结构**：
```typescript
interface SettingConfig {
  // 外观
  theme: "light" | "dark" | "system";
  language: string;
  font_size: number;
  sidebar_width: number;
  
  // 编辑器
  tab_size: number;
  line_numbers: boolean;
  word_wrap: boolean;
  
  // 行为
  auto_update: boolean;
  telemetry: boolean;
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

### 配置 Schema 定义

```rust
fn config_schema() -> Value {
    json!({
        "api_base": {
            "type": "string",
            "title": "API Base URL",
            "default": "https://api.openai.com/v1"
        },
        "api_key": {
            "type": "string",
            "title": "API Key",
            "secret": true
        },
        // ...
    })
}
```

---

## API 完整参考

### Home 插件

| 路径 | Action | 描述 |
|------|--------|------|
| `/` | - | 获取插件列表 |
| `/config` | get | 获取所有配置 |
| `/config` | set | 设置配置（批量） |
| `/config` | save | 保存配置到文件 |
| `/config` | load | 从文件加载配置 |
| `/config` | collect | 收集所有子插件配置 |

### Agent 插件

| 路径 | Action | 描述 |
|------|--------|------|
| `/agent` | - | 获取子插件列表和能力 |
| `/agent/config` | get | 获取所有子插件配置 |
| `/agent/config` | set | 分发配置到子插件 |
| `/agent/@llm` | * | 路由到 LLM 插件 |
| `/agent/@session` | * | 路由到会话插件 |

### OpenAI 插件

| 路径 | Action | 描述 |
|------|--------|------|
| `/agent/openai` | status | 获取状态 |
| `/agent/openai` | list_models | 列出可用模型 |
| `/agent/openai` | chat | 发送聊天请求 |
| `/agent/openai/config` | get | 获取配置 |
| `/agent/openai/config` | set | 设置配置 |
| `/agent/openai/config` | schema | 获取配置 Schema |

### Session 插件

| 路径 | Action | 描述 |
|------|--------|------|
| `/agent/session` | get | 获取会话 |
| `/agent/session` | append | 追加消息 |
| `/agent/session` | clear | 清除会话 |
| `/agent/session` | list | 列出所有会话 |
| `/agent/session` | get_context | 获取 LLM 上下文 |
| `/agent/session/config` | get/set | 配置管理 |

### Memory 插件

| 路径 | Action | 描述 |
|------|--------|------|
| `/agent/memory` | store | 存储记忆 |
| `/agent/memory` | recall | 回忆记忆 |
| `/agent/memory` | forget | 删除记忆 |
| `/agent/memory` | list | 列出所有记忆 |
| `/agent/memory` | search | 搜索记忆 |
| `/agent/memory/config` | get/set | 配置管理 |

### Tools 插件

| 路径 | Action | 描述 |
|------|--------|------|
| `/agent/tools` | list | 列出可用工具 |
| `/agent/tools` | execute | 执行工具 |
| `/agent/tools/config` | get/set | 配置管理 |

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

### 3. 配置迁移

```yaml
# config.yaml 包含版本号
version: "1.0"

# 升级时自动迁移
fn migrate_config(config: &mut Value) {
    let version = config.get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("1.0");
    
    if version == "1.0" {
        // 迁移逻辑
        config["version"] = json!("1.1");
    }
}
```

---

## 附录

### A. 能力标识常量

```rust
pub const CAPABILITY_LLM: &str = "llm";
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

# Symbio 错误码参考

> **文档类型：参考** — 错误码的完整清单与处理建议。

## 错误码约定

错误码是 **ABI 的一部分**，跨版本禁止随意变更。

```rust
// symbio_core/error.rs（节选）— PluginError 是枚举，非结构体
pub enum PluginError {
    NotFound(String),          // -> "NOT_FOUND"
    NotImplemented,            // -> "NOT_IMPLEMENTED"
    ValidationError(String),   // -> "VALIDATION_ERROR"
    // ... 共 12 个变体，完整清单见下表
}

impl PluginError {
    /// 获取机器可读的错误码（跨版本稳定，属 ABI 的一部分）
    pub fn code(&self) -> &'static str;
}
```

错误经 `PluginPayload` 的 Error 形态返回：`code` 为机器可读码（下表），`message` 为人类可读信息。

---

## 标准错误码

| 错误码 | HTTP 等价 | 含义 | 常见场景 |
|--------|-----------|------|----------|
| `NOT_FOUND` | 404 | 路由路径不存在或子插件未找到 | 路径拼写错误、插件未注册 |
| `VALIDATION_ERROR` | 400 | 输入参数格式或内容校验不通过 | payload 缺少必填字段 |
| `INTERNAL_ERROR` | 500 | 内部执行异常 | 代码 panic、外部 API 异常 |
| `TIMEOUT` | 408 | 请求超时 | 网络延迟、LLM 响应慢 |
| `FORBIDDEN` | 403 | 权限不足或触发安全策略 | Shell 命令被策略拦截 |
| `NOT_IMPLEMENTED` | 501 | 插件调用未实现 | 插件未实现该子路径 |
| `RATE_LIMITED` | 429 | 请求频率受限 | 触发限流策略 |
| `PARSE_ERROR` | 400 | 解析错误 | JSON 反序列化失败 |
| `ABORTED` | 499 | 操作中止 | 客户端取消、流被中断 |
| `RETRY_WITHOUT_CONTEXT_ID` | 409 | 上下文丢失，需要重试 | 会话上下文过期 |
| `STREAM_ERROR` | 502 | 流解析错误 | 流式响应格式异常 |
| `COMPRESSION_FAILED` | 500 | 压缩失败 | 历史消息压缩失败 |

---

## 错误响应格式

### JSON 格式

```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "路由路径不存在: session/chat/unknown",
    "details": {
      "path": "session/chat/unknown",
      "available": ["session/chat/send", "session/chat/abort", "session/open"]
    }
  }
}
```

### PluginFrame 格式

```rust
PluginFrame::Error(
    "路由路径不存在: agent/unknown".to_string(),
    Some(serde_json::json!({
        "code": "NOT_FOUND",
        "details": { "path": "agent/unknown" }
    }))
)
```

---

## 错误处理最佳实践

### 后端 (Rust)

```rust
use thiserror::derive;

#[derive(Debug, thiserror::Error)]
pub enum MyError {
    #[error("配置缺失: {0}")]
    ConfigMissing(String),  // code = "CONFIG_ERROR"
    
    #[error("外部 API 失败: {0}")]
    ExternalApiFailed(String),  // code = "EXTERNAL_ERROR"
}
```

### 前端 (TypeScript)

```typescript
try {
  const result = await invoke('route_v2', { request });
} catch (error) {
  const { code, message, details } = parseError(error);
  
  switch (code) {
    case 'NOT_FOUND':
      // 提示用户检查路径
      break;
    case 'FORBIDDEN':
      // 提示权限不足
      break;
    case 'TIMEOUT':
      // 提供重试选项
      break;
    default:
      // 通用错误提示
  }
}
```

---

## 错误码扩展

插件可以定义自己的错误码，但必须：

1. **前缀命名**：`{PLUGIN}_{ERROR}` (如 `MODEL_RATE_LIMIT`)
2. **文档登记**：在此文档添加条目
3. **向后兼容**：已发布的错误码不可删除或改义

### 已知扩展错误码

| 错误码 | 来源插件 | 含义 |
|--------|----------|------|
| `MODEL_AUTH_ERROR` | model | API Key 无效或过期 |
| `MODEL_RATE_LIMIT` | model | 请求频率超限 |
| `BUNDLE_VALIDATION` | agent | agent 目录 manifest 校验失败 |
| `BUNDLE_NOT_FOUND` | agent | agent 目录实例不存在 |
| `MCP_CONNECTION` | mcp | MCP Server 连接失败 |
| `TELEGRAM_AUTH` | telegram | Bot Token 无效 |

---

## 调试技巧

### 启用调试日志

```bash
# Rust 后端
RUST_LOG=debug cargo run

# 前端
VITE_LOG_LEVEL=debug npm run dev
```

### 追踪调用链

每个请求的 `metadata` 应包含 `trace_id`，后端会记录在日志中：

```
INFO trace_id=abc123 path=session/chat/send Start routing
DEBUG trace_id=abc123 path=session/chat/send Routing finished
```

---

> **维护原则**：新增错误码必须在此文档登记，并说明触发条件与处理建议。

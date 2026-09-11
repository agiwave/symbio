# Symbio 配置参考

> **文档类型：参考** — 配置文件结构与字段说明。

## 配置文件位置

| 文件 | 用途 |
|------|------|
| `~/.symbio/config.yaml` | 主配置文件 |
| `~/.symbio/plugins/<plugin>/<id>/provider.json` | 插件专属配置 (如 model) |
| `~/.symbio/agents/` | Agent Bundle 存储目录 |

---

## 主配置 (config.yaml)

```yaml
# ~/.symbio/config.yaml

# 工作区根路径 (可选，默认 ~/.symbio)
workdir: ~/.symbio

# 插件挂载配置
plugins:
  # 每个插件一个条目
  agent:
    plugin_provider: agent    # 插件类型 (对应 submit_object_creator! 注册的 ID)
    enabled: true
  session:
    plugin_provider: session
    enabled: true
  model:
    plugin_provider: model
    enabled: true
  local:
    plugin_provider: local
    enabled: true
  web:
    plugin_provider: web
    enabled: true
  skill:
    plugin_provider: skill
    enabled: true
  mcp:
    plugin_provider: mcp
    enabled: true
  telegram:
    plugin_provider: telegram
    enabled: false

# 存储配置
storage:
  backend: dir                   # dir 或 sqlite
  path: ~/.symbio/storage        # DirStorage 路径
  # sqlite_path: ~/.symbio/symbio.db  # SQLite 路径 (backend=sqlite 时)

# 嵌入配置 (向量检索)
embedding:
  provider: fastembed
  model: BAAI/bge-small-en-v1.5
  dimension: 384

# 日志配置
logging:
  level: info                    # debug / info / warn / error
  format: pretty                 # pretty / json / compact
```

---

## 插件配置

### Model 插件

```json
// ~/.symbio/plugins/model/<id>/provider.json
{
  "default_provider_id": "openai_main",
  "providers": {
    "openai_main": {
      "provider_type": "openai_chat",
      "api_key": "sk-...",
      "model": "gpt-4-turbo",
      "base_url": "https://api.openai.com/v1",
      "temperature": 0.7,
      "max_tokens": 4096,
      "min_interval_ms": 1000
    },
    "anthropic_backup": {
      "provider_type": "anthropic_messages",
      "api_key": "sk-ant-...",
      "model": "claude-3-opus-20240229",
      "base_url": "https://api.anthropic.com/v1"
    }
  }
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `provider_type` | String | ✅ | `openai_chat` / `openai_responses` / `anthropic_messages` / `gemini_api` |
| `api_key` | String | ✅ | API 密钥 |
| `model` | String | ✅ | 模型名称 |
| `base_url` | String | ❌ | 自定义 API 端点 (用于代理) |
| `temperature` | Float | ❌ | 采样温度 |
| `max_tokens` | Int | ❌ | 最大输出 token 数 |
| `min_interval_ms` | Int | ❌ | 最小请求间隔 (限流) |

### MCP 插件

```yaml
# 在 config.yaml 或独立配置中
mcp_servers:
  filesystem:
    transport: stdio
    command: "npx"
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/path"]
  web_fetch:
    transport: http
    url: "http://localhost:3000/mcp"
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `transport` | String | ✅ | `stdio` 或 `http` |
| `command` | String | stdio 时 | 可执行文件 |
| `args` | List[String] | stdio 时 | 命令行参数 |
| `url` | String | http 时 | HTTP 端点 |

### Telegram 插件

```yaml
telegram:
  bots:
    my_bot:
      token: "123456:ABC-DEF..."
      allowed_chats: [123456789]
      webhook_url: "https://example.com/webhook"
```

### Gateway 插件

扁平键配置，整体持久化于 `symbio.plugins.gateway`（前端"设置页"经 `gateway/config/get` / `gateway/config/set` 按键读写，`set` 为整体替换）：

| 键 | 默认值 | 说明 |
|----|--------|------|
| `inbound_enabled` | `false` | 是否启动 HTTP/WS 入站服务（默认仅 Tauri IPC，不监听端口） |
| `inbound_protocol` | `native` | `native`（仅 IPC）或 `http`（监听端口） |
| `inbound_bind` | `127.0.0.1` | 监听地址 |
| `inbound_port` | `9231` | 监听端口 |
| `inbound_token` | 空 | Bearer 令牌；**为空仅允许回环地址**，非回环监听必须设置 |
| `inbound_readonly` | `false` | 只读模式：仅放行查询类路径（白名单见 `gateway/config.rs::is_readonly_allowed`） |

> 出站配置（前端连向何处）已不在本表——连接目标由前端"系统目录"切换器统一管理（localStorage 为权威），经 `initGatewayTransport` 决定 native / http 出站，不再持久化于网关插件配置。
>
> 只读白名单（精确匹配）：`entities/list`、`entities/get`、`entities/status`、`entities/detail`、`entities/providers`、`session/get_messages`、`config/get`、`home/get_homedir`、`work/get_workspace`，另有 `config/get`、`entities/list`、`entities/get`、`entities/detail` 四个前缀匹配。设计定位是**兜底而非完整安全边界**：即便令牌泄露到可信内网，也只能读取而无法触发写操作与命令执行。
>
> **网关自身配置不在白名单内**：`gateway/*` 接口恒走 native（前端不经 HTTP 访问本插件），且 `gateway/config/get` 会返回 `inbound_token`——只读模式下一旦放行即可读走令牌，故 `gateway/config/get` 与 `gateway/config/set` 一律拒绝。

---

## 环境变量

| 变量 | 用途 | 示例 |
|------|------|------|
| `RUST_LOG` | Rust 日志级别 | `debug`, `symbio=trace` |
| `VITE_LOG_LEVEL` | 前端日志级别 | `debug`, `info` |
| `SYMBIO_WORKDIR` | 覆盖工作区路径 | `/custom/path` |
| `OPENAI_API_KEY` | OpenAI API Key (备用) | `sk-...` |
| `ANTHROPIC_API_KEY` | Anthropic API Key (备用) | `sk-ant-...` |

---

## 配置热加载

部分配置支持热加载 (无需重启)：

| 配置 | 热加载方式 |
|------|-----------|
| `plugins/*/config` | 通过 `route("{plugin}/config", ...)` 实时更新 |
| `mcp_servers` | 注册/注销 MCP Server 即时生效 |
| `logging.level` | 通过 tracing-subscriber 动态调整 |

---

## 配置验证

启动时验证：

1. **必填字段**：缺失则启动失败
2. **插件注册**：`plugin_provider` 必须在 `ObjectCreatorRegistry` 中存在
3. **路径合法性**：`workdir` 必须存在且可写

---

> **维护原则**：新增配置项必须在此文档登记，包括类型、默认值、验证规则。

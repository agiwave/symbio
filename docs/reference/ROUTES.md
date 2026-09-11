# Symbio 路由参考

> **文档类型：参考** — 所有可用路径的完整清单。

## 路由语法

```
{container}/{plugin}/{action}
```

- `{container}` - 容器插件 (如 `worker`)
- `{plugin}` - 目标插件 (如 `agent`, `session`, `model`)
- `{action}` - 具体能力 (如 `chat`, `list`, `config`)

> `worker` 前缀可省略也可显式写出：`session/chat/send` 与 `worker/session/chat/send` 等价
> （前者由 home 兜底转发给 worker）。统一实体注册表登记的 `session` / `model` 前缀带 `worker/`。

## 通用路由

### 内省

| 路径 | 用途 | 返回 |
|------|------|------|
| `_root` | 查询当前节点子插件拓扑 | 子树结构 |

### 配置管理

常量定义见 `symbio_core/paths.rs`（`CONFIG_GET` = `config/get`、`CONFIG_SET` = `config/set`）。

| 路径 | 用途 | 方法 |
|------|------|------|
| `{plugin}/config/get` | 获取配置 | GET (payload 空) |
| `{plugin}/config/set` | 设置配置 | POST (payload 含配置) |
| `{plugin}/config/schema` | 获取配置 JSON Schema | GET |

> `config/schema` 目前仅 `session`、`model` 提供；其余插件按各自 `config/get` 返回的结构直接读写。
> 各插件 `config/set` 内部会自行向父级发起 `save_config`，配置才真正落盘。

### 统一实体管理

资源型插件的标准实体操作（`list` / `get` / `upload` / `delete` / `status`，可选 `detail` / `watch`），详见下文「通用：统一实体管理」。

---

## Home 插件（根容器）

根插件，自身终结以下路径；其余路径按首段剥离后转发给 `worker`（Composite），
未命中的一律兜底转发给 `worker`（源码：`plugins/home/plugin.rs` 的 `route`）。

| 路径 | 用途 | 备注 |
|------|------|------|
| `home/get_homedir` | 返回当前系统目录（homedir）与 bootstrap 路径 | 只读；gateway 只读白名单放行 |
| `home/reload` | 热重载：切换 homedir（可选）+ 重建全部子插件 | 前端切换系统目录后调用 |
| `work/set_workspace` | 切换工作区，写入 `work.workdir` 与最近使用列表 | 写操作 |
| `work/get_workspace` | 读取当前工作区、展开路径与最近工作区列表 | 只读；gateway 只读白名单放行 |
| `entities/providers` | 宿主级实体 provider 注册表（前端启动拉取生成左侧导航） | 顺序可由 `symbio.provider_order` 覆盖 |
| `save_config` | 将内存配置原子化写入 `<homedir>/config.yaml` | 由子插件 `config/set` 上行触发 |

---

## Agent 插件

### 管理路由

| 路径 | 用途 |
|------|------|
| `agent/bundle/list` | 列出所有 Agent Bundle |
| `agent/bundle/get` | 获取单个 Bundle 详情 |
| `agent/bundle/upload` | 上传 Bundle (zip 或 JSON) |
| `agent/bundle/export` | 导出 Bundle |
| `agent/bundle/delete` | 删除 Bundle |
| `agent/bundle/preview` | 预览 Bundle 内容 |

### 身份工具

| 路径 | 用途 |
|------|------|
| `agent/agent_identity` | 返回 Agent 人格/提示词片段 |

### 统一实体

| 路径 | 用途 |
|------|------|
| `agent/entities/*` | OAB Bundle 实体（list/get/upload/delete/status/detail）；条目为容器，内部托管 `prompt` / `skill` / `mcp` 三类文件级子实体 |

---

## Session 插件

### 会话管理

| 路径 | 用途 | 返回类型 |
|------|------|----------|
| `session/chat/send` | 发起 AI 对话（流式；实际入口） | `Session` |
| `session/chat/abort` | 中止进行中的对话 | `Empty` |
| `session/get_messages` | 获取对话历史 | `Data` |
| `session/open` | 打开/创建会话 | `Data` |
| `session/update` | 更新会话属性 | `Data` |
| `session/clear` | 清空会话历史 | `Data` |
| `session/chat/clear_messages` | 清空指定消息 | `Data` |
| `session/chat/delete_message` | 删除单条消息 | `Data` |
| `session/chat/update_message` | 更新单条消息 | `Data` |
| `session/append` | 追加消息 | `Data` |
| `session/heartbeat/trigger` | 触发一次心跳 | `Data` |
| `session/config/get` \| `config/set` \| `config/schema` | 会话配置读写与 Schema | `Data` |
| `session/entities/*` | 会话作为统一实体（不可 upload，创建走前端专属 editor） | `Data` |

### 聊天流程

```
1. route("session/chat/send", {messages, agent_id, ...})  → 返回 Session
2. 从 PluginChannel 接收 PluginFrame
3. 帧类型:
   - Data({type: "text_delta", content: "..."})     → 增量文本
   - Data({type: "tool_use", name: "local/shell"})  → 工具调用
   - Data({type: "done"})                           → 完成
   - Error(msg, details)                            → 错误
```

---

## Model 插件

| 路径 | 用途 | 返回类型 |
|------|------|----------|
| `model/chat` | 调用 LLM 推理 (流式) | `Session` |
| `model/status` | 获取当前 Provider 状态 | `Data` |
| `model/config/get` \| `config/set` \| `config/schema` | Provider 配置读写与 Schema | `Data` |
| `model/entities/*` | 模型 Provider 作为统一实体（list/get/upload/delete/status） | `Data` |

### 配置结构

```yaml
# ~/.symbio/plugins/model/<id>/provider.json
{
  "default_provider_id": "openai_main",
  "providers": {
    "openai_main": {
      "provider_type": "openai_chat",
      "api_key": "sk-...",
      "model": "gpt-4",
      "base_url": "https://api.openai.com/v1"
    }
  }
}
```

### 支持的协议

| protocol_id | 提供商 | API 端点 |
|-------------|--------|----------|
| `openai_chat` | OpenAI | `/v1/chat/completions` |
| `openai_responses` | OpenAI | `/v1/responses` |
| `anthropic_messages` | Anthropic | `/v1/messages` |
| `gemini_api` | Google | `generateContent` |

---

## Local 插件

| 路径 | 用途 | 安全风险 |
|------|------|----------|
| `local/shell` | 执行 Shell 命令 | ⚠️ 高危 |
| `local/file_read` | 读取文件 | 安全 |
| `local/file_write` | 写入文件 | ⚠️ 中 |
| `local/file_edit` | 编辑文件 | ⚠️ 中 |
| `local/glob_search` | 文件模式搜索 | 安全 |
| `local/content_search` | 内容搜索 (ripgrep) | 安全 |

### Shell 命令策略

高危操作触发 `ToolApprovalRequest`，需用户显式授权。

---

## Web 插件

| 路径 | 用途 |
|------|------|
| `web/http_request` | 发起 HTTP 请求 |
| `web/web_search` | 网页搜索 |
| `web/web_fetch` | 抓取网页内容 |

---

## Skill 插件

| 路径 | 用途 |
|------|------|
| `skill/entities/list` | 列出已安装 Skill |
| `skill/entities/get` | 获取 Skill 详情 |
| `skill/entities/upload` | 安装 Skill |
| `skill/entities/delete` | 卸载 Skill |
| `skill/entities/status` | Skill 启用/禁用状态 |
| `skill/execute` | 按名称执行 Skill（载荷 `{name, args}`） |
| `skill/config/get` \| `config/set` | Skill 插件配置读写 |

### Skill 结构

```
skills/<name>/
├── SKILL.md          # Skill 定义 (frontmatter + 说明)
└── ...               # 附带资源
```

---

## MCP 插件

| 路径 | 用途 |
|------|------|
| `mcp/entities/list` | 列出 MCP Server |
| `mcp/entities/get` | 获取 Server 详情 |
| `mcp/entities/upload` | 注册 Server |
| `mcp/entities/delete` | 注销 Server |
| `mcp/entities/status` | Server 启用/禁用状态 |
| `mcp/config/get` \| `config/set` | 插件元数据读写（实际 server 数据在 `~/.symbio/plugins/mcps/<name>/server.json`） |

### MCP 配置

```yaml
mcp_servers:
  my_server:
    transport: stdio        # 或 http
    command: "npx"
    args: ["-y", "mcp-server"]
    # transport: http
    # url: "http://localhost:3000"
```

---

## Telegram 插件

| 路径 | 用途 |
|------|------|
| `telegram/send` | 发送消息 |
| `telegram/get_updates` | 拉取更新（长轮询） |
| `telegram/set_chat_id` | 绑定 chat id |
| `telegram/start_listener` | 启动后台监听 |
| `telegram/stop_listener` | 停止后台监听 |
| `telegram/status` | 运行状态 |
| `telegram/config/get` \| `config/set` | Bot 配置读写 |

> Telegram 未接入统一实体协议（不在 `provider_registry()` 中），Bot 配置走 `config/get|set`。

---

## 通用：统一实体管理（`{plugin}/entities/*`）

资源型插件实现统一实体协议（`symbio_core/entities.rs` 的 `EntityProvider` trait），路径约定 `<plugin>/entities/<op>`：

| 路径 | 用途 |
|------|------|
| `{plugin}/entities/list` | 列出实体 |
| `{plugin}/entities/get` | 读取单个实体 |
| `{plugin}/entities/upload` | 导入/创建实体 |
| `{plugin}/entities/delete` | 删除实体 |
| `{plugin}/entities/status` | 实体状态（启用/禁用） |
| `{plugin}/entities/detail` | 详情页定义下发（`DetailDefinition`；可选能力） |
| `{plugin}/entities/watch` / `unwatch` | 订阅/取消订阅容器子实体变更（可选，树视图挂载期配对调用） |

已注册 provider（`symbio_core/entities.rs` 的 `provider_registry()`，顺序即前端导航默认顺序）：

| kind | 路径前缀 | 可 upload | 容器子实体 |
|------|----------|-----------|------------|
| `session` 会话 | `worker/session` | 否（走 SessionStore） | 子会话 |
| `model` 模型 | `worker/model` | 是 | — |
| `agent` 智能体 | `agent` | 是 | `prompt` / `skill` / `mcp` |
| `skill` 技能 | `skill` | 是 | — |
| `mcp` MCP | `mcp` | 是 | — |
| `setting` 设置 | `setting` | 否（分区固定） | — |

展示顺序可由服务器端配置 `symbio.provider_order`（`{kind: order}`）覆盖，无需改代码。
能力开关与协议约定见 [PROTOCOLS.md](../architecture/PROTOCOLS.md)「统一实体管理」，机制详解见 [design/entity-management-mechanism.md](../design/entity-management-mechanism.md)。

---

## Gateway 插件

HTTP/WebSocket 入站网关（`plugins/gateway/server.rs`），外部客户端与 route_v2 同构接入插件树：

| HTTP 端点 | 用途 |
|------|------|
| `POST /api/v1/invoke` | 同步调用（`PluginMessageWire` → `PluginPayloadWire`） |
| `WS /api/v1/ws` | 双向流：首帧 `PluginMessageWire`，后续双向 `PluginFrame` |
| `GET /api/v1/health` | 健康检查（`{"ok":true}`） |

### 作为插件的路由

挂载于 `worker`（Composite）之下，外部入站请求最终由 gateway 转发给父级 composite：

| 路径 | 用途 |
|------|------|
| `gateway/config/get` | 读取网关配置（扁平键 `inbound_*`） |
| `gateway/config/set` | 整体替换配置 → 上行 `save_config` → **内部 stop + start 重建监听** |
| `gateway/status` | 运行状态（是否启用、协议、监听地址与端口、是否已在监听） |

> `gateway/*` 自身接口**恒走 native**（前端不经 HTTP 访问本插件）。前端出站分发（native / http）由「系统目录」切换器管理，不再由 gateway 配置驱动。

安全：非回环地址需 `inbound_token` 鉴权（回环地址免鉴权）；`inbound_readonly` 开启后仅放行只读白名单
（`config/get`、`entities/list|get|detail|status`、`entities/providers`、`session/get_messages`、
`home/get_homedir`、`work/get_workspace`），详见 [CONFIGURATION.md](CONFIGURATION.md)。

---

## Setting 插件

| 路径 | 用途 |
|------|------|
| `setting/list` | 列出设置分区清单（general / model / …） |
| `setting/get` | 读取单个分区的配置项 |
| `setting/config/get` \| `config/set` | 系统配置整体读写 |
| `setting/entities/list` | 设置分区作为统一实体（其余实体操作返回未实现） |

> 新增可配置插件需在 `setting` 的 `SETTING_SECTIONS` 与 `detail_definition()` 登记；
> 各分区的保存由前端 editor 经对应插件 `config/set` 自持完成。

---

## Hook 插件

| 路径 | 用途 |
|------|------|
| `hook/register` | 注册钩子 |
| `hook/trigger` | 触发钩子 |
| `hook/list` | 列出已注册钩子 |

### 系统事件

| 事件 | 触发时机 |
|------|----------|
| `pre_tool_use` | 工具调用前 |
| `post_tool_use` | 工具调用后 |
| `pre_file_write` | 文件写入前 |
| `post_file_write` | 文件写入后 |
| `pre_shell_execute` | Shell 执行前 |
| `post_shell_execute` | Shell 执行后 |
| `user_prompt_submit` | 用户提交提示词 |
| `session_start` | 会话开始 |
| `session_end` | 会话结束 |

---

## Event Bus 插件

| 路径 | 用途 |
|------|------|
| `event_bus/subscribe` | 订阅事件 |
| `event_bus/publish` | 发布事件 |

---

> **维护原则**：新增路由必须在此文档登记，包括路径、用途、输入输出格式。

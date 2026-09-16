# Symbio 路由参考

> **文档类型：参考** — 所有可用路径的完整清单。

## 路由语法

```
{container}/{plugin}/{action}
```

- `{container}` - 容器插件 (如 `worker`)
- `{plugin}` - 目标插件 (如 `agent`, `session`, `model`)
- `{action}` - 具体能力 (如 `chat`, `list`, `status`)

> **资源与配置都不走这条语法**：资源统一经 `vdfs/*`（`.vdfs/…` 寻址），配置就是
> 资源树里的一个文档（`.vdfs/<插件>/PLUGIN.yml`）。本文件的 `{plugin}/*` 路由只剩
> 那些**不是资源**的能力（会话编排、模型推理、网关状态…）。

> `worker` 前缀可省略也可显式写出：`session/chat/send` 与 `worker/session/chat/send` 等价
> （前者由 home 兜底转发给 worker）。

## 通用路由

### 内省

| 路径 | 用途 | 返回 |
|------|------|------|
| `_root` | 查询当前节点子插件拓扑 | 子树结构 |

### 配置管理（**没有配置路由**）

> **`{plugin}/config/get` / `{plugin}/config/set` 已整体下线**（`CONFIG_GET` /
> `CONFIG_SET` 常量、各插件的两个分支、`save_config` 上行链、`config/schema`
> 都已删除）。
>
> 配置现在就是一个**普通的可寻址文档**：`<插件目录>/PLUGIN.yml`，对外地址
> `.vdfs/<插件>/PLUGIN.yml`（`ext = form`）。读写用的就是 `vdfs/read` /
> `vdfs/write`——与任何其它资源同一条链路、同一套寻址，因此前端与 LLM 用同一种
> 方式改配置。定义与校验归**配置的拥有者**，默认值从它自己的 `Default` 读出。
> 见 [design/vdfs.md](../design/vdfs.md) §3.4。

### 资源（VDFS）

资源型插件**不提供任何资源类私有路由**：其资源统一经 VDFS 寻址
（`.vdfs/<插件名>/…`，操作
`vdfs/list|tree|stat|read|write|mkdir|delete|move|edit|search|watch|unwatch|action`）。
每个 `.vdfs` 子目录由对应插件自己的 `impl VdfsProvider` 提供，落盘（需要持久化的那些）
经 `symbio/src/providers/vdfs_service/`——见 [design/vdfs.md](../design/vdfs.md) §13.4。
历史上的 `{plugin}/entities/*` 一族端点自 S11 起无任何路由，本文件不再登记。

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

> `save_config` **已下线**：配置不再由父插件聚合落盘，各插件写自己的
> `<插件目录>/PLUGIN.yml`（`ConfigFile::apply` = 校验 → 落内存 → 落自己的文件 →
> 广播）。`home` 只保留 `home/*` 与 `work/*`。

---

## Agent 插件

### 管理路由（**全部下线**）

> Bundle 的**访问 / 新建 / 删除 / 导出已全部由 VDFS 承担**（`.vdfs/agent/…`）：
> `vdfs/list` / `vdfs/read` / `vdfs/write`（新建类型 `zip`，即整包导入）/
> `vdfs/delete` / 节点动作 `export`（`vdfs/action`）。
> 故 `bundle/list|get|upload|delete|preview` 已于 S12 删除、`bundle/export`
> 于 S13 删除——**agent 插件不再有任何自有协议路由**，`route()` 直接返回
> `NotFound` 并指引到 VDFS。

### 身份能力（**不是路由**）

| 能力名 | 用途 |
|------|------|
| `agent_identity` | 返回 Agent 人格/提示词片段（由能力管理器 `invoke` 调用，不经 `route`） |

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

> `session/config/get` / `config/set` **已下线**：会话配置在
> `.vdfs/session/PLUGIN.yml`（`ext = form`），读写走 `vdfs/read` / `vdfs/write`。

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

> **`model` 已无自有路由**（Phase E-② 后 `ModelProvider::execute_turn` 由 `session`
> 在会话循环内直连调用，不占路由；配置读写全在 VDFS 上）。本插件不再有路由条目。

历史入口（**均已下线**，列此仅作迁移指引）：`model/chat` 收归 session 直连（改用
`session/chat/send`）；`model/config/*` 迁到 `.vdfs/model/<id>` 与 `.vdfs/model/PLUGIN.yml`；
`model/status` 改由节点动作 `vdfs/action { action: "test" }` 返回；`model/chat_sync`
本就是 NotImplemented 占位。

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
| `skill/execute` | 按名称执行 Skill（载荷 `{name, args}`） |

> `skill/config/get` / `config/set` **已下线**，且本插件**不设配置文档**——
> 技能清单与内容不设私有路由：已安装技能经 `.vdfs/skill` 寻址
> （一个技能 = 一个目录，主文件 `SKILL.md`，条目内部可下钻）。

### Skill 结构

```
skills/<name>/
├── SKILL.md          # Skill 定义 (frontmatter + 说明)
└── ...               # 附带资源
```

---

## MCP 插件

> **本插件已无自有路由**：`mcp/config/get` / `config/set` 已下线，且**不设配置文档**
> ——配置就是它的资源树。Server 清单与详情不设私有路由：注册 / 列举 / 启停 /
> 连通性自检一律经 `.vdfs/mcp`（一个 server = 一个目录，主文件 `server.json`），
> 「测试连接」是节点动作 `vdfs/action { action: "test" }`。

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

> `telegram/config/get` / `config/set` **已下线**：本插件挂载 `.vdfs/telegram`，
> 内容就是一份配置文档 `.vdfs/telegram/PLUGIN.yml`（`ext = form`），读写走
> `vdfs/read` / `vdfs/write`。

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
| `gateway/status` | 运行状态（是否启用、协议、监听地址与端口、是否已在监听） |

> `gateway/config/get` / `config/set` **已下线**：配置就是
> `.vdfs/gateway/PLUGIN.yml`（扁平键 `inbound_*`）。前端「设置」页点开的正是这份
> 表单；写入走 `vdfs/write` → `ConfigFile::apply` 落盘后，本插件在自己的 `write`
> 里做副作用（**内部 stop + start 重建监听**）——机制不引入回调抽象。
>
> `gateway/*` 自身接口**恒走 native**（前端不经 HTTP 访问本插件）。前端出站分发（native / http）由「系统目录」切换器管理，不再由 gateway 配置驱动。

安全：非回环地址需 `inbound_token` 鉴权（回环地址免鉴权）；`inbound_readonly` 开启后仅放行只读白名单
（`vdfs/list|tree|stat|read|search`、`session/get_messages`、`home/get_homedir`、
`work/get_workspace`；`gateway/*` 显式排除，且 `vdfs/read` 只要落在任何插件的
`PLUGIN.yml` 上就拒绝——配置可能含凭据），详见 [CONFIGURATION.md](CONFIGURATION.md)。

---

## Setting 插件

> **本插件已无自有路由**：`setting/config/get` / `config/set` 与更早的
> `setting/list` / `setting/get` 全部下线。
>
> 分区清单与取值分别由 `.vdfs/setting` 的 `vdfs/list` / `vdfs/read` 承担。
> 清单 = **各插件交出来的配置条目 + 自有分区**（`appearance` / `about`）：
> 前者由各插件在 `traverse` 里经 `announce_configurable` 声明，容器用共享收集器
> 收下并写回请求 ctx（见 [design/vdfs.md](../design/vdfs.md) §13.1）——因此
> **新增一个可配置插件不需要在本插件登记任何东西**；自有分区的数据在前端 store，
> `read` / `write` 对它们恒 `Forbidden`。

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

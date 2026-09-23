# Symbio 路由参考

> **文档类型：参考** — **所有可用路径的完整清单**。本文**人工维护**，因此也是唯一会漂移
> 的一份：`docs/CURRENT.md` §1 的「自有路由」列由代码生成，必然与代码一致——**两者不一致时以它为准**。
>
> - **守卫**：[`scripts/plugin-entry-audit.mjs`](../../scripts/plugin-entry-audit.mjs) 的 **E-006**
>   （清单里的路径前缀必须是插件目录名）。
> - **地址规则**（前缀是**插件目录名**、绝对地址 vs 相对臂、`traverse` 的两个端点）见
>   [design/plugin-route-address.md](../design/plugin-route-address.md)。
> - **退役路由**：不在此处保留叙述——去向见本文 §退役路由索引。
>
> ⚠️ **本文只写「现在有哪些路由」**。某条路由**为什么**被删 / 被迁，是决策与变更史，
> 归 [DECISIONS.md](../DECISIONS.md) 与 `git log`（见 [docs/README.md](../README.md#文档职责边界一个事实只有一个-owner)）。

## 路由语法

```
{container}/{plugin}/{action}
```

- `{container}` - 容器插件 (如 `worker`)
- `{plugin}` - 目标插件 (如 `agent`, `session`, `model`)
- `{action}` - 具体能力 (如 `chat`, `list`, `status`)

> `worker` 前缀可省略也可显式写出：`session/chat/send` 与 `worker/session/chat/send` 等价
> （前者由 home 兜底转发给 worker）。

> **资源与配置都不走这条语法**：资源统一经 `vdfs/*`（`<根>/…` 寻址），配置就是
> 资源树里的一个文档（`<根>/<插件>/PLUGIN.yml`）。本文件的 `{plugin}/*` 路由只剩
> 那些**不是资源**的能力（会话编排、模型推理、网关状态…）。

## 通用路由

### 内省

| 路径 | 用途 | 返回 |
|------|------|------|
| `_root` | 查询当前节点子插件拓扑 | 子树结构 |

### 配置管理（**没有配置路由**）

配置就是一个**普通的可寻址文档**：`<插件目录>/PLUGIN.yml`，对外地址
`<根>/<插件>/PLUGIN.yml`（`ext = form`）。读写用的就是 `vdfs/read` / `vdfs/write`——
与任何其它资源同一条链路、同一套寻址，因此前端与 LLM 用同一种方式改配置。
定义与校验归**配置的拥有者**，默认值从它自己的 `Default` 读出（见 [design/vdfs.md](../design/vdfs.md) §3.4）。

### 资源（VDFS）

资源型插件**不提供任何资源类私有路由**：其资源统一经 VDFS 寻址（`<根>/<插件名>/…`）。
**操作闭集见 [CURRENT.md](../CURRENT.md) §3.2**（源码 `plugins/vdfs/protocol.rs::VDFS_OPS`，
有测试锁计数）——本文不抄那份清单。
每个 `<根>` 子目录由对应插件自己的 `impl VdfsProvider` 提供，落盘经
`symbio/src/providers/vdfs_service/`（见 [design/vdfs.md](../design/vdfs.md) §11 / §13.4）。

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

> **两条 `work/*` 臂在此终结**（`home` 是系统根、不挂前缀）：与 `work` **插件**的命名空间
> 同名，但 `work` 插件的 `route` 恒 `NotFound`，故不冲突。

---

## Agent 插件

**本插件无自有协议路由**：agent 目录的访问 / 新建 / 删除 / 导出全部由 VDFS 承担
（`<根>/agent/…`）——`vdfs/list` / `vdfs/read` / `vdfs/write`（新建类型 `zip`，即整包导入）/
`vdfs/delete` / 节点动作 `export`（`vdfs/action`）。故 `route()` 直接返回 `NotFound` 并指引到 VDFS。

**能力贡献（不是路由）**：唯一 LLM 工具 `agent_run`（启动 / 续跑子智能体，源码
`plugins/agent/host/subagent.rs`），由模型经能力调用触达，不经 `route`。
**LLM 可见工具清单以 [CURRENT.md](../CURRENT.md) §2 为准**，本文件不重复登记。

---

## Session 插件

### 会话管理

| 路径 | 用途 | 返回类型 |
|------|------|----------|
| `session/chat/send` | 发起 AI 对话（流式；实际入口） | `Session` |
| `session/chat/abort` | 中止进行中的对话 | `Empty` |

> 会话与消息的增删改查**全部**经 VDFS 地址完成；`chat/send` 与 `chat/abort` 是编排 / 控制，
> 不是数据操作。两者的**实时面**都走 `event_bus` 的 `vdfs` 频道（见
> [PROTOCOLS.md](../architecture/PROTOCOLS.md) §事件总线频道）。

### 会话级操作已并入 VDFS（专用路由不再存在）

| 操作 | 入口 |
|---|---|
| 列会话清单 | `vdfs/list(<根>/session)` |
| 读整份转写 | `vdfs/read(<根>/session/<id>)` |
| 新建会话（id 由后端生成） | `vdfs/write(<根>/session, { create: true })` |
| 新建 / 打开**具名**会话（id 由调用方给） | `vdfs/write(<根>/session/<id>, { create: true })`——不存在则就地创建，已存在则浅合并 metadata |
| 删除会话 | `vdfs/delete(<根>/session/<id>)` |
| 改 metadata / 标题 | `vdfs/write(<根>/session/<id>)` |
| 改写某条消息 | `vdfs/write(<根>/session/<id>/message/<mid>)` |
| 删该条及其后 | `vdfs/action(…/message/<mid>, "truncate")` |
| 清空历史 | `vdfs/action(…/message, "clear")` |

> **发言仍只有聊天协议一处**（`chat/send`）：新增消息会触发一整轮编排，不是一次写入；
> 而**改写与删除**是普通的 VDFS 节点操作——判据是「触发不触发编排」，不是「碰不碰消息」。
> 机制细节见 [`session/docs/vdfs-session-messages.md`](../../symbio/src/plugins/session/docs/vdfs-session-messages.md)。

### 聊天流程

```
1. route("session/chat/send", {messages, agent_id, ...})  → 返回 Session
2. 从 PluginChannel 接收 PluginFrame
3. 帧类型:
   - Data({type: "text_delta", content: "..."})     → 增量文本
   - Data({type: "tool_use", name: "local/content_search"})  → 工具调用
   - Data({type: "done"})                           → 完成
   - Error(msg, details)                            → 错误
```

---

## Model 插件

**本插件无自有路由**：`ModelProvider::execute_turn` 由 `session` 在会话循环内**直连调用**，
不占路由；配置读写全在 VDFS 上（`<根>/model/<id>` 与 `<根>/model/PLUGIN.yml`）。

### 支持的协议

| protocol_id | 提供商 | API 端点 |
|-------------|--------|----------|
| `openai_chat` | OpenAI | `/v1/chat/completions` |
| `openai_responses` | OpenAI | `/v1/responses` |
| `anthropic_messages` | Anthropic | `/v1/messages` |
| `gemini_api` | Google | `generateContent` |

> 协议适配内化于 `plugins/model/protocols/`，机制见 `plugins/model/README.md`；
> 为什么支持多协议见 [ADR-004](../DECISIONS.md)。

---

## Local 插件

> **路由按工具名动态分发**：`local/<工具短名>`，与 LLM 工具是同一份集合
> （`route()` 里 `tool_impls.iter().find(|t| t.name() == path)`）。

| 路径 | 用途 | 安全风险 |
|------|------|----------|
| `local/cmd`（Windows）/ `local/sh`（macOS / Linux） | 执行 Shell 命令（工具名按操作系统取） | ⚠️ 高危 |
| `local/content_search` | 内容搜索 (ripgrep) | 安全 |
| `local/todo_write` | 会话任务清单（`LastOnly` 保留策略） | 安全 |
| `local/codebase_search` | 语义代码搜索 | 安全 |
| `local/ask_user` | 向用户提结构化问题（单问题或 1~4 批量，自动补 `Other`） | 安全 |

> **文件编辑类原生工具已迁入 VDFS 物理层**，由 `vdfs` 插件以
> `vdfs_read` / `vdfs_write` / `vdfs_edit` / `vdfs_search` 等统一暴露，本插件不再提供。
> `ask_user` 产 `user_prompt` 节点等用户回答（与工具审批同一套回填机制）；自动模式下
> 不产节点、返回 `tool_unavailable` 让模型自行继续。

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

> 本插件**不设配置文档**。技能清单与内容不设私有路由：已安装技能经 `<根>/skill` 寻址
> （一个技能 = 一个目录，主文件 `SKILL.md`，条目内部可下钻）。

### Skill 结构

```
skills/<name>/
├── SKILL.md          # Skill 定义 (frontmatter + 说明)
└── ...               # 附带资源
```

---

## MCP 插件

**本插件无自有路由，也不设配置文档**——配置就是它的资源树。Server 清单与详情不设私有路由：
注册 / 列举 / 启停 / 连通性自检一律经 `<根>/mcp`（一个 server = 一个目录，主文件 `server.json`），
「测试连接」是节点动作 `vdfs/action { action: "test" }`。

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

> 本插件挂载 `<根>/telegram`，内容就是一份配置文档 `<根>/telegram/PLUGIN.yml`（`ext = form`），
> 读写走 `vdfs/read` / `vdfs/write`。

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

> 配置就是 `<根>/gateway/PLUGIN.yml`（扁平键 `inbound_*`）；写入走 `vdfs/write` →
> `ConfigFile::apply` 落盘后，本插件在自己的 `write` 里做副作用（**内部 stop + start 重建监听**）。
> `gateway/*` 自身接口**恒走 native**（前端不经 HTTP 访问本插件）。

安全：非回环地址需 `inbound_token` 鉴权（回环地址免鉴权）；`inbound_readonly` 开启后仅放行只读白名单
（`vdfs/list|tree|stat|read|search`、`home/get_homedir`、`work/get_workspace`；`gateway/*` 显式排除，
且 `vdfs/read` 只要落在任何插件的 `PLUGIN.yml` 上就拒绝——配置可能含凭据），
详见 [CONFIGURATION.md](CONFIGURATION.md)。

---

## Setting 插件

**本插件无自有路由**。分区清单与取值分别由 `<根>/setting` 的 `vdfs/list` / `vdfs/read` 承担。

清单 = **各插件交出来的配置条目 + 自有分区**（`appearance` / `about`）：前者由各插件在
`traverse` 里经 `announce_configurable` 声明，容器用共享收集器收下并写回请求 ctx
（见 [design/vdfs.md](../design/vdfs.md) §13.1）——因此**新增一个可配置插件不需要在本插件
登记任何东西**；自有分区的数据在前端 store，`read` / `write` 对它们恒 `Forbidden`。

---

## Hook 插件

> **命名空间是目录名 `hook`，不是 `hooks`。** 容器（`composite`）按**目录名**建实例表并在
> `route` 里按它分发（「目录名 = 实例名」），所以目录名才是真正的路由前缀。
> 真实的调用点见 `symbio_core::paths::HOOK_FIRE`。

| 路径 | 用途 |
|------|------|
| `hook/register` | 注册钩子 |
| `hook/fire` | 触发钩子（源码 `route()` 臂为 `fire`，无 `trigger`） |
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
| `event_bus/subscribe` | 订阅事件（连接级 SSE 风格推送） |
| `event_bus/ping` | 存活探测 |

> `event_bus/publish` **不存在**：本插件是进程内帧广播（如 VDFS 变更、总线自身握手），
> 发布方在进程内直接调用，不经路由。

---

## 退役路由索引

**只登记「路径 → 现在去哪」，不写过程叙述**（决策与理由见 [DECISIONS.md](../DECISIONS.md)、
变更史见 `git log`，完整迁移审计见
[`docs/archive/legacy-route-migration.md`](../archive/legacy-route-migration.md)）：

| 退役路径 | 现在的入口 |
|---|---|
| `{plugin}/config/get` · `{plugin}/config/set` · `{plugin}/config/schema` | `vdfs/read` / `vdfs/write(<根>/<插件>/PLUGIN.yml)` |
| `{plugin}/entities/*` | 各插件自己的 `impl VdfsProvider`（`<根>/<插件名>/…`） |
| `model/chat` · `model/status` · `model/chat_sync` | `session/chat/send` · 节点动作 `vdfs/action { action: "test" }` |
| `agent/list` · `get` · `upload` · `delete` · `preview` · `export` | `<根>/agent` 上的 `vdfs/*` + 节点动作 `export` |
| `session/append` · `session/open` | 编排器直连（`orchestrator/entry.rs`） |
| `session/update` | `vdfs/write(<根>/session/<id>, { create: true })`（具名新建，即 upsert） |
| `session/clear` | `vdfs/delete(<根>/session/<id>)` |
| `session/chat/update_message` | `vdfs/write(…/message/<mid>)` |
| `session/chat/delete_message` | `vdfs/action(…/message/<mid>, "truncate")` |
| `session/chat/clear_messages` | `vdfs/action(…/message, "clear")` |
| `session/get_messages` | 进程内 VDFS 纯接口探测（`stat("<挂载名>/<会话id>")`） |
| `session/options/list` | 会话配置表单字段（`node.schema` / `node.attributes.metadata`） |
| `session/stream` | `event_bus` 的 `vdfs` 频道 |
| `session/heartbeat/trigger` | **能力取消**（往会话发一轮提示词即可） |
| `hook/trigger` · `hooks/*` | `hook/fire`（命名空间是 `hook`） |
| `event_bus/publish` | 进程内直调（不经路由） |

---

> **维护原则**：新增路由必须在此文档登记（路径、用途、输入输出格式）；删除路由时**只把路径
> 挪进 §退役路由索引**，不在此处写过程叙述。

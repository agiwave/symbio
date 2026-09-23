# Symbio 路由参考

> **文档类型：参考** — 所有可用路径的完整清单。

> **本文是人工维护的清单**，因此也是唯一会漂移的那一份——`docs/CURRENT.md` 的
> 「自有路由」列由代码生成，必然一致。守本文的是
> [`scripts/plugin-entry-audit.mjs`](../../scripts/plugin-entry-audit.mjs) 的 **E-006**
> （判前缀合法性：清单里必须能提到已退役的路由，但**前缀不能写错**——
> `hooks/fire` 就曾在这里长期存在）。
>
> 地址规则（前缀是**插件目录名**、绝对地址 vs 相对臂、`traverse` 的两个端点）见
> [`docs/design/plugin-route-address.md`](../design/plugin-route-address.md)。

## 路由语法

```
{container}/{plugin}/{action}
```

- `{container}` - 容器插件 (如 `worker`)
- `{plugin}` - 目标插件 (如 `agent`, `session`, `model`)
- `{action}` - 具体能力 (如 `chat`, `list`, `status`)

> **资源与配置都不走这条语法**：资源统一经 `vdfs/*`（`<根>/…` 寻址），配置就是
> 资源树里的一个文档（`<根>/<插件>/PLUGIN.yml`）。本文件的 `{plugin}/*` 路由只剩
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
> `<根>/<插件>/PLUGIN.yml`（`ext = form`）。读写用的就是 `vdfs/read` /
> `vdfs/write`——与任何其它资源同一条链路、同一套寻址，因此前端与 LLM 用同一种
> 方式改配置。定义与校验归**配置的拥有者**，默认值从它自己的 `Default` 读出。
> 见 [design/vdfs.md](../design/vdfs.md) §3.4。

### 资源（VDFS）

资源型插件**不提供任何资源类私有路由**：其资源统一经 VDFS 寻址
（`<根>/<插件名>/…`，操作
`vdfs/list|tree|stat|read|write|mkdir|delete|move|edit|search|watch|unwatch|action`）。
每个 `<根>` 子目录由对应插件自己的 `impl VdfsProvider` 提供，落盘（需要持久化的那些）
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

> agent 目录的**访问 / 新建 / 删除 / 导出已全部由 VDFS 承担**（`<根>/agent/…`）：
> `vdfs/list` / `vdfs/read` / `vdfs/write`（新建类型 `zip`，即整包导入）/
> `vdfs/delete` / 节点动作 `export`（`vdfs/action`）。
> 故 `agent` 的旧 `list|get|upload|delete|preview` 路由已于 S12 删除、导出动作
> 于 S13 删除——**agent 插件不再有任何自有协议路由**，`route()` 直接返回
> `NotFound` 并指引到 VDFS。

### 能力贡献（**不是路由**）

> agent 插件不贡献路由，只贡献 LLM 工具：唯一工具 `agent_run`（启动 / 续跑子智能体，
> 源码 `plugins/agent/host/subagent.rs`）由模型经能力调用触达，不经 `route`。
> **LLM 可见工具清单以 [CURRENT.md](../CURRENT.md) §2 为准**，本文件不重复登记。
>
> （历史 `agent_identity` 能力已随 OAB v1 能力体系一同移除——见 `symbio_core/ids.rs` 顶部。）

---

## Session 插件

### 会话管理

| 路径 | 用途 | 返回类型 |
|------|------|----------|
| `session/chat/send` | 发起 AI 对话（流式；实际入口） | `Session` |
| `session/chat/abort` | 中止进行中的对话 | `Empty` |

> **本表只剩「不是数据 CRUD」的两条**（2026-09-23）：会话与消息的增删改查**全部**
> 经 VDFS 地址完成，`chat/send` 与 `chat/abort` 是编排 / 控制，不是数据操作。
> 退役记录见下文，映射表见
> [`legacy-route-migration.md`](../../symbio/src/plugins/session/docs/legacy-route-migration.md)。

> **实时面已迁回，迁移已落地**（ADR-025，2026-09-23；S27 收口）。`session/stream` 路由
> **已删除**（上表不再列它），实时面走 `event_bus` 的 `vdfs` 频道：信封 `{path, data?}`
> （操作枚举整个退役——**`path` 恒为被变更节点自身的地址**，会话是容器、其下是若干并列
> 的集合，集合项形状统一为 `<sid>/<集合段>/<项 id>`：消息的落点是 `<sid>/message/<mid>`，
> `data` 就是那条 `ChatMessage`，**身份在地址末段**；运行态落在会话节点 `<sid>`，
> `data` = 全量节点视图；资源信号无载荷，回读收敛）。实时面与历史面**同一条**
> `vdfs/watch`，不再分家：消息是 `<根>/session/<id>/message` 这个**文件夹**里的**文件**，
> 流式输出是该文件内容的增长。
>
> 本条此前写的是「实时面走 `session/stream`，历史面走 VDFS」，给出的两条理由是
> ① VDFS 变更频道**没有流内序号**（丢帧不可检测）；② 载荷全量而消费端要
> **猜「追加还是替换」**。**两条都错，错在同一个地方——把「数据的属性」当成了
> 「传输的属性」**：
>
> - **顺序不是投递属性**：`ChatMessage.seq` 是消息在 `消息` 这个文件夹里的**位置**，
>   由写入者分配、随节点下发，消费端按它排序（前端 `sortTranscript` 按 `seq`，
>   缺失回退 `timestamp`）。两个并行工具的变更**混着到**、后生成的**先到**，显示
>   都正确——因为每条变更都指向一个明确的 `path`，而节点自带位置。丢帧由**幂等重读**
>   兜底（`event_bus` 满通道 → 保留订阅 + 补送 resync 标记）。
> - **「追加」不是新取值，是 `updated` 上的可选 `delta` 字段**：`delta` 有 ⇒ 尾部追加；
>   无 ⇒ 回读（节点形态在 `vdfs/read` 里）。消费端不需要猜——`delta` 的有无就是答案。
>   这是消息流的既有经验照搬：`ChatMessage` 帧**就是节点视图，没有操作枚举**
>   （`delta` 追加 / `content` 替换 / `status = removed` 移除，语义全在字段上）。
>
> **会话运行态同理**：它是会话节点（`<id>`）的 `status`，走 `updated`（无 `delta` ⇒ 回读），
> 不再需要与消息帧「共用 `seq` 空间」那条推理——那条推理成立的**前提**（顺序是投递属性）
> 本身是错的。
>
> 于是 `session/stream` 的三条存在理由（顺序 / 背压 / 免回读）**全部不成立**，它与
> `symbio_core::transcript_stream` 一并退役；`event_bus` 的 `KIND_VDFS` 成为**唯一**实时通道。
> 消费端相应改为 `subscribeVdfsChanged`（前端）、`vdfs/watch`（CLI）、转播桥（子智能体）。
> 落库转写与 `消息` 目录投影（`vdfs/read` / `vdfs/list` / `vdfs/action`）不受影响。
>
> 判据不变：一个变更取值（或载荷字段）必须有**生产性生产者**。批次 G（`8969ec2`）删除
> `delta` 时它确实零生产者（消息域已迁走）；消息域搬回来，**就有了**。
> 结论不同是因为**输入不同**，不是反复。详见
> [ADR-025](../DECISIONS.md#adr-025-顺序是节点属性delta-是updated的传输形态)。

> **`session/append`、`session/open`、三条消息路由与 `session/heartbeat/trigger` 已退役**（2026-09-18）：
> - `append` —— 消息追加的唯一入口是聊天协议（`chat/send`），而编排自身的落库走引擎直连
>   （`orchestrator/entry.rs` 的 `open_chat_session` + `append_messages`）。
>   该路由的最后形态是「为一次数据追加搭 invoke 信封」，纯开销。
> - `open` —— 它返回的是**进程内句柄**（`ChatSessionHandle`），而句柄交付早已改由编排器
>   直接塞进 `chat_ctx`（`SESSION_HANDLE`），不走路由。
> - `chat/update_message` / `chat/delete_message` / `chat/clear_messages` —— 三条都是
>   **纯存储操作**（不触发编排），已迁到 VDFS（见下表）。它们曾与 `vdfs/write` /
>   `vdfs/action` 各有一份实现，正是要消灭的那种重复。
> - `heartbeat/trigger` —— **不是迁移，是能力整体取消**：它唯一的入口是选项面板上的
>   「立即心跳」按钮，而那个按钮的作用与「在输入框里直接发一条消息」完全重复
>   （心跳的实质就是往会话发一轮提示词）。心跳机制本身（配置、后台调度器、LLM 侧
>   `heartbeat` 工具的 `set`/`get`/`cancel`）未受影响。
> - **`get_messages`**（2026-09-23）—— 它唯一的消费方是 `agent_run` 的**续会话存在性校验**，
>   而「在不在」是资源层的问题、该由 VDFS 回答：现在走**进程内纯接口探测**
>   （`Plugin::get_vfs_provider()` → `stat("<挂载名>/<会话id>")`，判 `attributes.message_count`），
>   既不再为回答「在不在」读回整份历史，也不再占一条协议。
> - **`options/list`**（2026-09-23）—— **不是迁移，是机制整体下线**：会话选项就是
>   **会话配置表单的字段**，与模型 / 智能体 / SKILL 的新建表单同源。定义改随
>   `node.schema` / `new_types[].schema` 下发，值走 `node.attributes.metadata`，
>   写走 `vdfs/write(<根>/session/<id>, {"metadata": …})`。同一件事曾有两条下发通道
>   （`OptionNode` 节点协议 vs `DetailDefinition` 方言），而没有任何守卫会因
>   「两边说的不一样」变红。见 `docs/archive/session-options-unification.md` 与
>   [`session-options.md`](../../symbio/src/plugins/session/docs/session-options.md)。
>
>   > **本表的历史缺项**：`session/options/list` **从未登记在本表**（它不在
>   > 「会话管理」表里），因此这次下线在本文只体现为这条说明。E-006 只校验
>   > 「前缀合法性」——「漏登记」是这份人工清单的固有风险，故此处显式补记。
> - **`update`**（2026-09-23）—— **会话 metadata 的写入入口收敛为 `vdfs/write`**。
>   它唯一比 VDFS 多出来的东西是「**客户端指定会话 id**」（CLI 自己 `gen_id` 后
>   upsert），而 VDFS 对**具名目标 + 不存在**的约定本来就是「**就地创建**，
>   名字即身份」（`VdfsProvider::write` 的 `create` 位表；`vdfs_service::entry::id_of`
>   同义）——那条理由因此消失，会话 provider 一并对齐了这个通用规则（此前它无论
>   有无名字都自己生成 id、把名字只当标题，于是「写到的地址」与「建出来的地址」
>   是两个地方）。CLI 现在写
>   `vdfs/write(<根>/session/<id>, {"create": true, "text": {"metadata": …}})`
>   ——一次调用同时覆盖「新建」与「改元数据」，正是旧路由的 upsert 语义。
>   消费方：`cli/src/client.rs::ensure_session`、`agent/host/subagent.rs::register_subsession`。
>
> 审计与迁移记录见
> [`symbio/src/plugins/session/docs/legacy-route-migration.md`](../../symbio/src/plugins/session/docs/legacy-route-migration.md)。

> **会话级操作已并入 VDFS**（专用路由不再存在或不再被前端使用）：
>
> | 操作 | 入口 |
> |---|---|
> | 列会话清单 | `vdfs/list(<根>/session)` |
> | 读整份转写 | `vdfs/read(<根>/session/<id>)` |
> | 新建会话（id 由后端生成） | `vdfs/write(<根>/session, { create: true })` |
> | 新建 / 打开**具名**会话（id 由调用方给） | `vdfs/write(<根>/session/<id>, { create: true })`——不存在则就地创建，已存在则浅合并 metadata |
> | **删除会话** | `vdfs/delete(<根>/session/<id>)` |
> | **改 metadata / 标题** | `vdfs/write(<根>/session/<id>)` |
> | **改写某条消息** | `vdfs/write(<根>/session/<id>/message/<mid>)` |
> | **删该条及其后** | `vdfs/action(…/message/<mid>, "truncate")` |
> | **清空历史** | `vdfs/action(…/message, "clear")` |
>
> `session/clear` **已退役**（与 `vdfs/delete` 共用 `delete_session_internal`）；
> `session/update` **已退役**（2026-09-23）——CLI 的「客户端指定会话 id」由
> **具名新建**承担：`vdfs/write(<根>/session/<id>, {"create": true, …})`
> 不存在则就地创建、已存在则浅合并 metadata，一次调用即 upsert。
> `Session::merge_metadata_object` 因此只剩一个调用方，语义不可能漂移。
>
> **发言仍只有聊天协议一处**（`chat/send`）：新增消息会触发一整轮编排，不是一次写入。
> 但**改写与删除**是普通的 VDFS 节点操作——判据是「触发不触发编排」，不是「碰不碰消息」。
> 见 [`symbio/src/plugins/session/docs/vdfs-session-messages.md`](../../symbio/src/plugins/session/docs/vdfs-session-messages.md) §5.2。
>
> `session/config/get` / `config/set` **已下线**：会话配置在
> `<根>/session/PLUGIN.yml`（`ext = form`），读写走 `vdfs/read` / `vdfs/write`。

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

> **`model` 已无自有路由**（Phase E-② 后 `ModelProvider::execute_turn` 由 `session`
> 在会话循环内直连调用，不占路由；配置读写全在 VDFS 上）。本插件不再有路由条目。

历史入口（**均已下线**，列此仅作迁移指引）：`model/chat` 收归 session 直连（改用
`session/chat/send`）；`model/config/*` 迁到 `<根>/model/<id>` 与 `<根>/model/PLUGIN.yml`；
`model/status` 改由节点动作 `vdfs/action { action: "test" }` 返回；`model/chat_sync`
本就是 NotImplemented 占位。

### 配置结构

```yaml
# ~/.symbio/model/<id>/provider.json
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

> **路由按工具名动态分发**：`local/<工具短名>`，与 LLM 工具是同一份集合
> （`route()` 里 `tool_impls.iter().find(|t| t.name() == path)`）。

| 路径 | 用途 | 安全风险 |
|------|------|----------|
| `local/cmd`（Windows）/ `local/sh`（macOS / Linux） | 执行 Shell 命令（工具名按操作系统取） | ⚠️ 高危 |
| `local/content_search` | 内容搜索 (ripgrep) | 安全 |
| `local/todo_write` | 会话任务清单（`LastOnly` 保留策略） | 安全 |
| `local/codebase_search` | 语义代码搜索 | 安全 |
| `local/ask_user` | 向用户提结构化问题（单问题或 1~4 批量，自动补 `Other`） | 安全 |

> 文件编辑类原生工具（`file_read` / `file_write` / `file_edit` / `glob_search` 等）
> **已迁入 VDFS 物理层**，由 `vdfs` 插件以 `vdfs_read` / `vdfs_write` / `vdfs_edit` /
> `vdfs_search` 等统一暴露（见 §资源（VDFS）），本插件不再提供。
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

> `skill/config/get` / `config/set` **已下线**，且本插件**不设配置文档**——
> 技能清单与内容不设私有路由：已安装技能经 `<根>/skill` 寻址
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
> 连通性自检一律经 `<根>/mcp`（一个 server = 一个目录，主文件 `server.json`），
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

> `telegram/config/get` / `config/set` **已下线**：本插件挂载 `<根>/telegram`，
> 内容就是一份配置文档 `<根>/telegram/PLUGIN.yml`（`ext = form`），读写走
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
> `<根>/gateway/PLUGIN.yml`（扁平键 `inbound_*`）。前端「设置」页点开的正是这份
> 表单；写入走 `vdfs/write` → `ConfigFile::apply` 落盘后，本插件在自己的 `write`
> 里做副作用（**内部 stop + start 重建监听**）——机制不引入回调抽象。
>
> `gateway/*` 自身接口**恒走 native**（前端不经 HTTP 访问本插件）。前端出站分发（native / http）由「系统目录」切换器管理，不再由 gateway 配置驱动。

安全：非回环地址需 `inbound_token` 鉴权（回环地址免鉴权）；`inbound_readonly` 开启后仅放行只读白名单
（`vdfs/list|tree|stat|read|search`、`home/get_homedir`、
`work/get_workspace`；`gateway/*` 显式排除，且 `vdfs/read` 只要落在任何插件的
`PLUGIN.yml` 上就拒绝——配置可能含凭据），详见 [CONFIGURATION.md](CONFIGURATION.md)。

---

## Setting 插件

> **本插件已无自有路由**：`setting/config/get` / `config/set` 与更早的
> `setting/list` / `setting/get` 全部下线。
>
> 分区清单与取值分别由 `<根>/setting` 的 `vdfs/list` / `vdfs/read` 承担。
> 清单 = **各插件交出来的配置条目 + 自有分区**（`appearance` / `about`）：
> 前者由各插件在 `traverse` 里经 `announce_configurable` 声明，容器用共享收集器
> 收下并写回请求 ctx（见 [design/vdfs.md](../design/vdfs.md) §13.1）——因此
> **新增一个可配置插件不需要在本插件登记任何东西**；自有分区的数据在前端 store，
> `read` / `write` 对它们恒 `Forbidden`。

---

## Hook 插件

> **命名空间是目录名 `hook`，不是 `hooks`。** 容器（`composite`）按**目录名**建实例表
> 并在 `route` 里按它分发（`composite.rs`：「目录名 = 实例名」），所以目录名才是真正的
> 路由前缀。`PluginMeta::new` 曾写死 `"hooks"`，且 `Plugin::meta()` 全仓无生产消费方
> ——那个名字**从不参与路由**，却让本表与 `hook/README.md` 长期写着不存在的 `hooks/*`。
> 2026-09-18 已把首参改为 `PLUGIN_HOOK`（`hook`），三者对齐。
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

> `event_bus/publish` **不存在**：本插件是进程内帧广播（如 VDFS 变更、
> 总线自身握手），发布方在进程内直接调用，不经路由。
> 历史上的 `pending/snapshot` 路由已随会话事件频道一并废除（回放缓冲的唯一数据源
> 是按 `session_id` 灌入的事件帧，VDFS 变更发布方传 `session_id = None`，缓冲永远为空）。

---

> **维护原则**：新增路由必须在此文档登记，包括路径、用途、输入输出格式。

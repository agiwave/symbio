---
kind: ref
id: RE-003-ROUTES
title: Symbio 路由参考
status: active
created: 2026-09-07
---

# Symbio 路由参考

> **文档类型：参考** — **所有可用路径的完整清单**。本文**人工维护**，因此也是唯一会漂移
> 的一份：`docs/CURRENT.md` §1 的「自有路由」列由代码生成，必然与代码一致——**两者不一致时以它为准**。
>
> - **守卫**：[`scripts/plugin-entry-audit.mjs`](../../scripts/plugin-entry-audit.mjs) 的 **E-006**
>   （清单里的路径前缀必须是插件目录名）。
> - **地址规则**（前缀是**插件目录名**、绝对地址 vs 相对臂、`traverse` 的两个端点）见
>   [design/AR-009-plugin-route-address.md](../arch/AR-009-plugin-route-address.md "AR-009-plugin-route-address")。
> - **退役路由**：不在此处保留叙述——去向见本文 §退役路由索引。
>
> ⚠️ **本文只写「现在有哪些路由」**。某条路由**为什么**被删 / 被迁，是决策与变更史，
> 归 [ADR 索引](../adr/INDEX.md) 与 `git log`（见 [docs/README.md](../README.md#文档职责边界一个事实只有一个-owner "docs/README")）。
>
> - **每条路由的一句话职责不在本文**：它写在**注册处**——插件 `route()` 里那条 `match`
>   臂正上方的注释，由 [CURRENT.md §6.3](../CURRENT.md#63-前端--宿主路由) 提取成表。
>   本文保留的是**路由契约**：调用方、返回类型、备注、退役索引——那些只有这里有。
>   （旧版本文有一列手写「用途」，与臂上注释是同一句事实的两份副本；按 PL-011 §6
>   「原表与迁移不拆批」收敛为指路，owner 归代码。）

## 路由语法

```
{container}/{plugin}/{action}
```

- `{container}` - 容器插件 (如 `worker`)
- `{plugin}` - 目标插件 (如 `agent`, `session`, `model`)
- `{action}` - 具体能力 (如 `chat`, `list`, `status`)

> `worker` 前缀可省略也可显式写出：`session/stats` 与 `worker/session/stats` 等价
> （前者由 home 兜底转发给 worker）。

> **资源与配置都不走这条语法**：资源统一经 `vdfs/*`（`<根>/…` 寻址），配置就是
> 资源树里的一个文档（`<根>/<插件>/PLUGIN.yml`）。本文件的 `{plugin}/*` 路由只剩
> 那些**不是资源**的能力（会话编排、模型推理、网关状态…）。

## 通用路由

### 内省

| 路径 | 用途（→ §6.3） | 返回 |
|------|------|------|
| `_root` | [§6.3](../CURRENT.md#63-前端--宿主路由) | 子树结构 |

### 配置管理（**没有配置路由**）

配置就是一个**普通的可寻址文档**：`<插件目录>/PLUGIN.yml`，对外地址
`<根>/<插件>/PLUGIN.yml`（`ext = form`）。读写用的就是 `vdfs/read` / `vdfs/write`——
与任何其它资源同一条链路、同一套寻址，因此前端与 LLM 用同一种方式改配置。
定义与校验归**配置的拥有者**，默认值从它自己的 `Default` 读出（见 [design/AR-013-vdfs.md](../arch/AR-012-vdfs-frontend.md "AR-012-vdfs-frontend") §3.4）。

### 资源（VDFS）

资源型插件**不提供任何资源类私有路由**：其资源统一经 VDFS 寻址（`<根>/<插件名>/…`）。
**操作闭集见 [CURRENT.md](../CURRENT.md) §3.2**（源码 `plugins/vdfs/protocol.rs::VDFS_OPS`，
有测试锁计数）——本文不抄那份清单。
每个 `<根>` 子目录由对应插件自己的 `impl VdfsProvider` 提供，落盘经
`symbio/src/providers/vdfs_service/`（见 [design/AR-013-vdfs.md](../arch/AR-012-vdfs-frontend.md "AR-012-vdfs-frontend") §11 / §13.4）。

---

## Home 插件（根容器）

根插件，自身终结以下路径；其余路径按首段剥离后转发给 `worker`（Composite），
未命中的一律兜底转发给 `worker`（源码：`plugins/home/plugin.rs` 的 `route`）。

| 路径 | 用途（→ §6.3） | 备注 |
|------|------|------|
| `home/get_homedir` | [§6.3](../CURRENT.md#63-前端--宿主路由) | 只读；gateway 只读白名单放行 |
| `home/reload` | [§6.3](../CURRENT.md#63-前端--宿主路由) | 前端切换系统目录后调用 |
| `work/set_workspace` | [§6.3](../CURRENT.md#63-前端--宿主路由) | 写操作 |
| `work/get_workspace` | [§6.3](../CURRENT.md#63-前端--宿主路由) | 只读；gateway 只读白名单放行 |

> **两条 `work/*` 臂在此终结**（`home` 是系统根、不挂前缀）：这是**工作区切换**的
> 控制面（读写 `home` 自己的 `work.workdir` 配置），与 `memory` 插件的记忆挂载点
> （`<根>/memory`）互不相干；`memory` 插件的 `route` 恒 `NotFound`，故不冲突。

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

| 路径 | 用途（→ §6.3） | 返回类型 |
|------|------|------|
| `session/stats` | [§6.3](../CURRENT.md#63-前端--宿主路由) | `SessionStats` |

> **会话编排不再有私有路由**：发言、恢复、中止全部落在会话自己的地址上（见下表）。
> `stats` 是唯一剩下的会话路由——它是**读数**，不是数据操作。
> 实时面都走 `event_bus` 的 `vdfs` 频道（见 [AR-004-PROTOCOLS.md](../arch/AR-004-PROTOCOLS.md "AR-004-PROTOCOLS") §事件总线频道）。
>
> `stats` 的读数只能调已有的投影、不得另写一份统计口径（[plan/PL-010 §4](../plan/PL-010-价值验收与基线埋点.md "PL-010-价值验收与基线埋点")
> 的复算判据）；实现见 [`symbio/src/plugins/session/stats.rs`](../../symbio/src/plugins/session/stats.rs)。
>
> 读侧闸（[plan/PL-001 §7](../plan/PL-001-核心架构.md "PL-001-核心架构")）：**不声明 `principal` = 本机默认读数**；
> 声明了才按 `can_see` 判可见域（`thread_private` 缺省，C10）——属主全量、**非属主读数为空**
> （含矩阵内主体；`has_wal` 仍为真，于是「有源但不给你看」与「没有源」可分辨）。
> 载荷只回答「我是谁」，不接受声明「能看到什么」——那等于给自己授权。
> 可见域**只判一次**：四列、不变量、声誉、就绪集与转写列共用同一个答案，读不出时声誉是**整列**
> `{}`（不是零值条目）——全有全无，免得「没人立过约」与「不给你看」同形；
> 就绪集与转写列跟四列同形（空切片 ⇒ `{ready: []}` / `{entries: []}`）——「没有就绪任务 /
> 没有对话」与「不给你看」由 `has_wal` 与列的上下游分辨。

### 会话级操作已并入 VDFS（专用路由不再存在）

> 地址清单与各自职责见 [CURRENT.md §6.3](../CURRENT.md#63-前端--宿主路由)：
>
> 列会话清单 · 读整份转写 · 新建会话（id 由后端生成） · 新建 / 打开**具名**会话（id 由调用方给） · 删除会话 · 改 metadata / 标题 · 改写某条消息 · 删该条及其后 · 列待消费的用户消息（收件箱） · **发一条用户消息 / 取消一条排队中的消息** · 清空待消费队列 · **中止进行中的一轮** · **恢复（审批 / 回填答案 / 重试）**

> **一条用户消息只有一个入口**：往收件箱写一条（不再有「发消息」专用路由），
> 因此子智能体空间这类**没有调用方**的会话也能被驱动——写它的收件箱即可。
> 新增消息触发一整轮编排（在空间空闲时由空间自己消费）；而**改写与删除**是普通的 VDFS
> 节点操作——判据是「触发不触发编排」，不是「碰不碰消息」。两者都在
> [`symbio/src/plugins/session/docs/AR-022-vdfs-session-messages.md`](../../symbio/src/plugins/session/docs/AR-022-vdfs-session-messages.md "AR-022-vdfs-session-messages")。
>
> **运转选项（provider / mode / risk）不在写入面上**：它们的唯一 owner 是会话 metadata
> （`vdfs/write(<根>/session/<id>, { create: true })` upsert），消费链的回退顺序是
> 「调用级 → 会话 metadata → 默认值」（[ADR-066](../adr/ADR-066.md "ADR-066")）。
> 恢复的 `mode` / `risk_level` 是这一格的仅有的两个**调用级**字段——恢复本就是
> 「用当时的选项把当时那一轮做完」，与消息写入不是同一层级。

### 聊天流程

```
1. vdfs/write(<根>/session/<id>/inbox, 纯文本)   → 入队（受理即返回）
2. 空间空闲时自行消费该条并发起一整轮编排（spawn）
3. 下行：event_bus/subscribe + vdfs/watch(<根>/session) 收 vdfs 频道
   帧类型:
   - 会话节点 <…>/session/<id>：全量节点视图（status 离开 working = 本轮结束）
   - 消息节点 <…>/message/<mid>：delta 追加 / content 替换 / status = removed
```

---

## Classify 插件（对话面：判决）

「这一轮该直接回答、还是派给工具循环」——只输出**枚举**，不产出面向用户的文本。

| 路径 | 用途（→ §6.3） | 返回类型 |
|------|------|------|
| `classify/decide` | [§6.3](../CURRENT.md#63-前端--宿主路由) | `Verdict` |

> 调用方只有 `session`（编排层）：它投影事实、发起调用，并按判决**执行**对应路径。
> 契约 `DecideRequest` / `Verdict` 见 [`symbio_core::schemas::dialog`](../../symbio/src/symbio_core/schemas/dialog.rs)。
> 不挂载本插件 ⇒ 路由 `NotFound` ⇒ 全部输入直接进工具循环（卸载平凡值）。
> 两条产线（规则短路 0 次 LLM / 快速档 1 次静默 LLM）见
> [classify 插件的 README](../../symbio/src/plugins/classify/README.md "symbio/src/plugins/classify/README")。

---

## Compose 插件（对话面：措辞）

「首响 / 答话 / 汇报」的措辞——只输出**文本**，不做判决。

| 路径 | 用途（→ §6.3） | 返回类型 |
|------|------|------|
| `compose/wording` | [§6.3](../CURRENT.md#63-前端--宿主路由) | `String` |

> 调用方只有 `session`。本插件**不写转写**：文本由 `session` 落库，转写只有一个写入者。
> 契约 `ComposeRequest` 见 [`symbio_core::schemas::dialog`](../../symbio/src/symbio_core/schemas/dialog.rs)；
> 其中 `snapshot`（`RunSnapshot`）是 `session` 投影的运行现状，**只有 `Report` 读它**。
> 出参**空串 = 没有对话面文本**（平凡值），调用方据此不写节点。
>
> 不挂载本插件 ⇒ 路由 `NotFound` ⇒ `session` 按「缺插件」处理：`Answered` 拿不到措辞时
> **降级进工具循环**（不沉默），`Escalate` 没有首响，`Report` 只是"这次没说"且不消耗
> 汇报配额（卸载平凡值）。
> 三条产线（模板 0 次 LLM / `from_context` 生成 1 次静默 LLM / `Report` 填表 0 次 LLM）见
> [compose 插件的 README](../../symbio/src/plugins/compose/README.md "symbio/src/plugins/compose/README")。
> 调用方开关 `SessionConfig::compose_enabled`（`<session 目录>/PLUGIN.yml`）；
> `Report` 的**触发**另归 `SessionConfig::progress_enabled` 等四个旋钮（见
> [`chat_loop/progress.rs`](../../symbio/src/plugins/session/chat_loop/progress.rs)）——
> 措辞与触发是两件事，各有各的开关。

---

## Model 插件

**本插件无自有路由**：`ModelProvider::execute_turn` 由 `session` 在会话循环内**直连调用**，
不占路由；配置读写全在 VDFS 上（`<根>/model/<id>` 与 `<根>/model/PLUGIN.yml`）。

### 支持的协议

| protocol_id | 提供商 | API 端点 |
|------|------|------|
| `openai_chat` | OpenAI | `/v1/chat/completions` |
| `openai_responses` | OpenAI | `/v1/responses` |
| `anthropic_messages` | Anthropic | `/v1/messages` |
| `gemini_api` | Google | `generateContent` |

> 协议适配内化于 `symbio/src/plugins/model/protocols/`，机制见 `symbio/src/plugins/model/README.md`；
> 为什么支持多协议见 [ADR-004](../adr/ADR-004.md)。

---

## Local 插件

> **路由按工具名动态分发**：`local/<工具短名>`，与 LLM 工具是同一份集合
> （`route()` 里 `tool_impls.iter().find(|t| t.name() == path)`）。

| 路径 | 用途（→ §6.3） | 安全风险 |
|------|------|------|
| `local/cmd`（Windows）/ `local/sh`（macOS / Linux） | [§6.3](../CURRENT.md#63-前端--宿主路由) | ⚠️ 高危 |
| `local/content_search` | [§6.3](../CURRENT.md#63-前端--宿主路由) | 安全 |
| `local/todo_write` | [§6.3](../CURRENT.md#63-前端--宿主路由) | 安全 |
| `local/codebase_search` | [§6.3](../CURRENT.md#63-前端--宿主路由) | 安全 |
| `local/ask_user` | [§6.3](../CURRENT.md#63-前端--宿主路由) | 安全 |

> **文件编辑类原生工具已迁入 VDFS 物理层**，由 `vdfs` 插件以
> `vdfs_read` / `vdfs_write` / `vdfs_edit` / `vdfs_search` 等统一暴露，本插件不再提供。
> `ask_user` 产 `user_prompt` 节点等用户回答（与工具审批同一套回填机制）；自动模式下
> 不产节点、返回 `tool_unavailable` 让模型自行继续。

### Shell 命令策略

高危操作触发 `ToolApprovalRequest`，需用户显式授权。

---

## Web 插件

> 地址清单与各自职责见 [CURRENT.md §6.3](../CURRENT.md#63-前端--宿主路由)：
>
> `web/http_request` · `web/web_search` · `web/web_fetch`

---

## Skill 插件

> 地址清单与各自职责见 [CURRENT.md §6.3](../CURRENT.md#63-前端--宿主路由)：
>
> `skill/execute`

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

> 地址清单与各自职责见 [CURRENT.md §6.3](../CURRENT.md#63-前端--宿主路由)：
>
> `telegram/send` · `telegram/get_updates` · `telegram/set_chat_id` · `telegram/start_listener` · `telegram/stop_listener` · `telegram/status`

> 本插件挂载 `<根>/telegram`，内容就是一份配置文档 `<根>/telegram/PLUGIN.yml`（`ext = form`），
> 读写走 `vdfs/read` / `vdfs/write`。

---

## Gateway 插件

HTTP/WebSocket 入站网关（`plugins/gateway/server.rs`），外部客户端与 route_v2 同构接入插件树：

> 地址清单与各自职责见 [CURRENT.md §6.3](../CURRENT.md#63-前端--宿主路由)：
>
> `POST /api/v1/invoke` · `WS /api/v1/ws` · `GET /api/v1/health`

### 作为插件的路由

挂载于 `worker`（Composite）之下，外部入站请求最终由 gateway 转发给父级 composite：

> 地址清单与各自职责见 [CURRENT.md §6.3](../CURRENT.md#63-前端--宿主路由)：
>
> `gateway/status`

> 配置就是 `<根>/gateway/PLUGIN.yml`（扁平键 `inbound_*`）；写入走 `vdfs/write` →
> `PluginConfigFile::apply` 落盘后，本插件在自己的 `write` 里做副作用（**内部 stop + start 重建监听**）。
> `gateway/*` 自身接口**恒走 native**（前端不经 HTTP 访问本插件）。

安全：非回环地址需 `inbound_token` 鉴权（回环地址免鉴权）；`inbound_readonly` 开启后仅放行只读白名单
（`vdfs/list|tree|stat|read|search`、`home/get_homedir`、`work/get_workspace`；`gateway/*` 显式排除，
且 `vdfs/read` 只要落在任何插件的 `PLUGIN.yml` 上就拒绝——配置可能含凭据），
详见 [RE-001-CONFIGURATION.md](RE-001-CONFIGURATION.md "RE-001-CONFIGURATION")。

---

## Plugin_manager 插件（插件管理）

**本插件无自有路由**。清单与取值分别由 `<根>/plugin_manager` 的 `vdfs/list` / `vdfs/read` 承担。

它是本智能体**插件集合**的门面：清单 = **各插件交出来的配置条目 + 自有分区**
（`appearance` / `about`）。前者由各插件在 `traverse` 里经 `capability_announce_configurable` 声明，
容器用共享收集器收下并写回请求 ctx（见 [design/AR-013-vdfs.md](../arch/AR-012-vdfs-frontend.md "AR-012-vdfs-frontend") §13.1）——
因此**新增一个可配置插件不需要在本插件登记任何东西**；自有分区的数据在前端 store，
`read` / `write` 对它们恒 `Forbidden`。

配置条目**不是第二种东西**：它就是那个插件的配置，入口挂在
`<根>/plugin_manager/<插件名>`，读 / 写转发到容器根下的 `<插件名>/PLUGIN.yml`——
同一份配置仍然只有一个**文件**、一份**定义**（拥有者给的那份）。

装配动作（启用 / 停用 / 卸载）与「添加插件」也走这里，但**动词在容器**：本插件只把
`Action(enable|disable)`（插件名进载荷）、`Delete(<插件名>)`、根上的 `Write`（安装）
转发回所在容器（见 `composite/vdfs.rs` 的「插件注册表」一节）。必需插件（构造者经
`REQUIRED_PLUGINS` 声明）**不可删除，但可停用**；界面底座插件（`ASSEMBLY_UNDISABLABLE_PLUGINS`）
连停用也拒绝——停掉它就没有界面再把它打开。

---

## Hook 插件

> **命名空间是目录名 `hook`，不是 `hooks`。** 容器（`composite`）按**目录名**建实例表并在
> `route` 里按它分发（「目录名 = 实例名」），所以目录名才是真正的路由前缀。
> 真实的调用点见 `symbio_core::plugin::route::ROUTE_HOOK_FIRE`。

> 地址清单与各自职责见 [CURRENT.md §6.3](../CURRENT.md#63-前端--宿主路由)：
>
> `hook/register` · `hook/fire` · `hook/list`

### 系统事件

> 地址清单与各自职责见 [CURRENT.md §6.3](../CURRENT.md#63-前端--宿主路由)：
>
> `pre_tool_use` · `post_tool_use` · `pre_file_write` · `post_file_write` · `pre_shell_execute` · `post_shell_execute` · `user_prompt_submit` · `session_start` · `session_end`

---

## Event Bus 插件

> 地址清单与各自职责见 [CURRENT.md §6.3](../CURRENT.md#63-前端--宿主路由)：
>
> `event_bus/subscribe` · `event_bus/ping`

> `event_bus/publish` **不存在**：本插件是进程内帧广播（如 VDFS 变更、总线自身握手），
> 发布方在进程内直接调用，不经路由。

---

## 退役路由索引

**只登记「路径 → 现在去哪」，不写过程叙述**（决策与理由见 [ADR 索引](../adr/INDEX.md)、
变更史见 `git log`，完整迁移审计见
[`docs/archive/legacy-route-migration.md`](../archive/legacy-route-migration.md)）：

> 地址清单与各自职责见 [CURRENT.md §6.3](../CURRENT.md#63-前端--宿主路由)：
>
> `{plugin}/config/get` · `{plugin}/config/set` · `{plugin}/config/schema` · `{plugin}/entities/*` · `model/chat` · `model/status` · `model/chat_sync` · `agent/list` · `get` · `upload` · `delete` · `preview` · `export` · `session/append` · `session/open` · `session/update` · `session/clear` · `session/chat/send` · `session/chat/abort` · `session/chat/update_message` · `session/chat/delete_message` · `session/chat/clear_messages` · `session/get_messages` · `session/options/list` · `session/stream` · `session/heartbeat/trigger` · `hook/trigger` · `hooks/*` · `event_bus/publish`

---

> **维护原则**：新增路由必须在此文档登记（路径、用途、输入输出格式）；删除路由时**只把路径
> 挪进 §退役路由索引**，不在此处写过程叙述。

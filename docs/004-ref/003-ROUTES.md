---
kind: ref
id: RE-003-ROUTES
title: Symbio 路由参考
status: active
created: 2026-09-07
lists: [route-contracts=owner]
---

# Symbio 路由参考

> **文档类型：参考（手写）** — 本文是路由的**机制叙述 + 契约表**。
>
> - **地址与一句话职责的唯一表是 [RE-006 §2.3](006-当前事实表.md#23-前端--宿主路由 "RE-006-当前事实表")**（从插件 `route` 的 `match` 臂提取，改代码即变）——本文不列第二张地址表。
> - **契约表**（下文）：只放只有人知道的事——调用方 / 返回类型 / 备注，以地址为**键**。键必须是 §2.3 里的静态路由（多一个键判红，见 `scripts/facts.test.mjs`）；可以缺（新路由还没人写契约行）。
> - 改契约只改本文这张表；改地址 = 改代码（`route` 加 `match` 臂 + 臂上一句职责注释）。
> - 「仓内无调用方」的口径 = `plugin-entry-audit` 路由表的 `refs=0`（只统计仓内；网关会把外部 `path` 原样转发，故对外 API 天然 `refs=0`——`telegram` 六条即此形态）。

## 路由契约表

<!-- list:route-contracts owner -->

| 地址 | 调用方 | 返回类型 | 备注 |
|---|---|---|---|
| `classify/decide` | `session`（编排层） | `Verdict` | 契约 `DecideRequest` / `Verdict` 见 `symbio_core::dialog`；不挂载本插件 ⇒ 路由 `NotFound` ⇒ 全部输入直接进工具循环 |
| `compose/wording` | `session`（编排层） | `String` | 空串 = 没有对话面文本（平凡值），调用方据此不写节点 |
| `event_bus/ping` | 仓内无调用方 | pong + 当前订阅数 | 存活探测 |
| `event_bus/subscribe` | 前端实时面 / CLI | 帧订阅连接 | 收 `kind = "vdfs"` 频道；CLI 侧须与 `vdfs/watch` 配对（曾漏后者） |
| `gateway/status` | 前端（进程内 route，不经 HTTP） | 运行状态 JSON | `gateway/*` 显式排除于只读白名单 |
| `home/get_homedir` | 前端（`services/home.ts`） | `home_get_homedir::Response` | 只读；**经 gateway 不可达**（home 是 worker 的父，入站 router 不向上转发） |
| `home/get_workspace` | 前端（`services/home.ts`） | `home_get_workspace::Response` | 只读；读 home 自己的 `work.workdir` 配置段；同样经 gateway 不可达 |
| `home/reload` | 前端（`services/home.ts`） | `home_reload::Response` | 写操作；切换 homedir（可选）并重建全部子插件 |
| `home/set_workspace` | 前端（`services/home.ts`） | `home_set_workspace::Response` | 写操作；写入 home 自己的 `work.workdir` 与最近使用列表 |
| `hook/fire` | `session`（工具执行器）等插件 | `SimpleResponse` | 生命周期点调用；core 常量 `ROUTE_HOOK_FIRE` |
| `hook/list` | 仓内无调用方 | hook 清单 | — |
| `hook/register` | 仓内无调用方 | `SimpleResponse` | — |
| `session/stats` | 前端（`services/sessionStats.ts`）/ CLI（`client.rs`） | `SessionStats` | 只读读数；不得另写统计口径（DE-016 §4 复算判据） |
| `skill/execute` | 仓内无调用方 | `SkillResponse` | 载荷 `{name, args}`，返回技能定义正文 |
| `telegram/get_updates` | 外部客户端（gateway 原样转发） | 更新数组 | 长轮询 |
| `telegram/send` | 外部客户端（gateway 原样转发） | `telegram_send::Response` | — |
| `telegram/set_chat_id` | 外部客户端（gateway 原样转发） | 绑定结果 JSON | 绑定目标 chat id |
| `telegram/start_listener` | 外部客户端（gateway 原样转发） | `{success, message}` | 长轮询入队，走会话地址面 |
| `telegram/status` | 外部客户端（gateway 原样转发） | `telegram_status::Response` | 通道运行状态 |
| `telegram/stop_listener` | 外部客户端（gateway 原样转发） | `{success, message}` | — |

> 运行期动态分发的地址（`local/<工具短名>` / `web/<工具短名>` / `vdfs/<操作>` / 容器挂载名）没有可静态枚举的 `match` 臂，不在本表——各插件节有说明；`vdfs/*` 的逐条定义见 [RE-006 §2.2](006-当前事实表.md#22-vdfs-操作 "RE-006-当前事实表")。

## 路由语法

```
{container}/{plugin}/{action}
```

- `{container}` - 容器插件 (如 `worker`)
- `{plugin}` - 目标插件 (如 `agent`, `session`, `model`)
- `{action}` - 具体能力 (如 `chat`, `list`, `status`)

> `worker` 前缀可省略也可显式写出：`session/stats` 与 `worker/session/stats` 等价
> （前者由 home 兜底转发给 worker）。
>
> **资源与配置都不走这条语法**：资源统一经 `vdfs/*`（`<根>/…` 寻址），配置就是资源树里的一个文档（`<根>/<插件>/PLUGIN.yml`）。本文件的 `{plugin}/*` 路由只剩那些**不是资源**的能力（会话编排、模型推理、网关状态…）。

### 配置管理（**没有配置路由**）

配置就是一个**普通的可寻址文档**：`<插件目录>/PLUGIN.yml`，对外地址`<根>/<插件>/PLUGIN.yml`（`ext = form`）。读写用的就是 `vdfs/read` / `vdfs/write`——与任何其它资源同一条链路、同一套寻址，因此前端与 LLM 用同一种方式改配置。定义与校验归**配置的拥有者**，默认值从它自己的 `Default` 读出（见 [DE-010](../003-design/010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范") §3 地址模型）。

### 资源（VDFS）

资源型插件**不提供任何资源类私有路由**：其资源统一经 VDFS 寻址（`<根>/<插件名>/…`）。**操作闭集见 [RE-006 §2.2](006-当前事实表.md#22-vdfs-操作 "RE-006-当前事实表")**（源码 `plugins/vdfs/protocol.rs::VDFS_OPS`，有测试锁计数）——本文不抄那份清单。每个 `<根>` 子目录由对应插件自己的 `impl VdfsProvider` 提供，落盘经 `symbio/src/providers/vdfs_service/`（见 [DE-010](../003-design/010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范") §2 分层与落点 / §8 一致性要求）。

---

## Home 插件（根容器）

根插件，自身终结以下路径；其余路径按首段剥离后转发给 `worker`（Composite），未命中的一律兜底转发给 `worker`（源码：`plugins/home/plugin.rs` 的 `route`）。四条静态路由（`home/get_homedir` / `home/get_workspace` / `home/reload` / `home/set_workspace`）的契约见上表。

> **这四条臂全部以 `home/` 开头**（`home` 是系统根、前缀即目录名）：工作区切换与 homedir 管理是同一份 home 自有状态（`PLUGIN.yml` 的 `work` / `homedir` 配置段），故同一条命名空间。它与 `memory` 插件的记忆挂载点（`<根>/memory`）互不相干；`memory` 插件的 `route` 恒 `NotFound`，故不冲突。

---

## Agent 插件

**本插件无自有协议路由**：agent 目录的访问 / 新建 / 删除 / 导出全部由 VDFS 承担（`<根>/agent/…`）——`vdfs/list` / `vdfs/read` / `vdfs/write`（新建类型 `zip`，即整包导入）/`vdfs/delete` / 节点动作 `export`（`vdfs/action`）。故 `route` 直接返回 `NotFound` 并指引到 VDFS。

**能力贡献（不是路由）**：唯一 LLM 工具 `agent_run`（启动 / 续跑子智能体，源码`plugins/agent/host/subagent.rs`），由模型经能力调用触达，不经 `route`。**LLM 可见工具清单以 [RE-006 §2.1](006-当前事实表.md#21-llm-可见工具 "RE-006-当前事实表") 为准**，本文件不重复登记。

---

## Session 插件

唯一静态路由 `session/stats`（契约见上表）。**会话编排不再有私有路由**：发言、恢复、中止全部落在会话自己的地址上。

> **会话级操作已并入 VDFS（专用路由不再存在）**：列会话清单 · 读整份转写 · 新建会话（id 由后端生成）· 新建 / 打开**具名**会话（id 由调用方给）· 删除会话 · 改 metadata / 标题 ·改写某条消息 · 删该条及其后 · 列待消费的用户消息（收件箱） · **发一条用户消息 / 取消一条排队中的消息** ·清空待消费队列 · **中止进行中的一轮** · **恢复（审批 / 回填答案 / 重试）**——地址清单与各自职责见 [RE-006 §2.3](006-当前事实表.md#23-前端--宿主路由 "RE-006-当前事实表") 与 [AR-022](../../symbio/src/plugins/session/docs/109-vdfs-session-messages.md "DE-109-vdfs-session-messages")。
>
> **一条用户消息只有一个入口**：往收件箱写一条（不再有「发消息」专用路由），因此子智能体空间这类**没有调用方**的会话也能被驱动——写它的收件箱即可。新增消息触发一整轮编排（在空间空闲时由空间自己消费）；而**改写与删除**是普通的 VDFS节点操作——判据是「触发不触发编排」，不是「碰不碰消息」。
>
> **运转选项（provider / mode / risk）不在写入面上**：它们的唯一 owner 是会话 metadata（`vdfs/write(<根>/session/<id>, { create: true })` upsert），消费链的回退顺序是「调用级 → 会话 metadata → 默认值」（[D-066](../003-design/012-会话输入与地址.md "DE-012-会话输入与地址")）。恢复的 `mode` / `risk_level` 是这一格的仅有的两个**调用级**字段——恢复本就是「用当时的选项把当时那一轮做完」，与消息写入不是同一层级。
>
> `stats` 的读数只能调已有的投影、不得另写一份统计口径
> （[DE-016](../003-design/016-工程落地与验证.md "DE-016-工程落地与验证") §4 的复算判据）；实现见 [`symbio/src/plugins/session/stats.rs`](../../symbio/src/plugins/session/stats.rs)。
>
> 读侧闸（[DE-001](../003-design/001-核心架构.md "DE-001-核心架构") §7）：**不声明 `principal` = 本机默认读数**；声明了才按 `can_see` 判可见域（`thread_private` 缺省，C10）——属主全量、**非属主读数为空**（含矩阵内主体；`has_wal` 仍为真，于是「有源但不给你看」与「没有源」可分辨）。载荷只回答「我是谁」，不接受声明「能看到什么」——那等于给自己授权。可见域**只判一次**：四列、不变量、声誉、就绪集与转写列共用同一个答案，读不出时声誉是**整列**`{}`（不是零值条目）——全有全无，免得「没人立过约」与「不给你看」同形；就绪集与转写列跟四列同形（空切片 ⇒ `{ready: []}` / `{entries: []}`）——「没有就绪任务 / 没有对话」与「不给你看」由 `has_wal` 与列的上下游分辨。

### 聊天流程

```
1. vdfs/write(<根>/session/<id>/inbox, 纯文本) → 入队（受理即返回）
2. 空间空闲时自行消费该条，派发 loop/<id> 的 Action("run_turn")（见下节）
3. 下行：event_bus/subscribe + vdfs/watch(<根>/session) 收 vdfs 频道
 帧类型:
 - 会话节点 <…>/session/<id>：全量节点视图（status 离开 working = 本轮结束）
 - 消息节点 <…>/message/<mid>：delta 追加 / content 替换 / status = removed
```

---

## Loop 插件（单环驱动器）

**无 `route` 臂**（`route` 恒 `NotFound`）。唯一入口是 VDFS 动作面：

| 地址 | 动作 | 调用方 | 返回 |
|---|---|---|---|
| `<根>/loop/<sid>`（容器内相对地址 `loop/<sid>`） | `run_turn` | `session` 的轮生命周期（`orchestrator/consume.rs`），**仅在进程内** | `VdfsActionResult`，`data` 为 `TurnReport` |

授权三条（缺一即拒绝）：宿主 ctx 上有 `TURN_DISPATCH`（`WIRE=false` 的进程内键，线上不可伪造）；
`dispatch.spec.session_id == <sid>`；动作名逐字等于 `run_turn`。动作体就是整轮 future，
结局以 `TurnReport` 交回 session，不另开频道。

> **停用 `loop` ⇒ 该地址 `NotFound`、不再开新轮**；在飞的轮持有交付包与端口的 `Arc`，
> 存活期长于插件实例，会在有限时间内收敛（守卫用例见
> [`composite.test.rs`](../../symbio/src/plugins/composite/composite.test.rs)）。
> 本插件不贡献能力行与常驻任务，只贡献 1 条 Mount 行（[D-086](../003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表")）。

---

## Classify 插件（对话面：判决）

「这一轮该直接回答、还是派给工具循环」——只输出**枚举**，不产出面向用户的文本。唯一路由 `classify/decide`（契约见上表）。

> 调用方只有 `session`（编排层）：它投影事实、发起调用，并按判决**执行**对应路径。契约 `DecideRequest` / `Verdict` 见 [`symbio_core::dialog`](../../symbio/src/symbio_core/dialog/mod.rs)。不挂载本插件 ⇒ 路由 `NotFound` ⇒ 全部输入直接进工具循环（卸载平凡值）。两条产线（规则短路 0 次 LLM / 快速档 1 次静默 LLM）见 [classify 插件的 README](../../symbio/src/plugins/classify/README.md "symbio/src/plugins/classify/README")。

---

## Compose 插件（对话面：措辞）

「首响 / 答话 / 汇报」的措辞——只输出**文本**，不做判决。唯一路由 `compose/wording`（契约见上表）。

> 调用方只有 `session`。本插件**不写转写**：文本由 `session` 落库，转写只有一个写入者。契约 `ComposeRequest` 见 [`symbio_core::dialog`](../../symbio/src/symbio_core/dialog/mod.rs)；其中 `snapshot`（`RunSnapshot`）是 `session` 投影的运行现状，**只有 `Report` 读它**。出参**空串 = 没有对话面文本**（平凡值），调用方据此不写节点。
>
> 不挂载本插件 ⇒ 路由 `NotFound` ⇒ `session` 按「缺插件」处理：`Answered` 拿不到措辞时**降级进工具循环**（不沉默），`Escalate` 没有首响，`Report` 只是"这次没说"且不消耗汇报配额（卸载平凡值）。三条产线（模板 0 次 LLM / `from_context` 生成 1 次静默 LLM / `Report` 填表 0 次 LLM）见 [compose 插件的 README](../../symbio/src/plugins/compose/README.md "symbio/src/plugins/compose/README")。调用方开关 `SessionConfig::compose_enabled`（`<session 目录>/PLUGIN.yml`）；`Report` 的**触发**另归 `SessionConfig::progress_enabled` 等四个旋钮（判据行 `policy.report` 与 `report_due` 视图见 [`loop_rows.rs`](../../symbio/src/plugins/loop/loop_rows.rs)）——措辞与触发是两件事，各有各的开关。

---

## Model 插件

**本插件无自有路由**：`ModelProvider::execute_turn` 由 `session` 在会话循环内**直连调用**，不占路由；配置读写全在 VDFS 上（`<根>/model/<id>` 与 `<根>/model/PLUGIN.yml`）。

### 支持的协议

| protocol_id | 提供商 | API 端点 |
|------|------|------|
| `openai_chat` | OpenAI | `/v1/chat/completions` |
| `openai_responses` | OpenAI | `/v1/responses` |
| `anthropic_messages` | Anthropic | `/v1/messages` |
| `gemini_api` | Google | `generateContent` |

> 协议适配内化于 `symbio/src/plugins/model/protocols/`，机制见 `symbio/src/plugins/model/README.md`；为什么支持多协议见 [D-004](../../symbio/src/plugins/model/README.md "symbio/src/plugins/model/README")。

---

## Local 插件

> **路由按工具名动态分发**：`local/<工具短名>`，与 LLM 工具是**同一份集合**（`route` 里 `tool_impls.iter.find(|t| t.name == path)`）——成员清单与风险等级见 [RE-006 §2.1](006-当前事实表.md#21-llm-可见工具 "RE-006-当前事实表")（工具面），本文不列第二遍。`shell` 族工具名按操作系统取（Windows = `cmd`），静态抽取只见占位名。

> **文件编辑类原生工具已迁入 VDFS 物理层**，由 `vdfs` 插件以`vdfs_read` / `vdfs_write` / `vdfs_edit` / `vdfs_search` 等统一暴露，本插件不再提供。`ask_user` 产 `user_prompt` 节点等用户回答（与工具审批同一套回填机制）；自动模式下不产节点、返回 `tool_unavailable` 让模型自行继续。

### Shell 命令策略

高危操作触发 `ToolApprovalRequest`，需用户显式授权。

---

## Web 插件

> **路由按工具名动态分发**：`web/<工具短名>`，与 LLM 工具同一份集合（同 `local`）。三个工具（`web_search` / `web_fetch` / `http_request`）的清单与语义见 [RE-006 §2.1](006-当前事实表.md#21-llm-可见工具 "RE-006-当前事实表")。

---

## Skill 插件

> 唯一静态路由 `skill/execute`（契约见上表）。

> 本插件**不设配置文档**。技能清单与内容不设私有路由：已安装技能经 `<根>/skill` 寻址（一个技能 = 一个目录，主文件 `SKILL.md`，条目内部可下钻）。

### Skill 结构

```
skills/<name>/
├── SKILL.md # Skill 定义 (frontmatter + 说明)
└── ... # 附带资源
```

---

## MCP 插件

**本插件无自有路由，也不设配置文档**——配置就是它的资源树。Server 清单与详情不设私有路由：注册 / 列举 / 启停 / 连通性自检一律经 `<根>/mcp`（一个 server = 一个目录，主文件 `server.json`），「测试连接」是节点动作 `vdfs/action { action: "test" }`。

### MCP 配置

```yaml
mcp_servers:
 my_server:
 transport: stdio # 或 http
 command: "npx"
 args: ["-y", "mcp-server"]
 # transport: http
 # url: "http://localhost:3000"
```

---

## Telegram 插件

> 六条静态路由的契约见上表。本插件是**对外通道**：这些路由仓内没有调用方，但网关会把外部请求里的 `path` 原样转发给容器 `route`，所以它们是**对外开放接口**，不是死代码。

> 本插件挂载 `<根>/telegram`，内容就是一份配置文档 `<根>/telegram/PLUGIN.yml`（`ext = form`），读写走 `vdfs/read` / `vdfs/write`。

---

## Gateway 插件

HTTP/WebSocket 入站网关（`plugins/gateway/server.rs`），外部客户端与 route_v2 同构接入插件树。四类端点：`POST /api/v1/invoke`（对齐 `route_v2`）、`WS /api/v1/ws`（首帧即请求体，此后双向 `PluginFrame`）、`GET /api/v1/events`（SSE，`event_bus/subscribe` 的只读包装，信封原样、仍需另走 `vdfs/watch`）、`GET /api/v1/health`。

令牌只走 `Authorization: Bearer`；浏览器 WS 经 `Sec-WebSocket-Protocol: symbio.v1, bearer.<token>` 携带（握手只回选 `symbio.v1`）。查询串令牌不被接受。`POST /api/v1/invoke` 可带 `Idempotency-Key`：分派前登记，TTL 内重复回 409，失败释放（语义见 [DE-013](../003-design/013-HTTP开放接口.md "DE-013-HTTP开放接口") §5.1.1）；SSE 在只读模式下拒绝（§5.5）。

### 作为插件的路由

挂载于 `worker`（Composite）之下，外部入站请求最终由 gateway 转发给父级 composite：自身唯一静态路由 `gateway/status`（契约见上表）。

> 配置就是 `<根>/gateway/PLUGIN.yml`（扁平键 `inbound_*`）；写入走 `vdfs/write` →`PluginConfigFile::apply` 落盘后，本插件在自己的 `write` 里做副作用（**内部 stop + start 重建监听**）。`gateway/*` 自身接口**恒走 native**（前端不经 HTTP 访问本插件）。
>
> 安全：非回环地址需 `inbound_token` 鉴权（回环地址免鉴权）；`inbound_readonly` 开启后仅放行只读白名单（`vdfs/root|list|tree|stat|read|search`；`gateway/*` 显式排除，且 `vdfs/read` 只要落在任何插件的 `PLUGIN.yml` 上就拒绝——配置可能含凭据），详见 [RE-001-CONFIGURATION](001-CONFIGURATION.md)。

---

## Plugin_manager 插件（插件管理）

**本插件无自有路由**。清单与取值分别由 `<根>/plugin_manager` 的 `vdfs/list` / `vdfs/read` 承担。

它是本智能体**插件集合**的门面：清单 = **各插件交出来的配置条目 + 自有分区**（`appearance` / `about`）。前者由各插件在 `traverse` 里经 `capability_announce_configurable` 声明，容器用共享收集器收下并写回请求 ctx（见 [DE-010](../003-design/010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范") §2 分层与落点）——因此**新增一个可配置插件不需要在本插件登记任何东西**；自有分区的数据在前端 store，`read` / `write` 对它们恒 `Forbidden`。

配置条目**不是第二种东西**：它就是那个插件的配置，入口挂在`<根>/plugin_manager/<插件名>`，读 / 写转发到容器根下的 `<插件名>/PLUGIN.yml`——同一份配置仍然只有一个**文件**、一份**定义**（拥有者给的那份）。

装配动作（启用 / 停用 / 卸载）与「添加插件」也走这里，但**动词在容器**：本插件只把`Action(enable|disable)`（插件名进载荷）、`Delete(<插件名>)`、根上的 `Write`（安装）转发回所在容器（见 `composite/vdfs.rs` 的「插件注册表」一节）。必需插件（构造者经`REQUIRED_PLUGINS` 声明）**不可删除，但可停用**；界面底座插件（`ASSEMBLY_UNDISABLABLE_PLUGINS`）连停用也拒绝——停掉它就没有界面再把它打开。

---

## Hook 插件

> **命名空间是目录名 `hook`，不是 `hooks`。** 容器（`composite`）按**目录名**建实例表并在`route` 里按它分发（「目录名 = 实例名」），所以目录名才是真正的路由前缀。三条静态路由（`register` / `fire` / `list`）的契约见上表。

### 系统事件

> `pre_tool_use` · `post_tool_use` · `pre_file_write` · `post_file_write` · `pre_shell_execute` · `post_shell_execute` · `user_prompt_submit` · `session_start` · `session_end`

---

## Event Bus 插件

进程内事件总线：聚合各插件发出的帧并向订阅连接广播（连接级 SSE 风格推送）。两条静态路由（`subscribe` / `ping`）的契约见上表。

> `event_bus/publish` **不存在**：本插件是进程内帧广播（如 VDFS 变更、总线自身握手），发布方在进程内直接调用，不经路由。

---

## 退役路由索引

**只登记「路径 → 现在去哪」，不写过程叙述**（决策与理由见决策索引、变更史见 `git log`，完整迁移审计见 [`docs/archive/legacy-route-migration.md`](../archive/legacy-route-migration.md)）：

> `{plugin}/config/get` · `{plugin}/config/set` · `{plugin}/config/schema` · `{plugin}/entities/*` · `model/chat` · `model/status` · `model/chat_sync` · `agent/list` · `get` · `upload` · `delete` · `preview` · `export` · `session/append` · `session/open` · `session/update` · `session/clear` · `session/chat/send` · `session/chat/abort` · `session/chat/update_message` · `session/chat/delete_message` · `session/chat/clear_messages` · `session/get_messages` · `session/options/list` · `session/stream` · `session/heartbeat/trigger` · `hook/trigger` · `hooks/*` · `event_bus/publish`

---

> **维护原则**：新增路由 = 在插件 `route` 加 `match` 臂 + 臂上一句职责注释（地址与职责自动进 [RE-006 §2.3](006-当前事实表.md#23-前端--宿主路由 "RE-006-当前事实表")），契约（调用方 / 返回类型 / 备注）来本文的契约表补一行；删除路由时**只把路径挪进 §退役路由索引**，不在此处写过程叙述。

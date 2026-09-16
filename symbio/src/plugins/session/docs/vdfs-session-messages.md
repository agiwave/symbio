# 会话消息的 VDFS 化 —— 转写即列表，流式即追加

> 前置阅读：[vdfs.md](../../../../../docs/design/vdfs.md)（机制规范）、[vdfs-frontend.md](../../../../../docs/design/vdfs-frontend.md)（前端页面规范）。
>
> 本文回答一个问题：**会话消息如何整体并入 VDFS 体系**，以及为什么
> 「会话列表本身也是列表，流模式其实就是列表项的变更，变更类型为追加」
> 不是一句类比，而是可以直接落地的机制判断。

---

## 1. 命题

迁移前是**两套并行的读链路**：

| 数据 | 读入口 | 实时入口 |
|---|---|---|
| 会话清单 | `vdfs/list`（`.vdfs/session`，S8 已迁移） | VDFS 事件总线（`kind = "vdfs"`） |
| **会话消息（转写）** | `session/get_messages`（专用协议） | 会话事件总线（`kind = "session"`，`ChatEventType`） |

也就是说：**清单在 VDFS 里，消息不在**。而消息恰恰是会话的主体。

命题是：**消息不需要自己的读协议，它就是一张列表**。而流式输出也不是另一种
模式，它是「列表最后一项的内容被追加」——一种**追加型变更**。

> **S16–S19 完成后**：转写的读入口是 `.vdfs/session/<sid>`（一次 `read` 拿整份
> 历史），实时入口是 `kind = "vdfs"` 的变更。`session/get_messages` 前端不再调用，
> `kind = "session"` 只承载会话级事件。**两套读链路收敛为一套。**

---

## 2. 为什么成立

### 2.1 转写天然满足列表的全部条件

| 列表的性质 | 转写的实际情况 |
|---|---|
| 有序 | `seq` 是**唯一权威顺序锚点**（见 `chat_message.rs` 的 `seq` 文档） |
| 每项有稳定标识 | `ChatMessage.id` |
| 每项有独立内容 | `content`（正文） |
| 项可增、可删、可改 | 追加、删除某条及其后、状态迁移 |
| 顺序与内容分离 | 顺序 = `seq`；内容 = `content` |

所以「会话内部」和「`.vdfs/session` 根目录」在机制上是**同一种东西**，
只是层级不同。既然根目录已经是 VDFS 列表，没有理由内层反而不是。

### 2.2 流式不是模式，是变更

`ChatMessage::apply_patch` 已经把语义写死了：

> `content`：`Text` / `Reasoning` 走 **增量追加**（SSE delta 语义）；
> `ToolCall` 与 `Parts` 走 **全量替换**。

即：**流式 = 对某个列表项做「尾部追加」**。这正好对应 VDFS 变更词汇里缺的那一格：

| VDFS 变更 | 语义 | 消费者动作 |
|---|---|---|
| `created` | 多了一个节点 | 列表插入一项 |
| `updated` | 节点变了，**内容全量** | 重读该节点 |
| **`appended`**（新增） | 节点**尾部多了这些字**，增量 | 拼接 `delta`，不重读 |
| `deleted` | 节点没了 | 列表移除一项 |
| `renamed` | 节点换了地址 | 改键 |

`appended` 不是为聊天开的后门：它是**任何追加型数据**（日志、转写、生成中的
文档）都需要的通用变更类型。一次流式响应有几十到上百帧；若一律用 `updated` +
重读，流量是 O(n²)——机制上必须能表达「增量」。

---

## 3. 地址与节点形状

### 3.1 地址空间

```text
.vdfs/session/                    会话清单（已有）
.vdfs/session/<sid>               单个会话（ext = session，点开即聊天工作区）
.vdfs/session/<sid>/消息          转写列表  ← 新增（l 位）
.vdfs/session/<sid>/消息/<mid>    单条消息  ← 新增（ext = message，r 位）
.vdfs/session/<sid>/子会话[/<sub>]  子会话清单 / 单个子会话（已有）
.vdfs/session/<sid>/工作目录[/<rel>] 工作目录树（已有）
```

`消息` 是会话**本体**（转写），因此排在内置子目录的第一位。

### 3.2 列表项的分工：正文进内容，结构进 attributes

```text
read(.vdfs/session/<sid>/消息/<mid>)
  → VdfsContent::text  = 这条消息的正文（流式追加的正是它）
  → VdfsNode.attributes = { role, type, parent_id, seq, error, meta }
  → VdfsNode.ext        = "message"
  → VdfsNode.status     = pending | streaming | waiting_user_action | failed | active
  → VdfsNode.title      = 角色（工具调用补工具名：「助手 · vdfs_list」）
  → VdfsNode.description = 正文首行预览（限 60 字符）
```

为什么这样切分：

- **正文必须在内容里**——否则「追加」无法表达（JSON 文档的尾部追加没有语义）；
- **结构必须在 `attributes` 里**——`role` / `type` / `parent_id` 是场景数据，
  VDFS 只透传，渲染器按 `ext` 自行取用（与 `session_node` 挂 `message_count` 同一手法）；
- **组合节点（`turn` / `tool_call`）没有正文**：`read` 退化为它们的 JSON 视图，
  使前端与 LLM 在同一地址上都能取到完整结构。

### 3.3 顺序

列表顺序**只认 `seq`**（稳定排序，缺 `seq` 的排最后并保持原有相对顺序）。
这与 `ChatMessage.seq` 的既有契约完全一致：`timestamp` 会并列、会回拨、会缺失，
`seq` 由存储在写入时分配、单调且永不并列。

---

## 4. 变更语义（实时链路）

订阅地址：`.vdfs/session/<sid>/消息`（或更上层的 `.vdfs/session`）。

| 触发 | 变更 | 载荷 | 消费者 |
|---|---|---|---|
| 新增一条消息 | `created` | `node` + `content` | 插入列表项（**零回读**） |
| 正文流式追加 | `appended` | `delta` | 尾部拼接，**不重读** |
| 状态迁移 / 全量替换 | `updated` | `node` + `content` | 就地替换（**零回读**） |
| 删除某条及其后 | `deleted` | — | 移除 |
| 清空转写 | `deleted` | `path = …/消息` | 清空列表 |

**唯一订阅通道**：`kind = "vdfs"`。会话不再需要 `kind = "session"` 承载消息
（`title` / `status` 等会话级事件继续走原通道）。

### 4.1 载荷为什么必须挂在事件上

只发「哪里、怎么变」而不发「变成了什么」，消费者就只能自己回读：

| 方案 | 每条消息的额外往返 |
|---|---|
| 只发路径（消费者回读） | `created` / `updated` 各 1–2 次（`stat` + `read`） |
| 载荷随事件（本设计） | **0** |

一次会话轮次会产生十几次 `created` / `updated`，回读方案是净增几十次 IPC；
而既有的 `kind = "session"` 消息通道本来就是**零回读**的（它直接下发完整补丁）。
若不把载荷带上，迁到 VDFS 就是一次**性能退步**——统一机制的代价不该由热路径承担。

反过来，`appended` 每帧都发，载荷必须保持只有 `delta`：把整节点挂在每帧上，
流量立刻退化成 O(n²)，正好抵消 `appended` 存在的意义。

于是规则是：**热路径窄（只有增量），冷路径全（节点视图 + 内容快照）**。

> 载荷是**可选**的：provider 不填时消费者回退到回读（`stat` + `read`）。
> 因此这是纯增益扩展，不构成对任何 provider 的强制。

---

## 5. 写入为什么不走 VDFS

**发言不是一次文件写入，而是一次动作。**

`chat/send` 会触发一整轮编排：模型调用 → 工具执行 → 多轮循环 → 流式落库。
把它建模成 `vdfs/write` 会造成两处语义错位：

1. `vdfs/write` 是**同步返回**的存储操作，而发言是长事务；
2. `vdfs/write` 的语义是「把这段内容存到那个地址」，而发言的语义是
   「以这段内容为输入，跑一轮编排」。

因此 `write` 在 `.vdfs/session/<sid>/消息` 上**明确拒绝**（`Forbidden`），
而不是静默降级。

> 这不是「两套写路径」。VDFS 侧**没有**消息的写路径——只有读路径。
> 写入的唯一入口仍然只有一处（聊天协议），前端与 LLM 只是在**同一个地址**上
> 读同一份数据。机制上「一条消息只有一个地址」，没有产生第二份真相。

---

## 6. 迁移路线

### S16 —— 机制与读路径（已完成）

- `symbio_core::vdfs_provider`：新增 `VDFS_CHANGE_APPENDED`、`VDFS_EXT_MESSAGE`；
  `VdfsChange` 新增 `delta: Option<String>` 与 `VdfsChange::appended(path, delta)`；
- `plugins/vdfs/protocol`：`VdfsChangeEvent` 新增 `delta`；
- `plugins/vdfs/host`：`to_change_event` 透传 `delta`；
- `plugins/vdfs/fs`：`watch` 包装器透传 `delta`（并把路径补成展示口径）；
- `plugins/composite/vdfs`：`watch` 包装器透传 `delta`（并把路径补成树内全口径）；
- `plugins/session/plugin`：`.vdfs/session/<sid>/消息[/<mid>]` 的
  `list` / `stat` / `read`；`write` 明确拒绝。

**S16 完成后即可用**：前端能在 VDFS 里浏览任意会话的转写；
LLM 能通过 `vdfs_read` 在**同一地址空间**里读会话历史——
这是「同一集合（前端与 LLM 恒等）」在会话上的直接兑现。

### S17 —— 变更发射（把既有流式补丁翻译成 `appended`）（已完成）

#### 发射点：不是 `broadcast_message_update`，而是**消费循环**

原计划把翻译挂在 `chat_loop::broadcast_message_update` 上。实施时发现这个判断
**不成立**：那个函数只是**众多**发射点之一。写 `StreamEvent::Update` 到前端的
地方至少有——`turn.rs::emit_update`（模型流式，3 处）、`chat_loop`（4 处）、
`tool_executor`（8 处）、`resume.rs`（恢复重写）、`ask_user`、`local/shell`
（工具输出流）、`agent/host/subagent`（子会话转发）。挂在其中任何一个上，
其余路径的消息都不会产生 VDFS 变更。

真正的收口在 **`orchestrator::run_chat_loop_task` 的消费循环**
（`PluginFrame::Data` 分支）。它是**全部**补丁汇入前端的必经之路——上游有多少个
发射点都无所谓，到这里只剩一个；且 `session_id` 与 `SessionPlugin` 都在作用域内，
不需要引入 `MessageChangeSink` 之类的窄接口。

于是结构变成：**入口有两个，出口只有一个**。

| 入口 | 场合 |
|---|---|
| 消费循环 | 流式逐帧补丁（模型 / 工具 / 子会话 / 审批 / 恢复） |
| `persist_failure` | 错误或崩溃后由服务端定稿的终态 |

两者都是真实时刻，无法合并成一处调用；但「补丁到前端」与「变更进 VDFS」必须
同生共死——因此把两件事绑进同一个函数 `SessionPlugin::emit_message_patch`。
漏发一次变更，VDFS 列表就与前端流式视图**永久**不一致，且没有任何机制会纠正
它（两条链路互不校验）。

#### 映射：判据不重算，由合并函数**回报**

| 补丁形状 | 变更 |
|---|---|
| 新 `id`（此前未出现） | `created` |
| 尾部追加了 `delta` | `appended`，`delta` = 被拼进去的那一段 |
| 其余（状态迁移 / 全量替换） | `updated` |
| 删除 | `deleted`（沿用既有路径） |

关键实现细节：`merge_message_patch` **返回**它追加的那段文本
（`Option<String>`），`message_change` 只做词汇翻译、不做二次判断。

> 若让 `message_change` 自己再判一次「这个补丁算不算追加」，判据就有两个实现，
> 迟早与合并方式分叉：合并按追加、变更判成替换 → 消费者按 `delta` 拼接得到错误
> 内容，而且是**静默**的错误。让唯一决定合并方式的地方顺带说出它是哪种方式，
> 这种漂移在结构上就不可能发生。

#### 必须一并解决的一件事：在途消息的可读性

流式期间消息**还没落库**（`persist_messages` 只在每轮结束时写盘）。若 VDFS 列表
只读存储，那么 `created` / `appended` 到达时消费者去 `list` 会一无所获——
「转写即列表」当场失效。

因此 `ActiveSessionState` 增加 `live_messages`：**与消费循环收集流式补丁的那个
缓冲是同一个 `Arc`**（不是第二份拷贝）。一份数据、两个视图：

- 前端实时流：逐帧 `StreamEvent::Update`；
- VDFS 转写列表：`transcript_of(id)` = 落库转写 ∪ 在途缓冲。

合并规则：同 id 时在途版本胜出（它更新），但 **`seq` 从落库版本继承**——顺序锚点
只由存储在写入时分配，在途副本没有；不继承的话同一条消息会以「有 seq / 无 seq」
两种形态被排到列表的两个位置。

生命周期：每轮开始清空（防残留）→ 本轮落库后清空（`persist_failure` 内、
以及消费循环收尾处）。

#### 一处「看起来是 bug、查下来不是」的记录：`watch` 的路径口径

实现时一度认定 `SessionPlugin::watch` 有缺陷：它忽略被订阅的 `path`，把广播源里
provider 根口径的路径（`abc/消息/m1`）原样转发，看起来会被容器再补一遍前缀、
拼成 `session/abc/消息/abc/消息/m1`。

**核对后确认不是缺陷。** 关键在于容器的回填只补**挂载名（首段）**，不补被订阅的
整条路径——`CompositeVdfs::watch` 把 `(dir, rel)` 拆开：`rel` 传给 provider，
回填时只加 `dir`：

```text
watch("session/abc/消息") → dir = "session", rel = "abc/消息"
provider 报 "abc/消息/m1" → 容器补成 "session/abc/消息/m1"  ✓
若先剥成 "m1"            → 容器补成 "session/m1"          ✗
```

即：**`watch` 的 `path` 与上报的 `VdfsChange.path` 同在 provider 根坐标系**，
两处都做前缀处理反而会错。原实现是对的，「修」它才是引入缺陷。

> 之所以值得记下来：这段的措辞（「provider 子树内相对路径」）天然有歧义——
> 「子树」既可读作 *provider 自己的子树*，也可读作 *被订阅的那棵子树*。
> 已在 `vdfs.md` §9 与 `SessionPlugin::watch` 的文档注释里把两种读法的后果写明，
> 免得下一个人再走一遍这条弯路。

副作用（已知、可接受）：订阅者会收到兄弟子树的变更——订阅一个会话的转写会看到
别的会话的事件。消费者按路径前缀过滤即可，正确性不受影响，只是多些流量。

### S18 —— 前端消费（已完成）

落点四处，各解决一件不同的事：

| 落点 | 做了什么 |
|---|---|
| `composables/useVdfs.ts` | 识别 `appended`：命中当前详情就**就地拼接**，未命中什么也不做；**都不重拉** |
| `components/vdfs/VdfsMessageDetail.vue` | `ext = message` 的只读渲染器（正文 + role/type/status/error） |
| `services/vdfsTranscriptSync.ts` | 转写的 VDFS 实时消费端：`kind = "vdfs"` → store，**逐路径串行** |
| `stores/sessions.ts` | 读入口改为 VDFS：`readVdfs('.vdfs/session/<sid>')` 一次拿整份历史 |

#### 为什么追加**绝不能**触发重拉

`appended` 不改变任何节点的存在、顺序与预览首行，只让某一项的正文变长。若让它
走通用分支（`affects(cwd, path) → scheduleRefresh()`），流式每一帧都会挂一次
防抖刷新——而"绑定地址一层的变更"还会顺带重拉左栏导航。于是：

- 流量退化成 O(n²)（`appended` 存在的唯一理由被抵消）；
- 更糟的是**每次重拉都会重读当前详情**，与在途增量正面竞争（见下）。

因此 `appended` 在 `onChange` 的**最前面**分流并 `return`：命中就拼接，不命中就
忽略。这是它与其他四种变更在消费端的唯一分野。

#### 刷新竞争：两种消费端，两种对策

`appended` 是**乐观应用**，而 `created` / `updated` / 未知 id 会触发一次回读。
回读响应与在途增量之间存在竞争，且**失败是静默的**：

```text
t0  节点本地内容 "abc"
t1  收到 appended "def"        → 本地 "abcdef"   ✓
t2  一个更早发出的读响应到达    → 本地被整体替换回 "abc"   ← 旧快照覆盖了新内容
t3  收到 appended "ghi"        → 盲目拼接成 "abcghi"      ← 静默损坏
```

两个消费端的正确对策**不同**，因为它们的约束不同：

**(a) `useVdfs`（视图）——刷新代际守卫。**
维护「已应用到哪个路径、应用了几次」；`select` 发起读取时取快照，响应回来若计数
已变则**丢弃该响应**（本地内容比快照新，而节点增删另有 `created` / `deleted` 兜底）。
配套的必要条件：**同一节点的重读不得清空正文缓冲**——否则被丢弃的响应会留下一片
空白（把 `"abcdef"` 变成 `"def"`）。清空只在换节点时发生。

**(b) `vdfsTranscriptSync`（数据）——逐路径串行。**
转写是**数据**，不能靠丢弃来"收敛"（丢掉的是真实内容）。因此同一路径的变更串成
一条顺序链：`created` 的回读没落地，后面的 `appended` 就不开始。链上每一环等前
一环结束，**竞争在结构上不存在**，不需要任何守卫。不同路径互不阻塞。

> 为什么不在协议里塞"追加后长度"来自校验：长度单位必须与消费端一致（JS 是
> UTF-16 代码单元，Rust 是字节），选错单位会在中文文本上触发**持续误判**。
> 串行链不需要这个前提，也就没有这个坑。

> 之所以写这么细：这类竞争不会报错、不会崩溃，只会让 UI 悄悄少一段字或多一段字，
> 而本项目此前已经踩过两次同类坑（`StreamChildIds` 的重复节点、sessionBusWatcher
> 的 HMR 双订阅叠字）。两个消费端都补了 HMR 守卫，同一个坑不踩第三次。

### S19 —— 收敛旧通道（已完成）

- `kind = "session"` 只保留**会话级**事件（`status` / `error` / `abort` / 连接）。
  消息相关的 `ChatEventType.Update` / `Delete` 在消费端**不处理**：再 patch 一次就会
  与 VDFS 通道形成双写（流式文本叠字）。
  这两个类型**仍会从总线到达**——进程内的 subagent 宿主需要 `Update`（子会话审批
  透传 + 累积 Assistant 文本，见 `agent/host/subagent.rs`），所以后端必须继续发布；
  前端只是不动作。消费端的两个 `case` 已删除（`switch` 无 `default`，空分支是可证明
  的空操作），该约定以注释形式留在 `sessionBusWatcher` 的 `switch` 上方。
- 中止（`Abort`）分支里的「扫 streaming 节点收敛」整段删除：在途消息的终态由
  服务端 `persist_failure` 定稿并经 VDFS 变更下发，此处再扫一遍是第二份实现。
- 顺带清掉一条**死事件**：`{type:'title'}` 曾由 `handlers::invoke_update` 与
  `orchestrator::ensure_auto_title` 两处发布，但消费端在本仓库历史中从未存在过
  （`git log -S` 可证）。标题的对外可见性早已由实体机制承担——
  `publish_entity_changed` 本就携带 `display_title`。两处发布已删除。
- **由此暴露并修掉一个真缺陷：自动命名到不了任何清单。**
  `ensure_auto_title` 原先只落盘（`{type:'title'}` 无人消费），而
  `invoke_update` 是**同时**发实体事件 + VDFS 变更的。两条路走两套链路，
  结果首条消息生成标题后，侧栏项名与聊天头部标题都停在「新对话」，
  直到下一次整表重拉（页面挂载 / `created` 防抖）才更新。
  修法是把 `ensure_auto_title` 对齐成**同构的两条信号**——标题变更不该因
  发起者不同而走不同链路：

  | 信号 | 通道 | 收敛到 |
  |---|---|---|
  | `publish_entity_changed(.., "updated", display_title)` | `kind = "entity"` | 会话清单 store（`titles` + 清单项，**零重拉**） |
  | `notify_change(session_id, updated)` | `kind = "vdfs"` | VDFS 左栏导航（按直接子节点变更 `refreshNav`） |

  前端侧配套：`stores/sessions.ts` 的实体 `updated` 分支改为**就地消费**
  事件携带的 `display_title`（写 `titles` 与清单项），不再等重拉。

  **终局（2026-09-15）**：上面的「双发」也只是过渡形态——`kind = "entity"` 频道
  连同 `publish_entity_changed` / `publish_entity_status` 已整体删除，会话只剩
  `kind = "vdfs"` 一条变更通道。标题 / 状态一类变化的落地方式因此改为：
  **该会话节点的一次 `updated`**（session 侧的 `notify_change(sid, "updated")`），
  前端收到后重读 `vdfs/stat` 取新标题与状态。`notify_change` 不携带 `node`
  载荷（不为此扩展 core 协议），故这条路径上**没有**「就地消费事件字段」这一说，
  一律防抖重拉；带载荷的增益投递只存在于 provider 自己实现的 `watch` 里
  （消息转写的 `appended` + `delta` 就是它）。
- 前端死文件 `schemas/session_get_messages.ts` 删除（`getSessionMessages` 包装器
  失效后它失去唯一引用）；`session/get_messages` **后端路由保留**——它仍被
  `agent/host/subagent.rs` 用来读父会话历史。
- **删除也必须发变更**：`resume.rs` 的 `StreamEvent::Delete` 由消费循环转成
  VDFS `deleted`（消费循环是全部补丁的唯一收口，删除帧同样经过它）；
  `chat/clear_messages` / `chat/delete_message` / `chat/update_message` 这三条
  **前端自发起、已在本地收敛**的路由则各有一个「只发变更、不发前端帧」的入口。
  漏掉任何一条，VDFS 视图都会残留一个已不存在的节点且永不纠正。

---

## 7. 不变量（迁移全程必须成立）

1. **顺序锚点仍是 `seq`**——VDFS 列表序、前端消息表、LLM 上下文三者同源；
2. **一条消息只有一个地址**：`.vdfs/session/<sid>/消息/<mid>`；
3. **正文只有一个位置**：节点内容。`attributes` 只放结构，不放正文；
4. **前端与 LLM 恒等**：同一地址、同一份数据、同一组访问位；
5. **变更只有一个通道**：`kind = "vdfs"`；
6. **追加是增量语义**：`appended` 必须携带 `delta`，消费者不得借此触发重读；
7. **写入入口唯一**：消息写入只有聊天协议一处，VDFS 侧显式拒绝；
8. **补丁与变更同生共死**：任何送达前端的消息补丁都经
   `SessionPlugin::emit_message_patch`，因此必然产生对应的 VDFS 变更。
   出口只有一个——新增发射路径时若绕开它，VDFS 视图会**永久**落后，
   而两条链路互不校验，不会有人发现；
9. **在途消息可读**：`list` / `stat` / `read` 看到的转写 = 落库 ∪ 在途。
   在途缓冲与消费循环收集的是**同一个 `Arc`**——不是两份需要同步的拷贝；
10. **变更判据不重算**：`appended` 与否由 `merge_message_patch` 的返回值决定，
    不在翻译层另判一次。
11. **删除也发变更**：消息级删除（`resume` 的 `Delete` 帧、`chat/delete_message`
    / `chat/clear_messages`）同样产生 VDFS `deleted`，否则列表会残留已不存在的节点。
12. **热路径窄、冷路径全**：`appended` 只带 `delta`；`created` / `updated` 带
    `node` + `content`。前者逐帧发，后者每轮几次——载荷宽度按频率分配。
13. **同路径变更串行**：转写消费端按路径串成顺序链，回读与增量在结构上不竞争。

---

## 8. 取舍与风险

| 取舍 | 说明 |
|---|---|
| 组合节点退化为 JSON 视图 | `turn` / `tool_call` 无正文；强行扁平化会丢失层级。JSON 视图是诚实的降级，`attributes.type` 让渲染器知道该按什么渲染 |
| 图片消息只暴露文本部分 | `MessageContent::Parts` 里的 `image_url` 不在正文中。需要时再走 `binary` 内容通道（本期不做） |
| `appended` 增加了协议面 | 但它是通用能力（任何追加型数据都需要），不是聊天专用后门；且与 `updated` 的区分是**流量与语义双重必要** |
| 变更信封多了两个可选字段 | `node` / `content` 使 `created` / `updated` **零回读**。若不带，迁移就是净增几十次 IPC/轮次——统一机制的代价不该由热路径承担。字段可选，不填时消费者回退回读 |
| `message_status` 把 `completed` 与「未标注」都归为 `active` | VDFS 节点只有一套状态词汇，不为场景再造一套。消费端把 `active` 读作 `completed`——两者在渲染上一致（都不是进行中），因此是无害的有损映射 |
| 会话叶子同时是「文档」与「内部区段之父」 | `.vdfs/session/<sid>` 是叶子（`rw`，内容是整份会话 JSON），其下又挂 `消息` / `子会话` / `工作目录`。这不是矛盾：**叶子是会话本体，区段是它的视图**。好处是整份历史一次 `read` 拿到，省掉专用协议 |
| 在途缓冲与落库转写有重叠窗口 | 每轮结束时落库，随后清空在途缓冲。重叠期内同 id 以在途版本为准且继承 `seq`，因此列表不会出现两份或错序；真正需要警惕的是「清空早于落库」——清空点因此都放在落库之后 |
| 刷新竞争是**静默**失败 | 见 S18 小节。不会报错、不会崩溃，只会让 UI 少一段字或多一段字。两个消费端各选了一种对策（视图：代际守卫；数据：逐路径串行），没有都不做 |
| 转写消费端会漏掉"未收到 `created` 就先收到 `appended`" | 此时 `patchMessage` 会造一个只有增量的桩。实际不可达（`created` 与 `appended` 经同一路径的同一条顺序链），但若未来出现别的发布顺序，桩会以"内容不全"的形式暴露而非静默错乱 |
| 地址规则曾有三份实现 | `..` 判定与前缀比较此前在 `physical.rs`、`local/policy`、`host::search_via` 各写一份，且**三处各漏一处**：`src\..\..\..\Windows` 绕过守卫、`/etcfoo` 被误伤、`a/..` 漏判。现已收敛到 `symbio_core::vdfs_provider`（`has_parent_segment` / `path_within`），规则与测试各只有一份 |
| 变更转发曾逐字段重建 | `fs` / `composite` 的 watch 包装原先逐字段复制 `VdfsChange`，新增字段会被静默丢在转发层（且无编译错误）。现改用 `VdfsChange::map_paths` 一次覆盖全部路径，漏转发在结构上不可能发生 |

---

## 9. 与既有文档的关系

- [vdfs.md](../../../../../docs/design/vdfs.md)：机制规范。本文是它在**会话场景**上的应用，不引入新机制——
  `appended` 是变更类型的一个取值，`消息` 是一个普通子目录，`ext = message`
  是一个普通扩展名。
- [vdfs-frontend.md](../../../../../docs/design/vdfs-frontend.md)：该文件的 S1–S15 记录仍然有效；
  S16–S19 的落地见本文 §6。
- [vdfs.md](../../../../../docs/design/vdfs.md) §13.4：会话子目录由 `session` 插件自己的 `impl VdfsProvider`
  提供（清单与消息视图背后是**同一份存储**，不存在第二份数据）。曾经的「会话实体」
  抽象（`EntityProvider` / `EntityStore`）与其历史形态见
  [archive/entity-provider-mechanism.md](../../../../../docs/archive/entity-provider-mechanism.md)。

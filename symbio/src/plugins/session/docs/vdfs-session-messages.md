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
| 会话清单 | `vdfs/list`（`<根>/session`，S8 已迁移） | VDFS 事件总线（`kind = "vdfs"`） |
| **会话消息（转写）** | `session/get_messages`（专用协议） | 会话事件总线（`kind = "session"`，`ChatEventType`） |

也就是说：**清单在 VDFS 里，消息不在**。而消息恰恰是会话的主体。

命题是：**消息不需要自己的读协议，它就是一张列表**。而流式输出也不是另一种
模式，它是「列表最后一项的内容被追加」——一种**追加型变更**。

> **S16–S19 完成后**：转写的读入口是 `<根>/session/<sid>`（一次 `read` 拿整份
> 历史），实时入口是 `kind = "vdfs"` 的变更。`session/get_messages` 前端不再调用。
> **两套读链路收敛为一套。**
>
> **S26 续（2026-09-23）：这条协议整体退役。** 它剩下的唯一消费方是后端 `agent_run`
> 的**续会话存在性校验**——而「会话在不在」同样是 VDFS 的事：现在走**进程内纯接口探测**
> （`Plugin::get_vfs_provider()` → `stat("<挂载名>/<sid>")`，判 `attributes.message_count`），
> 既不再为回答「在不在」读回整份历史，也不再占一条路由。会话域至此只剩三条：
> `chat/send`（发言 / 编排）· `chat/abort`（控制）· `stream`（实时面，⚠️ S26 待退役）。
> `update` 也已退役（`vdfs/write` 的 `create` 位覆盖了它唯一多出来的能力，2026-09-23）。
> 见 [legacy-route-migration.md](./legacy-route-migration.md) §3.4.1。
>
> **S22 续**：当时剩下的一小块——`kind = "session"` 上的会话级事件（`Status` / `Error` /
> `Abort`）——也已废除（会话运行态即会话节点，见 `node-state-streaming.md`）。
> 会话域实时只余 `kind = "vdfs"` 一条频道。
>
> **S23 续（消息实时面改为单条转写流）**：消息的实时出口不再是 VDFS 变更，而是
> `worker/session/stream` 一条转写流——载荷是 `NodeOp`（`upsert` / `append` / `remove` /
> `reset` / `warn`）+ 会话内单调 `seq`（见 `symbio_core/transcript_stream.rs` 与
> `services/transcriptStream.ts`）。因此本文中一切「实时 = `kind = "vdfs"` 的
> `created` / `appended` / `updated`」的描述（§2.2 引的 `ChatMessage::apply_patch`、
> §S17 的 `merge_message_patch`、§4 的补丁形状对照表）**描述的是当时的模型**——
> 那两个函数**已删除**，消息不再走 VDFS 变更；保留下来是为了记录推导过程。
> **读面不受影响**：`read(<根>/session/<sid>)` 仍是一次拿整份历史，§2.1 的地址
> 与 §3.4 的 `seq` 对账**仍然有效**。
>
> **S24 续（帧收成一条消息）**：`NodeOp` / `NodeChange` **已删除**——转写流每帧的载荷
> 就是一条 `ChatMessage` 本身（`delta` 追加 / `content` 整条替换 / `status = removed`
> 删除），会话级告警下沉为 `TranscriptWriter::warn` 的独立通道。上段提到的
> `upsert` / `append` / `reset` / `warn` 变体随之消失，映射关系见
> `node-state-streaming.md` §6。
>
> **S25 续（会话运行态并入同一条流）**：会话**运行态**也改走这条转写流（`type =
> transcript_session` 的帧，与消息帧**共用同一个 `seq` 空间**），VDFS 侧不再携带会话
> 节点快照。因此本文的命题「会话消息如何整体并入 VDFS 体系」在**实时面**已被反转：
> 消息与运行态都**不**走 VDFS 了，`kind = "vdfs"` 只剩会话**资源**变更（创建 / 删除 /
> 改名 / 标题 / metadata）。理由是**快照的来源必须有序或幂等**——VDFS 是一条独立无序
> 通道，其上的快照会与转写流竞争（一次迟到的自动命名就能把运行态回退）。**读面仍归
> VDFS**（`read(<根>/session/<sid>)`），本文对「消息即列表、转写即列表项」的读面推导
> 依旧成立。见 `node-state-streaming.md` §11。
>
> **S26 续（实时面迁回 VDFS，ADR-025，2026-09-23）：上一段的结论被反转，本文的原始命题
> 重新成立。** 消息的实时出口**回到** `kind = "vdfs"`：消息**就是**
> `<根>/session/<sid>/message/<mid>` 这个**文件**，流式输出是该文件内容的**增长**——
> 发 `updated` + `delta`（`delta` 有 ⇒ 尾部追加；无 ⇒ 回读）。会话运行态是会话节点
> （`<根>/session/<sid>`）的 `status`，同样走 `updated`。`session/stream` 与
> `symbio_core::transcript_stream` 一并退役。
>
> **S25 那段的两条理由都被判定为错**：① 「快照的来源必须有序或幂等」——**只有「幂等」
> 那一半对**；「有序」把**陈旧写入**误诊成了**乱序**（单写入者 + 单通道 FIFO 下不存在
> 「后到的是旧的」）。② 「消息与运行态必须共用 `seq` 空间」——顺序是**节点属性**
> （`ChatMessage.seq`）不是投递属性，这条「顺序保证」**无从需要**。
>
> ⚠️ **但形态与 S16–S22 不同**：**没有 `appended` 这个取值**。「追加」不是一种操作，
> 而是 `updated` 上的一个可选 `delta` 字段——这正是消息流自己的经验
> （`ChatMessage` 帧「就是节点视图，没有操作枚举」）。因此本文 §2.2 的
> `apply_patch` / §S17 的 `merge_message_patch` / §4 的补丁形状对照表**仍然只是历史**：
> 它们描述的是「按 `change` 取值分派操作」的旧模型，而现行模型里**语义全在字段上**。
> 见 [`docs/design/vdfs.md`](../../../../../docs/design/vdfs.md) §9。

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

所以「会话内部」和「`<根>/session` 根目录」在机制上是**同一种东西**，
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
| `deleted` | **这一个**节点没了 | 列表移除一项 |
| **`truncated`**（新增） | 该节点**及其之后全部**没了 | 按 `seq` 取区间移除 |
| `renamed` | 节点换了地址 | 改键 |

`appended` 不是为聊天开的后门：它是**任何追加型数据**（日志、转写、生成中的
文档）都需要的通用变更类型。一次流式响应有几十到上百帧；若一律用 `updated` +
重读，流量是 O(n²)——机制上必须能表达「增量」。

同理，**`truncated` 与 `deleted` 必须是两个值，而不是在 `deleted` 上挂一个
`cascade` 布尔**。两者描述的不是粒度差异而是**语义**差异：

| | 含义 | 与顺序的关系 |
|---|---|---|
| `deleted` | **这一个**节点没了 | 无关（重试一轮时删它的子树、恢复工具调用时删旧结果子节点） |
| `truncated` | 从这里**到列表末尾**全没了 | 有关（区间起点即 `path`，其余由消费者按自己的顺序取） |

若共用 `deleted` 逐条下发，会同时坏掉两件事：消费者收到的每一条都长得一样，
「删这一个」与「从这里删到末尾」只能靠外部知识去猜；且变更数与历史长度线性相关
——删一条早期消息要发上百条。而挂一个 `cascade: bool` 则让「是哪种删除」变成两个
字段必须一起读才正确（本仓库反复否决的「状态 + 平行标志位」写法）。

拆开之后，`truncated` **只需一条**：前端按 `seq` 顺序取「该节点及其后」，与它是
怎么被删的、被删了几条都无关（`stores/sessions.ts::removeFrom`）。

---

## 3. 地址与节点形状

### 3.1 地址空间

```text
<根>/session/                    会话清单（已有）
<根>/session/<sid>               单个会话（ext = session，点开即聊天工作区）
<根>/session/<sid>/message          转写列表  ← 新增（l 位）
<根>/session/<sid>/message/<mid>    单条消息  ← 新增（ext = message，r 位）
<根>/session/<sid>/subsession[/<sub>]  子会话清单 / 单个子会话（已有）
<根>/session/<sid>/workdir[/<rel>] 工作目录树（已有）
```

`消息` 是会话**本体**（转写），因此排在内置子目录的第一位。

### 3.2 列表项的分工：正文进内容，结构进 attributes

```text
read(<根>/session/<sid>/message/<mid>)
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

**`seq` 不随整表重写而改变。** 这是硬约束，不是优化：`seq` 是**消费者（前端）手里的
顺序锚点**——前端在流式阶段就按本地游标排好了序，整表重写若把既有消息改成新号，
两边就各持一套互不相容的序号，同一条消息会以两种形态排到列表的两个位置。

因此 `replace_messages` 的契约是**只补缺号**（`assign_seq`）：无号项从**相邻的既有序号
向外让位**——开头段整体落在首号之前（往下）、末尾段接在末号之后（往上）、夹缝用间隙；
**既有序号一个都不改**。压缩（前缀重写）的列表是 `[快照, 保留区…]`，快照的槽位序号由
调用方显式给出（`compression::snapshot_slot_seq` = `保留区首条 seq − 1`），使新列表
**本来就单调**，`assign_seq` 于是退化为"只填缺号"。

> 历史病灶：旧实现从 `base`（= 旧列表最大 seq）**向上**无条件填号，于是快照拿到
> `base+1`（成了最大），保留区沿用旧低号却因单调性检查不通过而被逐个改号。
> 实测会话 `mtmae8j2wxam4dhrei` 的保留区因此从 `631..642` 被抬到 `870..881`。

### 3.4 落库回包 = 换号的唯一时机（因此「每条落库消息都必须有出口」）

前端手里的 `seq` 有两个来源：**未落库**的节点由本地游标发号（`nextSeq`，为了在
服务端回包前就能排序），**落库后**由存储发号。正常情况两者相等，回包只是把本地号
换成同一个数字。但这依赖一条不变量：

> **每一条被落库的消息都必须发一次变更（带存储分配的 `seq`）。**

漏一条，那条消息就**永远只有本地号**。本地游标一旦与存储水位错开（截断 / 重试 /
压缩重写之后很常见），同一棵树上就并存两套号——排序随之错位，而只有整份回读
（刷新 / 重开会话）才会恢复。

历史上恰好漏掉的是**用户自己发的那条**：它由 `orchestrator/entry.rs` 直连存储追加
（`append_messages`），不发任何变更。因此新增 `emit_persisted_message`：落库后从存储
**读回权威版本**再发（`updated`，`existed = true`）。为什么读回来发、而不是把入参那条
发出去：`append_messages` 是在临界区内给消息补号的（只改它自己的副本），调用方手里那条
仍然没有号——发它等于把「没有号」写进前端。

前端侧的对账（`stores/sessions.ts`）：同一条消息先有本地号、后又收到存储号且
**两者不等** ⇒ 两套号已分叉。此时**不在流式期间回读**——回读的合并语义允许用存储副本
整条覆盖在途节点，而存储那份可能是半截的（续写前的落库就是这种形态）；改为记下标记，
等会话转空闲再回读一次收敛（`reconcileTranscript`）。

---

## 4. 变更语义（实时链路）

订阅地址：`<根>/session/<sid>/message`（或更上层的 `<根>/session`）。

| 触发 | 变更 | 载荷 | 消费者 |
|---|---|---|---|
| 新增一条消息 | `created` | `node` + `content` | 插入列表项（**零回读**） |
| 正文流式追加 | `appended` | `delta` | 尾部拼接，**不重读** |
| 状态迁移 / 全量替换 | `updated` | `node` + `content` | 就地替换（**零回读**） |
| 落库回包（含用户发言） | `updated` | `node` + `content` | 就地替换成权威版本（**零回读**）；权威 `seq` 由此换入（§3.4） |
| 工具恢复：删旧子节点 | `deleted` | — | 移除**这一项**（与顺序无关） |
| 删除某条及其后 | `truncated` | — | 按 `seq` 取「该节点及其后」移除（**一条**变更） |
| 清空转写 | `deleted` | `path = …/message` | 清空列表 |

**整表重写（L2 语义压缩）也必须发变更。** 压缩把旧历史蒸馏成快照、新列表变成
`[快照, 保留区…]`——这是**列表构成**的改变，不是某一条消息的状态迁移。它由
`emit_transcript_rewritten` 下发：被压掉的逐条 `deleted`（**只删这一个**，与顺序无关）
+ 新首条 `created`。**不用 `truncated`**：那条的语义是"该节点及其后全部没了"，
而这里是从**头部**替换——语义不同，不能借用。

> 历史病灶：压缩一度**完全不发** VDFS 变更（只写存储），前端于是永久停在压缩前的历史，
> 而没有任何机制会纠正它。
>
> 另一处已移除的同类路径：压缩的"输入超限本地机械兜底截断"曾走同一条 `replace_messages`
> ——它同样**必须**发变更（否则被截掉的消息会一直留在前端）。该路径已整体删除：
> 压缩失败不再裁剪历史，见 `node-state-streaming.md` §6 S20.8。

**唯一订阅通道**：`kind = "vdfs"`。会话**任何**面向外部的变化都走它——消息走消息节点，
`title` / `status` / 结局走会话节点（S22 之后连“会话级事件走原通道”这半句也不成立了）。
本小节开头那句“载荷必须挂在事件上”因此仍是成立的命题：**变更**就是那条通道，而载荷始终
随它一起下发（这才是零回读的来源）。

### 4.0 变更**不重放**——因此消费端必须能自愈

`ChangeSubscriptions::notify` 在订阅表为空时**直接返回**（watch 登记是
`subscribeVdfsChanged` 里 fire-and-forget 的异步动作），消费循环也会在 `is_working`
翻转时丢掉手上那一帧。**丢一次不会重来**：前端只做增量收敛，从不整表重拉。

因此消费端不能假设"变更一定到齐"，必须持有一条**可独立判定**的收敛判据。
消息域用的是这一条：**会话节点报"不忙"时，服务端不可能还有节点停在非终态**
（节点补丁恒先于会话状态下发），据此回读收敛——见
`node-state-streaming.md` §8.20 / §6 S20.7。

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

## 5. 写入：**发言**不走 VDFS，**改写与删除**走

### 5.1 发言不是一次文件写入，而是一次动作

`chat/send` 会触发一整轮编排：模型调用 → 工具执行 → 多轮循环 → 流式落库。
把它建模成 `vdfs/write` 会造成两处语义错位：

1. `vdfs/write` 是**同步返回**的存储操作，而发言是长事务；
2. `vdfs/write` 的语义是「把这段内容存到那个地址」，而发言的语义是
   「以这段内容为输入，跑一轮编排」。

因此 `write` 在 `<根>/session/<sid>/message`（**列表本身**）上**明确拒绝**（`Forbidden`），
`create` 意图在**消息节点**上也一律驳回——新增消息即发言。不是静默降级。

### 5.2 但改写与删除是普通的节点操作（2026-09-18 收窄）

本节原先写的是「VDFS 侧**没有**消息的写路径——只有读路径」。**那句话现在不成立**，
它把两件事混成了一件：

| 操作 | 是不是「发言」 | 入口 |
|---|---|---|
| **新增**一条消息 | 是（触发一整轮编排） | 聊天协议（唯一） |
| **改写**既有消息的字段 | **不是**（纯存储改写，不触发任何编排） | `write(<sid>/message/<mid>)` |
| **删**该条及其后 | 不是 | `action(<sid>/message/<mid>, "truncate")` |
| **清空**历史 | 不是 | `action(<sid>/message, "clear")` |

判据是**「触发不触发编排」**，不是「碰不碰消息」。改写一条消息与改会话标题在机制上是
同一类事——都是「把内容存到那个地址」，地址语义完全成立。

> **原先为什么会写错**：把「发言是动作」这条正确结论**外推**成了「消息域整体只读」。
> 而 `write` / `delete` 的三条旧路由（`chat/update_message` / `chat/delete_message` /
> `chat/clear_messages`）当时仍在，本身就说明消息域有写路径——只是走的是另一套机制。
> 审计与改判见 [legacy-route-migration.md](./legacy-route-migration.md) §3。

> **不变的是这一条**：新增消息仍然只有一个入口（聊天协议）；
> 「一条消息只有一个地址」也没有变——改写与删除用的正是那个地址。

---

## 6. 迁移路线

### S16 —— 机制与读路径（已完成）

- `symbio_core::vdfs_provider`：新增 `VDFS_CHANGE_APPENDED`、`VDFS_EXT_MESSAGE`；
  `VdfsChange` 新增 `delta: Option<String>` 与 `VdfsChange::appended(path, delta)`；
- `plugins/vdfs/protocol`：`VdfsChangeEvent` 新增 `delta`；
- `plugins/vdfs/host`：`to_change_event` 透传 `delta`；
- `plugins/vdfs/fs`：`watch` 包装器透传 `delta`（并把路径补成展示口径）；
- `plugins/composite/vdfs`：`watch` 包装器透传 `delta`（并把路径补成树内全口径）；
- `plugins/session/plugin`：`<根>/session/<sid>/message[/<mid>]` 的
  `list` / `stat` / `read`；`write` 明确拒绝。

**S16 完成后即可用**：前端能在 VDFS 里浏览任意会话的转写；
LLM 能通过 `vdfs_read` 在**同一地址空间**里读会话历史——
这是「同一集合（前端与 LLM 恒等）」在会话上的直接兑现。

### S17 —— 变更发射（把既有流式补丁翻译成 `appended`）（已完成）

#### 发射点：不是 `broadcast_message_update`，而是**消费循环**

原计划把翻译挂在 `chat_loop::broadcast_message_update` 上。实施时发现这个判断
**不成立**：那个函数只是**众多**发射点之一。写 `MessageChange::Upsert` 增量帧的
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
| 删除 | 不在本表内：走专用入口（`deleted` = 删一个节点 / `truncated` = 从这里删到末尾） |

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

- 前端实时流：逐帧 `MessageChange::Upsert`；
- VDFS 转写列表：`transcript_of(id)` = 落库转写 ∪ 在途缓冲。

合并规则：同 id 时在途版本胜出（它更新），但 **`seq` 从落库版本继承**——顺序锚点
只由存储在写入时分配，在途副本没有；不继承的话同一条消息会以「有 seq / 无 seq」
两种形态被排到列表的两个位置。

生命周期：每轮开始清空（防残留）→ 本轮落库后清空（`persist_failure` 内、
以及消费循环收尾处）。

#### 一处「看起来是 bug、查下来不是」的记录：`watch` 的路径口径

实现时一度认定 `SessionPlugin::watch` 有缺陷：它忽略被订阅的 `path`，把广播源里
provider 根口径的路径（`abc/message/m1`）原样转发，看起来会被容器再补一遍前缀、
拼成 `session/abc/message/abc/message/m1`。

**核对后确认不是缺陷。** 关键在于容器的回填只补**挂载名（首段）**，不补被订阅的
整条路径——`CompositeVdfs::watch` 把 `(dir, rel)` 拆开：`rel` 传给 provider，
回填时只加 `dir`：

```text
watch("session/abc/message") → dir = "session", rel = "abc/message"
provider 报 "abc/message/m1" → 容器补成 "session/abc/message/m1"  ✓
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
| `stores/sessions.ts` | 读入口改为 VDFS：`readVdfs('<根>/session/<sid>')` 一次拿整份历史 |

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
  这两个类型当时**仍会从总线到达**——进程内的 subagent 宿主需要 `Update`（子会话审批
  透传 + 累积 Assistant 文本，见 `agent/host/subagent.rs`），所以后端当时必须继续发布；
  前端只是不动作。消费端的两个 `case` 已删除（`switch` 无 `default`，空分支是可证明
  的空操作），该约定以注释形式留在 `sessionBusWatcher` 的 `switch` 上方。

  > **S22 收尾**：那批“仍会到达”的帧也是过渡态的最后一截——subagent 宿主与 CLI
  > 都改成订阅 VDFS 变更后，`kind = "session"` 的发布点、常量与消费分支全部删除（见
  > `node-state-streaming.md` §8.10 / §9）。
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
  失效后它失去唯一引用）；`session/get_messages` 后端路由**当时**保留——它还被
  `agent/host/subagent.rs` 当作续会话存在性探针。**2026-09-23 该路由已整体退役**
  （探针改走进程内 VDFS 纯接口 `stat`，见本文 §1 的 S26 注记）。
- **删除也必须发变更**：`resume.rs` 的 `MessageChange::Remove` 由消费循环转成
  VDFS `deleted`（消费循环是全部补丁的唯一收口，删除帧同样经过它）——这是
  **逐节点**删除（删一棵子树，后面的消息留着）；
  `chat/clear_messages` / `chat/update_message` 两条**前端自发起、已在本地收敛**的
  路由各有一个「只发变更、不发前端帧」的入口；`chat/delete_message` 的入口是
  `emit_transcript_truncated`，发的是 **`truncated`**——因为它的语义是「目标及其后
  全部」，一条就够，不必按被删节点数发 N 条。
  漏掉任何一条，VDFS 视图都会残留一个已不存在的节点且永不纠正。

---

### S23 —— 协议去补丁化（`Upsert` = 完整快照，追加 = 显式帧）（已完成）

S17 建立的「合并 + 翻译」机制在验证期暴露了它的结构性缺陷：**补丁语义本身就是
缺陷**。`Upsert` 帧里装的是残缺 `ChatMessage`（content 只有增量、status 只带状态），
消费端要靠四套各自为政的合并实现猜发射端意图——schema 层 `ChatMessage::apply_patch`、
编排层 `merge_message_patch`、存储层 `update_messages` 的合并、前端
`mergeMessagePatch`。content 是追加还是替换、meta 是合并还是清空，全靠 role/type
隐式推断；「None 是不碰还是清空」这类歧义直接造成过线上缺陷（`name` 键冲突、
幽灵节点伪造）。四套语义只要一套漂移，表现就是**静默**的显示错乱。

**裁决：通道上只有显式操作，没有补丁。** 每个帧变元只有一种含义，接收端只做
帧面动作、不做解释：

| 帧 | 含义 | 消费动作 |
|---|---|---|
| `Upsert { message }` | **完整消息快照** | 按 id 整条替换（不存在则创建） |
| `Append { message_id, delta }` | 往已存在消息的 Text 尾部追加 | 缓冲追加 → VDFS `appended` 窄载荷 |
| `Remove` / `Warn` | 删除 / 会话级告警 | （不变） |

发射端纪律：**读-改-写发生在状态所有者处**。状态迁移帧从权威转写取完整副本
→ 应用终态 → 整条广播（`emit_parent_finalized` / `emit_tool_running` /
`finalize_assistant_turn`）；正文流式首帧是完整快照、后续 delta 走 `Append`
窄帧（O(delta)，与 S17 的流量论证一致）。merge 语义在**存储层**也不复存在：
`update_messages` 整个删除（父节点终态随 `persist_messages` 落库，镜像同步是
整条替换），`ChatMessage::apply_patch`、`merge_message_patch` 一并删除。

协议违例（对未知 id 追加 / 终态帧找不到父节点）现在**当场报错丢弃**，
不再静默造幽灵节点或假装无事发生——暴露问题，不掩盖问题。

附带修正（同一错误假设的产物，一并清除）：

- `converge_inflight` 删除「扫存储」分支——持久层写入不变量保证存储只有终态，
  「存储里的在途节点」不存在（历史分支还把 status=None 的普通消息误判为在途）；
- 工具结果子节点的广播直接用落库的 `tool_msg`（截断标记 / 存档路径不再只活在存储里）；
- 协议失败（模型没给出调用 id）的兜底父节点改为**完整** ToolCall 终态节点并补入
  转写落库——结果子节点不再悬空。

---

## 7. 不变量（迁移全程必须成立）

1. **顺序锚点仍是 `seq`**——VDFS 列表序、前端消息表、LLM 上下文三者同源；
2. **一条消息只有一个地址**：`<根>/session/<sid>/message/<mid>`；
3. **正文只有一个位置**：节点内容。`attributes` 只放结构，不放正文；
4. **前端与 LLM 恒等**：同一地址、同一份数据、同一组访问位；
5. **变更只有一个通道**：`kind = "vdfs"`；
6. **追加是增量语义**：`appended` 必须携带 `delta`，消费者不得借此触发重读；
7. **新增消息的入口唯一**：发言只有聊天协议一处（`create` 意图在消息路径上被驳回）。
   **改写与删除**是普通 VDFS 节点操作（`write` / `action`），不触发编排——见 §5.2；
8. **补丁与变更同生共死**：任何送达前端的消息补丁都经
   `SessionPlugin::emit_message_patch`，因此必然产生对应的 VDFS 变更。
   出口只有一个——新增发射路径时若绕开它，VDFS 视图会**永久**落后，
   而两条链路互不校验，不会有人发现；
9. **在途消息可读**：`list` / `stat` / `read` 看到的转写 = 落库 ∪ 在途。
   在途缓冲与消费循环收集的是**同一个 `Arc`**——不是两份需要同步的拷贝；
10. **变更判据不重算**：`appended` 与否由**帧面**决定——`Append` 帧译为
    `appended`，`Upsert` 帧译为 `created` / `updated`（按在途缓冲里是否已存在），
    不在翻译层另判一次。（S23 之前由 `merge_message_patch` 的返回值决定，
    该函数已随补丁语义一并删除。）
11. **删除也发变更**：消息级删除（`resume` 的 `Delete` 帧、`action("truncate")` /
    `action("clear")`）同样产生 VDFS 变更，否则列表会残留已不存在的节点。
12. **`deleted` 有歧义才配自己的值**：`deleted` = 「**这一个**节点没了」（与顺序无关）；
    `truncated` = 「该节点及其之后全部没了」。不得用 `deleted` 逐条下发后者
    ——消费者无从分辨，且变更数与历史长度线性相关；也不得在 `deleted` 上挂
    `cascade` 布尔（「是哪种删除」变成两个字段必须一起读）。
    **清空**（`action("clear")`）**不需要**新值：它落在**列表目录**这个地址上，
    `deleted` 在此处只有一种读法（目录没了 ⇒ 条目都没了），地址已把语义定死。
13. **截断的区间由消费者算**：`truncated` 只给**区间起点**（即 `path`），
    消费者按自己的 `seq` 顺序取「该节点及其后」。前端与后端同源排序，
    因此这条计算不需要任何额外载荷，也不怕通知漏发。
14. **热路径窄、冷路径全**：`appended` 只带 `delta`；`created` / `updated` 带
    `node` + `content`。前者逐帧发，后者每轮几次——载荷宽度按频率分配。
15. **同路径变更串行**：转写消费端按路径串成顺序链，回读与增量在结构上不竞争。

---

## 8. 取舍与风险

| 取舍 | 说明 |
|---|---|
| 组合节点退化为 JSON 视图 | `turn` / `tool_call` 无正文；强行扁平化会丢失层级。JSON 视图是诚实的降级，`attributes.type` 让渲染器知道该按什么渲染 |
| 图片消息只暴露文本部分 | `MessageContent::Parts` 里的 `image_url` 不在正文中。需要时再走 `binary` 内容通道（本期不做） |
| `appended` 增加了协议面 | 但它是通用能力（任何追加型数据都需要），不是聊天专用后门；且与 `updated` 的区分是**流量与语义双重必要** |
| 变更信封多了两个可选字段 | `node` / `content` 使 `created` / `updated` **零回读**。若不带，迁移就是净增几十次 IPC/轮次——统一机制的代价不该由热路径承担。字段可选，不填时消费者回退回读 |
| `message_status` 把 `completed` 与「未标注」都归为 `active` | VDFS 节点只有一套状态词汇，不为场景再造一套。消费端把 `active` 读作 `completed`——两者在渲染上一致（都不是进行中），因此是无害的有损映射 |
| 会话叶子同时是「文档」与「内部区段之父」 | `<根>/session/<sid>` 是叶子（`rw`，内容是整份会话 JSON），其下又挂 `消息` / `子会话` / `工作目录`。这不是矛盾：**叶子是会话本体，区段是它的视图**。好处是整份历史一次 `read` 拿到，省掉专用协议 |
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

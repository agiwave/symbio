# 会话旧路由审计 —— 哪些可以删、哪些值得迁到 VDFS

> 前置阅读：[vdfs-session-messages.md](./vdfs-session-messages.md)（转写已 VDFS 化）、
> [vdfs.md](../../../../../docs/design/vdfs.md)（机制规范）。
>
> 本文回答一个问题：`plugin.rs::route` 路由表里剩下的会话路由，**还有哪些是旧机制的残留**——
> 哪些可以整体删除，哪些值得迁到 VDFS，哪些必须保留（以及为什么必须保留）。
>
> 判据只有两条，都可核对：**① 有没有真实消费方**（全仓 grep，含 CLI / gateway / 内部直调）；
> **② VDFS 侧有没有等价能力**（读 provider 的 `impl VdfsProvider`，不看文档承诺）。
>
> **一条前提**：路由存在 ≠ 它的实现是死的。`invoke_*` 方法常被**内部直调**
> （不经路由），因此「删路由」与「删实现」必须分开判定。本文每档都写明删到哪一层。

> **现状校正（批次 E + G，2026-09-22）**：本文写于迁移**之前**，§3.3 的「变更」列
> 有四处已被后续改动推翻，读那张表时以本条为准：
>
> | 本文写法 | 实际 |
> |---|---|
> | `updated` 带 `node` + `content`（零回读） | 变更**不带任何载荷**（`VdfsChange` 只有 `path` + `change`）；资源变更一律防抖重拉 |
> | 截断 → **起始消息**上 `truncated` | 区间删除**一条 VDFS 变更都不发**；实时通知走转写流的 `status = removed` 帧 |
> | 清空 → **列表目录**上 `deleted` | 同上，**不发变更**；回执里的 `deleted_ids` 才是权威列表 |
> | 「`truncated` 已在 `VDFS_CHANGE_*` 与前端消费端落地」（§3.2） | `truncated` 与 `renamed` / `appended` 三个取值**已整体删除**（无生产性生产者） |
>
> 结论层面本文仍然有效：**区间删除走 `action` 而不是 `delete`** 的判断（语义 + 回执两点）
> 完全成立，且这正是「不发变更」的依据——见 `symbio_core::vdfs_provider` 的
> `VDFS_ACTION_TRUNCATE` 文档与 `docs/design/vdfs.md` 的词汇表。

---

## 1. 结论（三档清单）

路由表共 11 条（`plugin.rs:474-492`）。按上述两条判据分档：

| 档 | 路由 | 判定 | 依据 |
|---|---|---|---|
| **A ✅已删** | `session/append` | **整条链路死**（路由 + 实现 + schema） | 全仓仅 `plugin.rs:478` 一处引用。实现看似被 `entry.rs:232` 内部调用，但那处是**绕路由的内部调用**（为一次纯数据操作搭 invoke 信封），改直连后实现一并变死——见 §2.1 |
| **A ✅已删** | `session/open` | **路由 + 实现 + schema 全死** | 路由无调用方；`open_session_handle` 的真实调用方 `entry.rs:291` 是**直连 Rust 方法**。CLI 的通道来自 `event_bus/subscribe`（`cli/src/client.rs:113-121`），**不是** `session/open`——`client.rs:12` 那句注释是过期的 |
| **B ✅已删** | `session/chat/update_message` | → `vdfs/write(<sid>/消息/<mid>)` | 唯一消费方是前端（`stores/sessions.ts:836`）；**纯存储改写，不触发编排** |
| **B ✅已删** | `session/chat/delete_message` | → `vdfs/action(<sid>/消息/<mid>, "truncate")` | 同上（`stores/sessions.ts:816`）；`action` 的返回载荷**能保住 `deleted_ids` 回执** |
| **B ✅已删** | `session/chat/clear_messages` | → `vdfs/action(<sid>/消息, "clear")` | 同上（`stores/sessions.ts:855`）；与截断同走**动作**——同一区段的删除只有一种入口形态（见 §3.3） |
| **B ✅已删** | `session/get_messages` | → **进程内 VDFS 纯接口探测**（`stat("<挂载名>/<sid>")`） | 唯一消费方 `agent/host/subagent.rs::validate_subsession_exists`。当初判「不改」的两条硬化理由（后端拿不到 root provider / 线路信封留在 vdfs 插件内）**已被现成 API 化解**——见 §3.4.1（2026-09-23） |
| ~~**B 待定**~~ **已下线** | `session/update` | ~~迁 `vdfs/write` 需先改 CLI~~ **理由已消失** | 唯一消费方 `cli/src/client.rs`。前两轮判「**不是纯收益**」（要动 `--session <ID>` 语义或扩 VDFS create，且 CLI 会话 id 会变格式）——**2026-09-23 改判并整条下线**：那条「扩 VDFS create」被证明不是扩张而是**对齐**（会话 provider 是唯一违反「有名字时 id 来自地址」的 provider），于是 `--session <ID>` 语义与 id 格式都**不必动**。见 §3.5 |
| **C 保留** | `session/chat/abort` | 不可迁 | 控制信号，不是数据变更 |
| **C 保留** | `session/heartbeat/trigger` | 不可迁 | 心跳调度入口，不是会话 CRUD |
| **D 可评估** | `session/chat/send` | **§5 的否决只覆盖 `vdfs/write`，不覆盖 `vdfs/action`** | 见 §4——三条可核对的反证表明它协议层就是触发器 |
| ~~**C 保留**~~ **已下线** | `session/options/list` | ~~不可迁~~ **不是「不可迁」，是「不该存在」** | 当初判 C 档的理由（「级联选项收集，不是会话 CRUD」）回答的是「能不能迁」，没问「该不该有第二条下发通道」。**2026-09-23 改判并整条下线**：选项 = 会话配置表单的字段，定义随 `node.schema` 下发、值走 `attributes.metadata`、写走 `vdfs/write`。见 §5 |

**A 档 + B 档「可迁」四项均已落地**（本文所记的那批路由 11 → 5 条；其后
`heartbeat/trigger` 又按「能力整体取消」退役、`stream` 随转写流新增，
`options/list` 与 `update` 于 2026-09-23 先后下线）。会话域的**当前**三条是：
`chat/send`（D，待评估）· `chat/abort` · `stream`。
`get_messages` 于 2026-09-23 按 §3.4 的「正确归宿」退役——但走的是**现成的纯接口**
（不占协议、不依赖线路信封），见 §3.4.1。

> **S26 后记（ADR-025，2026-09-23）：`stream` 也退役，会话域回到两条**
> （`chat/send` · `chat/abort`）。它是**唯一一条不因「迁到 VDFS」而退役、而是因为
> 「它要解决的问题不存在」而退役**的路由：会话实时面**迁回 VDFS 变更**
> （`updated` + `delta`），而 `stream` 当初是为「VDFS 变更频道承载不了实时面」建的
> ——那个判断**把「数据的属性」当成了「传输的属性」**（顺序是节点属性、追加是
> `updated` 上的字段），前提不成立。**至此会话域的路由表里没有一条「数据 CRUD」**，
> 与本文的结论一致。

**消息与会话的增删改查已全部经 VDFS 地址完成**——本轮的实质目标达成：
`update` / `delete` / `clear` 三条走 VDFS 后，消息域只剩**读**一条（`vdfs/read`，早已 VDFS 化）
与**发言**一条（`chat/send`，动作语义）。剩下的 3 条里没有一条是「消息或会话的 CRUD」。

---

## 2. A 档：可直接删除（死路由）—— ✅ 已完成

> 本节记录**判定过程**，不是待办。两条路由已于 2026-09-18 落地删除，
> 删除面与验证见 §7 / §8.1。

### 2.1 `session/append` —— **整条链路可删**（路由 + 实现 + schema）

```rust
// plugin.rs:478
"append" => self.invoke_append(ctx.clone()).await?,
```

它是「往会话里追加消息」的直通口，而发言早就收敛到 `chat/send`（触发编排）。
转写读路径 VDFS 化（S16–S19）之后，它连最后一个潜在用途也失去了——全仓引用只有路由臂本身。

**唯一的「实现还活着」的理由是一处绕路由的内部调用**，而那一处本身就该改：

```rust
// entry.rs:227-245 —— 现在
let append_req = session_append::Request { session_id: sid_spawn.clone(), messages: vec![msg.clone()] };
let _ = ctx_spawn.set_payload(append_req);              // ① 搭信封
if let Err(e) = this_spawn.invoke_append(ctx_spawn.clone()).await { ... }  // ② 调路由处理函数
```

`invoke_append`（`handlers.rs:30`）的全部内容就是「解载荷 → `open_chat_session` →
`append_messages`」。也就是说这里**为一次纯数据操作构造了一个 invoke 信封**
（fork ctx + `set_payload` + 走一遍路由方法），只为拿到一个引擎句柄——而**同一个会话引擎
在这一轮的第 290 行还会被再构造一次**（`open_session_handle`，用于交付 `SESSION_HANDLE`）。

`PersistentChatSession::new` 是纯结构构造（`chat_session.rs:232-243`，无 I/O），
所以重复构造本身无害；**真正多余的是信封**。改成直连：

```rust
// 一次构造，两处使用：本轮落库 + 交付 chat_ctx
let engine = this_spawn.open_session_handle(Some(sid_spawn.clone())).await;
// … append 用 engine 里的句柄直调 append_messages
// … 第 290 行的 SESSION_HANDLE 交付复用同一个 engine
```

**行为等价性**（已核对，不是推测）：

- 对**所有有生产者的 id**（真实 id）两条路径产出同一个 `Arc<dyn ChatSession>`
  ——`open_chat_session`（`plugin.rs:390-400`）就是 `open_session_handle` 的持久分支
  （`handlers.rs:288-292`），逐字相同。
- 两条失败路径语义不同，改造时必须分别保留：append 失败 → 报错并**中止本轮**
  （`entry.rs:237-241`）；句柄交付失败 → 只 warn、继续跑（model 侧回退内存会话，
  `entry.rs:300-302`）。**hoist 之后要按用途分派，不能合并成一个错误分支。**

**删除面**：路由臂 + `invoke_append`（`handlers.rs:30`）+ `schemas/session/session_append.rs`
（改造后 `entry.rs:229` 的最后一处引用消失）。
`open_session_handle` / `open_chat_session` **保留**（各有其它真实调用方）。

> **顺带发现（不是本次迁移引入的）**：`open_session_handle` 的 `_t_` 前缀分支
> （`handlers.rs:289`，判定「内存 ephemeral 会话」）在全仓**没有任何生产者**
> ——`_t_` 只在这一个地方被引用。它是一条没有上游的约定，可以评估删除。
> 注意它与 `open_chat_session` 的语义冲突（后者对 `_t_` 也返回**持久**会话），
> 但因为没有生产者，这个冲突目前不可达。

### 2.2 `session/open` —— 路由 + 实现 + schema 全死

```rust
// plugin.rs:479
"open" => return self.invoke_open(ctx.clone()).await,
```

`invoke_open`（`handlers.rs:303`）返回 `PluginPayload::Native(ChatSessionHandle)` ——
一个**进程内句柄**。它当初的用途是「让 model/chat 拿到会话引擎」，而那条链路已经改掉了：

- `orchestrator/entry.rs:284-295` 向 `chat_ctx` **直接塞** `SESSION_HANDLE`（同一个 key），
  拿到句柄走的是 `open_session_handle` 这个 **Rust 方法**（`entry.rs:291`），不经过路由；
- `chat_loop/io.rs:44-58` 从 `ctx.get(SESSION_HANDLE)` 读，句柄缺失时兜底内存会话。

**句柄交付这条路还在，但它已经不走路由了。** 路由臂是改架构时留下的空壳。

CLI 侧的注释（`cli/src/client.rs:12`）说「需先 `session/open` 拿通道」，**与代码不符**：
`start()` 实际是 `route("event_bus/subscribe")` 拿 `PluginPayload::Session(c)`（`client.rs:113-121`）。
同句也出现在 `cli/docs/architecture.md:32`。两处都是过期描述，**不是消费方**。

**删除面**：路由臂 + `invoke_open`（`handlers.rs:303`）+ `schemas/session/session_open.rs`
（全仓仅被 `invoke_open` 引用，见 `schemas/session/mod.rs:8`）。
`open_session_handle`（`handlers.rs:279`）**保留**——它是 `entry.rs:291` 的真实依赖。

> 顺带修两处过期注释：`cli/src/client.rs:12`、`cli/docs/architecture.md:32`。

---

## 3. B 档：值得迁到 VDFS

### 3.1 为什么「转写只读」这条规则被过度外推了

`vdfs-session-messages.md` §5 的论证是**正确的，但只覆盖了「发言」**：

> 发言不是一次文件写入，而是一次动作。`chat/send` 会触发一整轮编排……

这个论证成立。但 §5 由此得出的结论（不变量 7：「消息写入只有聊天协议一处，VDFS 侧显式拒绝」）
把它外推到了**整个消息域**——而 `update` / `delete` / `clear` 三条路由**完全不触发编排**：

| 操作 | 触发编排？ | 是「动作」还是「存储」？ |
|---|---|---|
| 发言（`chat/send`） | ✅ 模型调用 → 工具执行 → 多轮循环 | **动作** |
| 编辑某条消息的正文 | ❌ | **存储** |
| 删除某条及其后 | ❌ | **存储** |
| 清空历史 | ❌ | **存储** |

后三条就是**对已有节点的一次改写/删除**，与「改会话标题」在机制上是同一类事——
而「改会话标题」早就迁到 `vdfs/write` 了（`services/session.ts:203-212`）。

**当前的不对称因此是明显的**：消息**读**走 VDFS（`<根>/session/<sid>/消息/<mid>`），
消息**写**走三条专用路由——同一个地址，两个方向两套协议。这正是「同一件事的两条路径」。

### 3.2 迁移的落地成本极低（因为 VDFS 侧只差两个分支）

迁移不是「新造一套写路径」。以下东西**已经存在**：

| 已有件 | 位置 |
|---|---|
| 消息地址 `message_path(sid, mid)` | `plugin/nodes.rs:304` |
| 消息节点 / 正文 / 定位 `message_node` / `message_text` / `message_of` | `nodes.rs:526` / `:594` |
| 变更发射 `emit_transcript_truncated` / `emit_transcript_cleared` / `emit_message_updated` | `plugin.rs:142` / `:150` / `:158` |
| 截断变更词汇 `truncated` | 已在 `VDFS_CHANGE_*` 与前端消费端落地 |
| 变更订阅表 `change_subs` | `plugin.rs:96` |

**只差 `impl VdfsProvider` 里的两个分支**：`write` 的 `Messages { mid: Some(_) }` 与
`delete` 的 `Messages` —— 目前它们落到拒绝臂（`vdfs_provider.rs:336-340` / `:471-473`）。

### 3.3 地址与语义映射（落地形态）

| 旧路由 | 新入口 | 语义 | 变更 | 回执 |
|---|---|---|---|---|
| `chat/update_message` | `write(<sid>/消息/<mid>)`，body = 消息字段补丁 | 按**地址**定位，**只覆盖提供的字段**（与 `invoke_update_message` 逐字一致） | `updated`（带 `node` + `content`，零回读） | `{path, created}` |
| `chat/delete_message` | `action(<sid>/消息/<mid>, "truncate")` | 「该节点及其之后全部没了」 | **起始消息**上 `truncated`（只给区间起点） | **`VdfsActionResult` 可带载荷** → 保住 `deleted_ids` |
| `chat/clear_messages` | `action(<sid>/消息, "clear")` | 列表清空，会话本体 / 元数据 / 工作目录保留 | **列表目录**上 `deleted` | `VdfsActionResult`（无载荷） |

补丁的 `id` 由**地址**补齐——`{"content":"…"}` 是合法补丁。`ChatMessage::id` 在结构里
是必填字段（它同时是存储层主键），不在反序列化前补上，不带 `id` 的补丁会**先一步**被拒，
于是「补丁是字段子集、未提供的保持不变」这条承诺在 `id` 上就成了假的
（首轮回归测试即抓到，见 §7）。补齐之后 `patch_message` 的「id 与地址不符即报错」
依然成立：**地址是消息身份的唯一权威**，缺 `id` 是省略、给了别的 `id` 才是错误。

**为什么截断用 `action` 而不是 `delete`**（这一条同时解掉两个阻抗）：

1. **语义**：VDFS 的 `delete` 语义是「**这一个**节点没了」（对应 `deleted`），而会话的消息删除是
   「**从这里到末尾**全没了」（对应 `truncated`）。拿 `delete` 表达后者，会让「删一个节点却删掉了
   它后面所有」成为一条**没人能预期的默认行为**。文档 §4 已明确否决过用 `cascade: bool` 表达这种差异。
2. **回执**：前端 `deleteMessage` 用后端返回的权威 `deleted_ids` 做幂等对齐
   （`stores/sessions.ts:818-819`：「本地推算若因锚点缺失等原因偏窄，这里补齐」），
   而 `vdfs/delete` 只回 `VdfsDeleteResponse { path }`（`host.rs:378`）。
   **`action` 的返回类型 `VdfsActionResult` 可以带载荷**，所以走 action 能把 `deleted_ids`
   原样带回来——这是 `vdfs/delete` 方案做不到的。

`action` 恰恰是为这种情况准备的：它是 **provider 自持的动词**，VDFS 只透传
`(节点路径, 动作标识, 载荷)`、不解释语义（`symbio_core/vdfs_provider.rs:1293-1310`）。
「从这里截断」是一个动作，不是一个删除。前端已有 `runVdfsAction` 封装（`services/vdfs.ts:190`）。

**为什么清空也跟着走 `action`**（原计划是 `delete(<sid>/消息, recursive)`）：

清空**本可以**走 `delete`——`deleted` 落在**列表目录**这个地址上只有一种读法
（目录没了 ⇒ 里面的条目都没了），语义没有歧义，也不必为它新造一个变更值。这正是不设
`cleared` 变更值的原因：`truncated` 需要自己的值，是因为 `deleted` 表达不了
「从这里删到末尾」；清空没有这个歧义，**地址已经把它定死了**。

改走动作是为了让**同一个区段的删除只有一种入口形态**：截断必须走动作（语义 + 回执），
若清空走 `delete`，使用者就得记「哪种删除走哪个入口」——而两者本就是同一段代码的
两个相邻分支。代价是回执从 `{path}` 变成 `VdfsActionResult`，但清空不需要载荷，无损失。

### 3.4 `session/get_messages` —— **已于 2026-09-23 退役**（本节保留当时的推导与改判）

唯一消费方 `agent/host/subagent.rs:358-383` 用它做**子会话存在性校验**：

```rust
gm_ctx.set(PATH, "session/get_messages".to_string());
// … 判 !r.messages.is_empty()
```

它想知道的其实只是「这个会话在不在、有没有消息」。**看起来**该换 `vdfs/stat`
（`VdfsNode.attributes.message_count` 就是现成的判据，`nodes.rs:141-143`），
而且能省一次全量读。**但本轮实测后判定：不做。** 三条理由，按硬度排序：

> **后记（S22，2026-09-20）：先例已经出现了，但本节结论不变。**
> 子智能体转播与 CLI 现在都在**进程内**消费 VDFS——它们读的是**变更**（`vdfs/watch` 登记 +
> `kind = "vdfs"` 帧 + `symbio_core::vdfs_provider::VdfsChange`，全是 core 类型），
> 而不是下面说的那种「`vdfs/*` 请求响应的线路信封」。故①的“没有先例”已成历史，而由它推出的
> 两条代价（插件间编译期依赖 / 硬编码路由串）**一条都没变**——这正是当时“不做”的真正理由。

**① 后端没有「进程内消费 VDFS」的先例，且它被架构刻意挡住。**

VDFS 的**线路信封**（`vdfs/*` 请求响应 + 协议路径常量 `VDFS_STAT` / `VdfsPathRequest`）
定义在 `plugins/vdfs/protocol.rs`，**不在 core**。`symbio_core::vdfs/mod.rs` 的模块边界
写得明确：

> `symbio_core/vdfs_provider.rs` 纯接口 ｜ `plugins/vdfs/protocol.rs` 线路信封
> **core 只暴露纯接口，线上形状与访问层都在 vdfs 插件内部**

因此 `subagent.rs` 想走 VDFS 只有两条路，都更差：

- **依赖 vdfs 插件的线路类型** ⇒ 新增一条**插件 → 插件**的编译期依赖（agent 依赖 vdfs 的
  线上形状）。而它现在依赖的 `session_update` / `session_chat` 都在
  `symbio_core::schemas::session`——**core 里的共享形状**，性质完全不同。
  用一条插件间耦合换掉一条 core 共享形状的依赖，是**倒退**。
- **硬编码路由串**（`"vdfs/stat"` + `json!({"path": …})`）⇒ 违反仓库「不硬编码路由」的约定，
  且丢掉类型检查。

**② 走纯接口也不行：后端拿不到 root provider。**

`get_vdfs_root()` 在 `DefaultToolVisitor` 上（`symbio_core/tools.rs:132-135`），而它是
**每次 `collect_capabilities` 现造的**，不是全局单例。`subagent.rs` 在工具执行期运行，
手上只有 `parent: Arc<dyn Plugin>`（`agent/host/plugin.rs:141` 的 `ctx.parent()`）。
要让它走纯接口，得**新造一个全局 accessor**——为一个调用点新增一项 core 机制。

**③ 收益不抵：它换掉的是 1 条路由 + 1 个 schema。**

`session/get_messages` 是**会话域最后一条读路由**，删掉它确实干净。但代价是上面两条
架构级选择之一。对比 B 档其余三项（`update` / `delete` / `clear`）：那三项的 VDFS 侧
**只差 provider 的一个分支**，地址、节点、变更词汇全都现成，所以是纯收益。这一项不是。

**⇒ 当时的结论：`session/get_messages` 保留，gateway 只读白名单里的它一并保留。**
（该结论随后被推翻——不是推翻理由，而是理由已被现成 API 化解，见下。）

### 3.4.1 2026-09-23 落地：不必做那个「独立架构决定」，用现成的纯接口

当初判「不做」的每一条硬化理由，今天都能**直接核对代码**地化解——既不需要下沉线路信封，
也不需要新造 core 门面：

| 当时的顾虑 | 实际 |
|---|---|
| 后端拿不到 root provider（`get_vdfs_root` 挂在「每次 collect 现造」的 visitor 上） | 容器自己就是 `Plugin`，`Plugin::get_vfs_provider()`（core 的查询接口）**直接给出它的 VDFS 视图**（`CompositeVdfs`）——纯接口，与 root 槽位无关 |
| 走 VDFS 就得依赖 `plugins/vdfs/protocol.rs` 的线路信封（插件 → 插件编译期依赖）或硬编码路由串 | 探测**不经线路**：`VdfsProvider::stat(&vdfs_context(ctx), "session/<sid>")`。用到的 `vdfs_context` / `VdfsProvider` / `VdfsNode` 全在 `symbio_core` |
| 要读回整份历史（专用路由的副作用） | `stat` 只回节点：`attributes.message_count` 就是「有没有消息」的现成判据——顺带省掉一次全量读 |

作用域正确性**由机制保证**：容器把子插件按**实例名**列为 VDFS 子目录
（`composite/vdfs.rs::children_of`），而 `route` 分发用的也是同一个键
（`composite.rs::parse_path` + `instances.get(name)`），因此 `session/<sid>` 命中的
正是 `session/chat/send` 将要写入的那个会话插件实例——子智能体分形子树里同样成立。

于是 `agent/host/subagent.rs::validate_subsession_exists` 从「走一条专用协议」变成
「一次纯接口 `stat`」（只把 `NotFound` 读作「不存在」，其余错误照实上抛）。随之删除：
路由臂 `get_messages`、`invoke_get_messages`、`session_get_messages.rs` schema 与
`SESSION_GET_MESSAGES` 常量，以及 gateway 只读白名单里的那一项。
防回归沿用既有形态：`handlers.test.rs::migrated_session_routes_stay_retired` 里加了这一条。

**「正确归宿」仍未做，但现在只是可选优化**：把线路信封下沉 `symbio_core::schemas::vdfs`、
或给 core 一个统一的 VDFS 访问门面，收益是「所有消费方用同一套门面」——
而不再是「某一条路由能不能删」。为一个调用点提前做那个决定，仍然是把小问题换成大问题。

### 3.5 `session/update` —— 前两轮判「不是纯收益」，2026-09-23 **改判并整条下线**

`handlers.rs` 写的保留理由是：

> CLI 需要**客户端指定会话 id**（`cli/src/client.rs` 自己 `gen_id` 后 upsert），
> 而 VDFS 新建会话是 provider 生成 id。

核对 CLI 实际用法：`ensure_session` 用 `gen_id("cli")` 生成 id，然后调
`session/update` **upsert**。它承担两件事：新建（id 客户端生成）、
改元数据（`/workdir`、`/provider`、`/session <ID>` 后重新落库）。

**前两轮的理论迁移方式**是「新建走 `vdfs/write(root, {create:true})` 并用**返回的** id，
已有会话走 `vdfs/write(<sid>)`」。于是判定为**不是纯收益**：它要动 `--session <ID>` 的
语义（用户给的名字不再生效）或扩 VDFS create，而且 CLI 会话 id 会从 `cli<时间戳>`
变成 provider 的短 GUID——**用户可见的行为变化**，不只是内部重构。收益（消灭最后一条
会话级写路由 + 修掉「`/session <拼错的 ID>` 静默造孤儿会话」）与代价都真实，取舍权交给用户。

> **2026-09-23 补记（会话选项 schema 化之后）**：本路由的**前端调用方已归零**。
> 旧形态下状态型选项经 `SESSION_STATE_ENDPOINT = worker/session/update` 落库，
> 那是它在前端的唯一用途；选项改用 `vdfs/write(<根>/session/<id>, {"metadata": …})`
> 之后，`tauri/src/services/session.ts::updateSession` 已直接走 VDFS 写通道。
> **当前唯一消费方只剩 CLI**。

#### 2026-09-23 改判：那条「扩 VDFS create」不是扩张，是**对齐**

用户拍板退役后重新核对，发现前两轮把两条路**混为一谈**了——它们都叫「扩 VDFS create」，
但性质完全不同：

| 备选 | 是什么 | 结论 |
|---|---|---|
| **给 VDFS 的 create 请求加一个 `id` 字段** | 真·扩张协议：多一个「id 归使用方」的旁路 | 否决——与「id 是存储细节」的原则直接冲突，且只有会话需要 |
| **让会话 provider 遵守 VDFS 已有的具名新建语义** | 合规化：什么都不加，只是不再例外 | **采纳** |

依据是仓库**已有的**两份规范，都不是新发明的规则：

- `symbio_core/vdfs_provider.rs::VdfsProvider::write` 的「两种目标形态」表：
  **具名节点 + 不存在 ⇒ 就地创建**；只有**目录自身**才「名字由 provider 生成」。
  同一份文档还写着「⚠️ 具名 + 目标不存在时不要一律报 `NotFound`：使用方要的是
  『给了名字就写得进去』——配置型资源的地址**就是它的身份**（`model` / `mcp` / `skill`
  皆如此）」。
- `providers/vdfs_service/entry.rs::id_of` / `auto_id` 的注释：
  「**有名字**时 id 来自地址（使用方给），**没名字**时 id 由这里生成（provider 给）」。

**会话 provider 是唯一例外**：它无论有没有名字都自己生成 id，把名字只当标题
（`title_from_new_path`），于是「写到的地址」与「建出来的地址」是两个地方。
对齐之后：

- `vdfs/write(<根>/session/<id>, {create:true, metadata})` —— **不存在则就地创建、
  已存在则浅合并 metadata**，一次调用即 upsert，正是旧路由的全部职责；
- 具名目标的名字是**身份**，不再兼作标题（拿 `cli18f3a2` 当标题只会污染侧栏）；
  标题仍由 `display_title` 从首条消息派生，或由调用方显式给 `title`。

**于是代价全部消失**：`--session <ID>` 语义不变（id 仍由客户端指定）、
CLI 会话 id 格式不变（仍是 `cli<时间戳>`）、e2e 里 8 个按 id 读落盘会话的用例一行不改。
唯一的行为变化是「写具名地址 + `create`」在目标已存在时由「再造一个」变成「覆盖」——
那是 `create` 位的定义本身（只回答「不存在时怎么办」），仓库内没有依赖旧行为的调用方
（前端新建一律写挂载根、不给名字）。

**遗留**：`/session <拼错的 ID>` 仍是**静默创建**（具名新建合法），未改成报 `NotFound`
——那属于 REPL 的交互取舍，不是路由退役的副作用；要改是独立决定。

**删除面**（本轮落地）：`plugin.rs` 的路由臂、`handlers.rs::invoke_update`、
`SESSION_UPDATE` 常量、`schemas/session/session_update.rs` schema，
以及两处调用点改为 VDFS（CLI 走 `vdfs/write`，子会话登记走进程内纯接口
`register_subsession`）。防回归沿用既有形态：
`handlers.test.rs::migrated_session_routes_stay_retired` 里加了 `update` 一条。

---

## 4. D 档：`chat/send` —— §5 的否决只覆盖 `vdfs/write`

**上一轮已经核实过，本轮复核仍然成立。** §5 的标题是「写入为什么不走 VDFS」，
但正文论证的其实只是「**为什么不走 `vdfs/write`**」——两者不是一回事，
因为 `vdfs/action` 的存在使前者成立而后者不成立。三条可核对的反证：

1. **协议层它就是触发器，不是长事务**：`handle_chat_send_oneoff` 的文档注释自己写着
   「**响应立刻返回**，流式事件由 bus 推送」（`entry.rs:98`），实际编排在 `tokio::spawn` 里
   （`entry.rs:203`）。§5 说的「长事务」指的是**编排**长，不是**响应**长。
2. **前端不使用它的返回值**：`useChatConnection.ts:216` `await callPlugin(CHAT_SEND, ...)`
   之后直接丢弃返回值。
3. **流式已完全走 VDFS**（S17–S19），前端不再依赖 send 的响应通道。

而 `send` 的形状恰好就是 action 的形状：`(节点路径, 动作标识, 载荷)` —— 地址是
`<根>/session/<sid>/消息`，动作是「发言」，载荷是那一条用户消息。

**但 `send` 不等于「添加消息」**（这决定了它不能收敛成 `write`）：

- 它含一次添加（`entry.rs` 里对 `append_messages` 的直调），但同一轮还会派生 N 条
  （turn / reasoning / tool_call / tool_result），条数与内容**调用方无从预知**；
- 同一端点还有两个分支：`resume`（不添加消息）与 `ping`
  （`is_ping`，**不落库、不计活动**，`entry.rs:209-224`）。把 send 收敛成「添加消息」
  会让 `ping` 失去地址。

**⇒ 结论：`send` → `vdfs/action` 是一个**值得评估但未成熟**的选项。
前置条件是先决定 `ping` 的去处**（它不落库，因此没有「消息节点」可挂——
要么给它单独的 action 地址，要么承认它是健康检查、挪出会话协议）。
在 `ping` 有归宿之前，`chat/send` 保持现状。

---

## 5. C 档：为什么必须保留

| 路由 | 保留理由 |
|---|---|
| `chat/abort` | 控制信号（中断正在跑的一轮），不是对数据的变更。VDFS 的 13 个操作里没有「中断」这一格。 |

### 5.0 `options/list` 的改判（2026-09-23，已下线）

它曾是 C 档的第二条，理由写的是「级联选项**收集**（各层经 `OptionVisitor` 汇流），
是遍历期机制，与会话数据无关」。这条理由**本身没错**，但它回答的是「能不能迁 VDFS」——
而这条路由的真问题是**「该不该有第二条下发通道」**。

会话选项（工作目录 / 智能体 / Model / 运行模式 / 风险等级 / 心跳）本来就是
**会话配置表单的字段**，与模型 / 智能体 / SKILL 的新建表单是同一种东西。别的资源
早就用「列表项自带 `schema`」下发定义，只有会话另开了一套节点协议
（`OptionNode` + `action.endpoint` + `action.bind` 点路径 + `display` 策略），
于是同一件事有两份形状、两条通道——而**没有任何守卫会因「两边说的不一样」变红**。

迁移期靠「同一次广播喂两套产物」维持一致，那只是权宜；解药是删掉其中一套。
**已整条下线**：路由与 `handle_list_options`、`schemas/options.rs`、`OptionVisitor`
的旧产物槽、agent/model 的旧节点分支、前端 `services/options.ts` 一族全部删除。
现行规范见 [session-options.md](./session-options.md)，完整账目见
`docs/design/session-options-unification.md`。

> **判据补记**：`heartbeat/trigger`（§5.1）也是被同一条「存在判据」改判的——
> 「能不能迁」之外还要问「该不该有」。`options/list` 是这条判据的第二个受害者。

---

## 5.1 E 档：`heartbeat/trigger` 为什么是「取消」而不是「保留」

初版审计把它判为 **C 档（保留）**，理由写的是「心跳调度入口，不是会话 CRUD」——
这条理由本身没错，但它回答的是**「能不能迁 VDFS」**，而这条路由的真问题是
**「该不该存在」**。改判发生在补上第二个判据之后：

| 判据 | 提问 | `heartbeat/trigger` 的答案 |
|---|---|---|
| 迁移判据 | VDFS 有没有等价能力？ | 没有。它不是数据变更。**→ 这条只证明「不该迁」** |
| **存在判据** | 这个能力有没有**独立**存在的理由？ | 没有。见下 |

**能力重复的论证**：心跳的实质是「到点了，往会话发一轮提示词」。
用户想立刻做一次，**在输入框里直接发一条消息就是完全等价的动作**——
而且是同一个动作、同一条路径（`chat/send`）、同一份提示词。
而「立即心跳」按钮做的也是同一件事，只是**绕开了对话本身**。

于是它的存在意义只剩「让用户不用打字就能发那条消息」，而代价是：
- 同一件事有**两个入口**（输入框 / 按钮），两者行为一旦分叉（例如按钮不看会话是否忙、
  或按钮用的提示词与当前编辑中的不一致）就是一类难以察觉的 bug；
- 它是一个独立的命令型选项，要占一份声明、门控与前端渲染路径，
  却不为系统带来任何输入框做不到的能力。

**它不是「没用」，而是「重复」**——所以处理方式是删掉，而不是保留。

**删除后能力有没有损失？没有。** 心跳机制本身完好：
- 配置（`metadata.heartbeat`）与后台调度器（`HEARTBEAT_TICK_SECS = 15`）未动；
- LLM 可见的 `heartbeat` 工具仍有 `set` / `get` / `cancel` 三个 action，
  **从来没有「立即触发」这一格**——即连工具面也不曾暴露这个能力；
- 唯一被删的是那个按钮，以及它背后这个只为按钮存在的路由。

**这是一条可复用的判据**：审计一条路由时，先问「能不能迁」，再问「该不该有」。
前一问的答案（不能迁）**不能**推出后一问的答案（该保留）。

---

## 6. LLM 将获得改写消息的能力 —— ✅ 已按「全开」落地

`vdfs_write` / `vdfs_delete` / `vdfs_action` 是 **LLM 可见工具**（见 `docs/CURRENT.md` §2）。
消息节点一旦可写，模型就能**编辑或删除历史消息**。

**决策：全开**。判据是——LLM 能访问什么，**取决于 `CapabilityVisitor` 是否注册了那个工具**：
这是唯一入口，且完全可控。因此 provider 侧不需要再建一道「调用方是不是 LLM」的闸门。

这条判断把控制点放在**注册处**而不是**实现处**，与 VDFS 的分层一致：
provider 只回答「这个地址能不能写」，不回答「谁在写」。

被否决的两个选项（记录在案，避免下一轮重新推演）：

1. ~~**只读消息节点**~~：在 provider 的 `write` / `delete` 分支加「调用方是 LLM 则拒绝」
   ——provider 无法区分调用方，且会引入一条新的特殊逻辑，与本轮目标相反；
2. ~~**改走 action + 宿主确认**~~：代价是 `update` 也得包成 action，且确认闸门要另建一套。

若将来需要收紧，正确的位置是**注册处**（不把 `vdfs_write` 注册给某类 visitor），
不是 provider。

---

## 7. 执行顺序与状态

| 步 | 内容 | 状态 |
|---|---|---|
| S1 | 删 `session/append` 整条链路：`entry.rs` 的绕路由信封调用改成直连引擎（`open_chat_session` + `append_messages`），随之删路由臂 + `invoke_append` + `session_append.rs` | ✅ **已完成** |
| S2 | 删 `session/open` 路由臂 + `invoke_open` + `session_open.rs`；修 `cli/src/client.rs` 与 `cli/docs/architecture.md` 的过期注释 | ✅ **已完成** |
| S3 | `session/get_messages` → VDFS | ❌ **不做**（§3.4：替换路径的耦合比它更重） |
| S4 | provider 加消息 `write` 分支 → 前端 `updateMessage` 改走 VDFS | ✅ **已完成** |
| S5 | provider 加消息 `action("truncate")`（返回 `deleted_ids`）→ 前端 `deleteMessage` 改走 VDFS | ✅ **已完成** |
| S6 | provider 加消息 `action("clear")` → 前端 `clearMessages` 改走 VDFS | ✅ **已完成** |
| S7 | 文档同步：本文档、`CURRENT.md` 重生成、`CHANGELOG.md`、`ROUTES.md`、`vdfs-session-messages.md` | ✅ **已完成** |
| S8 | 删 `session/heartbeat/trigger` 整条能力（路由臂 + `handle_heartbeat_trigger_oneoff` + `heartbeat_trigger_option` 选项节点 + 两个常量） | ✅ **已完成**（§5.1） |

**S1–S6 全部落地：路由表 11 → 6 条**；S8 之后 **11 → 5 条**，`gate.mjs` 全绿。

### S4–S6 落地时对计划的两处修正

1. **清空从 `delete` 改成 `action`**（§3.3）：让同一区段的删除只有一种入口形态。
2. **补丁的 `id` 由地址补齐**（§3.3）：`ChatMessage::id` 是必填字段，不补的话
   「补丁是字段子集」这条承诺不成立。这条是**首轮回归测试抓出来的**。

### S4–S6 的测试

| 侧 | 用例 |
|---|---|
| 后端 `vdfs_provider.test.rs` | 新增 12 例：改写（只覆盖提供的字段 / 不带 id / id 冲突 / `create` 被拒 / 目标不存在）、截断（区间 + **一条**变更 + 目标不存在不发变更）、清空（会话本体保留 + 目录上 `deleted`）、`delete` 对区段被拒、动作不认识 / 放错地址 |
| 后端 `handlers.test.rs` | 删掉随 `invoke_delete_message` 退役的 3 例（契约已搬到 provider 测试，见该文件头的对照表），新增 `migrated_session_routes_stay_retired` |
| 前端 `session.spec.ts` | 新增 8 例：三条消息路径的**地址**、`deleted_ids` 回执映射、脏数据过滤、失败上抛 |

> **为什么值得单列**：删掉 3 例不是「丢了覆盖」——契约逐条搬到了 `vdfs_provider.test.rs`；
> 而新增的防回归用例把「5 条退役路由不得被加回来」钉住了，正是
> 「路由定义了但没有消费方」这类问题的**反向保险**。

> **为什么 S1 顺带改了 `entry.rs`**：删路由与删实现是两件事。`session/append` 路由
> 早就没有外部消费方，但 `invoke_append` 被编排器**绕路由**调着——那处调用本身是
> 「为一次数据追加搭 invoke 信封」的写法，改直连后实现才真正变死。
> 这类「内部调用伪装成路由调用」的写法要单独找：grep 路由串看不见它。

每步都过了 `scripts/gate.mjs` 全绿。S1/S2 未删任何测试；S4–S6 净增测试
（后端 +10、前端 +8），基线**只增不减**。

---

## 8. 影响面

### 8.1 删除面（S1–S6 全部落地）

| 层 | 删除物 |
|---|---|
| 后端路由 | `plugin.rs` **5 条臂**：`append` / `open` / `chat/update_message` / `chat/delete_message` / `chat/clear_messages`（原处留一段注释，列出各条的 VDFS 等价入口） |
| 后端 handler | `invoke_append` / `invoke_open` / `invoke_clear_messages` / `invoke_delete_message` / `invoke_update_message`（`handlers.rs` 315 → 159 行） |
| 后端 schema | `session_append.rs` / `session_open.rs` / `session_clear_messages.rs` / `session_delete_message.rs` / `session_update_message.rs`（含 `schemas/session/mod.rs`） |
| 后端编排 | `entry.rs` 的信封调用改为 `open_chat_session` + `append_messages` 直连；`orchestrator.rs` 去掉 `session_append` 导入 |
| 前端服务 | `services/session.ts` 的 `SESSION_ROUTES` 整表 + `callSession` + 三个端点函数（重写为**纯 VDFS 门面**） |
| 前端 schema | `session_clear_messages.ts` / `session_delete_message.ts` / `session_update_message.ts` |
| 文档 | `cli/src/client.rs` 与 `cli/docs/architecture.md` 的过期注释；`CURRENT.md` / `ROUTES.md` / `CHANGELOG.md` |
| 前端 schema | `schemas/session_clear_messages.ts` / `session_delete_message.ts` / `session_update_message.ts` |
| 文档 | `docs/reference/ROUTES.md`、`docs/reference/CONFIGURATION.md:203` |

**恒久保留**（它们本来就该在，只是不该被路由暴露）：
`open_session_handle` / `open_chat_session`（各有真实调用方）、`delete_session_internal`
（VDFS `delete` 的内部实现）。
（`invoke_get_messages` 与 `session_get_messages.rs` 曾在此列——2026-09-23 已随 §3.4.1 退役；
`invoke_update` 与 `session_update.rs` 曾在此列——同日随 §3.5 改判退役。）

> S1/S2 完成后 `handlers.rs` 从 315 行降到约 280 行；S4–S6 完成后会降到约 90 行
> ——只剩 `invoke_get_messages` / `invoke_update` / `delete_session_internal` /
> `open_session_handle` 四个函数。
>
> **实际收口（2026-09-23）**：`handlers.rs` 只剩 **2 个函数**
> （`delete_session_internal` / `open_session_handle`）——`invoke_get_messages` 与
> `invoke_update` 各自退役后，本文件已不含任何 invoke 路由实现体。

### 8.2 前端调用点（不改行为，只改传输）

| 调用点 | 现在 | 迁移后 |
|---|---|---|
| `ModelChatPanel.vue:282` → `deleteMessage` | `session/chat/delete_message` | `vdfs/action(<sid>/消息/<mid>, "truncate")` |
| `ModelChatPanel.vue:316` → `updateMessage` | `session/chat/update_message` | `vdfs/write(<sid>/消息/<mid>)` |
| `ChatMainPanel.vue:181` → `clearMessages` | `session/chat/clear_messages` | `vdfs/action(<sid>/消息, "clear")` |

store 层签名不变（`stores/sessions.ts:816/836/855`），只换内部实现——
**组件零改动**。这正是「机制化」的收益：特殊逻辑集中在 `services/session.ts` 一处，
删掉它就是删掉整个特殊面。

---

## 9. 与既有文档的关系

- [vdfs-session-messages.md](./vdfs-session-messages.md) §5 与不变量 7 需要**收窄**：
  从「消息写入只有聊天协议一处」改为「**发言**只有聊天协议一处；消息的**改写与删除**
  是普通 VDFS 写操作」。这不是推翻原结论，是把它限定在它真正论证过的范围内
  （§5 论证的是 `vdfs/write` 不合适，不是 VDFS 不合适）。
- [module-layout.md](./module-layout.md)：`handlers.rs` 的最终形态（只剩三个内部函数）
  应反映到该文档的模块分工表。
- `docs/reference/ROUTES.md`：迁移完成后路由表要重写。

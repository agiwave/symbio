# 会话输入改走地址：`<会话地址>/inbox` + 三个动作

> **状态：已落 ADR-031，§5.1–5.4 与 §6.1–6.5 已实施**（实施清单与未落地项见 §8）。
>
> 本文回答「会话的**输入**（发消息 / 停止 / 重试审批）应该落在哪」。
> 「收件箱怎么消费」见 `symbio/src/plugins/session/inbox.rs` 与 ADR-026；
> 「实时面怎么投递」见 ADR-025 与 `docs/design/vdfs.md` §9。本文不复述它们。

---

## 1. 症状

子智能体空间里发起会话后发消息，消息跑在**父智能体**的空间里；同一会话的「停止」
按钮点了没反应。

---

## 2. 根因（三条，逐条可核对）

### 根因 A：`chat/send` / `chat/abort` 是**全局路由**，落点恒为根 session 插件实例

- `symbio/src/plugins/session/plugin.rs:436-437` 把两条路由交给 `self`——而 `self`
  是**收到这次 `route` 的那个 `SessionPlugin` 实例**。
- 前端发的是 `worker/session/chat/send`（`tauri/src/constants/pluginPaths.ts:45`），
  由**根 composite** 分发 ⇒ 恒为根实例。
- 子智能体空间有**自己的** `SessionPlugin` 实例（`<根>/agent/<id>/session`），但它
  **没有路由入口**：`symbio/src/plugins/agent/host/plugin.rs:577-584` 明确
  `NotFound` 并指路 VDFS。`agent/<id>` 只在 agent 插件的 VDFS 视图里作为挂载点存在。
- 后果：消息进的是**根实例的收件箱**、由根实例消费（`inbox.rs:194`），编排拿到的
  `parent` 是根 composite（`orchestrator/entry.rs:249`）⇒ 人格 / 工具 / 存储全是
  父智能体的。**这就是「发到了父智能体」。**
- `chat/abort` 更安静：根实例 `active_mgr.get_or_create(子 sid)` 造出一个**空状态**，
  `handle_abort`（`orchestrator/consume.rs:369`）读到 `abort_signal = None` ⇒
  什么都不做（还要空等 3s 兜底）。停止无效，且根实例里多一个永生幽灵状态。

### 根因 B：前端会话层**按 id 寻址**，且只认一个全局挂载目录

- `tauri/src/services/vdfsScheme.ts:64-101`：`cachedMountDir` 是**一个**值，来自根
  清单 ⇒ 恒为 `<根>/session`。
- `tauri/src/stores/sessionTranscriptSync.ts:309`：`path.startsWith(\`${S.mountDir}/\`)`
  ——子智能体会话的变更地址 `<根>/agent/<id>/session/<sid>/message/<mid>` **前缀不
  匹配 ⇒ 前端根本收不到**。即使后端跑对了，显示链路也断在这里。
- `stores/sessions.ts:693` 新建会话也写这个全局 mountDir ⇒ 在子空间里点「新建会话」
  会建到根空间去。
- `components/session/ChatMainPanel.vue:95-97`：`:sessionId="store.activeId"`——地址
  在 `components/vdfs/Session.vue` 的 `watch(() => props.node?.name)` 就被丢掉了，
  只剩名字；两个空间里的同名 id 会撞车。

### 根因 C：输入面**没有地址这一维**，所以不可能送对

`chat/send` 的信封里只有 `session_id`（一个裸名字），没有「这个会话住在哪个空间」
的任何信息。ADR-026 已经在**后端**解决了「空间怎么自驱动」（写 inbox 即入队，
e2e `t15-subagent-inbox` 覆盖），但**入口那一跳没跟着改**：前端仍在调路由。

### 结论

不是「发送没走到 inbox」——后端 inbox 是对的。是**入口选错了**：路由是
address-less 的，而子智能体空间只有 address 可达。

---

## 3. 决策

1. **会话的输入统一表达为「对这个会话地址的一次写入 / 动作」。** 路由不再承担输入。
2. 三个入口，一条已存在、两条新增（见 §4 地址表）。
3. **`chat/send` / `chat/abort` 退役**：迁移期保留为薄别名（直接调同一批私有函数，
   不复制逻辑），迁移完成即删。它们不是「第二份实现」，而是「address-less 的入口」
   ——那正是 bug 本身。
4. **前端会话的身份从「id」升级为「地址」**（`mountDir` + `sid`）。
5. **用户消息的显示恒由后端通知驱动**；「排队中」也是一个可被观察的节点，任何发送
   方的消息都走同一条路进前端。

---

## 4. 地址表（输入面的唯一形态）

`<A>` = 会话地址 = `<挂载目录>/<sid>`（如 `<根>/session/abc`、
`<根>/agent/reviewer/session/sub-1`）。

| 意图 | 地址操作 | 语义 | 现状 |
|---|---|---|---|
| 发一条用户消息 | `write(<A>/inbox/<iid>)` | 入队；空间空闲时自己消费 | **已实现**（`vdfs_provider.rs:520`） |
| 取消一条排队消息 | `delete(<A>/inbox/<iid>)` | 队列项没了；已出队 ⇒ `NotFound` | **已实现** |
| 清空排队 | `action(<A>/inbox, "clear")` | 只清未消费的 | **已实现** |
| **停止正在跑的那一轮** | `action(<A>, "abort")` | 收敛在途 Turn | **新增** |
| **重试 / 审批 / 补充 / 回答** | `action(<A>/message/<mid>, "<动作>")` | 落在当时那条消息上 | **新增** |

三条落点的分界（与 ADR-026 已确立的口径一致，不新增概念）：

- **会话节点 `<A>`** 上的动作 = 对「正在跑的这一轮」；
- **`<A>/inbox/<iid>`** = 对「还没成为消息的消息」；
- **`<A>/message/<mid>`** = 对「已经是转写里的一条消息」。

「停止」在 UI 上是**两个动作**：`action(<A>, "abort")` + `action(<A>/inbox, "clear")`。
不给 `abort` 加 `clear_inbox` 开关——机制动词保持单一职责，组合发生在调用方。
（只 `abort` 不清队的话，消费者下一趟立刻取下一条继续跑，「点了停止它又跑起来了」。）

`<动作>` 的词表 = `ResumeAction` 的线上词形（`snake_case`，跨栈契约已由
`resume_action_wire_words_are_snake_case` 钉住）：
`retry_turn` / `retry_compaction` / `retry` / `approve` / `reject` / `supply` / `answer`。
**不另造一套动作名**：它们是同一批语义，多一套就多一处漂移。

---

## 5. 后端改动清单

### 5.1 新增 `VDFS_ACTION_ABORT`（`symbio_core/vdfs_provider.rs`，与 `truncate` / `clear` 并列）

`session_at` 的 `Action` 臂：

```
action(<A>, "abort")  →  self.abort_turn(id)   // 新私有方法
```

`abort_turn` = 现有 `handle_chat_abort_oneoff` 的函数体（取 state → `handle_abort`）。
**不复制**：路由版与动作版调同一个方法。回执 `{ok, message, data:{session_id}}`。
会话不在 / 没有在途轮次 ⇒ `ok:false` + 明确文案（不静默成功：那是现在
`chat/abort` 最坏的一面）。

### 5.2 新增 resume 动作（`messages_at` 的 `Action` 臂）

```
action(<A>/message/<mid>, <ResumeAction>)   payload: {args?, reason?, answer?, mode?, risk_level?}
```

实现 = 直连 `start_turn`（不入队——恢复必须落在当时那条消息上，见 ADR-026）。
忙碌 ⇒ 回执 `ok:false` + `session_busy`（前端据此复位 working，与现在
`useChatConnection.resume` 里的 `session_busy` 分支同款）。

需要 ctx：`messages_at` 已经收到 `ctx: &VdfsContext`，经 `host_ctx` 取回
`InvokeRequest` 即可（与 `agent_run` 的 `register_subsession` 同款手法）。

### 5.3 收件箱写入带上 `WORKDIR`

`inbox_at`（`vdfs_provider.rs:520`）现在传 `params = Request::default()`、
`workdir = None`，而 `chat/send` 传的是 `ctx.get(WORKDIR)`。走 VDFS 就丢了这个回退源。
改：`inbox_at` 接 `ctx`，入队时传 `host_ctx(ctx).get(WORKDIR)`。
（`mode` / `risk_level` / `provider_id` 不补——它们由会话 metadata 解析，
`start_turn` 的回退链已覆盖，且前端早已不在请求里透传 agent/provider。）

### 5.4 出队时发一条**无载荷**变更

`drain_inbox_once`（`inbox.rs:194-228`）`pop_front()` 后不发任何变更 ⇒ 订阅方会
永久保留一个已消失的队列项。补一条
`change_subs.notify(&VdfsChange::bare(inbox_item_path(sid, iid)))`，
与 `cancel_inbox_item` 同款（「载荷缺失 + 回读 NotFound」= 删除，ADR-025）。
**这是 §6.5 排队态显示能成立的前提。**

### 5.5 `chat/send` / `chat/abort` 退役

调用方迁移（全部在**根空间**或**本空间**，语义不变，只换入口）：

| 调用方 | 现在 | 改成 |
|---|---|---|
| 前端 | `callPlugin(CHAT_SEND / CHAT_ABORT)` | §6 的地址操作 |
| `cli/src/client.rs` | `session/chat/send` | `vdfs/write(<根>/session/<sid>/inbox)`（CLI 已会用 `vdfs/write` 建会话） |
| `plugins/telegram/plugin.rs:471` | `session/chat/send` | 同上；地址经 `vdfs/root` 拼（与 `agent_run::session_vdfs_addr` 同款） |
| `session/heartbeat.rs:197` | 直连 `handle_chat_send_oneoff` | 直连 `enqueue_inbox` |
| `agent/host/subagent.rs:338` | `parent.route(SESSION_CHAT_SEND)` | `parent.get_vfs_provider()` 写 `session/<sid>/inbox`（与它自己的 `register_subsession` 同款） |
| e2e t7 / t9 / t10 / t11 / t14 | 走路由 | 走地址（t15 已是范本） |

迁移期两条路并存（路由 = 薄别名），全部迁完删路由 + `SESSION_CHAT_SEND` /
`SESSION_CHAT_ABORT` 常量 + `CURRENT.md` §1 里 session 的自有路由行（重跑生成脚本）。

### 5.6 ⚠️ 退役的**前置条件**：写入面还表达不了「本次运行的选项」

实施时发现：`chat/send` 路由**本身已经就是「入队」**——`handle_chat_send_oneoff`
的 `message` 分支（`orchestrator/entry.rs:194-211`）只做一件事：

```
let params = Request { session_id: None, message: None, resume: None, ..req };
self.enqueue_inbox(&session_id, None, message, params, ctx.get(WORKDIR)).await;
```

也就是说路由**已经是**地址操作的薄包装，差别只在：**它能把 `params` 带进队列项**。
而队列项本来就带着它（`InboxItem.params`，`run_inbox_turn` 会 `..item.params.clone()`），
丢的只有 VDFS 写入面这一跳——`inbox_at` 的 `Write` 分支写死
`session_chat::Request::default()`（`plugin/vdfs_provider.rs:656`）。

**后果（逐个调用方核对过）**：

| 调用方 | 除消息外还要带什么 | 丢了会怎样 |
|---|---|---|
| `session/heartbeat.rs` | `include_history`、`mode: "auto"` | 心跳变交互模式，遇到需交互的工具会**产卡阻塞**（无人值守） |
| `agent/host/subagent.rs` | `provider_id`、`mode`、`risk_level` | 子会话的 provider 回退到自己的 metadata；而 `register_subsession` 只写了 `agent_id` / `workdir` ⇒ **子会话没有 provider** |
| `telegram` / CLI | 无 | 无影响 |

心跳可以原地改成**进程内** `self.enqueue_inbox(...)`（`params` 直接给，不经 VDFS）——
这本来就是 §5.5 表格里写的做法。**子智能体不行**：它不持有子会话那个插件实例，
只能经 `get_vfs_provider()` 写地址。

因此退役前必须先定一件事：**写入面怎么表达 `params`**。两个方向：

1. **包裹形状**：`{"message": {…ChatMessage…}, "params": {…}}`，同时兼容裸
   `ChatMessage`（靠「有没有 `message` 键」分辨）。代价：形状嗅探 + 协议面变宽，
   要动 `parse_inbox_message`、`protocol-mirror-audit`、`docs/design/vdfs.md`。
2. **另开一个字段**（如 `<A>/inbox/<iid>` 的写入改为「条目对象」，把 `params`
   列为可选平级字段）。与 1 同源，只是判别式不同。

**这不是机械迁移，是一次协议决定**（要不要让「地址写入」承载运行选项），
因此不顺手发明——它该有自己的 ADR。在它定下来之前，路由**保留**：
前端已不再调用它（本次已迁完），它只剩 CLI / telegram / agent_run / 心跳四个
后端调用方，且它们的行为与从前逐字节一致。

---

## 6. 前端改动清单

### 6.1 `services/vdfsScheme.ts`：从「一个」改成「按挂载目录」

- `ensureSessionScheme(mountDir)`：`Map<mountDir, {messagesSeg, inboxSeg}>` 缓存。
- 保留 `ensureSessionMountDir()` 作为**默认**入口（= 根清单里 `new_type.ext === 'session'`
  的那个），即 `<根>/session`——现有行为不变。
- **新增 `inboxSeg`**：按 `VDFS_KIND_INBOX` 从会话内部子项解析，与 `messagesSeg` 同款
  （段名是展示名，不写死——`vdfsScheme` 的模块头已经把这条写明了）。

### 6.2 `stores/sessions.ts`：会话条目带上 `mountDir`

- 条目增 `mountDir`（或等价的 `addr`）；`refreshList` 按挂载目录取。
- `activeId` → `activeAddr`（`id` 由末段派生，兼容现有读法）。
- `createSession` 写**当前**挂载目录，不再恒写全局那一个。
- `selectSession` 收地址；同名 sid 在不同空间不再撞车。

### 6.3 两个实时消费端：前缀从「一个」改成「一组」

`sessionTranscriptSync.ts` 与 `sessionNodeSync.ts` 各维护**已登记挂载目录集合**，
新增 `registerSessionMount(mountDir)`；进入子智能体空间（该空间的 `session` 挂载
被列出 / 选中）时登记。前缀匹配改成「命中任一已登记前缀」。

### 6.4 `Session.vue` → `ChatMainPanel` → `ModelChatPanel`：把地址传下去

- `Session.vue`：`watch(() => props.node?.path)` → `store.selectSession(addr)`
  （现在是 `node.name`，地址在这一跳被丢掉）。
- `ChatMainPanel.vue:95-97`：`:sessionAddr="store.activeAddr ?? ''"`。
- `useChatConnection`：入参 `sessionAddr`，`sid` 由末段派生。

### 6.5 `useChatConnection` 的三个动作改走地址

```
send(msg)   → writeVdfs(<A>/inbox/<msg.id>, JSON.stringify(msg))
abort()     → runVdfsAction(<A>, 'abort')  +  runVdfsAction(<A>/inbox, 'clear')
resume(p)   → runVdfsAction(<A>/message/<p.targetId>, p.action, {args, reason, answer, mode, risk_level})
```

- 删除 `CHAT_SEND` / `CHAT_ABORT` 常量。
- **继续不做乐观回显**（现有约定，注释已写明理由）：写 inbox ≠ 已落库。
- `working` 乐观置位保留（等待期的可视反馈）。

### 6.6 排队态显示（新增消费端，回应「前端不是唯一发送方」）

新增 `stores/sessionInboxSync.ts`：订阅 `<A>/inbox` 段，把队列项渲染成「待发送」
气泡；条目消失（出队变更，见 §5.4）即移除，落库后由 §6.3 的转写帧接手——**同一个
id**，所以是「换位置」不是「多一条」。

**为什么值得做**：用户消息一旦被消费，后端已发权威帧（落库回包，见
`symbio/src/plugins/session/docs/vdfs-session-messages.md` §3.4），
前端本来就能显示——无论发送方是前端、CLI、telegram 还是心跳。唯一看不见的窗口是
**排队中**；而这个窗口对「不是我发的消息」同样存在。把它补上，多发送方才是真闭合。

**代价**：`ModelChatPanel.showTyping` 那段「空白流 + working ⇒ 补骨架」的兜底可以
退掉一部分——有排队气泡时它不再必要（两者互斥，判据同 §6.3 的转写侧）。

---

## 7. 取舍与代价（不粉饰）

- **前端会话键从 id 变地址**：`stores/sessions` / 两个 sync / 三个组件都要动，
  是本次最大的一块。不做它就只能修好「发送」，修不好「显示」——子空间跑起来的消息
  前端仍然收不到（根因 B）。
- **退役两条路由要动 5 个调用方 + 5 个 e2e 用例**。收益是一次性的：输入面从此只有
  一个入口，不会再出现「路由能到、地址到不了」这类分叉。
- **「停止」变成两次 IPC**。可接受：按钮是低频操作，换来机制动词的单一职责。
- **排队态显示引入一个 UI 状态**（待发送）。它同时解决两个历史别扭：①排队的消息
  看起来已经发完了（ADR-026 明确要避免）；②别的发送方的消息在消费前不可见。
- **未解决（本次不做）**：`<A>/inbox` 写入目前不携带 `mode` / `risk_level`，
  由会话 metadata 回退。若将来出现「同一次发送要覆盖会话默认运行模式」的需求，
  再在正文里开一个可选字段，不预先加。

---

## 8. 实施清单（状态）

**决策已落 ADR-031**（`docs/DECISIONS.md`）。本节的「为什么」不复述 ADR，
只列**改了什么、还差什么**。

### 8.1 已落地

后端（§5.1–5.4）：

- [x] `VDFS_ACTION_ABORT` + `session_at` 的 `Action` 臂
- [x] `messages_at` 的 resume 臂（`ResumeAction` 线上词形 +
      `{args, reason, answer, mode, risk_level}`）
- [x] `ActiveSessionManager::get`（查询，不 `get_or_create`）
- [x] `abort_turn`——路由版与动作版**同一份实现**
- [x] `inbox_at` 收 `ctx`、入队带 `WORKDIR`
- [x] `drain_inbox_once` 出队发无载荷变更
- [x] `SessionPlugin::self_ref` / `me()`（provider 拿 `&self`，「开一轮」要 `Arc<Self>`）

前端（§6.1–6.5）：

- [x] `vdfsScheme`：按挂载目录缓存 + `inboxSeg`（按 `kind` 认）
- [x] 两个实时消费端：**一组**已登记挂载目录 +
      `registerTranscriptMount` / `registerSessionMount`（`MainLayout` 接线）
- [x] `sessions` store：`sessionSpace` / `sessionMounts` / `activeAddr` /
      `setSessionSpace` / `mountDirOf`；新建写当前空间；叶子操作按空间寻址
- [x] `services/session.ts`：七个函数各收一个可选 `mountDir`
- [x] `Session.vue` → `ChatMainPanel` → `ModelChatPanel`：传地址而不是名字
- [x] `VdfsWorkbench`：按「当前目录声明可新建 `session`」声明当前空间
      （草稿态没有地址，它是唯一能说出「我在哪个空间」的地方）
- [x] `useChatConnection`：三个动作改走地址

### 8.2 未落地

- [ ] **§5.5 退役两条路由**——前置条件见 §5.6（写入面还表达不了运行选项）。
      前端已无调用方；后端四个调用方（CLI / telegram / `agent_run` / 心跳）保持原样。
- [ ] **§6.6 排队态显示**——「前端不是唯一发送方」的闭合件。它不是本次症状的
      成因（成因是发送落点 + 订阅前缀），但少了它，别的发送方的消息在**被消费前**
      仍不可见。

### 8.3 与 §6 原计划的偏差（有意）

1. **§6.2 说 `activeId → activeAddr`**：实际是**并存**。会话 id 是后端生成的短 guid、
   跨空间全局唯一，所以 store 的状态字典仍以 id 为键，另记「它在哪个空间」。
   把键换成地址不会多解决任何问题，只会让每个消费方都要先解析一次地址才能查表。
   理由写在 `stores/sessions.ts` 模块头。

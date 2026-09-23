# 会话实时面迁回 VDFS watch —— 实施设计

> **文档类型：设计（实施前的方案）** — 决策与理由见
> [ADR-025](../DECISIONS.md#adr-025-顺序是节点属性delta-是updated的传输形态)；
> 机制规范见 [`design/vdfs.md`](./vdfs.md) §9 与
> `symbio_core/vdfs_provider.rs` 的「变更通知」小节。
> 本文只回答「**怎么改、改哪些文件、哪一步不能拆**」。
>
> **实施状态（2026-09-23）**：**全部落地**——机制层（`VdfsChange` 信封）、生产者
> （`Transcript::emit` / `emit_session_state` / 转播桥）、消费端（前端三个 store 订阅 +
> `useVdfs` + CLI `client.rs`）、退役三段（`transcript_stream` / `session/stream` 路由 /
> `services/transcriptStream.ts`）均已切换。
>
> **追记：`e2e/` 那一行（§7 表格）当时漏了。** 三个抓包用例（T9 / T10 / T11）仍在向
> 已退役的 `session/stream` 订阅——那条路由不再存在，于是它们**零帧可收**：T9/T11 表现为
> 超时，T10 表现为「流上没有 reasoning 节点」。症状像"模型没产出内容"，实际是订阅接在
> 一条死路由上。现已按本节设计迁移：共用
> `helpers.subscribeSessionRealtime`（`event_bus/subscribe` + `vdfs/watch` 两步，
> watch 的是**父目录** `<根>/session` 而非 `<根>/session/<sid>`——订阅一次覆盖所有会话，
> 且会话尚不存在时父目录一定在），断言改用**到达序**（单一 FIFO）而非逐帧 `seq`。
>
> **本文写于实施前，两处与最终形态不同，读时以 `design/vdfs.md` §9 与 ADR-025 追记为准：**
>
> 1. **信封没有操作枚举**（本文 §2 表里的 `created` / `updated` / `deleted` 已整个退役）：
>    语义全在 `data` 的字段上——`delta` 追加 / `content` 替换 / `status = removed` 移除。
> 2. **`path` 恒为被变更节点自身的地址**（本文 §2 的地址图已经是对的）：
>    消息的落点是 `<sid>/message/<mid>` 这个**节点**，身份即末段。会话是**容器**，其下是
>    若干**并列的集合**（消息 / 子会话 / 记忆 / 工作目录，后续还会有任务列表、请求队列……），
>    集合项形状统一为 `<sid>/<集合段>/<项 id>`。**曾一度实现成「落点 = 消息目录
>    `<sid>/message` + 身份在 `data.id`」**，收口时改回本文的原设计——理由见 ADR-025 追记。

---

## 1. 先完整理解现行消息流的设计（这是新方案的全部依据）

现行实时面是 `session/stream` 一条转写流（`symbio_core::transcript_stream`），
`Transcript::apply` 是**唯一写入点**。它值得照搬的**不是**「一条流」，而是下面六条：

| # | 设计 | 出处（可核对） |
|---|---|---|
| 1 | **帧就是节点视图，没有操作枚举**——`delta` 有 ⇒ 尾部追加、`content` 有 ⇒ 整条替换、`status = removed` ⇒ 就地移除；**语义由字段本身给出** | `ChatMessage.delta` 的文档；`transcript_stream::NodeEvent` 的形状表 |
| 2 | **`delta` 是传输形态，`content` 是节点形态**——图里只留累积后的 `content` | `transcript.rs:316` 的原话：「图里只留累积后的 `content`：`delta` 是传输形态，不是节点形态。」 |
| 3 | **帧自给自足**——未知 id 用帧内信息建占位，不依赖任何先行帧 | `Transcript::apply` 的 `or_insert_with` 分支 |
| 4 | **协议违例显式拒绝**——同帧 `delta` + `content` ⇒ 报错丢弃，**不占 seq** | `transcript.rs:243-250` |
| 5 | **顺序是节点属性**——`ChatMessage.seq` 是权威锚点，缺失回退 `timestamp` | `plugin/nodes.rs::ordered`（「`seq` 是唯一权威顺序锚点」）；前端 `sortTranscript` |
| 6 | **删除是状态迁移不是操作**——`removed` 就地移除，「不产生任何清空重读」 | `transcript.rs:255-269` |

**两个不同的 `seq`**（这是本方案最容易搞错的地方）：

- `NodeEvent.seq` = **帧序号**（投递保证，缺口检测用）——**随本次迁移消失**；
- `ChatMessage.seq` = **消息在 `<sid>/message` 这个文件夹里的位置**（排序锚点）——**保留**。

把两者混为一谈，正是「顺序＝投递属性」这个历史性理解错误的化石。

**由此得到的判据（本次立）**：转写流存在的三条理由逐条失效——

| 理由 | 现状 |
|---|---|
| 需要**流内序号** | 顺序是节点属性（第 5 条），投递层不需要序号 |
| 需要**背压恢复** | `event_bus::try_publish` 已对等：满通道**保留订阅** + 补送 resync 指令（`ea8a460`） |
| 需要**免回读** | `delta` 无条件提供（第 1、2 条）；其余靠 `vdfs/read`（幂等） |

---

## 2. 目标形态

```
<根>/session/<sid>                    会话节点   ── 运行态 = 它的 status
<根>/session/<sid>/message/<mid>          消息文件   ── 流式 = 它内容的增长
```

| `change` | `delta` | 消费端动作 |
|---|---|---|
| `created` | — | 插入该节点（**回读**拿身份：`role` / `parent_id` / `seq` / 正文基线） |
| `updated` | **有** | **尾部追加**，零回读 |
| `updated` | — | 节点变了，回读（`stat` / `read`） |
| `deleted` | — | 就地移除 |
| `created` / `deleted` 带 `delta` | — | **协议违例**（报错丢弃，与第 4 条同源） |

**取值集合不变**（仍是 `created` / `updated` / `deleted`）。**没有 `appended`**——
「追加」不是一种操作，而是 `updated` 上的一个可选字段。机制里**没有任何会话特化**：
任何「内容会增长」的 provider 都能用同一条。

---

## 3. 三个必须想清楚的地方

### 3.1 读与流的协调（`delta` 与 `read` 打架怎么办）

`delta` 是**唯一**的正文增量来源；`read` 只供**身份**与**基线**。

- **订阅时已在列表里的节点**：`list` / `read` 给的 `content` 是**基线**，此后 `delta` 追加。
- **新节点（`created`）**：插占位 → 回读拿身份。**若回读期间有 `delta` 落地，丢弃回读的
  `content`、只取身份**——因为内容只追加，本地增量**总是**真值的前缀，而回读快照可能落后。
- **丢失**：`event_bus` 满通道 → resync → **整份重读**，此时**接受**回读的 `content` 并重置基线。

这条规则**不是新发明的**：批次 G 删掉的 `appendGuard` 就是它（「读取前记下该路径已应用过
几次追加，响应回来若这个数变了 → 丢弃该响应」）。本次把它**加回**，并把它要覆盖的场景从
「详情视图」扩到「转写列表」。

> **为什么不能靠「谁新谁赢」**：两个字符串都是真值的前缀，长的那个不一定更接近
> ——必须按「本地已应用了几个增量」这个**代际**判，而不是比长度。

### 3.2 路径与「谁来补前缀」

- `Transcript` 产出的是 **provider 子树内的相对路径**：`<sid>/message/<mid>`、`<sid>`。
- 补成展示地址（`<根>/session/...`）由**门面**做，机制已存在：`VdfsChange::map_paths`
  （`composite/vdfs.rs` / `vdfs/fs.rs` / `agent/host/vdfs.rs` 三处包装各调一次）。
  **本方案不改它**——这正是 `map_paths` 是「唯一翻译点」的意义：新增 `delta` 字段
  自动随行（`to_change_event` 已补 `delta: change.delta.clone()`）。

### 3.3 `Transcript.seq` 的语义要改（不删）

现状：`Transcript.seq` 是**帧序号**，只在 `emit` 里 `+= 1`。
目标：它是**位置序号分配器**——**首次见到**一条消息时分配，写进内存图，
供 `read` / `list` 通过 `overlay_live` / `ordered` 拿到。

**为什么必须提前到创建时**：在途消息（尚未落库）当前 `seq = None`，前端只能回退
`timestamp`；两条**并行工具**在同一毫秒创建时就没有权威顺序了。VDFS 变更**不带节点载荷**，
消费端只能从 `read` 拿 `seq`——所以 `seq` 必须在 `read` 能得到的地方（内存图）里就已经有。

**不变量 #28 随之改述**：

> 旧：「任何发布出去的**帧**都在 `Transcript` 里取号。」
> 新：「任何**消息节点**的**位置序号**都在 `Transcript` 里分配。」

---

## 4. 改动清单（逐文件）

### 4.1 生产端

| 文件 | 改动 |
|---|---|
| `symbio/src/plugins/session/transcript.rs` | `Transcript` 增字段 `changes: Arc<vdfs::ChangeSubscriptions>`（`new` 注入）。`emit` 改为：算路径 → 判 `change`（首次见 ⇒ `created`；`status == Removed` ⇒ `deleted`；其余 ⇒ `updated`，带 `delta` 时走 `with_delta`）→ `changes.notify(...)`。`emit_session_state` 改为 `changes.notify(VdfsChange::new(sid, "updated"))`。**删** `publish_frame` / `publish_session_state` 的 import 与调用 |
| `…/session/active.rs`（`ActiveSessionState` 构造处） | 把 `plugin.change_subs.clone()` 传进 `Transcript::new` |
| `…/session/orchestrator/broadcast.rs` | 运行态出口由「发转写流帧（先取节点视图）」改为「发 VDFS 变更（不带快照）」——**节点视图那一步可以整段删掉** |
| `…/session/plugin.rs` | 删 `handle_stream_subscribe` 与路由臂 `"stream"`；`session_node_of` 若只服务 stream 则一并删（`list` / `stat` 仍用则保留） |
| `…/session/plugin/nodes.rs` | 无需改（`ordered` / `overlay_live` 已经按 `seq` 排） |
| `symbio/src/symbio_core/transcript_stream.rs`（+ `.test.rs`） | **删除** |
| `…/agent/host/subagent.rs` | 转播桥回到 `event_bus` + `vdfs/watch`；`stream_relay_bridge` 的参数表随之变 |
| `…/telegram/plugin.rs` | 若订转写流则改订 VDFS 变更 |

### 4.2 消费端（前端）

| 文件 | 改动 |
|---|---|
| `tauri/src/services/transcriptStream.ts` | **删除**（合帧窗口 / 缺口检测 / `reconcileTranscript` 的替代者一并消失） |
| `tauri/src/services/eventBus.ts` | 无需改（`subscribeVdfsChanged` 已就绪，`delta` 随 `VdfsChange` 到达） |
| **新增** `tauri/src/stores/sessionTranscriptSync.ts` | 按 §3.1 的规则把变更落到 store：`<sid>` ⇒ 运行态回读 → `applySessionState`；`<sid>/message/<mid>` ⇒ `created` 插占位 + 回读身份 / `updated`+`delta` 就地追加（带 `appendGuard`）/ `updated` 无 `delta` 回读 / `deleted` 就地移除 |
| `tauri/src/stores/sessions.ts` | 落地口 `applyTranscriptMessages` **保留**（调用方换成上面那个）；`applySessionState` 的零回读前提消失（改为回读 `stat` 收敛） |
| `tauri/src/composables/useVdfs.ts` | **加回** `applyAppend` + `appendGuard`（消息详情随 `delta` 就地增长） |
| `tauri/src/views/MainLayout.vue` | 启动函数换成 `startSessionTranscriptSync` |
| `tauri/src/composables/useChatConnection.ts`、`stores/sessionTranscript.ts`、`constants/pluginPaths.ts` | 接线与文档更新 |
| 测试：`streamFlow.spec.ts` / `toolRoundLiveFrames.spec.ts` / `sessions.spec.ts` / `useVdfs.spec.ts` | 改接线；合帧相关用例删除 |

### 4.3 消费端（CLI）

| 文件 | 改动 |
|---|---|
| `cli/src/client.rs` | 删 `session/stream` 订阅；改 `event_bus/subscribe` + `vdfs/watch(<根>/session/<sid>)`；`Frame` 由 `Transcript` / `Session` / `Resync` 改为 `Node(VdfsChange)` / `Resync`；`stream_frame_of` 换成 `vdfs_change_of` + resync 判定 |
| `cli/docs/architecture.md`、`cli/src/client.rs` 模块文档 | 按 §1 的六条定稿（本次已改过一次，落地时去掉「即将改写」的标记） |

### 4.4 退役与门禁

| 文件 | 改动 |
|---|---|
| `symbio/src/plugins/gateway/config.rs` | 撤掉「`session/stream` 未进只读白名单」的说明（对象不再存在） |
| `scripts/gate.d/_shared.mjs` | `rustTests` / `vitestTests` 基线按实测更新；`test-layout-audit` 的内联天花板随之调整 |
| `e2e/` | 适配（`session/stream` 的抓包用例改抓 `bus_event`） |

---

## 5. 同批约束（不可拆）

**生产者与消费端必须同批切换**，因为这是**同一个字节的两种读法**：

- 线上 `VdfsChange` 的 JSON keys 由 `["change","path"]` 变为**可选含 `delta`**；
- 且生产者**不再发** `session/stream`——未改的消费端会**彻底收不到消息**（不是降级，是空）。

机制层（`VdfsChange.delta`）**已经先行落地且向后兼容**（可选字段），因此它可以单独提交；
但 §4.1 的出口切换、§4.2 / §4.3 的消费端、§4.4 的退役**必须一次做完**。

---

## 6. 验证

| 项 | 手段 |
|---|---|
| 形状 | `protocol-mirror-audit`（C 组 / D 组镜像：后端 `VdfsChange` ↔ 前端 `schemas/vdfs.ts`） |
| 协议违例 | `created` / `deleted` 带 `delta` ⇒ 报错丢弃（与「`delta` + `content` 互斥」同源） |
| 读/流协调 | 单测：回读期间注入 `delta` ⇒ 回读 `content` 被丢弃、身份保留 |
| 顺序无关 | 单测：**乱序注入**两条并行工具的消息变更 ⇒ 按 `seq` 排出的显示顺序与注入顺序无关 |
| 丢失兜底 | 单测：resync 指令 ⇒ 按作用域整份重读；重读幂等 |
| 全量 | `cargo test --lib` ｜ `vitest run` ｜ `node scripts/gate.mjs --ci` ｜ e2e |

---

## 7. 风险与未定项

- **通知量回到「每 token 一条」**：`event_bus` 订阅通道容量已提到 4096（与 `session/stream`
  对等）。若实测 resync 频率上升，正确性不受影响，但会多出整份重读。
- **反向风险**：`VdfsChange` 加宽后，下一个改动者可能按「零生产者」再删一次。
  → 必须把**生产者清单**（`Transcript::apply`）留在词汇表旁（已做）。
- **未定项 A**：区间删除（`truncateIdsFrom`）当前方案是「前端本地一次性完成 + 后端逐条发
  `deleted` 供其他消费端收敛」——是否需要 `delta` 覆盖这个场景，待实施时按实测决定。
- **未定项 B**：`ChatMessage.seq` 提前到创建时分配（§3.3）是否本轮就做。**建议做**——
  否则并行工具在途消息没有权威顺序，前端的 `seq ?? timestamp` 兜底会退化成按毫秒并列。

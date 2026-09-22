/**
 * transcriptStream — 会话**实时面**的消费端（消息 + 会话运行态的唯一通道）
 *
 * ## 两种帧，一个 `seq` 空间
 *
 * 实时面是**一条流**（`worker/session/stream`）。信封都是 `{ type, data }`，
 * `data` 有两种：
 *
 * | `type` | 载荷 | 语义 |
 * |---|---|---|
 * | `transcript_event` | `NodeEvent` = `{ session_id, seq, message }` | 一条消息 |
 * | `transcript_session` | `SessionStateEvent` = `{ session_id, seq, node }` | 会话节点的**全量视图** |
 *
 * 要紧的不是"有两种帧"，而是**它们从同一个计数器取号**（后端 `Transcript::emit`
 * 与 `Transcript::emit_session_state`，`seq` 的唯一分配点）：于是**读到一个
 * `seq = N` 的会话运行态帧时，所有 `seq < N` 的帧必然已经落地**。
 *
 * 这条推理是**结构性的**（同一个计数器 + 单通道保序），不是调度巧合。它取代了
 * 原先的跨通道顺序假设：会话运行态走 `kind = "vdfs"`、消息走本流，两条通道各有
 * 自己的 `mpsc` 与泵任务，"谁先到"没有任何机制保证——因此「会话已离开 `working`」
 * 当时**推不出**「本轮消息终态帧都已到达」，只能挂一个 300ms 宽限复查兜底
 * （仍见非终态节点就整份回读）。现在那个缺口不存在了。
 *
 * 会话运行态帧**只在本轮有活跃转写时**发出（`seq` 的唯一分配点在 `Transcript`，
 * 而此刻必有活跃轮次）；没有在途轮次的资源变更（创建 / 删除 / 改名 / 标题）
 * 拿不到 `seq`，仍走 VDFS 的粗粒度信号——那部分由 `stores/sessionNodeSync.ts`
 * 收敛，**不携带节点快照**（快照只有两个来源：本流与 `list` / `stat` 回读）。
 *
 * ## 消息帧的协议：帧携带什么字段，本地图就变更什么
 *
 * | 帧里的字段 | 本地动作 |
 * |---|---|
 * | `delta` | 追加到该节点正文尾部（**该内容的首次传输**） |
 * | `content` | 整条替换该节点正文（幂等） |
 * | `status = removed` | 就地移除该节点 |
 * | `status`（其余） / `error` | 状态迁移 |
 * | `parent_id` / `role` / `type` / `name` / `tool_call_id` / `meta` / `seq` / `timestamp` | 有则合并（发射端持有当前完整值） |
 *
 * `delta` 与 `content` **互斥**：同帧携带即协议违例，后端写入点与前端落地都报错
 * 丢弃。前端因此永远不需要面对「拼接还是替换」的歧义。
 *
 * ## 为什么不需要「逐路径串行链」
 *
 * 每帧**自带所需事实**（增量或状态），应用是同步的，链不存在。
 *
 * ## 丢帧可检测（本模块的核心不变量）
 *
 * `seq` 在会话内单调递增（**帧序号**，与会话内排序锚点 `message.seq` 是两回事），
 * 因此「跳号」是可判定的：跳号即**已知有损**，消费端整份重读（`reload`）。重读源
 * 是 VDFS 读面（会话文档 = 存储 + 在途叠加），它本身就是权威——所以恢复路径不需要
 * 补帧、不需要缓存，也不依赖任何一帧的送达。这一点与后端
 * `transcript_stream::publish_frame` 的背压策略是一对：后端满通道时摘除订阅并投
 * resync 标记，前端收不到标记也能靠跳号自愈。
 *
 * 缺口检测**对两种帧一视同仁**（`advanceSeq`）：它们共用一个计数器，任何一帧漏掉
 * 都会让后续帧看起来跳号。
 *
 * ## 单订阅、跨页面
 *
 * 与 `stores/sessionNodeSync.ts` 同构：由**应用外壳**（`MainLayout`）显式启动，
 * 落地目标（sink）注入。多会话并发时所有会话的转写都落 store，
 * 否则从 A 切到 B 再切回 A 时，A 在后台收到的消息会丢失。
 *
 * ## 合帧：为什么消息帧要攒一小会儿再落地
 *
 * 后端**逐帧发布**（不变量 #28：`seq` 与发布逐帧无例外），一次回复在高速输出时
 * 可达数十~上百帧/秒。而前端**每落地一帧**都要付一份与历史长度成正比的成本：
 * store 提交一份新的消息字典（`updateMessages` 的全量浅拷贝）+ 重建消息树并重排
 * 子节点（`useChatConnection::messageTree`）。逐帧落地意味着单次回复的总成本是
 * `O(历史条数 × 帧数)`——长会话下这是端到端的主要热点。
 *
 * 因此本层把窗口内（`FLUSH_INTERVAL_MS`，约一帧时长）到达的**消息帧按会话攒成一批**
 * 一次交给 sink。这不是「丢帧」也不是「折帧」：
 *
 * - 每一帧仍**逐个**过 `seq` 缺口检测（顺序语义一点没变）；
 * - 落地时每一帧仍**逐条**应用（`delta` 追加 / `content` 替换 / `removed` 移除），
 *   只是合并进**同一次** store 提交与**同一次**树重建；
 * - 代价是 ≤48ms 的显示延迟，仍在人眼的「瞬时」阈值以内。
 *
 * 换句话说：`seq` 的语义在**帧**上，`O(n)` 的成本在**提交**上；合帧只动后者。
 *
 * **会话运行态帧不走合帧窗口**，而是先 `flushPendingFrames(sid)` 再落地：它与消息
 * 共用 `seq` 空间，同会话里 `seq` 更小的消息帧必然已在本地队列中——让运行态帧
 * 等满一个窗口，等于人为制造一段「后端已报空闲、本地转写却还是非终态」的窗口，
 * 而那正是上层过去要靠宽限复查去兜的东西。攒批的收益（省一次 store 提交）远小于
 * 丢掉这条结构性保证的代价。
 */

import { connectPlugin, type Connection, type ConnectEvent } from './plugin'
import { SESSION_STREAM } from '@/constants/pluginPaths'
import type { ChatMessage } from '@/schemas/chat_message'
import type { VdfsNode } from '@/schemas/vdfs'
import { logger } from '@/utils/logger'

/**
 * 一帧转写事件（与后端 `NodeEvent` 同构）。
 *
 * `message` 嵌套而非平铺：外层 `seq` 是**帧序号**、内层 `message.seq` 是**存储序号**，
 * 两个语义不同的 `seq` 平铺会撞进同一个 JSON 键。
 */
export interface NodeEvent {
  session_id: string
  /** 流内**单调递增**帧序号（会话内唯一权威的帧顺序锚点） */
  seq: number
  /** 消息本身。它是**帧的全部载荷**——没有任何独立的操作字段。 */
  message: ChatMessage
}

/**
 * 一帧**会话运行态**（与后端 `SessionStateEvent` 同构）。
 *
 * `node` 是会话节点的**全量视图**（`status` + `attributes.{outcome,error,warning}`）：
 * 消费端就地收敛，不回读。它与消息帧**共用 `seq` 计数器**——这正是本类型存在的
 * 意义（见模块文档「两种帧，一个 `seq` 空间」）。
 *
 * 语义上等价于「节点状态流」：状态是幂等的全量视图，本帧可以丢（丢了靠 `list`
 * 快照收敛），但**只要收到**，就带着"所有更早的帧都已落地"这条信息。
 */
export interface SessionStateEvent {
  session_id: string
  /** 与消息帧**同一个计数器**发出的帧序号 */
  seq: number
  /** 会话节点的全量视图 */
  node: VdfsNode
}

/**
 * 转写落地目标（**依赖倒置**：service 不反向依赖 Pinia store）。
 *
 * 三个动作：**应用一批消息帧**、**应用一帧会话运行态**、**整份重读**。前者是后端
 * `Transcript::apply` 在前端的镜像（delta 追加 / content 替换 / removed 移除 /
 * 其余字段合并），落地由应用外壳注入（`MainLayout` 传 `useSessionsStore()`），
 * 测试传普通对象即可。
 *
 * `messages` 收的是**一批**（同一会话、按到达顺序）而非单条：合帧的收益要落到
 * 「一次 store 提交」上，就必须让落地口知道这是一批。单帧场景传长度 1 的数组。
 */
export interface TranscriptStreamSink {
  /** 应用一批消息帧（同一会话、按到达顺序；帧语义全在字段上，本方法不做二次解释） */
  messages(sessionId: string, messages: ChatMessage[]): void
  /**
   * 应用一帧会话运行态（**全量节点视图**，幂等）。
   *
   * 调用前本层已经 `flushPendingFrames(sessionId)`：同会话里 `seq` 更小的消息帧
   * 都已落地。落地目标因此可以放心把「这一帧说会话不忙」当成「本轮转写已完整」。
   */
  applySessionState(sessionId: string, node: VdfsNode): void
  /** 序号缺口 / resync：清空本地转写并从存储整份重读 */
  reload(sessionId: string): void | Promise<void>
}

/** 帧类型（与后端帧信封的 `type` 字段逐字一致） */
const FRAME_EVENT = 'transcript_event'
const FRAME_SESSION = 'transcript_session'
const FRAME_RESYNC = 'transcript_resync'

/** 重连退避上限（与 `eventBus` 同量级） */
const RECONNECT_MAX_DELAY = 30000

/**
 * 合帧窗口（毫秒）。
 *
 * ## 取值理由（两个约束的交点）
 *
 * 收益随「窗口内落进多少帧」放大：窗口 `W`、帧率 `f` ⇒ 每 `W·f` 帧只提交一次，
 * 提交次数降为 `1/(W·f)`。而帧率由模型决定，不是常数——实测区间大致
 * 20~100 帧/秒（慢模型逐字、快模型成块、推理模型突发）。
 *
 * | 帧率 | `W=16ms` | `W=48ms` |
 * |---|---|---|
 * | 20/s | 1.0×（**等于不合**） | 1.0× |
 * | 50/s | 1.0× | **2.4×** |
 * | 100/s | 1.6× | **4.8×** |
 *
 * 约束的另一侧是**感知延迟**：人把 ~100ms 以内视为"瞬时"，而流式文本的观感在
 * 50ms 量级下仍与逐字无异（50 帧/秒时一次出 2~3 个字）。16ms 虽然"更保守"，
 * 但它在典型速率下**根本合不到帧**——等于白写一层缓冲。
 *
 * 取 48ms（60Hz 下 3 帧）：典型速率下有 2~5 倍提交削减，同时把显示延迟压在
 * "瞬时"阈值的一半以内。
 */
const FLUSH_INTERVAL_MS = 48

interface StreamState {
  sink: TranscriptStreamSink | null
  connection: Connection | null
  /** 会话 → 已应用的最大 `seq`（缺口检测的唯一依据） */
  lastSeq: Map<string, number>
  /** 会话 → 待落地的帧（保持到达顺序；flush 时按会话一次提交） */
  pending: Map<string, ChatMessage[]>
  /** 合帧定时器（有 pending 时才存在） */
  flushTimer: ReturnType<typeof setTimeout> | null
  reconnectTimer: ReturnType<typeof setTimeout> | null
  reconnectDelay: number
  /** 主动停止时置位：抑制断线重连（否则 stop 之后又自己连回来） */
  stopped: boolean
}

/**
 * 内部状态（globalThis 单例）。
 *
 * 理由与 `eventBus` 相同：Vite HMR 会重新执行本模块，模块级变量被重置后
 * 旧连接成为孤儿（后端订阅仍在、前端监听器仍在），再次启动即建立第二条连接
 * ——同一条消息送达两次。状态收敛到 globalThis 后，模块重载复用同一份。
 */
const _G = globalThis as typeof globalThis & { __symTranscriptStreamState?: StreamState }
const S: StreamState = _G.__symTranscriptStreamState ?? (_G.__symTranscriptStreamState = {
  sink: null,
  connection: null,
  lastSeq: new Map(),
  pending: new Map(),
  flushTimer: null,
  reconnectTimer: null,
  reconnectDelay: 1000,
  stopped: true,
})
// HMR：模块重载后复用旧 state 对象，新字段可能不存在——补齐而不是让它变成 undefined。
S.pending ??= new Map()
S.flushTimer ??= null

/**
 * 落地待应用的消息帧（按会话各提交一次）。
 *
 * **幂等**：pending 为空时是空操作。因此「先 flush 再 reload」与「reload 自己 flush」
 * 不会重复落地。
 *
 * @param sessionId 只落地这一个会话的待应用帧，其余会话的批次留在队列里（定时器
 *   也保留）。会话运行态帧用它换取一条结构性保证：同会话里 `seq` 更小的消息帧
 *   必然已在队列中，先落地它们，运行态帧带来的「本轮已完整」才成立（模块文档
 *   「合帧」一节的最后一段）。不传则落地全部——那是定时器与停止路径的用法。
 */
export function flushPendingFrames(sessionId?: string): void {
  const sink = S.sink
  if (sessionId === undefined) {
    if (S.flushTimer) {
      clearTimeout(S.flushTimer)
      S.flushTimer = null
    }
    const batches = [...S.pending.entries()]
    S.pending.clear()
    if (!sink || batches.length === 0) return
    deliver(sink, batches)
    return
  }
  const msgs = S.pending.get(sessionId)
  if (!msgs) return
  S.pending.delete(sessionId)
  if (S.pending.size === 0 && S.flushTimer) {
    clearTimeout(S.flushTimer)
    S.flushTimer = null
  }
  if (sink) deliver(sink, [[sessionId, msgs]])
}

/** 一批（或多批）消息帧交给落地目标；单批失败不牵连其余 */
function deliver(sink: TranscriptStreamSink, batches: [string, ChatMessage[]][]): void {
  for (const [sid, msgs] of batches) {
    try {
      sink.messages(sid, msgs)
    } catch (err) {
      logger.error('[transcript-stream]', `转写落地失败：${sid}`, err)
    }
  }
}

/** 丢弃待应用的帧（重读前调用：权威快照会覆盖它们，再应用就是重复追加）。 */
function dropPendingFrames(sessionId?: string): void {
  if (sessionId === undefined) {
    S.pending.clear()
  } else {
    S.pending.delete(sessionId)
  }
  if (S.pending.size === 0 && S.flushTimer) {
    clearTimeout(S.flushTimer)
    S.flushTimer = null
  }
}

/**
 * **立刻**整份重读某会话（不等合帧窗口）。
 *
 * 本模块里「权威重读」有两个入口，走的是**同一条路径**（丢弃待落地帧 → `sink.reload`）：
 *
 * | 入口 | 触发 |
 * |---|---|
 * | `advanceSeq` 的跳号分支 | 帧序号出现缺口 = 已知有损 |
 * | `handleStreamFrame` 的 resync 帧 / 重连 | 后端明示「你可能漏了帧」 |
 *
 * 语义：整份重读**替换**本地转写（`hydrateTranscript` 只按快照重建），因此不需要
 * 「补帧」；`lastSeq` 有意**不重置**——重读拿到的是权威状态，已包含此前的帧，
 * 后续帧继续按 `seq` 续接。
 *
 * （曾有第三个入口：会话运行态与消息分走两条通道，到达顺序无保证，于是上层在
 * 「会话离开运行态」后挂一次宽限复查，仍不收敛才调这里。批次 E 把运行态并进本流
 * 并共用 `seq` 空间后，那条推理成为结构性保证，复查连同它的宽限期一并删除。）
 */
function reloadSession(sessionId: string): void {
  const sink = S.sink
  if (!sink) return
  // 与跳号分支同理：待落地帧多半已被权威快照包含，先丢弃再应用会重复追加正文。
  dropPendingFrames(sessionId)
  void Promise.resolve(sink.reload(sessionId)).catch((err: unknown) =>
    logger.warn('[transcript-stream]', `重读转写失败：${sessionId}`, err),
  )
}

/** 入队一帧，并在需要时启动合帧定时器 */
function enqueueFrame(sessionId: string, message: ChatMessage): void {
  const q = S.pending.get(sessionId)
  if (q) q.push(message)
  else S.pending.set(sessionId, [message])
  if (!S.flushTimer) {
    S.flushTimer = setTimeout(() => {
      S.flushTimer = null
      flushPendingFrames()
    }, FLUSH_INTERVAL_MS)
  }
}

/**
 * 处理一帧流事件（连接回调与单测共用同一入口）。
 *
 * 分派只按**帧类型**（信封的 `type`）：`transcript_event` 应用消息、
 * `transcript_session` 应用会话运行态、`transcript_resync` 整份重读。
 * 没有需要推断的形状。
 */
export function handleStreamFrame(event: ConnectEvent): void {
  if (event.type === FRAME_EVENT) {
    const ev = event.data as NodeEvent | undefined
    if (ev) applyNodeEvent(ev)
    return
  }
  if (event.type === FRAME_SESSION) {
    const ev = event.data as SessionStateEvent | undefined
    if (ev) applySessionStateFrame(ev)
    return
  }
  if (event.type === FRAME_RESYNC) {
    // 后端明示「你可能漏了帧」：整份重读所有已知会话（不猜漏了哪一段）
    logger.warn('[transcript-stream]', '收到 resync 标记，整份重读在途会话')
    reloadAll()
    return
  }
  if (event.type === 'disconnected' || event.type === 'error') {
    if (event.type === 'error') logger.error('[transcript-stream]', '流连接错误', event.data)
    scheduleReconnect()
  }
}

/**
 * 帧序号推进与缺口检测——**两种帧共用**（它们本就共用一个计数器）。
 *
 * @returns 本帧可以落地；`false` 表示应丢弃（重复帧，或跳号已触发整份重读）。
 *
 * **顺序判定**（丢帧可检测的唯一落点）：
 * - `seq == last + 1`：连续，正常应用；
 * - `seq <= last`：重复帧（重连窗口内可能重发），丢弃——重复应用增量会叠字；
 * - `seq > last + 1`：**跳号 = 已知有损**，整份重读并丢弃本帧
 *   （重读拿到的是当前权威状态，已包含本帧的效力）。
 */
function advanceSeq(sessionId: string, seq: number): boolean {
  const last = S.lastSeq.get(sessionId)
  if (last !== undefined) {
    if (seq <= last) return false
    if (seq !== last + 1) {
      logger.warn(
        '[transcript-stream]',
        `转写流跳号（${last} → ${seq}），整份重读：${sessionId}`,
      )
      // 先把水位推到本帧：重读期间再来的帧按新水位续接，不会因"水位还是旧的"
      // 而反复触发重读。
      S.lastSeq.set(sessionId, seq)
      reloadSession(sessionId)
      return false
    }
  }
  S.lastSeq.set(sessionId, seq)
  return true
}

/**
 * 应用一条消息帧（顺序判定 → 入队待落地）。
 *
 * 水位先于载荷校验推进：`seq` 是后端为**每一个发布出去的帧**分配的，凡到达本端
 * 的帧都已占用一个号。若载荷不合法就不推进，下一帧会被误判成跳号，触发一次
 * 毫无必要的整份重读。
 */
export function applyNodeEvent(ev: NodeEvent): void {
  if (!S.sink) return
  const sid = ev.session_id
  if (!sid || typeof ev.seq !== 'number') return
  if (!advanceSeq(sid, ev.seq)) return
  if (!ev.message) {
    logger.warn('[transcript-stream]', `消息帧未携带消息体，已丢弃：${sid}`)
    return
  }

  // 帧就是一整条消息：语义全在字段上（delta 追加 / content 替换 / removed 移除 /
  // 其余合并），不存在需要分派的操作——入队后由落地目标按字段判定。
  enqueueFrame(sid, ev.message)
}

/**
 * 应用一帧会话运行态（顺序判定 → **先落地同会话的消息帧** → 交付节点视图）。
 *
 * 两个动作的顺序是本函数存在的全部理由：
 *
 * 1. `advanceSeq` 之后，`seq` 更小的帧**必然都已在本地队列里**（同一个计数器 +
 *    单通道保序），但它们还在等合帧窗口；
 * 2. 因此先 `flushPendingFrames(sid)` 把它们落地，再交付"会话不忙"这一帧——
 *    落地目标于是可以断言「本轮转写已完整」。
 *
 * 反过来（先交付运行态、消息帧继续等窗口）会人为制造一段「后端已报空闲、本地转写
 * 仍是非终态」的窗口——上层过去要靠宽限复查去兜的正是它。
 *
 * `node` 缺失（后端违反帧形状）时只记 warn 并丢弃：不猜、不回读。真丢一次不要紧，
 * 下一次 `list` 快照会收敛（状态是幂等的）。
 */
export function applySessionStateFrame(ev: SessionStateEvent): void {
  const sink = S.sink
  if (!sink) return
  const sid = ev.session_id
  if (!sid || typeof ev.seq !== 'number') return
  if (!advanceSeq(sid, ev.seq)) return
  if (!ev.node || typeof ev.node !== 'object') {
    logger.warn('[transcript-stream]', `会话运行态帧未携带节点视图，已丢弃：${sid}`)
    return
  }

  flushPendingFrames(sid)
  try {
    sink.applySessionState(sid, ev.node)
  } catch (err) {
    logger.error('[transcript-stream]', `会话运行态落地失败：${sid}`, err)
  }
}

/** 整份重读所有已知在途会话（resync 标记 / 重连后调用） */
function reloadAll(): void {
  const sink = S.sink
  if (!sink) return
  // 后端明示「你可能漏了帧」⇒ 权威快照会覆盖本地，先丢弃待落地帧（理由同跳号分支）。
  dropPendingFrames()
  for (const sid of [...S.lastSeq.keys()]) {
    void Promise.resolve(sink.reload(sid)).catch((err: unknown) =>
      logger.warn('[transcript-stream]', `重读转写失败：${sid}`, err),
    )
  }
}

/**
 * 启动转写流（先停旧连接再建新的 → 进程内天然单订阅）。
 *
 * @param sink 转写落地目标。**必须显式注入**（生产由应用外壳传 `useSessionsStore()`，
 *   测试可传普通对象）——本模块是 service，不认识 Pinia。
 */
export async function startTranscriptStream(sink: TranscriptStreamSink): Promise<void> {
  stopTranscriptStream()
  S.sink = sink
  S.stopped = false
  try {
    await connect()
  } catch (err) {
    // 连接失败不抛：断线重连是常态（后端可能尚未就绪），由退避重试兜住。
    logger.error('[transcript-stream]', '转写流连接失败，将退避重试', err)
    scheduleReconnect()
  }
}

/** 建立一条转写流连接（重连路径复用） */
async function connect(): Promise<void> {
  const conn = await connectPlugin(SESSION_STREAM, {}, handleStreamFrame)
  // stop 已经跑过（用户切页 / HMR）：这条连接是迟到的，立刻关掉
  if (S.stopped) {
    await conn.close()
    return
  }
  S.connection = conn
  S.reconnectDelay = 1000
  logger.info('[transcript-stream]', '转写流已连接', conn.connectionId)
}

/** 断线后按指数退避重连；重连成功即整份重读（断线窗口内的帧已不可追） */
function scheduleReconnect(): void {
  if (S.stopped || S.reconnectTimer) return
  const delay = S.reconnectDelay
  S.reconnectTimer = setTimeout(() => {
    S.reconnectTimer = null
    S.reconnectDelay = Math.min(S.reconnectDelay * 2, RECONNECT_MAX_DELAY)
    void connect()
      .then(() => reloadAll())
      .catch((err: unknown) => {
        logger.warn('[transcript-stream]', '转写流重连失败', err)
        scheduleReconnect()
      })
  }, delay)
}

/** 停止转写流（重复启动 / HMR / 测试时调用） */
export function stopTranscriptStream(): void {
  S.stopped = true
  if (S.reconnectTimer) {
    clearTimeout(S.reconnectTimer)
    S.reconnectTimer = null
  }
  // 已入队的帧仍是有效数据：先交给当前 sink 落地，再断链（否则这批帧静默丢失，
  // 表现为「切换页面时最后十几个字没出来」）。
  flushPendingFrames()
  const conn = S.connection
  S.connection = null
  S.sink = null
  S.lastSeq.clear()
  dropPendingFrames()
  S.reconnectDelay = 1000
  if (conn) {
    void conn.close().catch((err: unknown) =>
      logger.warn('[transcript-stream]', '关闭转写流连接失败', err),
    )
  }
}

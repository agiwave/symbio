/**
 * transcriptStream — 会话转写的**实时流**消费端（消息的唯一实时通道）
 *
 * ## 协议：帧就是一整条消息
 *
 * 实时面是**一条流**（`worker/session/stream`），每帧是后端 `NodeEvent`：
 * `{ session_id, seq, message }`——`message` 是一条与存储同构的 `ChatMessage`。
 *
 * 协议**没有独立的操作枚举**：帧携带什么字段，本地图就变更什么——
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
 * ## 单订阅、跨页面
 *
 * 与 `stores/sessionNodeSync.ts` 同构：由**应用外壳**（`MainLayout`）显式启动，
 * 落地目标（sink）注入。多会话并发时所有会话的转写都落 store，
 * 否则从 A 切到 B 再切回 A 时，A 在后台收到的消息会丢失。
 *
 * ## 合帧：为什么帧要攒一小会儿再落地
 *
 * 后端**逐帧发布**（不变量 #28：`seq` 与发布逐帧无例外），一次回复在高速输出时
 * 可达数十~上百帧/秒。而前端**每落地一帧**都要付一份与历史长度成正比的成本：
 * store 提交一份新的消息字典（`updateMessages` 的全量浅拷贝）+ 重建消息树并重排
 * 子节点（`useChatConnection::messageTree`）。逐帧落地意味着单次回复的总成本是
 * `O(历史条数 × 帧数)`——长会话下这是端到端的主要热点。
 *
 * 因此本层把窗口内（`FLUSH_INTERVAL_MS`，约一帧时长）到达的帧**按会话攒成一批**
 * 一次交给 sink。这不是「丢帧」也不是「折帧」：
 *
 * - 每一帧仍**逐个**过 `seq` 缺口检测（顺序语义一点没变）；
 * - 落地时每一帧仍**逐条**应用（`delta` 追加 / `content` 替换 / `removed` 移除），
 *   只是合并进**同一次** store 提交与**同一次**树重建；
 * - 代价是 ≤48ms 的显示延迟，仍在人眼的「瞬时」阈值以内。
 *
 * 换句话说：`seq` 的语义在**帧**上，`O(n)` 的成本在**提交**上；合帧只动后者。
 */

import { connectPlugin, type Connection, type ConnectEvent } from './plugin'
import { SESSION_STREAM } from '@/constants/pluginPaths'
import type { ChatMessage } from '@/schemas/chat_message'
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
 * 转写落地目标（**依赖倒置**：service 不反向依赖 Pinia store）。
 *
 * 只有两个动作：**应用一批消息帧**、**整份重读**。前者是后端的
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
  /** 序号缺口 / resync：清空本地转写并从存储整份重读 */
  reload(sessionId: string): void | Promise<void>
}

/** 帧类型（与后端帧信封的 `type` 字段逐字一致） */
const FRAME_EVENT = 'transcript_event'
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
 * 落地待应用的帧（按会话各提交一次）。
 *
 * **幂等**：pending 为空时是空操作。因此「先 flush 再 reload」与「reload 自己 flush」
 * 不会重复落地。
 */
export function flushPendingFrames(): void {
  if (S.flushTimer) {
    clearTimeout(S.flushTimer)
    S.flushTimer = null
  }
  const sink = S.sink
  const batches = [...S.pending.entries()]
  S.pending.clear()
  if (!sink || batches.length === 0) return
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
 * 本模块里「权威重读」有三个入口，走的是**同一条路径**（丢弃待落地帧 → `sink.reload`）：
 *
 * | 入口 | 触发 |
 * |---|---|
 * | `applyNodeEvent` 的跳号分支 | 帧序号出现缺口 = 已知有损 |
 * | `handleStreamFrame` 的 resync 帧 / 重连 | 后端明示「你可能漏了帧」 |
 * | 本函数 | 上层发现「本地转写与权威状态可能不一致」，但**没有序号证据** |
 *
 * 第三个入口服务于一条**跨通道**的顺序假设：会话运行态走 VDFS 变更、消息走本流，
 * 两条通道的到达顺序没有任何机制保证。因此「会话已离开 `working`」**不能**推出
 * 「本轮所有消息节点都已收到终态帧」——上层据此在宽限期后复查，仍不收敛才调这里
 * （见 `stores/sessions.ts::scheduleReconcileTranscript`）。
 *
 * 语义与另两个入口完全一致：整份重读**替换**本地转写（`hydrateTranscript` 只按快照
 * 重建），因此不需要「补帧」；`lastSeq` 有意**不重置**——重读拿到的是权威状态，
 * 已包含此前的帧，后续帧继续按 `seq` 续接。
 */
export function reconcileTranscript(sessionId: string): void {
  const sink = S.sink
  if (!sink) return
  // 与跳号分支同理：待落地帧多半已被权威快照包含，先丢弃再应用会重复追加正文。
  dropPendingFrames(sessionId)
  void Promise.resolve(sink.reload(sessionId)).catch((err: unknown) =>
    logger.warn('[transcript-stream]', `收敛重读转写失败：${sessionId}`, err),
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
 * `transcript_resync` 整份重读。没有需要推断的形状。
 */
export function handleStreamFrame(event: ConnectEvent): void {
  if (event.type === FRAME_EVENT) {
    const ev = event.data as NodeEvent | undefined
    if (ev) applyNodeEvent(ev)
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
 * 应用一条转写事件（顺序判定 → 消息落地）。
 *
 * **顺序判定**（丢帧可检测的落点）：
 * - `seq == last + 1`：连续，正常应用；
 * - `seq <= last`：重复帧（重连窗口内可能重发），丢弃——重复应用增量会叠字；
 * - `seq > last + 1`：**跳号 = 已知有损**，整份重读并丢弃本帧
 *   （重读拿到的是当前权威状态，已包含本帧的效力）。
 */
export function applyNodeEvent(ev: NodeEvent): void {
  const sink = S.sink
  if (!sink) return
  const sid = ev.session_id
  if (!sid || typeof ev.seq !== 'number' || !ev.message) return

  const last = S.lastSeq.get(sid)
  if (last !== undefined) {
    if (ev.seq <= last) return
    if (ev.seq !== last + 1) {
      logger.warn(
        '[transcript-stream]',
        `转写流跳号（${last} → ${ev.seq}），整份重读：${sid}`,
      )
      S.lastSeq.set(sid, ev.seq)
      // 待落地帧先丢弃：重读拿到的是权威快照（存储 + 在途），已入队的增量多半
      // 已被它包含，再应用一遍就是重复文本。
      dropPendingFrames(sid)
      void Promise.resolve(sink.reload(sid)).catch((err: unknown) =>
        logger.warn('[transcript-stream]', `重读转写失败：${sid}`, err),
      )
      return
    }
  }
  S.lastSeq.set(sid, ev.seq)

  // 帧就是一整条消息：语义全在字段上（delta 追加 / content 替换 / removed 移除 /
  // 其余合并），不存在需要分派的操作——入队后由落地目标按字段判定。
  enqueueFrame(sid, ev.message)
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

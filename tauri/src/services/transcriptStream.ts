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
 * 只有两个动作：**应用一条消息帧**、**整份重读**。前者是后端的
 * `Transcript::apply` 在前端的镜像（delta 追加 / content 替换 / removed 移除 /
 * 其余字段合并），落地由应用外壳注入（`MainLayout` 传 `useSessionsStore()`），
 * 测试传普通对象即可。
 */
export interface TranscriptStreamSink {
  /** 应用一条消息帧（帧语义全在字段上，本方法不做二次解释） */
  message(sessionId: string, message: ChatMessage): void
  /** 序号缺口 / resync：清空本地转写并从存储整份重读 */
  reload(sessionId: string): void | Promise<void>
}

/** 帧类型（与后端帧信封的 `type` 字段逐字一致） */
const FRAME_EVENT = 'transcript_event'
const FRAME_RESYNC = 'transcript_resync'

/** 重连退避上限（与 `eventBus` 同量级） */
const RECONNECT_MAX_DELAY = 30000

interface StreamState {
  sink: TranscriptStreamSink | null
  connection: Connection | null
  /** 会话 → 已应用的最大 `seq`（缺口检测的唯一依据） */
  lastSeq: Map<string, number>
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
  reconnectTimer: null,
  reconnectDelay: 1000,
  stopped: true,
})

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
      void Promise.resolve(sink.reload(sid)).catch((err: unknown) =>
        logger.warn('[transcript-stream]', `重读转写失败：${sid}`, err),
      )
      return
    }
  }
  S.lastSeq.set(sid, ev.seq)

  // 帧就是一整条消息：语义全在字段上（delta 追加 / content 替换 / removed 移除 /
  // 其余合并），不存在需要分派的操作——把「该做什么」交给落地目标按字段判定。
  sink.message(sid, ev.message)
}

/** 整份重读所有已知在途会话（resync 标记 / 重连后调用） */
function reloadAll(): void {
  const sink = S.sink
  if (!sink) return
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
  const conn = S.connection
  S.connection = null
  S.sink = null
  S.lastSeq.clear()
  S.reconnectDelay = 1000
  if (conn) {
    void conn.close().catch((err: unknown) =>
      logger.warn('[transcript-stream]', '关闭转写流连接失败', err),
    )
  }
}

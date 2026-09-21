/**
 * transcriptStream — 会话转写的**实时流**消费端（消息的唯一实时通道）
 *
 * ## 它替换了什么
 *
 * 消息实时面曾经寄生在 VDFS 变更频道上（`kind = "vdfs"` 的 `VdfsChange`，
 * 由已删除的 `vdfsTranscriptSync` 消费）。那条路有两个结构性问题：
 *
 * 1. **补丁语义**：`created` / `updated` / `appended` 只带部分字段与节点视图，
 *    消费端要自己判定「追加还是替换」，判定错就是叠字或倒退；
 * 2. **无序号**：VDFS 变更没有流内序号，丢一帧**不可检测**——用户看到的
 *    「工具一直进行中、刷新即愈」正是这一类。
 *
 * 现在实时面是**一条流**（`worker/session/stream`），每帧是后端
 * `NodeEvent`：`{ session_id, seq, op }`。
 *
 * | 操作 | 载荷 | 本地动作 |
 * |---|---|---|
 * | `upsert` | **完整消息快照** | 按 id 整条替换（不存在则创建） |
 * | `append` | 仅 `delta` | 追加到目标节点正文尾部（窄载荷，热路径逐帧；对正文 / 思考 / 工具参数 / 工具响应一律成立） |
 * | `remove` | `message_id` | 删掉这一个节点 |
 * | `reset` | — | 清空本地转写并**从存储整份重读** |
 * | `warn` | `warning` | 不在此处理（会话级状态，落在会话节点上） |
 *
 * ## 为什么不需要「逐路径串行链」
 *
 * 旧实现必须把同一路径的变更串成顺序链：`created` 在补丁形态下需要一次异步
 * `stat` + `read` 回读，而回读与紧随的 `appended` 竞争就会丢内容。
 * 现在每帧**自带完整事实**（快照或增量），应用是同步的，链自然消失。
 *
 * ## 丢帧可检测（本模块的核心不变量）
 *
 * `seq` 在会话内单调递增，因此「跳号」是可判定的：跳号即**已知有损**，
 * 消费端整份重读（`reload`）。重读源是 VDFS 读面（会话文档 = 存储 + 在途叠加），
 * 它本身就是权威——所以恢复路径不需要补帧、不需要缓存，也不依赖任何一帧的送达。
 * 这一点与后端 `transcript_stream::publish_frame` 的背压策略是一对：
 * 后端满通道时摘除订阅并投 resync 标记，前端收不到标记也能靠跳号自愈。
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
 * `op` 是后端 `NodeOp` 的 serde 内部标签（`#[serde(tag = "op")]`），
 * 其余字段按 `op` 各自成立——这正是「显式操作」与「补丁」的分界：
 * 载荷形状由操作决定，消费端不做「字段有没有来」的推断。
 */
export interface NodeEvent {
  session_id: string
  /** 流内**单调递增**序号（会话内唯一权威的帧顺序锚点） */
  seq: number
  op: 'upsert' | 'append' | 'remove' | 'reset' | 'warn'
  /** `upsert`：完整消息快照 */
  message?: ChatMessage
  /** `append` / `remove`：目标消息 id */
  message_id?: string
  /** `append`：追加的增量 */
  delta?: string
  /** `warn`：会话级告警（`null` = 清除） */
  warning?: string | null
}

/**
 * 转写落地目标（**依赖倒置**：service 不反向依赖 Pinia store）。
 *
 * 每个方法对应一个协议操作，一一对应、不做二次解释——store 侧的落地由
 * 应用外壳注入（`MainLayout` 传 `useSessionsStore()`），测试传普通对象即可。
 */
export interface TranscriptStreamSink {
  /** 完整快照：按 id 整条替换 */
  upsert(sessionId: string, message: ChatMessage): void
  /** 增量：追加到目标消息正文尾部 */
  append(sessionId: string, messageId: string, delta: string): void
  /** 删掉这一个节点 */
  remove(sessionId: string, messageId: string): void
  /** `reset` / 序号缺口：清空本地转写并从存储整份重读 */
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
 * 分派按**帧类型**（信封的 `type`），而操作语义按 `op`——两者都是闭集，
 * 没有需要推断的形状。
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
 * 应用一条转写事件（顺序判定 → 操作落地）。
 *
 * **顺序判定**（丢帧可检测的落点）：
 * - `seq == last + 1`：连续，正常应用；
 * - `seq <= last`：重复帧（重连窗口内可能重发），丢弃——重复应用 `append` 会叠字；
 * - `seq > last + 1`：**跳号 = 已知有损**，整份重读并丢弃本帧
 *   （重读拿到的是当前权威状态，已包含本帧的效力）。
 */
export function applyNodeEvent(ev: NodeEvent): void {
  const sink = S.sink
  if (!sink) return
  const sid = ev.session_id
  if (!sid || typeof ev.seq !== 'number' || !ev.op) return

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

  switch (ev.op) {
    case 'upsert':
      // 完整快照：整条替换。store 侧按 id 覆盖，不存在则创建。
      if (ev.message) sink.upsert(sid, ev.message)
      return
    case 'append':
      // 增量只带 delta，目标可以是**任何**正文仍在增长的节点：正文 / 思考 /
      // 工具调用参数（后端 `emit_append` 与 Text / Reasoning 同构）/ 工具响应
      // （`tool_executor` 透传子会话的 `Append`）。协议不在这里按节点类型分档。
      // 目标缺失 = 协议违例（`upsert` 必先于 `append`）——交给 store 留痕并丢弃。
      if (ev.message_id && typeof ev.delta === 'string') {
        sink.append(sid, ev.message_id, ev.delta)
      }
      return
    case 'remove':
      if (ev.message_id) sink.remove(sid, ev.message_id)
      return
    case 'reset':
      // 转写被截断 / 压缩重写：本地整份作废，从存储重读。
      S.lastSeq.delete(sid)
      void Promise.resolve(sink.reload(sid)).catch((err: unknown) =>
        logger.warn('[transcript-stream]', `重读转写失败：${sid}`, err),
      )
      return
    case 'warn':
      // 会话级告警落在**会话节点**上（`sessionNodeSync` 已按节点状态收敛），
      // 在消息层再写一次就是同一份真相的第二种写法。
      return
    default:
      logger.warn('[transcript-stream]', `未知的转写操作，已忽略：${String(ev.op)}`)
  }
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

/**
 * sessionTranscriptSync — 会话**实时面**的消费端（ADR-025：实时面走 VDFS 变更）
 *
 * ## 一条通道，两类地址
 *
 * 会话域的实时面只有一条（`event_bus` 的 `vdfs` 频道 + `vdfs/watch` 登记，
 * 由 `subscribeVdfsChanged` 一次办妥）。变更的**地址**区分两类内容：
 *
 * | 地址 | 语义 | 消费动作 |
 * |---|---|---|
 * | `<根>/session/<sid>` | 会话节点变更（运行态 / 资源） | `stat` 回读节点 → `applySessionState` |
 * | `<根>/session/<sid>/<消息段>/<mid>` | 一条消息 | 见下表 |
 *
 * 消息变更的取值集合仍是 `created` / `updated` / `deleted` 三个，**没有操作枚举**——
 * `delta` 的有无就是全部语义：
 *
 * | `change` | `delta` | 动作 |
 * |---|---|---|
 * | `deleted` | — | 就地移除（`status = removed` 帧走同一条落地路径） |
 * | 任意 | 有，且 id 已知 | **零回读**：`{ id, delta }` 直接落地（流式正文的绝大多数帧） |
 * | 任意 | 有，id 未知 | 先落增量（占位），再回读身份（见下） |
 * | `created` / `updated` | 无 | 回读（`stat` + `read`）拿身份与正文基线 |
 *
 * ## 读与流的协调（`delta` 与 `read` 打架怎么办）
 *
 * `delta` 是**唯一**的正文增量来源；`read` 只供**身份**与**基线**。规则
 * （与设计文档 §3.1 逐字一致）：
 *
 * - **订阅时已在列表里的节点**：回读的 `content` 是**基线**，此后 `delta` 追加；
 * - **新节点**：`created` 时插入占位 → 回读身份。**若回读期间有 `delta` 落地，
 *   丢弃回读的 `content`、只取身份**——内容只追加，本地增量**总是**真值的
 *   前缀，而回读快照可能落后；
 * - **丢失**：`event_bus` 满通道 → resync 指令 → **整份重读**，此时接受回读的
 *   `content` 并重置基线。
 *
 * 这条规则不是新发明的：批次 G 删掉的 `appendGuard` 就是它（「读取前记下该路径
 * 已应用过几次追加，响应回来若这个数变了 → 丢弃该响应」）。本次把它加回
 * （`useVdfs` 里是**详情视图**的那一份，本文件是**转写列表**的这一份——两个
 * 作废时机不同的守卫必须是两个实例，共用一个会互相误伤）。
 *
 * **为什么不能靠「谁新谁赢」**：两个字符串都是真值的前缀，长的那个不一定更接近
 * ——必须按「本地已应用了几个增量」这个**代际**判，而不是比长度。
 *
 * ## 「未知 id + delta」的次序
 *
 * 增量**先落地**（`applyTranscriptMessages` 对未知 id 自建占位），回读**后发**。
 * 回读返回时按代际判：期间有增量落地 → 丢弃 `content`、只取身份（本地已有
 * 增量链）；期间没有 → 接受 `content` 并**丢弃队列里该 id 的待落地增量**——
 * 它们都发生在回读发起之前，已包含在快照里，再应用就是重复追加。
 *
 * ## 合帧：为什么增量要攒一小会儿再落地
 *
 * 后端逐 token 发一条变更，前端每落地一批都要付一次 store 提交 + 一次消息树
 * 重建（O(历史条数)）。本层把窗口内（48ms）到达的**增量帧按会话攒批**一次交给
 * 落地口——与旧转写流（`services/transcriptStream`，已随 ADR-025 退役）的合帧
 * 同一套取值理由。**回读结果不等窗口**（低频且是身份/基线的权威来源）。
 *
 * ## 单订阅、跨页面
 *
 * 与 `stores/sessionNodeSync.ts` 同构：由**应用外壳**（`MainLayout`）显式启动，
 * 落地目标（sink）注入。多会话并发时所有会话的转写都落 store，否则从 A 切到 B
 * 再切回 A 时，A 在后台收到的消息会丢失。
 */

import { subscribeVdfsChanged } from '@/services/eventBus'
import { statVdfs, readVdfs } from '@/services/vdfs'
import { ensureVdfsSessionScheme } from '@/services/vdfsScheme'
import { type VdfsChange, type VdfsNode } from '@/schemas/vdfs'
import {
  MESSAGE_TYPE_TOOL_CALL,
  MESSAGE_TYPE_TURN,
  type ChatMessage,
} from '@/schemas/chat_message'
import { logger } from '@/utils/logger'

/**
 * 转写落地目标（**依赖倒置**：本模块不反向依赖 Pinia store）。
 *
 * 生产由应用外壳传 `useSessionsStore()`，测试传普通对象即可。
 */
export interface TranscriptSyncSink {
  /** 该会话的本地转写图里是否已有这条消息（决定 `delta` 走零回读还是回读） */
  hasMessage(sessionId: string, messageId: string): boolean
  /** 应用一批消息帧（语义与后端 `Transcript::apply` 的前端镜像一致） */
  applyTranscriptMessages(sessionId: string, messages: ChatMessage[]): void
  /** 应用一帧会话运行态（`stat` 回读的全量节点视图，幂等） */
  applySessionState(sessionId: string, node: VdfsNode): void
  /** resync：按会话整份重读（权威快照替换本地转写） */
  reload(sessionId: string): void | Promise<void>
}

/** 增量合帧窗口（毫秒）——取值理由见旧转写流的同名常量（48ms = 60Hz 3 帧） */
const FLUSH_INTERVAL_MS = 48

interface SyncState {
  sink: TranscriptSyncSink | null
  unsubscribe: (() => void) | null
  /** 地址方案缓存（mountDir + 消息段名） */
  mountDir: string
  messagesSeg: string
  /** 会话 → 待落地的增量帧（保持到达顺序；flush 时按会话一次提交） */
  pending: Map<string, ChatMessage[]>
  flushTimer: ReturnType<typeof setTimeout> | null
  /**
   * 消息路径 → 本地已应用的增量**代际**（`appendGuard` 的记法）。
   * 每应用一条增量 +1；回读发起时记下起点，返回时比对。
   */
  deltaGen: Map<string, number>
  /** 收到过消息变更的会话（resync 时按它整份重读） */
  seenSessions: Set<string>
}

/**
 * 内部状态（globalThis 单例）。
 *
 * 理由与 `eventBus` / 旧转写流相同：Vite HMR 会重新执行本模块，模块级变量被
 * 重置后旧订阅成为孤儿，再次启动即叠一层监听器——同一次变更被处理两次。
 * 状态收敛到 globalThis 后，模块重载复用同一份。
 */
const _G = globalThis as typeof globalThis & { __symTranscriptSyncState?: SyncState }
const S: SyncState = _G.__symTranscriptSyncState ?? (_G.__symTranscriptSyncState = {
  sink: null,
  unsubscribe: null,
  mountDir: '',
  messagesSeg: '',
  pending: new Map(),
  flushTimer: null,
  deltaGen: new Map(),
  seenSessions: new Set(),
})
// HMR：模块重载后复用旧 state 对象，新字段可能不存在——补齐而不是让它变成 undefined。
S.deltaGen ??= new Map()
S.seenSessions ??= new Set()

/** 消息节点的展示地址（订阅 handler 的路径过滤与守卫键共用这一种拼法） */
function messageAddr(sid: string, mid: string): string {
  return `${S.mountDir}/${sid}/${S.messagesSeg}/${mid}`
}

/** 某消息路径当前的增量代际（没应用过 = 0） */
function deltaGenOf(addr: string): number {
  return S.deltaGen.get(addr) ?? 0
}

/** 把待落地的增量帧交给落地口（按会话各提交一次） */
function flushPending(): void {
  if (S.flushTimer) {
    clearTimeout(S.flushTimer)
    S.flushTimer = null
  }
  const sink = S.sink
  if (!sink) {
    S.pending.clear()
    return
  }
  for (const [sid, msgs] of S.pending.entries()) {
    S.pending.delete(sid)
    try {
      sink.applyTranscriptMessages(sid, msgs)
    } catch (err) {
      logger.error('[transcript-sync]', `转写落地失败：${sid}`, err)
    }
  }
}

/** 丢弃某会话待落地的增量帧（重读前调用：权威快照会覆盖它们） */
function dropPending(sessionId?: string): void {
  if (sessionId === undefined) S.pending.clear()
  else S.pending.delete(sessionId)
  if (S.pending.size === 0 && S.flushTimer) {
    clearTimeout(S.flushTimer)
    S.flushTimer = null
  }
}

/** 入队一条增量帧，并在需要时启动合帧定时器 */
function enqueueDelta(sid: string, mid: string, delta: string): void {
  const q = S.pending.get(sid)
  const frame: ChatMessage = { id: mid, delta }
  if (q) q.push(frame)
  else S.pending.set(sid, [frame])
  if (!S.flushTimer) {
    S.flushTimer = setTimeout(flushPending, FLUSH_INTERVAL_MS)
  }
}

/**
 * 从节点视图 + 正文还原一条消息（后端 `nodes.rs::message_of_node` 的前端镜像）。
 *
 * 属性经 `#[serde(flatten)]` 平铺在节点顶层（role / type / parent_id / seq /
 * error / meta / tool_name / tool_call_id）；`status` 是节点字段；`updated_at`
 * 即消息 timestamp。正文有两种上线形态（后端 `message_text`）：
 * Turn / ToolCall 的正文是**整条消息的 JSON**（容器节点靠子节点呈现，正文承载
 * 结构），其余是纯文本。
 *
 * 返回 `null` = 状态词不在词表里（两侧已分叉，宁可丢这条也不造出状态错误的消息）。
 */
function messageOfNode(node: VdfsNode, text: string): ChatMessage | null {
  const v = node as Record<string, unknown>
  const status = typeof node.status === 'string' ? node.status : ''
  const known = [
    'pending', 'streaming', 'waiting_user_action',
    'completed', 'aborted', 'failed', 'removed',
  ]
  if (!known.includes(status)) return null
  const msgType = typeof v.type === 'string' ? v.type : undefined

  // 容器节点的正文是整条消息 JSON：结构字段以它为准（比 attributes 全）。
  if (msgType === MESSAGE_TYPE_TURN || msgType === MESSAGE_TYPE_TOOL_CALL) {
    try {
      const parsed = JSON.parse(text) as ChatMessage
      if (parsed && typeof parsed === 'object' && parsed.id) {
        return { ...parsed, id: node.name }
      }
    } catch {
      // 解析失败退回属性拼装——正文形态漂移不该让整条消息消失
    }
  }

  const out: ChatMessage = { id: node.name }
  if (typeof v.role === 'string') out.role = v.role as ChatMessage['role']
  if (msgType) out.type = msgType as ChatMessage['type']
  if (typeof v.tool_name === 'string') out.name = v.tool_name
  if (typeof v.parent_id === 'string') out.parent_id = v.parent_id
  if (typeof v.tool_call_id === 'string') out.tool_call_id = v.tool_call_id
  if (typeof v.seq === 'number') out.seq = v.seq
  if (typeof v.error === 'string') out.error = v.error
  if (v.meta != null && typeof v.meta === 'object') out.meta = v.meta as ChatMessage['meta']
  if (typeof node.updated_at === 'number') out.timestamp = node.updated_at
  if (status) out.status = status as ChatMessage['status']
  if (text) out.content = text
  return out
}

/** 回读一条消息节点（`stat` + `read`）并按代际规则落地。 */
async function readMessage(sid: string, mid: string): Promise<void> {
  const sink = S.sink
  if (!sink) return
  const addr = messageAddr(sid, mid)
  // 追加代际快照：读取期间若有增量落地，本地内容比这次响应新 → 丢弃 content
  const gen = deltaGenOf(addr)
  try {
    const [node, content] = await Promise.all([statVdfs(addr), readVdfs(addr)])
    if (!node) return // 已删（或会话没了）：删除另有 deleted 变更兜底
    const changed = deltaGenOf(addr) !== gen
    const text = content?.text ?? ''

    if (changed) {
      // 读取期间有增量落地：只取身份，不取 content（本地增量链是真值前缀）。
      const identity = messageOfNode(node, '')
      if (!identity) {
        logger.warn('[transcript-sync]', `消息节点状态词不可识别，丢弃：${addr}`)
        return
      }
      delete identity.content
      sink.applyTranscriptMessages(sid, [identity])
      return
    }

    const message = messageOfNode(node, text)
    if (!message) {
      logger.warn('[transcript-sync]', `消息节点状态词不可识别，丢弃：${addr}`)
      return
    }
    // 代际未变 ⇒ 快照包含队列里该 id 的全部待落地增量（它们都发生在读取发起
    // 之前），落地全量后丢弃队列增量，否则就是重复追加。
    dropQueuedDelta(sid, mid)
    sink.applyTranscriptMessages(sid, [message])
  } catch (err) {
    logger.warn('[transcript-sync]', `回读消息节点失败：${addr}`, err)
  }
}

/** 从待落地队列里移除某条消息的增量帧（回读接受 content 前调用） */
function dropQueuedDelta(sid: string, mid: string): void {
  const q = S.pending.get(sid)
  if (!q) return
  const rest = q.filter((m) => m.id !== mid)
  if (rest.length === 0) S.pending.delete(sid)
  else S.pending.set(sid, rest)
}

/** 回读会话节点并落地运行态（变更不带快照——`stat` 永远最新且幂等） */
async function readSessionState(sid: string): Promise<void> {
  const sink = S.sink
  if (!sink) return
  const addr = `${S.mountDir}/${sid}`
  try {
    const node = await statVdfs(addr)
    // 会话已删（`deleted` 由 sessionNodeSync 收敛）：本条无事可做
    if (!node) return
    sink.applySessionState(sid, node)
  } catch (err) {
    logger.warn('[transcript-sync]', `回读会话节点失败：${addr}`, err)
  }
}

/**
 * 处理一条变更（订阅回调与单测共用同一入口）。
 *
 * 分派只按**地址形状**（会话叶子 vs 消息项）与**字段**（`delta` 的有无），
 * 没有需要推断的操作枚举。
 */
export function handleVdfsChange(change: VdfsChange): void {
  const sink = S.sink
  if (!sink || !S.mountDir) return
  const path = change.path
  if (!path || !path.startsWith(`${S.mountDir}/`)) return

  // ① 会话叶子：`<mountDir>/<sid>`（恰一段）。信封没有操作枚举（S27）——
  //    `data` 是全量节点视图（与 stat 同源构造）时零回读就地收敛；无载荷
  //    （资源信号 / 删除）回读兜底（节点已删时 stat 得 null，静默返回）。
  const rel = path.slice(S.mountDir.length + 1)
  const segs = rel.split('/')
  if (segs.length === 1) {
    const sid = segs[0]
    if (!sid) return
    const view = change.data
    if (view != null && typeof view === 'object') {
      sink.applySessionState(sid, view as VdfsNode)
      return
    }
    void readSessionState(sid)
    return
  }

  // ② 消息目录：`<mountDir>/<sid>/<消息段>`（恰两段）。信封的 `path` 是消息
  //    所在的**目录**，载荷 `data` 就是那条 `ChatMessage`——身份在 `data.id`，
  //    语义在字段上。其余（记忆 / 工作目录 / 更深层级 / 无载荷）不认。
  if (segs.length !== 2 || segs[1] !== S.messagesSeg) return
  const sid = segs[0]
  if (!sid) return
  S.seenSessions.add(sid)
  const msg = change.data as
    | { id?: unknown; delta?: unknown; status?: unknown }
    | null
    | undefined
  if (!msg || typeof msg.id !== 'string' || !msg.id) return
  const mid = msg.id

  // 删除：`status = removed`（消息词汇的删除语义——信封上没有 deleted 取值）
  if (msg.status === 'removed') {
    dropQueuedDelta(sid, mid)
    try {
      sink.applyTranscriptMessages(sid, [{ id: mid, status: 'removed' }])
    } catch (err) {
      logger.error('[transcript-sync]', `删除落地失败：${sid}/${mid}`, err)
    }
    return
  }

  // 纯增量 + 身份已知 ⇒ **零回读**（流式正文的绝大多数帧）
  const addr = messageAddr(sid, mid)
  if (typeof msg.delta === 'string') {
    // 代际先于落地推进：回读的判定基准是「落地了几条」，与是否合帧无关
    S.deltaGen.set(addr, deltaGenOf(addr) + 1)
    enqueueDelta(sid, mid, msg.delta)
    if (sink.hasMessage(sid, mid)) return
    // 身份未知：增量先落（占位），回读补身份——返回时按代际决定取不取 content
    void readMessage(sid, mid)
    return
  }

  // 全量帧（首帧 / content 替换 / 状态迁移）：回读完整记录（幂等收敛）
  void readMessage(sid, mid)
}

/**
 * 启动会话转写同步（先停旧订阅再挂新的 → 进程内天然单订阅）。
 *
 * @param sink 落地目标。**必须显式注入**（生产由应用外壳传 `useSessionsStore()`，
 *   测试可传普通对象）——本模块不认识 Pinia。
 */
export async function startSessionTranscriptSync(sink: TranscriptSyncSink): Promise<void> {
  stopSessionTranscriptSync()
  S.sink = sink

  // 地址方案是运行期数据（段名是展示名，随后端下发），解析失败就不订阅：
  // 宁可没有订阅，也不要订到一个拼错的 prefix 上。
  try {
    const scheme = await ensureVdfsSessionScheme()
    S.mountDir = scheme.mountDir
    S.messagesSeg = scheme.messagesSeg
  } catch (err) {
    logger.error('[transcript-sync]', '会话挂载目录解析失败，订阅未启动', err)
    return
  }

  // 订阅范围 = 会话挂载目录的**整棵子树**（清单订阅用 directChildren 只看叶子，
  // 这里要看见消息项）。重同步：后端通道曾满 ⇒ 按本端见过的会话整份重读。
  S.unsubscribe = subscribeVdfsChanged(
    { prefix: S.mountDir },
    handleVdfsChange,
    () => {
      logger.warn('[transcript-sync]', '收到 resync 标记，整份重读在途会话')
      // 与跳号分支同理：待落地增量多半已被权威快照包含，先丢弃再重读
      // 会重复追加正文。
      dropPending()
      for (const sid of [...S.seenSessions]) {
        void Promise.resolve(sink.reload(sid)).catch((err: unknown) =>
          logger.warn('[transcript-sync]', `重读转写失败：${sid}`, err),
        )
      }
    },
  )
  logger.info('[transcript-sync]', '会话转写同步已启动', S.mountDir)
}

/** 停止会话转写同步（重复启动 / HMR / 测试时调用） */
export function stopSessionTranscriptSync(): void {
  if (S.unsubscribe) {
    S.unsubscribe()
    S.unsubscribe = null
  }
  if (S.flushTimer) {
    clearTimeout(S.flushTimer)
    S.flushTimer = null
  }
  S.pending.clear()
  S.sink = null
  // mountDir / messagesSeg 保留：地址方案与订阅无关，重启动省一次解析
  // deltaGen 保留：代际是「路径级」事实，不随订阅生命周期作废
  S.seenSessions.clear()
}

/** 测试辅助：清空全部内部状态（globalThis 单例跨用例残留时调用） */
export function resetSessionTranscriptSyncForTest(): void {
  stopSessionTranscriptSync()
  S.deltaGen.clear()
  S.seenSessions.clear()
  S.mountDir = ''
  S.messagesSeg = ''
}

// 供测试断言内部代际（不进生产路径）
export const __internals = { deltaGenOf, messageAddr }

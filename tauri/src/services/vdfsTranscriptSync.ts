/**
 * vdfsTranscriptSync — 会话转写的 VDFS 实时消费端
 *
 * ## 它替换了什么
 *
 * 会话转写原先是**专用协议 + 专用通道**：读走 `session/get_messages`，
 * 实时走 `kind = "session"` 的 `ChatEventType.Update` / `Delete`。
 *
 * 现在它只是一张**列表**：地址 `.vdfs/session/<sid>/消息`，每一项是一条消息；
 * 流式输出是列表项的**追加型变更**（`appended`）。于是读路径与实时路径都归
 * VDFS，`kind = "vdfs"` 成为消息的唯一变更通道（会话级事件——状态 / 标题 /
 * 生命周期——仍走原通道）。
 *
 * ## 为什么逐路径串行（本模块的核心不变量）
 *
 * 变更必须按到达顺序应用到同一条消息上。`created` / `updated` 在 provider
 * 未附带载荷时需要一次**异步回读**，而回读一旦并发就会与紧随其后的 `appended`
 * 竞争：
 *
 * ```text
 * created  m1（回读挂起）
 * appended m1 "abc"      → 本地 "abc"
 * 回读返回 ""（旧快照）    → 覆盖 → "abc" 丢失（静默）
 * ```
 *
 * 因此同一路径的变更串成一条**顺序链**（`enqueue`）：链上每一环等前一环结束。
 * 不同路径互不阻塞——两条消息的流式互不影响。
 *
 * ## 载荷优先，回读兜底
 *
 * 后端在 `created` / `updated` 上附带**节点视图 + 内容快照**
 * （`VdfsChange.node` / `.content`），因此常规路径**零回读**——这正是 VDFS
 * 承载转写不比既有专用通道更贵的原因。仅当 provider 未附带时才回退
 * `stat` + `read`（通用消费端的正确降级，不假设任何 provider 的行为）。
 */

import { subscribe as busSubscribe, type BusEvent } from './eventBus'
import { readVdfs, statVdfs } from './vdfs'
import {
  VFDS_CHANGE_APPENDED,
  VFDS_CHANGE_CREATED,
  VFDS_CHANGE_DELETED,
  VFDS_CHANGE_UPDATED,
  VFDS_EVENT_KIND,
  parseTranscriptPath,
  type VdfsChange,
  type VdfsNode,
} from '@/schemas/vdfs'
import { useSessionsStore } from '@/stores/sessions'
import type { ChatMessage, MessageStatus } from '@/schemas/chat_message'
import { logger } from '@/utils/logger'

let _unsubscribe: (() => void) | null = null

// HMR 守卫：模块热更新会重置 `_unsubscribe`，导致二次订阅 → 同一条消息被两个
// handler 各应用一次（流式文本叠字）。启动标记挂到 globalThis，跨模块重载幂等。
// （与 sessionBusWatcher 同一套理由，同一个坑不踩两次。）
const _G = globalThis as typeof globalThis & { __symTranscriptSyncStarted?: boolean }

/** 节点状态词 → 消息状态词。 */
function messageStatusOf(node: VdfsNode): MessageStatus | undefined {
  switch (node.status) {
    case 'pending':
      return 'pending'
    case 'streaming':
      return 'streaming'
    case 'waiting_user_action':
      return 'waiting_user_action'
    case 'failed':
      return 'failed'
    // VDFS 节点只有一套状态词汇（`VFDS_STATUS_*`）：`completed` 与「未标注」
    // 都落到 `active`。对消息而言两者渲染一致（都不是进行中），取 `completed`。
    case 'active':
      return 'completed'
    default:
      return undefined
  }
}

/** 节点视图 + 内容 → 前端 `ChatMessage`（结构取 attributes，正文取内容） */
export function messageFromNode(node: VdfsNode, content: string): ChatMessage {
  const attrs = node as unknown as Record<string, unknown>
  const msg: ChatMessage = {
    id: node.name,
    content,
  }
  const role = attrs.role
  if (typeof role === 'string') msg.role = role as ChatMessage['role']
  const type = attrs.type
  if (typeof type === 'string') msg.type = type as ChatMessage['type']
  const parentId = attrs.parent_id
  if (typeof parentId === 'string') msg.parent_id = parentId
  const seq = attrs.seq
  if (typeof seq === 'number') msg.seq = seq
  const error = attrs.error
  if (typeof error === 'string') msg.error = error
  const meta = attrs.meta
  if (meta && typeof meta === 'object') msg.meta = meta as Record<string, unknown>
  const status = messageStatusOf(node)
  if (status) msg.status = status
  if (node.updated_at) msg.timestamp = node.updated_at
  return msg
}

// ==================== 逐路径顺序链 ====================

/** path → 该路径上待执行任务的尾节点（保证同一路径严格串行） */
const chains = new Map<string, Promise<void>>()

/**
 * 把 `task` 接到 `key` 的顺序链尾部。
 *
 * 链上每一环都等前一环结束才开始——同一条消息的 `created` / `appended` /
 * `updated` 因此严格按到达顺序落地，不会因为某一步的异步回读而乱序。
 * 链尾执行完毕后自清理（只在仍是链尾时删，避免误删后续新任务）。
 */
export function enqueue(key: string, task: () => Promise<void>): Promise<void> {
  const prev = chains.get(key) ?? Promise.resolve()
  const next = prev.then(task, task)
  chains.set(key, next)
  void next.finally(() => {
    if (chains.get(key) === next) chains.delete(key)
  })
  return next
}

/** 测试用：等待某路径（或全部）的待办清空 */
export async function drain(key?: string): Promise<void> {
  if (key) {
    await (chains.get(key) ?? Promise.resolve())
    return
  }
  // 链尾可能因自清理而消失，循环到无待办为止（有界：每轮至少消费一条链）
  for (let i = 0; i < 64 && chains.size; i += 1) {
    await Promise.all([...chains.values()])
  }
}

// ==================== 变更 → store ====================

/**
 * 应用一条转写变更。
 *
 * 载荷优先级：`change.node` / `change.content`（后端附带）→ 回退 `stat` + `read`。
 * 回读失败即放弃这一条（不写入半成品），并由日志留痕——比写入残缺结构好。
 */
async function applyChange(change: VdfsChange, sessionId: string, messageId: string): Promise<void> {
  const store = useSessionsStore()

  if (change.change === VFDS_CHANGE_DELETED) {
    store.removeMessageById(sessionId, messageId)
    return
  }

  if (change.change === VFDS_CHANGE_APPENDED) {
    const delta = change.delta
    if (!delta) return
    // store.patchMessage 对非工具消息做**字符串追加**（与后端合并语义同源）
    store.patchMessage(sessionId, { id: messageId, content: delta })
    return
  }

  if (change.change !== VFDS_CHANGE_CREATED && change.change !== VFDS_CHANGE_UPDATED) return

  let node = change.node
  let content = change.content
  if (!node) {
    node = (await statVdfs(change.path)) ?? undefined
    if (!node) {
      logger.warn('[vdfs-transcript]', `节点视图缺失且 stat 失败：${change.path}`)
      return
    }
  }
  if (content === undefined) {
    const c = await readVdfs(change.path)
    content = c?.text ?? ''
  }

  const msg = messageFromNode(node, content)
  // 整条替换：`created` 是新项，`updated` 是权威全量（状态迁移 / 全量替换）
  store.putMessage(sessionId, msg)

  if (change.change === VFDS_CHANGE_CREATED || change.change === VFDS_CHANGE_UPDATED) {
    syncActivity(store, sessionId, msg)
  }
}

/**
 * 由**权威消息**派生会话活动文字 / 审批角标。
 *
 * 放在这里而不是事件处理函数里：事件只带路径与载荷，而活动文字要看
 * `status` + `type` + `name`——从完整消息派生是唯一不需要猜的位置。
 * 只在 `created` / `updated` 调用（`appended` 每帧都来，状态不会因此变化）。
 */
function syncActivity(
  store: ReturnType<typeof useSessionsStore>,
  sessionId: string,
  msg: ChatMessage,
): void {
  if (msg.status === 'streaming') {
    if (msg.type === 'reasoning') store.putStatus(sessionId, { activity: '正在思考…' })
    else if (msg.type === 'tool_call') store.putStatus(sessionId, { activity: `正在调用 ${msg.name || '工具'}…` })
    else store.putStatus(sessionId, { activity: '正在响应…' })
  } else if (msg.status === 'waiting_user_action') {
    store.putStatus(sessionId, { activity: '等待审批…', is_waiting_approval: true })
  } else if (msg.status === 'completed') {
    // 该条审批已了结（后端顺序处理工具，同一时刻仅一个 waiting_user_action）；
    // 若还有其他消息仍在等待，其 waiting_user_action 的更新会再次点亮。
    store.putStatus(sessionId, { activity: undefined, is_waiting_approval: false })
  } else if (msg.status === 'failed') {
    store.putStatus(sessionId, { activity: '失败', last_failed: true, is_waiting_approval: false })
  }
}

/** 清空某会话的转写（`deleted` 落在 `消息` 目录本身） */
function clearTranscript(sessionId: string): void {
  const store = useSessionsStore()
  for (const m of store.getSessionMessages(sessionId)) {
    store.removeMessageById(sessionId, m.id)
  }
}

/**
 * 启动转写同步（幂等）。
 *
 * 全局唯一、跨页面共享：多会话并发时**所有**会话的转写都要落 store，
 * 否则从 A 切到 B 再切回 A 时，A 在后台收到的消息会丢失。
 */
export function startTranscriptSync(): void {
  if (_unsubscribe || _G.__symTranscriptSyncStarted) {
    logger.warn('[vdfs-transcript]', 'already started')
    return
  }
  _G.__symTranscriptSyncStarted = true

  _unsubscribe = busSubscribe({ kind: VFDS_EVENT_KIND }, (busEvent: BusEvent) => {
    const change = busEvent.data?.data as VdfsChange | undefined
    if (!change || typeof change.path !== 'string') return

    const parsed = parseTranscriptPath(change.path)
    if (!parsed) return

    // 落在 `消息` 目录本身 = 整表清空
    if (!parsed.messageId) {
      if (change.change === VFDS_CHANGE_DELETED) {
        enqueue(change.path, async () => clearTranscript(parsed.sessionId))
      }
      return
    }

    const { sessionId, messageId } = parsed
    enqueue(change.path, () => applyChange(change, sessionId, messageId))
  })

  logger.info('[vdfs-transcript]', 'started')
}

/** 停止转写同步（一般不需要调用） */
export function stopTranscriptSync(): void {
  _G.__symTranscriptSyncStarted = false
  if (_unsubscribe) {
    _unsubscribe()
    _unsubscribe = null
    logger.info('[vdfs-transcript]', 'stopped')
  }
  chains.clear()
}

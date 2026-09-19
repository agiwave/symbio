/**
 * vdfsTranscriptSync — 会话转写的 VDFS 实时消费端
 *
 * ## 它替换了什么
 *
 * 会话转写原先是**专用协议 + 专用通道**：读走 `session/get_messages`，
 * 实时走 `kind = "session"` 的会话事件帧（`update` / `delete`）。
 * （前端那侧的帧类型描述 `ChatEventType` / `ChatEvent` 已随 `services/model.ts`
 * 一并退役——本模块改按**地址**分派后，前端不再需要这条通道的类型。）
 *
 * 现在它只是一张**列表**：地址 `<根>/session/<sid>/消息`，每一项是一条消息；
 * 流式输出是列表项的**追加型变更**（`appended`）。于是读路径与实时路径都归
 * VDFS，`kind = "vdfs"` 成为消息的唯一变更通道。
 *
 * ## 分派是「按地址」的，不是「按事件类型」的
 *
 * 入口只有一个：`sessionRouteOf(变更地址)`（`schemas/vdfs.ts`，纯函数、可单测）。
 * 它把地址解成 `session` / `messages` / `message` 三种目标，各自的处理互不知道
 * 对方存在——**没有 `switch (event.type)`**。
 *
 * 这不是风格问题：事件类型分派必须知道「之前发生过什么」（顺序一变就错），
 * 地址分派只认「这个地址现在的状态」。会话运行态因此可以整体搬到节点上，
 * 前端不再需要 `eventBus.replayBuffer` 那套防乱序缓冲。
 * 见 `symbio/src/plugins/session/docs/node-state-streaming.md` §5.1。
 *
 * 会话叶子（`<根>/session/<sid>`，即运行态的承载者）由 **store 自己**的
 * 订阅作用域处理（`stores/sessions.ts::applySessionNode`）——清单是它的状态，
 * 就地收敛零回读；本模块只负责转写。
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
 *
 * ## 删除有**两种**语义，不能合并
 *
 * | 变更 | 含义 | 本地动作 |
 * |---|---|---|
 * | `deleted` | **这一个**节点没了（工具调用恢复时删旧子节点） | 移除一项 |
 * | `truncated` | 该节点**及其之后全部**没了（删除某条消息） | 按 `seq` 取区间移除 |
 *
 * 两者的区别不是粒度而是**语义**：前者与顺序无关，后者描述的是一段区间。若都用
 * `deleted` 逐条下发，消费者既无法分辨，又要为删一条早期消息收下上百条通知——
 * 因此后端发一条 `truncated`，区间由本地按 `seq` 算（与后端 `drain(i..)` 同源）。
 */

import { subscribe as busSubscribe, type BusEvent } from './eventBus'
import { readVdfs, statVdfs } from './vdfs'
import { ensureVdfsSessionScheme, vdfsSessionScheme } from './vdfsScheme'
import {
  VDFS_CHANGE_APPENDED,
  VDFS_CHANGE_CREATED,
  VDFS_CHANGE_DELETED,
  VDFS_CHANGE_TRUNCATED,
  VDFS_CHANGE_UPDATED,
  VDFS_EVENT_KIND,
  VDFS_STATUS_ACTIVE,
  VDFS_STATUS_COMPLETED,
  VDFS_STATUS_FAILED,
  VDFS_STATUS_PENDING,
  VDFS_STATUS_STREAMING,
  VDFS_STATUS_WAITING_USER_ACTION,
  sessionRouteOf,
  type VdfsChange,
  type VdfsNode,
} from '@/schemas/vdfs'
import {
  MESSAGE_TYPE_REASONING,
  MESSAGE_TYPE_TOOL_CALL,
  type ChatMessage,
  type MessageStatus,
} from '@/schemas/chat_message'
import { logger } from '@/utils/logger'

/**
 * 转写落地目标（**依赖倒置**：service 不反向依赖 Pinia store）。
 *
 * 本模块是 service，若直接 `import { useSessionsStore }`，就把「UI 状态容器」
 * 拉进了服务层——service 从此无法脱离 Pinia 使用，测试也得先搭一个 store 实例。
 * 改为由调用方（应用外壳 `MainLayout`）注入真实 store；测试注入普通对象即可。
 *
 * 这里只声明本模块**真正用到**的那几个动作，而不是整个 store 的类型——
 * 窄接口既说明白「本模块对 store 的全部要求」，也让假实现几行就能写完。
 */
export interface TranscriptSink {
  putMessage(sessionId: string, msg: ChatMessage): void
  patchMessage(sessionId: string, patch: ChatMessage): void
  removeMessageById(sessionId: string, messageId: string): void
  /** 尾部截断：移除该消息**及其之后**的全部（区间由实现方按 `seq` 算） */
  removeFrom(sessionId: string, messageId: string): void
  putStatus(sessionId: string, partial: { activity?: string; is_waiting_approval?: boolean }): void
  getSessionMessages(sessionId: string): ChatMessage[]
}

/** 当前注入的落地目标（由 `startTranscriptSync` 设置） */
let _sink: TranscriptSink | null = null

let _unsubscribe: (() => void) | null = null

// HMR 守卫：模块热更新会重置 `_unsubscribe`，导致二次订阅 → 同一条消息被两个
// handler 各应用一次（流式文本叠字）。启动标记挂到 globalThis，跨模块重载幂等。
const _G = globalThis as typeof globalThis & { __symTranscriptSyncStarted?: boolean }

/**
 * 节点状态词 → 消息状态词。
 *
 * **原样透传**：后端 `message_status()` 已经把 `MessageStatus` 的序列化名直接写成
 * 节点 `status`，前端不再做任何「读回」式还原。`active` 只作为**旧数据的兜底别名**
 * 保留（历史上 `completed` 与「未标注」都被映射成 `active`），遇到即按已结束处理。
 */
function messageStatusOf(node: VdfsNode): MessageStatus | undefined {
  switch (node.status) {
    case VDFS_STATUS_PENDING:
    case VDFS_STATUS_STREAMING:
    case VDFS_STATUS_WAITING_USER_ACTION:
    case VDFS_STATUS_COMPLETED:
    case VDFS_STATUS_FAILED:
      return node.status
    case VDFS_STATUS_ACTIVE:
      // 旧数据别名：仅用于兼容已落库的历史节点，新节点不会再出现这个值
      return VDFS_STATUS_COMPLETED
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
  const store = _sink
  if (!store) return

  if (change.change === VDFS_CHANGE_TRUNCATED) {
    // 尾部截断：「该节点及其后全部」都没了。**由本地算区间**，不等后端把被删的
    // 每一条逐个通知回来（那是 N 次通知，且漏一条就永久残留一个后端不存在的节点）。
    // 判据是 store 里的 `seq` 顺序，与后端 `messages.drain(i..)` 同源。
    store.removeFrom(sessionId, messageId)
    return
  }

  if (change.change === VDFS_CHANGE_DELETED) {
    // 逐节点删除（工具调用恢复时删掉旧的 pending/failed 子节点）——**只删这一个**，
    // 与顺序无关。这与 `truncated` 的分界是语义上的，不是粒度上的。
    store.removeMessageById(sessionId, messageId)
    return
  }

  if (change.change === VDFS_CHANGE_APPENDED) {
    const delta = change.delta
    if (!delta) return
    // store.patchMessage 对非工具消息做**字符串追加**（与后端合并语义同源）
    store.patchMessage(sessionId, { id: messageId, content: delta })
    return
  }

  if (change.change !== VDFS_CHANGE_CREATED && change.change !== VDFS_CHANGE_UPDATED) return

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

  if (change.change === VDFS_CHANGE_CREATED || change.change === VDFS_CHANGE_UPDATED) {
    syncActivity(store, sessionId, msg)
  }
}

/**
 * 由**权威消息节点**派生会话活动文字 / 审批角标。
 *
 * 放在这里而不是事件处理函数里：事件只带路径与载荷，而活动文字要看
 * `status` + `type` + `name`——从完整节点派生是唯一不需要猜的位置。
 * 只在 `created` / `updated` 调用（`appended` 每帧都来，状态不会因此变化）。
 *
 * 全部由**节点状态**决定，不记录「上一条事件是什么」：
 * 同一份节点表必然派生出同一份文字（§5.2「派生是纯函数」）。
 */
function syncActivity(
  store: TranscriptSink,
  sessionId: string,
  msg: ChatMessage,
): void {
  if (msg.status === VDFS_STATUS_STREAMING) {
    if (msg.type === MESSAGE_TYPE_REASONING) store.putStatus(sessionId, { activity: '正在思考…' })
    else if (msg.type === MESSAGE_TYPE_TOOL_CALL)
      store.putStatus(sessionId, { activity: `正在调用 ${msg.name || '工具'}…` })
    else store.putStatus(sessionId, { activity: '正在响应…' })
  } else if (msg.status === VDFS_STATUS_WAITING_USER_ACTION) {
    store.putStatus(sessionId, { activity: '等待审批…', is_waiting_approval: true })
  } else if (msg.status === VDFS_STATUS_COMPLETED) {
    // 该条审批已了结（后端顺序处理工具，同一时刻仅一个 waiting_user_action）；
    // 若还有其他消息仍在等待，其 waiting_user_action 的更新会再次点亮。
    store.putStatus(sessionId, { activity: undefined, is_waiting_approval: false })
  }
  // `failed` 不在此处理：「这一轮失败了」是**会话节点**的状态
  // （`status = failed` + `attributes.error`），由 `applySessionNode` 一处落定。
  // 在消息层再写一次，就是同一份真相的第二种写法（且与节点状态可能不一致）。
}

/** 清空某会话的转写（`deleted` 落在 `消息` 目录本身） */
function clearTranscript(sessionId: string): void {
  const store = _sink
  if (!store) return
  for (const m of store.getSessionMessages(sessionId)) {
    store.removeMessageById(sessionId, m.id)
  }
}

/**
 * 启动转写同步（幂等）。
 *
 * 全局唯一、跨页面共享：多会话并发时**所有**会话的转写都要落 store，
 * 否则从 A 切到 B 再切回 A 时，A 在后台收到的消息会丢失。
 *
 * @param sink 转写落地目标。**必须显式注入**（生产由应用外壳传 `useSessionsStore()`，
 *   测试可传普通对象）——本模块是 service，不认识 Pinia。
 */
export function startTranscriptSync(sink: TranscriptSink): void {
  if (_unsubscribe || _G.__symTranscriptSyncStarted) {
    logger.warn('[vdfs-transcript]', 'already started')
    return
  }
  _G.__symTranscriptSyncStarted = true
  _sink = sink

  // 自己也要触发一次解析：本函数在 `refreshList` **之前**启动（见 MainLayout），
  // 而 refreshList 才是「有会话 ⇒ 推导得出转写段」的时机。若宿主不跑 refreshList，
  // 这里不补一次就会永久停在引导窗口里（转写变更一律被跳过）。
  // 零会话时会失败（转写段推导不出来）——那是预期，随后由 refreshList /
  // createSession 补上，故只留痕不抛。
  void ensureVdfsSessionScheme().catch((e: unknown) =>
    logger.warn('[vdfs-transcript]', '地址方案解析未完成（等清单到手后会补一次）', e),
  )

  _unsubscribe = busSubscribe({ kind: VDFS_EVENT_KIND }, (busEvent: BusEvent) => {
    const change = busEvent.data?.data as VdfsChange | undefined
    if (!change || typeof change.path !== 'string') return

    // **按地址分派**（唯一入口，纯函数）。会话叶子归 store 自己的作用域，
    // 这里只认转写；其余地址返回 null，直接跳过。
    //
    // 方案未解析完（启动引导窗口）时 `sessionRouteOf` 一律返回 null —— 此时
    // 也没有任何会话被展示，跳过是安全的；解析完成后自然开始收敛。
    const route = sessionRouteOf(vdfsSessionScheme(), change.path)
    if (!route) return

    if (route.target === 'messages') {
      // 落在 `消息` 目录本身 = 整表清空
      if (change.change === VDFS_CHANGE_DELETED) {
        enqueue(change.path, async () => clearTranscript(route.sessionId))
      }
      return
    }
    if (route.target !== 'message') return

    const { sessionId, messageId } = route
    enqueue(change.path, () => applyChange(change, sessionId, messageId))
  })

  logger.info('[vdfs-transcript]', 'started')
}

/** 停止转写同步（一般不需要调用） */
export function stopTranscriptSync(): void {
  _G.__symTranscriptSyncStarted = false
  _sink = null
  if (_unsubscribe) {
    _unsubscribe()
    _unsubscribe = null
    logger.info('[vdfs-transcript]', 'stopped')
  }
  chains.clear()
}

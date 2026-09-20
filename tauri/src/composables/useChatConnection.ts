import { shallowRef, computed, type ComputedRef, type InjectionKey } from 'vue'
import { callPlugin } from '@/services/plugin'
import { messageTextOf, type ChatMessage, type ResumeAction } from '@/schemas/chat_message'
import { logger } from '@/utils/logger'
import { useSessionsStore } from '@/stores/sessions'
import { isBlankContentNode, isInProgressMessage } from '@/stores/sessionTranscript'
import { VDFS_STATUS_ACTIVE, VDFS_STATUS_FAILED, VDFS_STATUS_WORKING } from '@/schemas/vdfs'
import { CHAT_SEND, CHAT_ABORT } from '@/constants/pluginPaths'
import { isWaitingStatus } from '@/registry/messageTypes'

export interface UseChatConnectionOptions {
  sessionId: string
  onSendComplete?: () => void
}

/**
 * 「会话恢复」注入键。
 *
 * 用它取代 `'resume'` 字符串键：字符串键拼错时 `inject` 静默返回 undefined，
 * 只能在运行期以一个「点了没反应」的形式暴露；改用 symbol + `InjectionKey`
 * 后，键名本身是类型的一部分，提供 / 注入两侧的签名由编译器对齐。
 *
 * 跨层传递仍用 provide/inject 而非逐层 props：恢复入口在**消息树的任意深度**
 * （子智能体里的 user_prompt、失败 ToolCall 的重试按钮），逐层透传要把一个
 * 回调穿过多层与本组件无关的容器。要收敛的是**契约的类型**，不是传递方式。
 */
export const RESUME_KEY: InjectionKey<(payload: ResumePayload) => void> = Symbol('chat-resume')

/** 会话恢复载荷（retry_turn/retry_compaction/retry/approve/reject/supply/answer 统一接口） */
export interface ResumePayload {
  /** 目标消息 ID（恢复锚点）
   *  - retry_turn：指向 Failed Turn（msg_type=Turn）
   *  - retry_compaction：指向 Failed 压缩节点（msg_type=Compression）
   *  - 其他 action：指向 ToolCall 父节点（msg_type=ToolCall） */
  targetId: string
  /** 恢复动作——词表在 `schemas/chat_message.ts`（后端 `ResumeAction` 的镜像，
   *  由 `scripts/protocol-mirror-audit.mjs` 的 C 组守着）。此处**不**再抄一份
   *  字面量联合：抄一份就等于多一份真相，改一个词要记得改两处。 */
  action: ResumeAction
  /** supply 时的补充参数（与原 args 浅合并） */
  args?: unknown
  /** reject 时的拒绝原因 */
  reason?: string
  /** answer（ask_user）时的答案对象 */
  answer?: unknown
  /** 目标会话 id（子智能体的工具调用需发回子会话） */
  targetSessionId?: string
}

export interface UseChatConnectionReturn {
  isLoading: ComputedRef<boolean>
  isWaitingApproval: ComputedRef<boolean>
  isConnected: ComputedRef<boolean>
  messageTree: ComputedRef<ChatMessage[]>
  /** 发送一条消息。会话参数（智能体 / 模型 / 模式 / 风险等级）由后端按
   *  `session.metadata` 解析——选择动作统一经级联选项机制落库，故此处不透传。 */
  send: (message: ChatMessage) => void
  abort: () => void
  removeMessage: (messageId: string) => void
  resume: (payload: ResumePayload) => void
}

/**
 * 多会话无状态 chat connection（v2 — 无 store 写入版）
 *
 * ## 设计（重要变化）
 *
 * - **所有状态写入由全局消费端负责**，本 composable 不订阅任何通道：
 *   消息与转写走 `services/vdfsTranscriptSync`（`kind = "vdfs"` 变更），
 *   会话运行态走 `stores/sessions.ts::applySessionNode`（同一个频道的会话叶子地址）。
 *   本 composable 只在**发起动作**时做乐观置位（随后被节点状态覆盖）。
 * - 组件订阅此 hook 只是为了：
 *   1. 拿到 `send` / `abort` 两个 one-off 命令
 *   2. 拿到派生自 store 的 `isLoading` / `isWaitingApproval` 给 UI
 *   3. 拿到 `messageTree`（直接来自 store）
 *
 * ## 多实例共存
 *
 * - 每个 `ModelChatPanel` 调一次 `useChatConnection`，都只读 store，互不干扰
 * - 切换会话时 `:key="activeId"` 触发组件 remount，旧的 useChatConnection 销毁，
 *   新的 useChatConnection 创建；store 状态保持不变，UI 立即显示
 * - 多个 session 并发时，消费端把每个会话的节点状态写入 store，
 *   缩略卡 SessionCard 通过 store.sessionStatuses 实时刷新
 *
 * ## 不再需要的旧机制
 *
 * - ❌ `messagesMap`（per-instance 缓存）—— store 是唯一权威
 * - ❌ `flushMessagesToStore`（onBeforeUnmount 写回）—— store 自动维护
 * - ❌ `onUpdateMessages` 回调—— store 直接读
 * - ❌ `kind = "session"` 事件订阅—— 状态是节点属性，不是事件（S20）
 *
 * ## `removeMessage` / `markRemoved`
 *
 * - 这两个保留在 composable 内，因为 retry 时的临时移除是 UI 局部语义
 * - 用 composable 内的 `localOverrides` 过滤，不污染 store
 * - 仅在 retry 流程（短窗口）内有效，组件销毁后失效
 */
export function useChatConnection(options: UseChatConnectionOptions): UseChatConnectionReturn {
  const store = useSessionsStore()

  const { onSendComplete } = options

  // 用于 `removeMessage`（本地用户主动删除一条消息的 UI 交互）
  const localOverrides = shallowRef<Record<string, Set<string>>>({}) // sessionId -> set of removed msgIds
  function getRemovedSet(): Set<string> {
    return localOverrides.value[options.sessionId] || new Set()
  }
  function markRemoved(messageId: string) {
    const next = { ...localOverrides.value }
    const cur = new Set(next[options.sessionId] || new Set())
    cur.add(messageId)
    next[options.sessionId] = cur
    localOverrides.value = next
  }

  // 从 store 派生 messageTree（按 parent_id 组织成树）
  //
  // 流式期间的性能关键：每个 token 都会触发本 computed 重算。若每次都
  // `{ ...msg }` 新建全部节点对象，keyed v-for 的 props 引用全变，整棵
  // 消息列表每帧全量重渲染。因此维护一个「上一轮节点」缓存：内容签名
  // 未变的消息直接复用旧节点对象，Vue 只重渲染真正变化的那一个节点
  // （通常是流式末端节点）。签名覆盖影响渲染的字段（content/status/
  // parent/children 身份），会话切换时整体失效。
  let nodeCacheSession = ''
  const nodeCache = new Map<string, ChatMessage>()
  function nodeSignature(
    msg: ChatMessage,
    parentId: string | undefined,
    childIds: string[] | undefined,
  ): string {
    return `${msg.status ?? ''}|${parentId ?? ''}|${childIds ? childIds.join(',') : ''}|${messageTextOf(msg.content)}`
  }
  function reuseNode(
    msg: ChatMessage,
    parentId: string | undefined,
    childIds: string[] | undefined,
  ): ChatMessage {
    const sig = nodeSignature(msg, parentId, childIds)
    const cached = nodeCache.get(msg.id)
    if (cached && (cached as { __sig?: string }).__sig === sig) return cached
    const node: ChatMessage = { ...msg }
    ;(node as { __sig?: string }).__sig = sig
    nodeCache.set(msg.id, node)
    return node
  }

  const messageTree = computed<ChatMessage[]>(() => {
    const sessionId = options.sessionId
    if (!sessionId) return []
    if (nodeCacheSession !== sessionId) {
      nodeCacheSession = sessionId
      nodeCache.clear()
    }
    const all = store.getSessionMessages(sessionId)
    const removed = getRemovedSet()
    const filtered = all.filter(m => !removed.has(m.id))

    // 识别「空内容叶子节点」：流模式下后端会先发一个 content 为空（如 "\n\n"）的
    // Text / Reasoning 节点（例如 reasoning 与 tool_call 之间的占位空节点），
    // 该节点并不持久化，但前端会短暂收到；渲染时直接过滤掉，避免对话流出现空白块。
    // 仅过滤"无子节点"的节点——带子节点的节点是容器（Turn/ToolCall），须保留。
    // 「空内容」判据走 `sessionTranscript.isBlankContentNode`（唯一实现）：
    // 等待骨架的兜底判定也要用它，两处不能各写一份。
    const rawChildren: Record<string, ChatMessage[]> = {}
    filtered.forEach(msg => {
      if (msg.parent_id) {
        if (!rawChildren[msg.parent_id]) rawChildren[msg.parent_id] = []
        rawChildren[msg.parent_id].push(msg)
      }
    })
    const parentsWithChildren = new Set(
      Object.keys(rawChildren).filter(pid => rawChildren[pid].length > 0),
    )
    const isEmptyLeaf = (msg: ChatMessage) =>
      !parentsWithChildren.has(msg.id) && isBlankContentNode(msg)

    const visible = filtered.filter(m => !isEmptyLeaf(m))

    const childrenMap: Record<string, ChatMessage[]> = {}
    visible.forEach(msg => {
      if (msg.parent_id) {
        if (!childrenMap[msg.parent_id]) childrenMap[msg.parent_id] = []
        childrenMap[msg.parent_id].push(msg)
      }
    })

    // 判「是不是根」原来是 `visible.find(...)` —— 每个节点都要线性扫一遍，
    // 整棵树 O(n²)；长会话（数千条）在流式期间每帧都要付一次。换成一次性建
    // 好的 id 集合，查表 O(1)。
    const visibleIds = new Set(visible.map(m => m.id))
    const rootMessages = visible.filter(msg => {
      return !msg.parent_id || !visibleIds.has(msg.parent_id)
    })

    rootMessages.sort((a, b) => {
      const sa = a.seq ?? a.timestamp ?? 0
      const sb = b.seq ?? b.timestamp ?? 0
      return sa - sb
    })
    Object.values(childrenMap).forEach(children => {
      children.sort((a, b) => {
        const sa = a.seq ?? a.timestamp ?? 0
        const sb = b.seq ?? b.timestamp ?? 0
        return sa - sb
      })
    })

    const buildNode = (
      msg: ChatMessage,
      parentId?: string,
      childIds?: string[],
      parentNode?: ChatMessage,
    ): ChatMessage => {
      const node = reuseNode(msg, parentId, childIds)
      // parent 反向引用不在缓存签名里（签名只含 id），每次重挂：
      // 升为根（父被删）时要清掉旧引用，避免渲染树上溯到已移除的节点
      if (parentNode) node.parent = parentNode
      else delete node.parent
      if (childIds) {
        node.children = childrenMap[node.id].map(child => {
          const grandkids = childrenMap[child.id]
          return buildNode(child, node.id, grandkids ? grandkids.map(c => c.id) : undefined, node)
        })
      } else {
        // 子节点全部消失（被删/被过滤）时清掉旧引用，避免缓存节点渲染幽灵子树
        delete node.children
      }
      return node
    }
    return rootMessages.map(msg => {
      const kids = childrenMap[msg.id]
      return buildNode(msg, undefined, kids ? kids.map(c => c.id) : undefined)
    })
  })

  // 从 store 派生 isLoading / isWaitingApproval
  const isLoading = computed(() => {
    const sid = options.sessionId
    if (!sid) return false
    return store.isSessionWorking(sid)
  })

  const isWaitingApproval = computed(() => {
    const sid = options.sessionId
    if (!sid) return false
    const msgs = store.getSessionMessages(sid)
    return msgs.some(msg => isWaitingStatus(msg.status))
  })

  async function send(msg: ChatMessage) {
    const outgoing: ChatMessage = { ...msg }
    const sid = options.sessionId
    logger.info('useChatConnection', `[${sid}] Sending message`)

    // 立即把用户消息写入会话 store（乐观更新，避免后端首帧覆盖不到）
    if (outgoing.id) {
      store.putMessage(sid, outgoing)
    }
    // 立即置为 working（让 UI 立即反映 send 已经发出）；
    // 同时清空会话级错误：新一轮交互开始，上一次失败不再"最新"。
    // 这是**乐观置位**，随后会被后端会话节点的权威状态覆盖（零回读）。
    store.putStatus(sid, { status: VDFS_STATUS_WORKING, activity: '处理中…', outcome: undefined })
    store.setSessionError(sid, null)
    store.setSessionStatus(sid, VDFS_STATUS_WORKING)

    // 运行模式：取会话记忆值（= `session.metadata.mode` 的本地镜像，选择经选项机制落库）。
    // 后端 orchestrator.handle_chat_send_oneoff 据此把 MODE 写入 chat ctx，
    // ask_user / emit_confirm_prompt 等据此决定"产 user_prompt 节点"还是"返回友好错误"。
    const mode = store.getSessionMode(sid)

    // 执行风险等级：与 mode 同级别的回退链——会话记忆值 > 'medium'。
    // 后端 orchestrator 据此把 RISK_LEVEL 写入 chat ctx，SecurityPolicy 三方法据此覆盖全局阈值。
    const riskLevel = store.getSessionRiskLevel(sid)

    try {
      await callPlugin(CHAT_SEND, {
        session_id: sid,
        // 智能体 / 模型 provider 不在请求中透传：后端按 `session.metadata`
        // （agent_id / provider_id）回退取值，选择动作统一经级联选项机制落库。
        message: outgoing,
        mode,
        risk_level: riskLevel
      }, 15000, {
        // 用会话自身的 workdir（后端 orchestrator 已用 session.metadata.workdir 兜底）
        workdir: store.getSessionWorkdir(sid) ?? '',
        session_id: sid
      })
    } catch (err: any) {
      const errText = `Send failed: ${err.message || String(err)}`
      // 请求本身失败（transport 级）：收敛为「以错误结束」这个状态值。
      // 后端若已开始工作，它的节点视图随后会覆盖这里——权威在服务端。
      store.putStatus(sid, { status: VDFS_STATUS_FAILED, activity: '错误', outcome: 'failed' })
      store.setSessionStatus(sid, VDFS_STATUS_FAILED)

      // 仅当没有任何 streaming/等待消息承载错误时，才落"会话级错误状态"（而非注入错误节点）：
      // 否则助手根级 Turn 会在 bus Error 事件中带上错误，避免"根级节点 + 会话级"重复报错。
      // 错误作为状态（不是消息树里的节点）展示在会话级错误条，许可重试（重新发送最后一条用户消息）。
      const hasStreaming = store.getSessionMessages(sid).some(isInProgressMessage)
      if (!hasStreaming) {
        store.setSessionError(sid, errText)
      }

      logger.error('useChatConnection', 'Failed to send', err)
      onSendComplete?.()
    }
  }

  function abort() {
    const sid = options.sessionId
    logger.info('useChatConnection', `[${sid}] aborting`)

    callPlugin(CHAT_ABORT, {
      session_id: sid
    }, 5000, {
      workdir: store.getSessionWorkdir(sid) ?? '',
      session_id: sid
    }).catch(err => {
      logger.error('useChatConnection', 'Failed to abort:', err)
    })
  }

  function removeMessage(messageId: string) { markRemoved(messageId) }

  /**
   * 会话恢复（retry_turn/retry_compaction/retry/approve/reject/supply/answer）。
   *
   * 内部走统一 `CHAT_SEND` 接口的 `resume` 分支（与发送用户消息共用同一端点）。
   * 后端语义（删除-重建模式）：
   * - retry_turn：删除 Failed Turn 及其所有子孙节点 → 重新走 LLM 请求
   * - retry_compaction：删除 Failed 压缩节点 → 重新执行一次上下文压缩
   *   （**不动历史**：压缩失败从不丢消息，重试只是再试一次 LLM 摘要）
   * - retry/approve/reject/supply/answer：删除旧子节点 → 重新执行工具或生成结果 → 创建新子节点
   *
   * 前端不在此处构造新消息——后端经 VDFS 广播变更：`deleted`（删旧节点）+
   * `updated` / `appended`（写新节点 + 父节点状态更新），由 `vdfsTranscriptSync`
   * 与 `sessions.applySessionNode` 就地收敛。
   *
   * 会话参数：智能体 / 模型 provider 由后端 `resolve_session_params` 从
   * `session.metadata` 回退解析；`mode` / `risk_level` 从会话记忆（metadata 的本地镜像）
   * 随请求携带，由后端写入 chat ctx，continuation chat_loop 通过 `ctx.fork()` 继承。
   */
  async function resume(payload: ResumePayload) {
    const targetSid = payload.targetSessionId || options.sessionId
    const sid = options.sessionId
    logger.info(
      'useChatConnection',
      `[${sid}] resume: action=${payload.action} targetId=${payload.targetId}${targetSid !== sid ? ` → ${targetSid}` : ''}`,
    )

    // 立即置为 working（让 UI 立即反映 resume 已经发出）；
    // 同一会话才切 working 状态，避免跨会话工具调用误改父会话状态。
    if (targetSid === sid) {
      store.putStatus(sid, { status: VDFS_STATUS_WORKING, activity: '处理中…', outcome: undefined })
      store.setSessionError(sid, null)
      store.setSessionStatus(sid, VDFS_STATUS_WORKING)
    }

    // 运行模式 / 风险等级：与 send 同级别的回退链——会话记忆值 > 默认值。
    // 后端 orchestrator.resolve_session_params 据此把 MODE / RISK_LEVEL 写入 chat ctx，
    // continuation chat_loop 通过 ctx.fork() 继承。
    const mode = store.getSessionMode(targetSid)
    const riskLevel = store.getSessionRiskLevel(targetSid)

    try {
      const resp = await callPlugin<{ status?: string }>(
        CHAT_SEND,
        {
          session_id: targetSid,
          // 智能体 / 模型 provider 不在此透传：后端 resolve_session_params 按
          // `session.metadata`（agent_id / provider_id）回退取值。
          mode,
          risk_level: riskLevel,
          resume: {
            target_id: payload.targetId,
            action: payload.action,
            args: payload.args ?? null,
            reason: payload.reason ?? null,
            answer: payload.answer ?? null,
          },
        },
        15000,
        {
          workdir: store.getSessionWorkdir(targetSid) ?? '',
          session_id: targetSid,
        },
      )
      // 防御性处理：后端 resume 分支有 is_working 守卫，忙碌时返回 session_busy
      // （HTTP 200 成功响应，不会进 catch）。此时必须复位前端 working 状态，
      // 否则 UI 会卡在"处理中…"且重试按钮不消失。
      if (resp?.status === 'session_busy') {
        logger.warn('useChatConnection', `[${sid}] resume rejected: session_busy`)
        if (targetSid === sid) {
          // 「会话忙」是**当前事实**（空闲），不是失败：回落 `active`，
          // 不置 `failed`——否则会显示一个并不存在的失败终态。
          store.putStatus(sid, { status: VDFS_STATUS_ACTIVE, activity: '会话忙' })
          store.setSessionStatus(sid, VDFS_STATUS_ACTIVE)
        }
      }
    } catch (err: any) {
      logger.error('useChatConnection', 'Failed to resume:', err)
      if (targetSid === sid) {
        store.putStatus(sid, { status: VDFS_STATUS_FAILED, activity: '错误', outcome: 'failed' })
        store.setSessionStatus(sid, VDFS_STATUS_FAILED)
      }
    }
  }

  return {
    isLoading,
    isWaitingApproval,
    isConnected: computed(() => true), // 始终视为已连接（状态由全局消费端收敛）
    messageTree,
    send,
    abort,
    removeMessage,
    resume,
  }
}
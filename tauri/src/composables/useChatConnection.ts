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
   *  `session.metadata` 解析——选择动作统一经会话选项栏落库，故此处不透传。 */
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
 * - **所有状态写入由全局消费端负责**，本 composable 不订阅任何通道：消息与会话
 *   运行态都走 `stores/sessionTranscriptSync`（VDFS 变更消费端——两类地址共用
 *   一个 `seq` 计数器），落地口是 `stores/sessions.ts`（`applyTranscriptMessages` /
 *   `applySessionState`）。本 composable 只在**发起动作**时做乐观置位
 *   （随后被节点状态覆盖）。
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
  // 消息列表每帧全量重渲染。因此维护一个「上一轮节点」缓存：**子树**未变的
  // 消息直接复用旧节点对象，Vue 只重渲染真正变化的那一条路径。
  //
  // ## 签名必须覆盖**整棵子树**，而不是「自身字段 + 直接子节点 id」
  //
  // 复用节点对象的唯一目的，是让下一次 v-for 的 `:node` 保持**同一个引用**：
  // 引用不变 ⇒ Vue 判定 props 未变 ⇒ 整个组件子树跳过更新
  // （`hasPropsChanged`）。这条优化成立的**前提**是「引用不变 ⇒ 渲染结果不变」，
  // 而渲染结果取决于**整棵子树**——容器节点（Turn / ToolCall）自身无正文，
  // 它画出来的每一块内容都在子节点里。
  //
  // 旧签名只覆盖自身字段与直接子 id（`childIds.join(',')`），于是「**已有**子节点的
  // 正文增长」不改变任何祖先的签名：祖先对象原样复用 → Vue 跳过整棵子树 →
  // 增量**永远进不了 DOM**。界面只在「结构变化（新增/删除子节点）或状态迁移」
  // 那一刻跳一下，把此前积攒的内容一次补齐——实测症状正是
  // 「流式期间正文/思考不动，正文结束后突然完整；手动刷新也完整」。
  //
  // `append` 窄帧（正文 / 思考 / 工具参数 / 工具响应增长的唯一通道）恰恰只改
  // 叶子内容，不碰任何容器字段，因此这条缺陷会命中**每一帧**。
  //
  // 所以签名按子树归纳：自身字段 + 每个子节点的签名（子节点签名同样含它自己的
  // 子树）。任一后代变化 ⇒ 沿途祖先签名变化 ⇒ 这条路径换新对象 ⇒ 只重渲染这条
  // 路径；其余子树引用不变，照旧跳过（优化不丢）。会话切换时整体失效。
  let nodeCacheSession = ''
  const nodeCache = new Map<string, ChatMessage>()
  function nodeSignature(
    msg: ChatMessage,
    parentId: string | undefined,
    children: ChatMessage[] | undefined,
  ): string {
    // type / name 必须进签名：同一条消息在流式过程中可能出现 type 迁移
    // （reasoning → text）或工具名补全，漏掉它们会让缓存的旧节点对象
    // 被复用，渲染器按旧 type 分派（Reason 块不渲染 / 工具名缺失）。
    // id 也在签名里：子节点签名以其 id 打头，「换成一个同内容的子节点」同样
    // 必须使祖先签名变化（否则被换掉的子节点在界面上永远不出现）。
    //
    // `meta` / `error` 同样必须进签名——**它们都会改变渲染结果**，而帧可以
    // 「只改这两者、不动其它字段」：
    // - `meta.started_at`（工具开始执行那一刻写入）：它只随一条状态帧到达，
    //   该帧不改内容也不改状态 ⇒ 签名不变 ⇒ 缓存节点被复用 ⇒ 前端读到的
    //   `meta` 仍是旧值 ⇒ **「运行中 12s」的秒数永远不出现**（工具行只显示
    //   「运行中」，用户无从判断是还在跑还是卡住了），直到下一次真实内容变化
    //   才补上——那时工具往往已经结束；
    // - `meta.recoverable` / `failure_kind` / `prompt` / `success`：分别决定
    //   重试入口、兜底文案、待响应表单、结果成败，同样都由「只改 meta」的帧
    //   下发；
    // - `error`：失败原因文本。
    // meta 是后端 flatten 出来的浅 JSON，序列化成本远低于一次错误的全量重渲染。
    const meta = msg.meta === undefined ? '' : JSON.stringify(msg.meta)
    const own = `${msg.id}|${msg.type ?? ''}|${msg.name ?? ''}|${msg.status ?? ''}|${parentId ?? ''}|${messageTextOf(msg.content)}|${msg.error ?? ''}|${meta}`
    const sub = children
      ? children.map((child) => (child as { __sig?: string }).__sig ?? '').join(',')
      : ''
    return `${own}#${sub}`
  }
  function reuseNode(
    msg: ChatMessage,
    parentId: string | undefined,
    children: ChatMessage[] | undefined,
  ): ChatMessage {
    const sig = nodeSignature(msg, parentId, children)
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

    // **先建子树、再定自身**：自身的签名要覆盖子节点的签名（见 `nodeSignature`），
    // 所以子节点必须先构造出来。
    const buildNode = (
      msg: ChatMessage,
      parentId?: string,
      parentNode?: ChatMessage,
    ): ChatMessage => {
      const kids = childrenMap[msg.id]
      const children = kids ? kids.map((child) => buildNode(child, msg.id)) : undefined
      const node = reuseNode(msg, parentId, children)
      // parent 反向引用不参与缓存签名（签名只含自身与子树），每次重挂：
      // 升为根（父被删）时要清掉旧引用，避免渲染树上溯到已移除的节点
      if (parentNode) node.parent = parentNode
      else delete node.parent
      if (children) {
        node.children = children
        // 子节点的 parent 指回本节点：本节点可能是**新造**的对象（子树有变化），
        // 旧的反向引用必须换掉，否则子节点上溯到的是上一轮的祖先。
        for (const child of children) child.parent = node
      } else {
        // 子节点全部消失（被删/被过滤）时清掉旧引用，避免缓存节点渲染幽灵子树
        delete node.children
      }
      return node
    }
    return rootMessages.map((msg) => buildNode(msg))
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

    // 立即把用户消息写入会话 store（乐观更新，避免后端首帧覆盖不到）。
    // 落地走与转写实时流**同一条**帧应用路径（`applyTranscriptMessage`）——
    // 乐观回显与流式帧是同一个动作：把一条消息合并进本地图。
    if (outgoing.id) {
      store.applyTranscriptMessage(sid, outgoing)
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
        // （agent_id / provider_id）回退取值，选择动作统一经会话选项栏落库。
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
   * 前端不在此处构造新消息——后端经转写流广播（帧语义全在字段上：
   * `status = removed` 删旧节点，`content` / `delta` 写新节点与父节点状态），
   * 由 `sessionTranscriptSync` 与 `sessions.applyTranscriptMessages` 就地收敛。
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
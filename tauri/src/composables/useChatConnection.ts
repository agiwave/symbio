import { shallowRef, computed, type ComputedRef } from 'vue'
import { callPlugin } from '@/services/plugin'
import type { ChatMessage } from '@/services/model'
import { logger } from '@/utils/logger'
import { getGlobalWorkdir } from '@/services/plugin'
import { useSessionsStore } from '@/stores/sessions'
import { CHAT_SEND, CHAT_ABORT } from '@/constants/pluginPaths'

export interface UseChatConnectionOptions {
  sessionId: string
  onSendComplete?: () => void
}

export interface SendOptions {
  /** 发往的目标会话 id（子智能体的 user_prompt 需发回子会话，而非当前会话） */
  targetSessionId?: string
  /** 本次发送的运行模式：auto（无人值守）/ interactive（默认，会话流内可交互）。
   *  为空时回退 `store.getSessionMode(targetSessionId)`（再回退默认 'interactive'）。 */
  mode?: 'auto' | 'interactive'
  /** 本次发送的执行风险等级阈值：low / medium / high。
   *  为空时回退 `store.getSessionRiskLevel(targetSessionId)`（再回退默认 'medium'）。
   *  与 agent_id / provider_id / mode 同级别：随 chat_send 传输，后端 orchestrator 写入 ctx[RISK_LEVEL]。 */
  riskLevel?: 'low' | 'medium' | 'high'
}

/** 会话恢复载荷（retry_turn/retry/approve/reject/supply/answer 统一接口） */
export interface ResumePayload {
  /** 目标消息 ID（Failed Turn 或 ToolCall，恢复锚点）
   *  - retry_turn：指向 Failed Turn（msg_type=Turn）
   *  - 其他 action：指向 ToolCall 父节点（msg_type=ToolCall） */
  targetId: string
  action: 'retry_turn' | 'retry' | 'approve' | 'reject' | 'supply' | 'answer'
  /** supply 时的补充参数（与原 args 浅合并） */
  args?: unknown
  /** reject 时的拒绝原因 */
  reason?: string
  /** answer（ask_user）时的答案对象 */
  answer?: unknown
  /** 目标会话 id（子智能体的工具调用需发回子会话） */
  targetSessionId?: string
  /** 选定的 Model Provider ID；与 send 同级别，确保 resume 时也使用当前窗口选择的 Provider。
   *  为空时后端从会话 metadata 回退。 */
  providerId?: string
}

export interface UseChatConnectionReturn {
  isLoading: ComputedRef<boolean>
  isWaitingApproval: ComputedRef<boolean>
  isConnected: ComputedRef<boolean>
  messageTree: ComputedRef<ChatMessage[]>
  send: (message: ChatMessage, agentId: string, providerId?: string, opts?: SendOptions) => void
  abort: () => void
  removeMessage: (messageId: string) => void
  resume: (payload: ResumePayload) => void
}

/**
 * 多会话无状态 chat connection（v2 — 无 store 写入版）
 *
 * ## 设计（重要变化）
 *
 * - **所有 bus 事件写入由全局 `SessionBusWatcher` 负责**，本 composable 不再订阅 bus 也不写 store
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
 * - 多个 session 并发时，SessionBusWatcher 把每个 session 的事件写入 store，
 *   缩略卡 SessionCard 通过 store.sessionStatuses 实时刷新
 *
 * ## 不再需要的旧机制
 *
 * - ❌ `messagesMap`（per-instance 缓存）—— store 是唯一权威
 * - ❌ `flushMessagesToStore`（onBeforeUnmount 写回）—— store 自动维护
 * - ❌ `onUpdateMessages` 回调—— store 直接读
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
  const messageTree = computed<ChatMessage[]>(() => {
    const sessionId = options.sessionId
    if (!sessionId) return []
    const all = store.getSessionMessages(sessionId)
    const removed = getRemovedSet()
    const filtered = all.filter(m => !removed.has(m.id))

    // 识别"空内容叶子节点"：流模式下后端会先发一个 content 为空（如 "\n\n"）的
    // Text / Reasoning 节点（例如 reasoning 与 tool_call 之间的占位空节点），
    // 该节点并不持久化，但前端会短暂收到；渲染时直接过滤掉，避免对话流出现空白块。
    // 仅过滤"无子节点"的节点——带子节点的节点是容器（Turn/ToolCall），须保留。
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
    function isEmptyContentNode(msg: ChatMessage): boolean {
      const t = msg.type || 'text'
      if (t !== 'text' && t !== 'reasoning') return false
      const c = msg.content
      const text =
        typeof c === 'string'
          ? c
          : c && typeof c === 'object' && 'text' in c
            ? (c as { text: string }).text
            : ''
      return !text || text.trim().length === 0
    }
    const isEmptyLeaf = (msg: ChatMessage) =>
      !parentsWithChildren.has(msg.id) && isEmptyContentNode(msg)

    const visible = filtered.filter(m => !isEmptyLeaf(m))

    const childrenMap: Record<string, ChatMessage[]> = {}
    visible.forEach(msg => {
      if (msg.parent_id) {
        if (!childrenMap[msg.parent_id]) childrenMap[msg.parent_id] = []
        childrenMap[msg.parent_id].push(msg)
      }
    })

    const rootMessages = visible.filter(msg => {
      return !msg.parent_id || !visible.find(m => m.id === msg.parent_id)
    })

    rootMessages.sort((a, b) => {
      const sa = a.sort_index ?? a.timestamp ?? 0
      const sb = b.sort_index ?? b.timestamp ?? 0
      return sa - sb
    })
    Object.values(childrenMap).forEach(children => {
      children.sort((a, b) => {
        const sa = a.sort_index ?? a.timestamp ?? 0
        const sb = b.sort_index ?? b.timestamp ?? 0
        return sa - sb
      })
    })

    const buildNode = (msg: ChatMessage, parent?: ChatMessage): ChatMessage => {
      const node: ChatMessage = { ...msg }
      if (parent) node.parent = parent
      if (childrenMap[node.id]) {
        node.children = childrenMap[node.id].map(child => buildNode(child, node))
      }
      return node
    }
    return rootMessages.map(msg => buildNode(msg))
  })

  // 从 store 派生 isLoading / isWaitingApproval
  const isLoading = computed(() => {
    const sid = options.sessionId
    if (!sid) return false
    return store.getSessionStatus(sid).is_working
  })

  const isWaitingApproval = computed(() => {
    const sid = options.sessionId
    if (!sid) return false
    const msgs = store.getSessionMessages(sid)
    return msgs.some(msg => msg.status === 'waiting_user_action')
  })

  async function send(msg: ChatMessage, agentId: string, providerId?: string, opts?: SendOptions) {
    const outgoing: ChatMessage = { ...msg }

    // 目标会话：默认当前会话；子智能体的 user_prompt 回答需发回子会话
    const targetSid = opts?.targetSessionId || options.sessionId
    const sid = options.sessionId
    logger.info('useChatConnection', `[${sid}] Sending message${targetSid !== sid ? ` → ${targetSid}` : ''}`)

    // 立即把用户消息写入目标会话 store（乐观更新，避免后端首帧覆盖不到）
    if (outgoing.id) {
      store.putMessage(targetSid, outgoing)
    }
    // 立即置为 working（让 UI 立即反映 send 已经发出）；
    // 同一会话才切 working 状态，避免跨会话回答误改父会话状态。
    if (targetSid === sid) {
      // 同时清空 last_failed：新一轮交互开始，上一次失败不再"最新"（避免重试成功后仍显示"上次失败"）。
      store.putStatus(sid, { is_working: true, activity: '处理中…', last_failed: false })
      store.setWorking(sid, true)
    }

    // 运行模式：优先取本次 opts.mode，否则回退目标会话已记忆的模式（再回退 'interactive'）。
    // 后端 orchestrator.handle_chat_send_oneoff 据此把 MODE 写入 chat ctx，
    // ask_user / emit_confirm_prompt 等据此决定"产 user_prompt 节点"还是"返回友好错误"。
    const mode = opts?.mode || store.getSessionMode(targetSid)

    // 执行风险等级：与 mode 同级别的回退链——opts.riskLevel > 会话记忆值 > 'medium'。
    // 后端 orchestrator 据此把 RISK_LEVEL 写入 chat ctx，SecurityPolicy 三方法据此覆盖全局阈值。
    const riskLevel = opts?.riskLevel || store.getSessionRiskLevel(targetSid)

    try {
      await callPlugin(CHAT_SEND, {
        session_id: targetSid,
        agent_id: agentId,
        provider_id: providerId || null,
        message: outgoing,
        mode,
        risk_level: riskLevel
      }, 15000, {
        workdir: getGlobalWorkdir(),
        session_id: targetSid
      })
    } catch (err: any) {
      // 仅当发往当前会话时才处理错误 UI（跨会话回答失败不影响父会话渲染）
      if (targetSid !== sid) {
        logger.error('useChatConnection', 'Failed to send (cross-session):', err)
        onSendComplete?.()
        return
      }

      const errText = `Send failed: ${err.message || String(err)}`
      store.putStatus(sid, { is_working: false, activity: '错误', last_failed: true })
      store.setWorking(sid, false)

      // 仅当没有任何 streaming/等待消息承载错误时，才落一条 ephemeral 错误；
      // 否则助手父节点会在 bus Error 事件中带上错误，避免根级 + ephemeral 重复报错。
      const hasStreaming = store.getSessionMessages(sid).some(
        m => m.status === 'streaming' || m.status === 'waiting_user_action'
      )
      if (!hasStreaming) {
        // 写入 ephemeral 错误消息（与 sessionBusWatcher 共用 `__bus_error__` 这个固定 id，
        // 避免同会话多次 send 失败时堆出多条错误消息）
        const errId = '__bus_error__'
        const existing = store.getSessionMessages(sid).find(m => m.id === errId)
        if (existing) {
          store.patchMessage(sid, {
            ...existing,
            content: errText,
            status: 'failed',
            meta: { ...(existing.meta || {}), ephemeral: true }
          })
        } else {
          store.putMessage(sid, {
            id: errId,
            role: 'assistant',
            content: errText,
            status: 'failed',
            meta: { ephemeral: true },
            timestamp: Date.now()
          })
        }
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
      workdir: getGlobalWorkdir(),
      session_id: sid
    }).catch(err => {
      logger.error('useChatConnection', 'Failed to abort:', err)
    })
  }

  function removeMessage(messageId: string) { markRemoved(messageId) }

  /**
   * 会话恢复（retry_turn/retry/approve/reject/supply/answer）。
   *
   * 内部走统一 `CHAT_SEND` 接口的 `resume` 分支（与发送用户消息共用同一端点）。
   * 后端语义（删除-重建模式）：
   * - retry_turn：删除 Failed Turn 及其所有子孙节点 → 重新走 LLM 请求
   * - retry/approve/reject/supply/answer：删除旧子节点 → 重新执行工具或生成结果 → 创建新子节点
   *
   * 前端不在此处构造新消息——后端通过 bus 广播 `Delete`（删旧节点）+ `Update`/
   * `Append`（写新节点 + 父节点状态更新）事件，由 `sessionBusWatcher` 写入 store。
   *
   * 参数有效性：与 send 同级别——`payload.providerId` 显式传入时优先；
   * `mode` / `risk_level` 从会话记忆取，由后端 `resolve_session_params` 写入 chat ctx，
   * continuation chat_loop 通过 `ctx.fork()` 继承。
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
      store.putStatus(sid, { is_working: true, activity: '处理中…', last_failed: false })
      store.setWorking(sid, true)
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
          agent_id: null, // resume 不传 agent_id（后端从 metadata 取）
          provider_id: payload.providerId || null,
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
          workdir: getGlobalWorkdir(),
          session_id: targetSid,
        },
      )
      // 防御性处理：后端 resume 分支有 is_working 守卫，忙碌时返回 session_busy
      // （HTTP 200 成功响应，不会进 catch）。此时必须复位前端 working 状态，
      // 否则 UI 会卡在"处理中…"且重试按钮不消失。
      if (resp?.status === 'session_busy') {
        logger.warn('useChatConnection', `[${sid}] resume rejected: session_busy`)
        if (targetSid === sid) {
          store.putStatus(sid, { is_working: false, activity: '会话忙', last_failed: true })
          store.setWorking(sid, false)
        }
      }
    } catch (err: any) {
      logger.error('useChatConnection', 'Failed to resume:', err)
      if (targetSid === sid) {
        store.putStatus(sid, { is_working: false, activity: '错误', last_failed: true })
        store.setWorking(sid, false)
      }
    }
  }

  return {
    isLoading,
    isWaitingApproval,
    isConnected: computed(() => true), // 始终视为已连接（事件由全局 watcher 接收）
    messageTree,
    send,
    abort,
    removeMessage,
    resume,
  }
}
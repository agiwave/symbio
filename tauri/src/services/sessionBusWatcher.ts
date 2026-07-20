/**
 * SessionBusWatcher — 全局会话事件监听器
 *
 * ## 设计目标
 *
 * 在多会话并发场景下，**所有会话的事件**（不仅当前 active 会话）都需要被监听并
 * 写入 store，以便：
 *
 * 1. SessionCard（左侧列表项 = 缩略窗口）能实时显示每个会话的状态点和预览
 * 2. 用户从 A 切到 B 再切回 A 时，A 的最终态已经在 store 中（不依赖 per-session useChatConnection）
 * 3. 前端重载/重连时，会话进度从 pending/snapshot 补齐后能正确合并到 store
 *
 * ## 与 useChatConnection 的关系
 *
 * - **本 watcher 负责"全量写 store"**：订阅所有 session 事件，update store
 * - **useChatConnection 负责"按需局部状态"**：仅 active 会话订阅，提供 isLoading/error 给 UI
 * - 两者**不重复写 store**——per-session useChatConnection 不再写 store
 *
 * ## 生命周期
 *
 * - 应用启动时调用一次 `startSessionBusWatcher()`
 * - 全局唯一，跨页面共享
 * - 卸载时调用 `stopSessionBusWatcher()`（通常不需要）
 */

import { subscribe as busSubscribe, type BusEvent } from './eventBus'
import { ChatEventType, type ChatEvent } from './model'
import { useSessionsStore } from '@/stores/sessions'
import { logger } from '@/utils/logger'

let _unsubscribe: (() => void) | null = null

/**
 * 启动全局会话事件监听（幂等）
 *
 * 应在应用启动时（`main.ts` 或 `App.vue.onMounted`）调用一次。
 */
export function startSessionBusWatcher(): void {
  if (_unsubscribe) {
    logger.warn('[session-bus-watcher]', 'already started')
    return
  }

  const store = useSessionsStore()

  _unsubscribe = busSubscribe(
    { kind: 'session', sessionId: null }, // null = 接收所有 session
    (busEvent: BusEvent) => {
      if (!busEvent.data) return
      const evt = busEvent.data.data as ChatEvent
      if (!evt || !evt.type) return
      const sid = busEvent.data.session_id
      if (!sid) return

      switch (evt.type) {
        case ChatEventType.Status:
          // 后端 Status 事件 → 更新 store 状态（覆盖所有会话，不分 active/background）
          if (evt.status === 'busy') {
            // 进入 busy 时审批态尚未可知，先复位（waiting_user_action 的 Update
            // 到达时会重新置位）；否则旧会话的"等待审批"角标会残留。
            // 同时清空 last_failed：新一轮交互已开始，上一次的失败不再"最新"。
            store.putStatus(sid, { is_working: true, activity: '处理中…', is_waiting_approval: false, last_failed: false })
            store.setWorking(sid, true)
          } else if (evt.status === 'idle') {
            // idle 表示一轮交互彻底结束（含审批已了结），复位审批角标
            store.putStatus(sid, { is_working: false, activity: undefined, is_waiting_approval: false })
            store.setWorking(sid, false)
          }
          break

        case ChatEventType.Update: {
          // Update 事件 → 写入/合并消息到 store。
          // 设计原则：消息本身**不**携带 session_id，避免每个消息重复存储 session_id。
          // session_id 来自 store.putMessage(sid, ...) / patchMessage(sid, ...) 的 sid 形参。
          const patch = evt.message
          if (patch && patch.id) {
            store.patchMessage(sid, patch)
            // 同步活动文字
            if (patch.status === 'streaming') {
              if (patch.type === 'reasoning') {
                store.putStatus(sid, { activity: '正在思考…' })
              } else if (patch.type === 'tool_call') {
                store.putStatus(sid, { activity: `正在调用 ${patch.name || '工具'}…` })
              } else {
                store.putStatus(sid, { activity: '正在响应…' })
              }
            } else if (patch.status === 'waiting_user_action') {
              // 进入等待审批：点亮卡片"等待审批"角标，并让看门狗能识别"审批等待超时"
              store.putStatus(sid, { activity: '等待审批…', is_waiting_approval: true })
            } else if (patch.status === 'completed') {
              // 该条审批已了结（后端顺序处理工具，同一时刻仅一个 waiting_user_action），
              // 复位审批角标；若还有其他消息仍在等待，其 waiting_user_action 的
              // Update 会再次点亮。
              store.putStatus(sid, { activity: undefined, is_waiting_approval: false })
            } else if (patch.status === 'failed') {
              store.putStatus(sid, { activity: '失败', last_failed: true, is_waiting_approval: false })
            }
          }
          break
        }

        case ChatEventType.Abort: {
          // Abort 事件 → 标记 streaming/waiting 为 completed/failed，并清空 activity
          const msgs = store.getSessionMessages(sid)
          for (const msg of msgs) {
            if (msg.status === 'streaming' || msg.status === 'waiting_user_action') {
              const textContent = typeof msg.content === 'string'
                ? msg.content
                : (Array.isArray(msg.content) ? (msg.content as any[]).filter(p => p.type === 'text').map(p => (p as any).text).join('') : '')
              if (textContent.trim().length === 0) {
                // 空内容消息从 store 中移除
                const mnext = { ...store.sessionMessages }
                const cur = { ...(mnext[sid] || {}) }
                delete cur[msg.id]
                mnext[sid] = cur
                store.sessionMessages = mnext
              } else {
                store.patchMessage(sid, { ...msg, status: 'completed' })
              }
            }
          }
          store.putStatus(sid, { is_working: false, activity: '已中止', is_waiting_approval: false })
          store.setWorking(sid, false)
          break
        }

        case ChatEventType.Error: {
          // Error 事件：把错误**统一呈现在助手父节点（turn）**上；
          // reasoning（思考）等子节点不再独立显示同一份错误，避免"根级 + 子级"重复报错。
          // 仅当没有任何 streaming 消息（错误发生在助手消息创建之前）时，
          // 才落一条 ephemeral 错误（id=`__bus_error__`），与下方父节点错误不重复。
          const msgs = store.getSessionMessages(sid)
          let anyStreaming = false
          let errorAssigned = false
          for (const msg of msgs) {
            if (msg.status !== 'streaming' && msg.status !== 'waiting_user_action') continue
            anyStreaming = true
            const isReasoningChild = msg.type === 'reasoning'
            if (isReasoningChild) {
              // 推理子节点：保留其文本但不以"错误"呈现；空推理块直接移除（与 Abort 一致）
              const text = typeof msg.content === 'string'
                ? msg.content
                : (Array.isArray(msg.content) ? (msg.content as any[]).filter(p => p.type === 'text').map(p => (p as any).text).join('') : '')
              if (text.trim().length === 0) {
                const mnext = { ...store.sessionMessages }
                const cur = { ...(mnext[sid] || {}) }
                delete cur[msg.id]
                mnext[sid] = cur
                store.sessionMessages = mnext
              } else {
                store.patchMessage(sid, { ...msg, status: 'completed' })
              }
              continue
            }
            // 助手父节点（turn）：承载错误文本（只挂一次）
            store.patchMessage(sid, {
              ...msg,
              status: 'failed',
              error: errorAssigned ? undefined : (evt.error || 'Unknown error')
            })
            errorAssigned = true
          }
          // 仅当没有 streaming 消息承载错误时，才落/覆盖 ephemeral 错误
          if (!anyStreaming) {
            const errorMsgId = '__bus_error__'
            const existing = msgs.find(m => m.id === errorMsgId)
            const ephemeralMsg = {
              id: errorMsgId,
              role: 'assistant' as const,
              content: evt.error || 'Unknown error',
              status: 'failed' as const,
              meta: { ephemeral: true },
              timestamp: Date.now()
            }
            if (existing) {
              store.patchMessage(sid, {
                ...existing,
                content: evt.error || 'Unknown error',
                status: 'failed',
                meta: { ...(existing.meta || {}), ephemeral: true }
              })
            } else {
              store.putMessage(sid, ephemeralMsg)
            }
          }
          store.putStatus(sid, {
            is_working: false,
            activity: '错误',
            last_failed: true,
            is_waiting_approval: false
          })
          store.setWorking(sid, false)
          break
        }

        case ChatEventType.Connected:
          // Connected 事件 → 标记为 working；新一轮交互开始，清空 last_failed
          if (evt.is_working === true) {
            store.putStatus(sid, { is_working: true, activity: '处理中…', last_failed: false })
            store.setWorking(sid, true)
          }
          break

        case ChatEventType.Disconnected:
          // 修复（CHAT_FLOW_ANALYSIS E-17）：
          // Disconnected 事件表示**前端连接断开**（不是后端 abort），
          // 不应自动清空业务 working 状态；否则用户在 A 上工作、Tab 切到 B
          // 导致 A 的 bus 连接断开时，A 的 working 会被误清。
          // 业务 working 收敛由后端 Status idle / Abort 事件负责。
          logger.debug('[session-bus-watcher]', `Disconnected: ${sid}`)
          break

        case ChatEventType.Delete: {
          // Delete 事件：后端通知前端精确删除某条消息。
          //
          // 触发场景：工具调用 resume（approve/reject/retry/supply/answer）时，
          // 后端先删掉旧的 pending/failed 子节点，然后广播本事件让前端同步删除，
          // 随后通过 Update/Append 写入新子节点 + 父节点状态更新。
          //
          // 不调用后端 API（删除已由后端完成）；仅同步前端 store。
          const messageId = (evt as any).message_id
          if (messageId) {
            store.removeMessageById(sid, messageId)
          }
          break
        }
      }
    }
  )

  logger.info('[session-bus-watcher]', 'started')
}

/**
 * 停止全局监听（一般不需要调用）
 */
export function stopSessionBusWatcher(): void {
  if (_unsubscribe) {
    _unsubscribe()
    _unsubscribe = null
    logger.info('[session-bus-watcher]', 'stopped')
  }
}

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
import { playCompletionChime } from './completionChime'
import { useSessionsStore } from '@/stores/sessions'
import { VDFS_STATUS_ACTIVE, VDFS_STATUS_WORKING } from '@/schemas/vdfs'
import { logger } from '@/utils/logger'

let _unsubscribe: (() => void) | null = null

// HMR 守卫：模块热更新会重置 `_unsubscribe`，导致 startSessionBusWatcher 二次订阅
// → 同一事件被两个 handler 各处理一次（流式消息逐词叠字）。启动标记挂到 globalThis，
// 跨模块重载保持幂等。
const _G = globalThis as typeof globalThis & { __symSessionBusWatcherStarted?: boolean }

/**
 * 启动全局会话事件监听（幂等）
 *
 * 应在应用启动时（`main.ts` 或 `App.vue.onMounted`）调用一次。
 */
export function startSessionBusWatcher(): void {
  if (_unsubscribe || _G.__symSessionBusWatcherStarted) {
    logger.warn('[session-bus-watcher]', 'already started')
    return
  }
  _G.__symSessionBusWatcherStarted = true

  const store = useSessionsStore()

  _unsubscribe = busSubscribe(
    { kind: 'session', sessionId: null }, // null = 接收所有 session
    (busEvent: BusEvent) => {
      if (!busEvent.data) return
      const evt = busEvent.data.data as ChatEvent
      if (!evt || !evt.type) return
      const sid = busEvent.data.session_id
      if (!sid) return

      // 本通道只承载**会话级**事件（状态 / 生命周期 / 错误 / 中止）。
      //
      // 消息本体（`ChatEventType.Update` / `Delete`）**不在此处理**（S19 收敛）：
      // 转写的读与实时都已归 VDFS——读走 `.vdfs/session/<sid>`（一次 read 拿整份
      // 历史），实时走 `kind = "vdfs"` 的变更（见 `vdfsTranscriptSync`）。
      //
      // 这两个事件类型仍会从总线到达（**进程内的 subagent 宿主需要它们**：
      // 子会话审批透传 + 累积 Assistant 文本，见 `agent/host/subagent.rs`），
      // 因此后端必须继续发布；但前端在这里**故意不处理**——再 patch 一次就会与
      // VDFS 通道形成双写（流式文本逐词叠字）。详见
      // `docs/design/vdfs-session-messages.md` §6 S19。
      switch (evt.type) {
        case ChatEventType.Status:
          // 后端 Status 事件 → 更新 store 状态（覆盖所有会话，不分 active/background）
          if (evt.status === 'busy') {
            // 进入 busy 时审批态尚未可知，先复位（waiting_user_action 的 Update
            // 到达时会重新置位）；否则旧会话的"等待审批"角标会残留。
            // 同时清空 last_failed / 会话级错误：新一轮交互已开始，上一次的失败不再"最新"。
            // 事件通道的 busy / idle 与 VDFS 节点 status（working / active）
            // 是同一件事的两个词表，此处是唯一的翻译点。
            store.putStatus(sid, { status: VDFS_STATUS_WORKING, activity: '处理中…', is_waiting_approval: false, last_failed: false })
            store.setSessionError(sid, null)
            store.setSessionStatus(sid, VDFS_STATUS_WORKING)
          } else if (evt.status === 'idle') {
            // idle 表示一轮交互彻底结束（含审批已了结），复位审批角标
            // 提示音：仅"忙碌 → 结束"的真实收尾才响（应用启动/事件重放等
            // 非工作态的 idle 不响），且 Abort/Error 已响过的同轮结束会被去重
            const wasWorking = store.isSessionWorking(sid)
            store.putStatus(sid, { status: VDFS_STATUS_ACTIVE, activity: undefined, is_waiting_approval: false })
            store.setSessionError(sid, null)
            store.setSessionStatus(sid, VDFS_STATUS_ACTIVE)
            if (wasWorking) playCompletionChime('completed', sid)
          }
          break

        case ChatEventType.Abort: {
          // 中止**不在这里收敛消息**（S19 收敛）。
          //
          // 在途消息的终态由服务端定稿：`orchestrator` 在中止出口先调
          // `persist_failure`（把失败 Turn 标 Failed、在途子节点定稿 Completed），
          // 而这些终态经 `emit_message_patch` 一并发出 VDFS 变更，由
          // `vdfsTranscriptSync` 落进 store——因此走到这里时消息已是终态，
          // 此处再扫一遍只会是第二份实现（且可能与 VDFS 通道打架）。
          //
          // 空内容节点也不需要在此删除：渲染层（`useChatConnection.messageTree`）
          // 本就过滤「空内容叶子」，删不删都不显示。
          //
          // 这里只做本通道真正独有的事：会话状态收敛 + 提示音。
          const wasWorkingBeforeAbort = store.isSessionWorking(sid)
          store.putStatus(sid, { status: VDFS_STATUS_ACTIVE, activity: '已中止', is_waiting_approval: false })
          store.setSessionStatus(sid, VDFS_STATUS_ACTIVE)
          if (wasWorkingBeforeAbort) playCompletionChime('aborted', sid)
          break
        }

        case ChatEventType.Error: {
          // Error 事件：**只信服务端失败终态**。后端在广播 Error 事件前，已通过
          // persist_failure 把失败节点（含 Turn）以 Update(status=failed, error) 推送
          // （上方 Update 分支已写入 store），此处不再做客户端启发式标记
          // （旧逻辑：扫描 streaming 节点分摊错误 / 清空空推理块），避免与
          // 刚收到的服务端终态冲突或刷新前后不一致。
          //
          // 错误作为"状态"而非"节点"（用户约定：错误在前后端都不应是会话节点）：
          // - 若存在 Failed 消息节点（根级 Turn 承载后端权威失败终态），错误已由该节点
          //   承载，前端只在根级 Turn 渲染 ⚠ + 重试，无需任何额外动作；
          // - 仅当没有任何失败消息节点（错误发生在任何消息创建之前，如 transport 级失败）
          //   时，才把错误落到**会话级错误状态**（setSessionError）——它是一条状态，
          //   不是消息树里的一个节点，UI 在会话级错误条里展示并许可重试。
          const wasWorkingBeforeError = store.isSessionWorking(sid)
          const msgs = store.getSessionMessages(sid)
          const hasFailedNode = msgs.some(
            (m) => m.status === 'failed' && !(m.meta as any)?.ephemeral,
          )
          if (!hasFailedNode) {
            store.setSessionError(sid, evt.error || 'Unknown error')
          }
          store.putStatus(sid, {
            status: VDFS_STATUS_ACTIVE,
            activity: '错误',
            last_failed: true,
            is_waiting_approval: false
          })
          store.setSessionStatus(sid, VDFS_STATUS_ACTIVE)
          // 提示音：异常结束（wasWorking 判定与去重同上）
          if (wasWorkingBeforeError) playCompletionChime('failed', sid)
          break
        }

        case ChatEventType.Connected:
          // Connected 事件 → 标记为 working；新一轮交互开始，清空 last_failed / 会话级错误
          // （`evt.is_working` 是**线路字段**，属后端事件载荷；本地一律转写成节点 status）
          if (evt.is_working === true) {
            store.putStatus(sid, { status: VDFS_STATUS_WORKING, activity: '处理中…', last_failed: false })
            store.setSessionError(sid, null)
            store.setSessionStatus(sid, VDFS_STATUS_WORKING)
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
      }
    }
  )

  logger.info('[session-bus-watcher]', 'started')
}

/**
 * 停止全局监听（一般不需要调用）
 */
export function stopSessionBusWatcher(): void {
  _G.__symSessionBusWatcherStarted = false
  if (_unsubscribe) {
    _unsubscribe()
    _unsubscribe = null
    logger.info('[session-bus-watcher]', 'stopped')
  }
}

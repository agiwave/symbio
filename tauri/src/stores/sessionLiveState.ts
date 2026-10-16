/**
 * 会话实时状态的落地层（从 `stores/sessions.ts` 按职责拆出，见 plan/13 批 C6）。
 *
 * 它只管三张按 `sessionId` 索引的表——运行态 / 会话级错误 / 会话级告警——以及
 * 读写它们的**唯一变更通道**。消息转写、空间寻址、清单刷新等都留在 store 里；
 * store 组合本 composable，把同名符号解构进自己的作用域，因此调用点零改动。
 *
 * 三张表都是 `shallowRef`：整体替换才触发更新，与原实现逐字一致。
 */

import { shallowRef } from 'vue'
import { isFailedStatus, isWorkingStatus } from '@/schemas/vdfs'
import type { SessionLiveStatus } from './sessionLive'

export function useSessionLiveState() {
  const sessionStatuses = shallowRef<Record<string, SessionLiveStatus>>({})

  // 会话级错误状态（"错误是状态，不是节点"原则的前端落地）：
  // 仅用于"没有任何失败消息节点、但会话整体因错误中止"的兜底场景（如 transport 级失败、
  // send 在首帧到达前就失败），此时没有"造成中止的节点"可挂错误，错误只能作为会话级状态存在。
  // 与消息树中的失败根级骨架（后端权威失败终态，前端只在根级骨架渲染重试）互斥：
  // 只要存在失败根级骨架，错误就由该节点承载，sessionErrors 保持 null。
  const sessionErrors = shallowRef<Record<string, string | null>>({})

  // 会话级告警状态（"告警是状态，不是事件"原则的前端落地）：
  // 来源唯一——会话节点的 `attributes.warning`（后端 `MessageChange::Warn` 写入，
  // 持久化失败 / 长度截断 / 工具轮次上限等**可恢复**提示；新一轮开始时后端清除）。
  // 与 `sessionErrors`（失败终态）互斥并存：告警不改变运行态，会话照常运行。
  const sessionWarnings = shallowRef<Record<string, string | null>>({})

  /** 会话是否运行中（运行态的唯一读法：节点 `status == working`） */
  function isSessionWorking(id: string): boolean {
    return isWorkingStatus(sessionStatuses.value[id]?.status)
  }

  /**
   * 会话上一轮是否以错误结束（唯一读法：节点 `status == 'failed'`）。
   *
   * 取代原先「`active` + `last_failed` 布尔」的写法：那是"状态 + 平行标志位"，
   * 要求读状态的人同时读两个字段，漏读一处就静默错。
   */
  function isSessionFailed(id: string): boolean {
    return isFailedStatus(sessionStatuses.value[id]?.status)
  }

  function getSessionStatus(id: string): SessionLiveStatus {
    return (
      sessionStatuses.value[id] || {
        is_waiting_approval: false,
        last_event_at: 0,
      }
    )
  }

  /**
   * 判断某 session 的实时状态是否已经"过期"（用于缩略卡显示"连接已断开"提示等）。
   *
   * 阈值与后端断流硬编码一致：30 分钟没有收到任何业务事件即视为过期。
   * 实际取值：与 `orchestrator.run_chat_loop` 的 `tokio::time::timeout(1800s, ...)` 保持一致。
   *
   * @returns null 表示状态新鲜；否则返回过期原因描述
   */
  function getSessionStaleReason(id: string, nowMs: number = Date.now()): string | null {
    const status = sessionStatuses.value[id]
    if (!status) return null // 从未有过事件，状态未知，不算过期
    const STALE_THRESHOLD_MS = 30 * 60 * 1000 // 30 分钟
    const elapsed = nowMs - status.last_event_at
    if (elapsed <= STALE_THRESHOLD_MS) return null
    if (isWorkingStatus(status.status)) return `已无响应 ${Math.floor(elapsed / 60000)} 分钟`
    if (status.is_waiting_approval) return `审批等待超时 ${Math.floor(elapsed / 60000)} 分钟`
    return `状态已过期 ${Math.floor(elapsed / 60000)} 分钟`
  }

  /**
   * 新建一条实时状态的初值。
   *
   * `last_event_at` 取**当前时刻**而不是 0：条目一旦存在，`getSessionStaleReason`
   * 就会按它算「多久没消息了」——初值若是 0，刚建出来的条目会被立刻判成
   * 「状态已过期 N 分钟」（N 自 Unix 纪元起算），凭空报出「连接已断开」。
   */
  function newLiveStatus(): SessionLiveStatus {
    return { is_waiting_approval: false, last_event_at: Date.now() }
  }

  /**
   * 提交一份新的实时状态映射 —— **唯一**写入口（与消息侧的 `commitMessages` 对位）。
   *
   * 收口的意义：全仓 `sessionStatuses.value =` 只应出现在这里，于是「谁在写实时
   * 状态」成为一眼可数的事实，不会再长出第五个、第六个手写 spread-and-assign 的地方。
   */
  function commitStatuses(next: Record<string, SessionLiveStatus>): void {
    sessionStatuses.value = next
  }

  /**
   * 对某个会话的实时状态做一次变更并提交 —— **唯一的变更通道**
   * （与消息侧的 `updateMessages` 对位）。
   *
   * 调用方只写「这一步要改什么」；展开 / 合并 / 整体替换三步不必各自重写——
   * 那三步此前在四处各写一遍，每处都得自己记得「必须换新对象」
   * （`shallowRef` 原地改不触发更新），漏一次就是界面**悄悄**不刷新。
   *
   * `mutate` 返回 `undefined` 表示**不提交**（没有要改的）。缺省状态由
   * `newLiveStatus()` 给，因此「第一次写某个会话」与「更新已有条目」是同一条路径。
   */
  function updateStatus(
    sessionId: string,
    mutate: (cur: SessionLiveStatus) => SessionLiveStatus | void,
  ): void {
    if (!sessionId) return
    const cur = sessionStatuses.value[sessionId] ?? newLiveStatus()
    const next = mutate(cur)
    if (!next) return
    commitStatuses({ ...sessionStatuses.value, [sessionId]: next })
  }

  /**
   * 更新某 session 的 live status（`updateStatus` 的「浅合并 + 自动时间戳」包装）。
   *
   * ## 行为
   *
   * - 浅合并 `partial` 到 `sessionStatuses[sessionId]`。
   * - **始终**自动把 `last_event_at` 更新为 `Date.now()`，即：
   *   - 想清空某个字段（`activity = undefined`）也要走 putStatus（不是直接改 ref）
   *   - 写入方**不需要**手动维护 `last_event_at`
   * - 写入方**也不应**通过 `partial.last_event_at` 覆盖自动时间戳（会被忽略）
   *
   * ## 调用方
   *
   * - `applySessionState`（会话节点状态迁移）
   * - `sessionTranscriptSync`（由消息节点派生活动文字 / 审批角标）
   * - `useChatConnection.send` / `resume` 的乐观置位
   * - `setSessionStatus` 同步 list.status 时
   * - 消息落地路径写 `last_preview`（取到预览才写）
   */
  function putStatus(sessionId: string, partial: Partial<SessionLiveStatus>) {
    // 强制覆盖 last_event_at：调用方无需、也不应手动维护这个时间戳
    const { last_event_at: _ignored, ...rest } = partial
    updateStatus(sessionId, (cur) => ({ ...cur, ...rest, last_event_at: Date.now() }))
  }

  /** 读取会话级错误状态（无失败根级骨架时的兜底错误；null = 无会话级错误） */
  function getSessionError(sessionId: string): string | null {
    return sessionErrors.value[sessionId] ?? null
  }

  /**
   * 设置/清除会话级错误状态。
   *
   * 两个来源，后者覆盖前者（服务端权威）：
   * - **节点属性**：`applySessionState` 取会话节点的 `attributes.error`
   *   （覆盖"错误发生在任何消息节点创建之前"的场景）；
   * - **本地乐观**：`useChatConnection` 在 send / resume 请求本身失败时落一条
   *   （此时后端可能还没产生任何节点）。
   *
   * 传 null 清除（新一轮交互开始时节点不带 error ⇒ 自动清空）。
   */
  function setSessionError(sessionId: string, err: string | null) {
    if (!sessionId) return
    sessionErrors.value = { ...sessionErrors.value, [sessionId]: err ?? null }
  }

  /** 读取会话级告警（null = 无告警） */
  function getSessionWarning(sessionId: string): string | null {
    return sessionWarnings.value[sessionId] ?? null
  }
  /** 设置/清除会话级告警（null 清除；权威来源是节点属性，见 `applySessionState`） */
  function setSessionWarning(sessionId: string, warning: string | null) {
    if (!sessionId) return
    sessionWarnings.value = { ...sessionWarnings.value, [sessionId]: warning ?? null }
  }

  return {
    sessionStatuses,
    sessionErrors,
    sessionWarnings,
    isSessionWorking,
    isSessionFailed,
    getSessionStatus,
    getSessionStaleReason,
    newLiveStatus,
    commitStatuses,
    updateStatus,
    putStatus,
    getSessionError,
    setSessionError,
    getSessionWarning,
    setSessionWarning,
  }
}

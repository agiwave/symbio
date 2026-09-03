/**
 * 会话状态管理（Pinia）
 *
 * ## 多会话缩略窗口架构
 *
 * - 每个 session 都有独立的状态空间（`sessionMessages[id]` / `sessionStatuses[id]`）
 * - 左侧列表项 = 缩略卡片：订阅 bus 状态事件，实时显示"最后一条消息预览 + 状态点"
 * - 中间主区 = 详细窗口：单一 activeId 详细渲染（与现有行为一致）
 * - 所有数据（历史 + 流式）都写入 store，组件**只读** — 消除切换赛跑
 *
 * ## 关键状态
 *
 * - `list`           : SessionListItem[]（来自后端 list + 实时 is_working 合并）
 * - `activeId`       : 当前"详细窗口"展示的会话
 * - `sessionMessages`: 实时 messages map，key 是 sessionId，value 是 `{msgId: ChatMessage}`
 *                      写入：useChatConnection 收 bus 事件时；loadMessages 时
 *                      读取：ModelChatPanel（详细）、SessionListPanel（缩略预览）
 * - `sessionStatuses`: 实时状态，key 是 sessionId
 *                      写入：useChatConnection 收 Status 事件时
 *                      读取：SessionListPanel（缩略卡的状态点 + 预览文本）
 */

import { defineStore } from 'pinia'
import { computed, ref, shallowRef } from 'vue'
import {
  listSessions,
  getSessionMessages as fetchSessionMessages,
  clearSession,
  createSessionId,
  updateSession,
  clearMessages as apiClearMessages,
  deleteMessage as apiDeleteMessage,
  updateMessage as apiUpdateMessage,
  type SessionListItem,
  type SessionMetadata
} from '@/services/session'
import { setGlobalWorkdir, getGlobalWorkdir, callPlugin } from '@/services/plugin'
import { logger } from '@/utils/logger'
import { CHAT_ABORT } from '@/constants/pluginPaths'
import type { ChatMessage } from '@/services/model'

/** 单个 session 的实时状态（用于缩略卡展示） */
export interface SessionLiveStatus {
  /** 后端报告当前 session 正在处理（Status busy / Connected.is_working=true） */
  is_working: boolean
  /** 是否有消息处于 waiting_user_action 状态（缩略卡显示"等待审批"角标） */
  is_waiting_approval: boolean
  /**
   * 最近一次"业务事件"到达时间（毫秒）。
   *
   * 注意：
   * - 这里"业务事件" = 后端 push 的 Update / Status / Error / Abort / Connected / SessionResumed
   *   经过 sessionBusWatcher 写 store 的时刻（即"前端感知到该事件的本地时间"）。
   * - `putStatus` 内部每次都会**自动更新**此字段（避免漏写）；
   *   `putMessage` 只在产生 assistant 文本预览时同步更新。
   * - 若需判断"状态是否过期"，请使用 `getSessionStaleReason()` 而不是直接读此字段。
   */
  last_event_at: number
  /** 当前活动状态文字（如 "正在思考..." / "正在调用工具 ls..."） */
  activity?: string
  /** 最后一条消息预览（assistant 的 text 内容） */
  last_preview?: string
  /** 终态：上次完成时是否失败 */
  last_failed?: boolean
}

export const useSessionsStore = defineStore('sessions', () => {
  // 列表（来自后端 list + 实时 is_working）
  const list = ref<SessionListItem[]>([])
  const activeId = ref<string | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)

  // 标题缓存（id -> title）
  const titles = ref<Record<string, string>>({})

  // 最近一次使用的 workdir（用于新建会话时自动填充）
  const lastUsedWorkdir = ref<string | null>(null)

  // 会话运行模式（auto / interactive），按 sessionId 记忆，切换会话不丢
  // 与 agent_id/provider_id/risk_level 同级别：持久化到 session.metadata.mode
  const sessionModes = ref<Record<string, 'auto' | 'interactive'>>({})
  // 会话执行风险等级（low / medium / high），按 sessionId 记忆，切换会话不丢
  // 与 agent_id/provider_id/mode 同级别：持久化到 session.metadata.risk_level
  const sessionRiskLevels = ref<Record<string, 'low' | 'medium' | 'high'>>({})

  // ── 多会话实时状态 ──
  // 每个 session 一份独立的 messages 字典（key: msgId, value: ChatMessage）
  // 用 shallowRef + 手动 triggerRef 保证响应式而不深 watch
  const sessionMessages = shallowRef<Record<string, Record<string, ChatMessage>>>({})
  const sessionStatuses = shallowRef<Record<string, SessionLiveStatus>>({})
  const sessionSortIndex = ref<Record<string, number>>({})

  // 兼容旧 API：返回 messages 数组形式（按 sort_index / timestamp 排序）
  function getSessionMessages(id: string): ChatMessage[] {
    const m = sessionMessages.value[id] || {}
    return Object.values(m).sort((a, b) => {
      const sa = a.sort_index ?? a.timestamp ?? 0
      const sb = b.sort_index ?? b.timestamp ?? 0
      return sa - sb
    })
  }

  function getSessionStatus(id: string): SessionLiveStatus {
    return sessionStatuses.value[id] || {
      is_working: false,
      is_waiting_approval: false,
      last_event_at: 0
    }
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
    if (status.is_working) return `已无响应 ${Math.floor(elapsed / 60000)} 分钟`
    if (status.is_waiting_approval) return `审批等待超时 ${Math.floor(elapsed / 60000)} 分钟`
    return `状态已过期 ${Math.floor(elapsed / 60000)} 分钟`
  }

  /** 写入或更新一条消息到指定 session（替换整个对象） */
  function putMessage(sessionId: string, msg: ChatMessage) {
    if (!sessionId || !msg.id) return
    const next = { ...sessionMessages.value }
    const cur = { ...(next[sessionId] || {}) }
    // 缺失 sort_index 时自动补一个，保证与流式 patch 的 sort_index 处于同一单调递增序列，
    // 否则用户消息（仅靠 timestamp ≈ epoch ms）会被排到小整数 sort_index 的助手节点之后。
    cur[msg.id] =
      msg.sort_index === undefined
        ? { ...msg, sort_index: nextSortIndex(sessionId) }
        : msg
    next[sessionId] = cur
    sessionMessages.value = next

    // 同步 status.last_preview（取最后一条 assistant 文本）
    if (msg.role === 'assistant' && typeof msg.content === 'string' && msg.content) {
      const preview = msg.content.length > 60 ? msg.content.slice(0, 60) + '…' : msg.content
      const snext = { ...sessionStatuses.value }
      const scur = { ...(snext[sessionId] || { is_working: false, is_waiting_approval: false, last_event_at: 0 }) }
      scur.last_preview = preview
      scur.last_event_at = Date.now()
      snext[sessionId] = scur
      sessionStatuses.value = snext
    } else if (msg.role === 'assistant' && Array.isArray(msg.content)) {
      const txt = (msg.content as any[])
        .filter((p) => p?.type === 'text')
        .map((p) => p?.text || '')
        .join('')
      if (txt) {
        const preview = txt.length > 60 ? txt.slice(0, 60) + '…' : txt
        const snext = { ...sessionStatuses.value }
        const scur = { ...(snext[sessionId] || { is_working: false, is_waiting_approval: false, last_event_at: 0 }) }
        scur.last_preview = preview
        scur.last_event_at = Date.now()
        snext[sessionId] = scur
        sessionStatuses.value = snext
      }
    }
  }

  /** 合并 patch 到指定 session 的某条消息（不替换，只覆盖 patch 提供的字段） */
  function patchMessage(sessionId: string, patch: ChatMessage) {
    if (!sessionId || !patch.id) return
    const next = { ...sessionMessages.value }
    const cur = { ...(next[sessionId] || {}) }
    const existing = cur[patch.id]
    if (!existing) {
      cur[patch.id] = {
        content: '',
        status: 'streaming',
        role: 'assistant',
        timestamp: Date.now(),
        sort_index: nextSortIndex(sessionId),
        ...patch
      }
    } else {
      // 沿用 useChatConnection 的合并语义：text/reasoning 追加，tool_call 替换
      const merged: ChatMessage = { ...existing, ...patch }
      if (patch.content != null) {
        // tool_call / tool 结果：全量替换（避免流式增量重复拼接）。
        // 注意：终态状态补发（content 为 null/undefined）时不应清空已有内容，
        // 因此仅在 patch 确实携带内容时才覆盖（ToolCall 的参数即存于自身 content）。
        const isFullReplace =
          existing.type === 'tool_call' ||
          patch.type === 'tool_call' ||
          patch.role === 'tool' ||
          existing.role === 'tool'
        if (isFullReplace) {
          merged.content = patch.content
        } else if (typeof patch.content === 'string') {
          merged.content = (typeof existing.content === 'string' ? existing.content : '') + patch.content
        } else {
          merged.content = patch.content
        }
      }
      if (patch.meta) {
        merged.meta = { ...(existing.meta || {}), ...patch.meta }
      }
      cur[patch.id] = merged
    }
    next[sessionId] = cur
    sessionMessages.value = next
  }

  function nextSortIndex(sessionId: string): number {
    const cur = sessionSortIndex.value[sessionId] ?? 0
    const n = cur + 1
    sessionSortIndex.value = { ...sessionSortIndex.value, [sessionId]: n }
    return n
  }

  /**
   * 更新某 session 的 live status。
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
   * - `sessionBusWatcher` 处理 Status / Update / Abort / Error / Connected 时
 * - `useChatConnection.send` / `abort` 收敛状态时
 * - `setWorking` 同步 list.is_working 时
   */
  function putStatus(sessionId: string, partial: Partial<SessionLiveStatus>) {
    if (!sessionId) return
    const next = { ...sessionStatuses.value }
    const cur = next[sessionId] || {
      is_working: false,
      is_waiting_approval: false,
      last_event_at: 0
    }
    // 强制覆盖 last_event_at：调用方无需、也不应手动维护这个时间戳
    const { last_event_at: _ignored, ...rest } = partial
    next[sessionId] = { ...cur, ...rest, last_event_at: Date.now() }
    sessionStatuses.value = next
  }

  /** 清空某个 session 的实时状态（删除时调用） */
  function dropSessionState(sessionId: string) {
    const mnext = { ...sessionMessages.value }
    delete mnext[sessionId]
    sessionMessages.value = mnext
    const snext = { ...sessionStatuses.value }
    delete snext[sessionId]
    sessionStatuses.value = snext
  }

  /** 用后端拉来的历史替换 store 中的实时缓存（loadMessages 调用） */
  function hydrateFromHistory(sessionId: string, messages: ChatMessage[]) {
    const map: Record<string, ChatMessage> = {}
    let idx = 0
    let waitingApproval = false
    let hasFailed = false
    for (const m of messages) {
      if (m.id) {
        map[m.id] = { ...m, sort_index: idx++ }
        // 还原"等待审批"状态，使会话卡片角标在重开会话时正确显示
        if (m.status === 'waiting_user_action') waitingApproval = true
        // 还原"失败"状态（Bug 1 修复）：切换会话再切回时，若历史中存在 Failed
        // 消息（如流模式 LLM 失败 / 手动模式工具失败的根 Turn），需让 last_failed
        // 重新置位，保证失败终态在重载后仍然可见、可重试。
        if (m.status === 'failed') hasFailed = true
      }
    }
    const next = { ...sessionMessages.value, [sessionId]: map }
    sessionMessages.value = next
    const snext = { ...sessionSortIndex.value, [sessionId]: idx }
    sessionSortIndex.value = snext
    const prevStatus = sessionStatuses.value[sessionId] ?? { is_working: false, is_waiting_approval: false, last_event_at: Date.now() }
    sessionStatuses.value = {
      ...sessionStatuses.value,
      [sessionId]: {
        ...prevStatus,
        is_waiting_approval: waitingApproval,
        last_failed: prevStatus.last_failed || hasFailed
      }
    }
  }

  /** 读取会话运行模式（默认 interactive） */
  function getSessionMode(id: string): 'auto' | 'interactive' {
    return sessionModes.value[id] || 'interactive'
  }
  /** 设置并记忆会话运行模式（fire-and-forget 持久化到 session.metadata.mode） */
  function setSessionMode(id: string, mode: 'auto' | 'interactive') {
    sessionModes.value = { ...sessionModes.value, [id]: mode }
    // fire-and-forget：ref 已同步变更（UI 立即响应），持久化失败仅警告
    updateSession(id, { mode }).catch((e) =>
      logger.warn('[sessions]', 'setSessionMode 持久化失败', e)
    )
  }

  /** 读取会话执行风险等级（默认 medium） */
  function getSessionRiskLevel(id: string): 'low' | 'medium' | 'high' {
    return sessionRiskLevels.value[id] || 'medium'
  }
  /** 设置并记忆会话执行风险等级（fire-and-forget 持久化到 session.metadata.risk_level） */
  function setSessionRiskLevel(id: string, level: 'low' | 'medium' | 'high') {
    sessionRiskLevels.value = { ...sessionRiskLevels.value, [id]: level }
    // fire-and-forget：ref 已同步变更（UI 立即响应），持久化失败仅警告
    updateSession(id, { risk_level: level }).catch((e) =>
      logger.warn('[sessions]', 'setSessionRiskLevel 持久化失败', e)
    )
  }

  // ---- 计算属性 ----
  const activeListItem = computed(() => list.value.find(s => s.id === activeId.value) || null)
  const activeTitle = computed(() => {
    if (!activeId.value) return '会话'
    return titles.value[activeId.value] || activeListItem.value?.metadata?.title || '新对话'
  })
  const activeWorkdir = computed(() => {
    const m = activeListItem.value?.metadata
    return (m?.workdir as string | undefined) || undefined
  })
  const isActiveWorking = computed(() => activeListItem.value?.is_working || false)
  const runningCount = computed(() => list.value.filter(s => s.is_working).length)

  // ---- 操作 ----

  /** 刷新会话列表（合并持久化 + 实时状态） */
  async function refreshList() {
    loading.value = true
    error.value = null
    try {
      const items = await listSessions()
      // 同步 lastUsedWorkdir
      for (const it of items) {
        const wd = it.metadata?.workdir
        if (typeof wd === 'string' && wd) {
          lastUsedWorkdir.value = wd
        }
        // 同步标题
        const t = it.metadata?.title
        if (typeof t === 'string' && t) {
          titles.value[it.id] = t
        } else if (it.message_count > 0 && !titles.value[it.id]) {
          // 缺标题时拉取首条消息作为标题
          try {
            const { messages } = await fetchSessionMessages(it.id)
            if (messages.length > 0) {
              titles.value[it.id] = extractPreview(messages[0])
            }
          } catch (e) {
            logger.warn('[sessions]', '加载首条消息失败', e)
          }
        }
      }
      // 回填会话级选择（mode / risk_level）到 store map
      // 与 agent_id/provider_id 不同，mode/risk_level 的 UI 状态在 store（不在 ModelChatPanel ref），
      // 故需在 refreshList 时从 metadata 回填，使切换会话/刷新页面后下拉框保留选中值。
      const modeBackfill: Record<string, 'auto' | 'interactive'> = {}
      const riskBackfill: Record<string, 'low' | 'medium' | 'high'> = {}
      for (const it of items) {
        const m = it.metadata
        if (!m) continue
        if (m.mode === 'auto' || m.mode === 'interactive') {
          modeBackfill[it.id] = m.mode
        }
        if (m.risk_level === 'low' || m.risk_level === 'medium' || m.risk_level === 'high') {
          riskBackfill[it.id] = m.risk_level
        }
      }
      if (Object.keys(modeBackfill).length > 0) {
        sessionModes.value = { ...sessionModes.value, ...modeBackfill }
      }
      if (Object.keys(riskBackfill).length > 0) {
        sessionRiskLevels.value = { ...sessionRiskLevels.value, ...riskBackfill }
      }

      // 合并实时 is_working：list 接口返回的是后端 ActiveSessionManager 的权威状态
      const liveStatuses = sessionStatuses.value
      list.value = items.map((it) => {
        const live = liveStatuses[it.id]
        return live ? { ...it, is_working: live.is_working } : it
      })

      // 关键：把后端权威 is_working 回填到 sessionStatuses（isLoading 的唯一来源）。
      // 否则页面重载/视图挂载后，运行中的会话在输入框显示为禁用的"发送"按钮，
      // 用户无法点击停止（stop 按钮失效 bug）。
      // 仅做 false→true 的升级：true→false 的收敛交给事件流的 idle/Abort/Error 事件，
      // 避免 list 快照与实时事件竞争时误降级。
      for (const it of items) {
        if (it.is_working && !liveStatuses[it.id]?.is_working) {
          putStatus(it.id, { is_working: true, activity: '处理中…' })
        }
      }
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e)
      logger.error('[sessions]', 'refreshList 失败', e)
    } finally {
      loading.value = false
    }
  }

  /**
   * 创建新会话。
   * - workdir 可选；不传则取 lastUsedWorkdir
   * - 创建后立即调用 session/update 写 metadata.workdir
   * - 写完之后再写入列表头部
   */
  async function createSession(workdir?: string): Promise<string> {
    const id = createSessionId()
    const now = Math.floor(Date.now() / 1000)
    const resolvedWorkdir = workdir ?? lastUsedWorkdir.value ?? undefined

    // 1. 立即在本地插入"未持久化"条目
    const local: SessionListItem = {
      id,
      message_count: 0,
      updated_at: now,
      is_working: false,
      metadata: { workdir: resolvedWorkdir, created_via: 'ui' }
    }
    list.value = [local, ...list.value]
    titles.value[id] = '新对话'
    activeId.value = id

    // 初始化空 messages / status
    const mnext = { ...sessionMessages.value, [id]: {} }
    sessionMessages.value = mnext
    const snext = { ...sessionStatuses.value, [id]: { is_working: false, is_waiting_approval: false, last_event_at: Date.now() } }
    sessionStatuses.value = snext

    // 2. 同步写后端 metadata（workdir / title），让后续 list 能拿到正确信息
    const meta: SessionMetadata = { created_via: 'ui' }
    if (resolvedWorkdir) meta.workdir = resolvedWorkdir
    try {
      await updateSession(id, meta)
      if (resolvedWorkdir) lastUsedWorkdir.value = resolvedWorkdir
    } catch (e) {
      logger.warn('[sessions]', 'updateSession(workdir) 失败（仅本地生效）', e)
    }

    return id
  }

  async function selectSession(id: string) {
    activeId.value = id
    // 同步全局 workdir：触发后续资源浏览器 / AI 工具调用上下文。
    // **不再提前 return**：即使 activeId 已是该会话（如刷新后 SessionView 恢复），
    // 也要让下面的"无 workdir 兜底"有机会执行。
    const wd = activeWorkdir.value
    if (wd) {
      setGlobalWorkdir(wd)
      return
    }
    // 兜底：该会话从未绑定 workdir（早期会话 / 未走会话内选择目录）时，
    // 若存在可用上下文（当前全局 或 最近使用 目录）则自动补绑定并持久化，
    // 避免"选择已有会话却卡在引导选择工作区"，让其直接进入聊天界面。
    const fallback = getGlobalWorkdir() ?? lastUsedWorkdir.value
    if (fallback) {
      try {
        await setActiveWorkdir(fallback)
      } catch (e) {
        logger.warn('[sessions]', `为会话 ${id} 兜底补绑 workdir 失败`, e)
      }
    }
  }

  async function deleteSession(id: string) {
    const target = list.value.find(s => s.id === id)
    if (!target) return

    // 删除前先 abort 活跃任务
    if (target.is_working) {
      try {
        await callPlugin(CHAT_ABORT, { session_id: id }, undefined, { session_id: id })
      } catch (e) {
        logger.warn('[sessions]', 'abort 失败', e)
      }
    }

    try {
      await clearSession(id)
    } catch (e) {
      logger.error('[sessions]', 'clearSession 失败', e)
      throw e
    }

    list.value = list.value.filter(s => s.id !== id)
    delete titles.value[id]
    // 清理 in-memory 状态
    dropSessionState(id)
    const snext = { ...sessionSortIndex.value }
    delete snext[id]
    sessionSortIndex.value = snext

    if (activeId.value === id) {
      activeId.value = list.value[0]?.id ?? null
      if (activeId.value) {
        const nextWd = activeWorkdir.value
        if (nextWd) setGlobalWorkdir(nextWd)
      }
    }
  }

  /**
   * 设置当前 active 会话的工作目录。
   * 同步写后端 metadata + 本地缓存 + 全局 workdir。
   */
  async function setActiveWorkdir(workdir: string) {
    if (!activeId.value) return
    const id = activeId.value
    // 1. 写后端
    try {
      await updateSession(id, { workdir })
    } catch (e) {
      logger.error('[sessions]', 'setActiveWorkdir 失败', e)
      throw e
    }
    // 2. 更新本地 list 条目
    const idx = list.value.findIndex(s => s.id === id)
    if (idx >= 0) {
      const cur = list.value[idx]
      list.value[idx] = {
        ...cur,
        metadata: { ...(cur.metadata || {}), workdir }
      }
    }
    // 3. 同步全局 workdir
    setGlobalWorkdir(workdir)
    lastUsedWorkdir.value = workdir
  }

  async function rename(id: string, title: string) {
    try {
      await updateSession(id, { title }, title)
    } catch (e) {
      logger.error('[sessions]', 'rename 失败', e)
      throw e
    }
    titles.value[id] = title
    const idx = list.value.findIndex(s => s.id === id)
    if (idx >= 0) {
      const cur = list.value[idx]
      list.value[idx] = {
        ...cur,
        metadata: { ...(cur.metadata || {}), title }
      }
    }
  }

  /**
   * 写入（合并）会话的心跳任务配置到 metadata.heartbeat。
   * 同步写后端 + 本地 list 条目（驱动缩略卡的心跳角标）。
   */
  async function setHeartbeat(id: string, config: SessionMetadata['heartbeat']) {
    try {
      await updateSession(id, { heartbeat: config })
    } catch (e) {
      logger.error('[sessions]', 'setHeartbeat 失败', e)
      throw e
    }
    const idx = list.value.findIndex(s => s.id === id)
    if (idx >= 0) {
      const cur = list.value[idx]
      list.value[idx] = {
        ...cur,
        metadata: { ...(cur.metadata || {}), heartbeat: config }
      }
    }
  }

  /** 主动设置会话的运行状态（来自 useSessionChat 的事件） */
  function setWorking(id: string, isWorking: boolean) {
    const idx = list.value.findIndex(s => s.id === id)
    if (idx >= 0) {
      list.value[idx] = { ...list.value[idx], is_working: isWorking }
    } else {
      // 列表里还没有该会话（极少见，比如刚收到状态事件），补一个
      list.value.unshift({
        id,
        message_count: 0,
        updated_at: Math.floor(Date.now() / 1000),
        is_working: isWorking,
        metadata: {}
      })
    }
    // 同步到 sessionStatuses
    putStatus(id, { is_working: isWorking })
  }

  /**
   * 加载（并缓存）会话消息到 store。
   * 总是从后端拉取最新历史，hydrate 到 `sessionMessages[id]`。
   * 返回消息数组（按 sort_index 排序）。
   *
   * 修复（CHAT_FLOW_ANALYSIS E-13）：失败时**抛出错误**而不是 swallow，
   * 让 ChatMainPanel 能显示错误状态 + 提供重试按钮。
   */
  async function loadMessages(id: string): Promise<ChatMessage[]> {
    const { messages: rawMsgs } = await fetchSessionMessages(id) // 错误会自然抛出（session/get_messages）
    const msgs = rawMsgs as unknown as ChatMessage[]
    hydrateFromHistory(id, msgs)
    // 同步首条消息预览（用于标题/缩略卡）
    if (msgs.length > 0 && !titles.value[id]) {
      titles.value[id] = extractPreview(msgs[0])
    }
    // 同步 list message_count / updated_at
    const idx = list.value.findIndex(s => s.id === id)
    if (idx >= 0) {
      const now = Math.floor(Date.now() / 1000)
      list.value[idx] = { ...list.value[idx], message_count: msgs.length, updated_at: now }
    }
    return getSessionMessages(id)
  }

  /**
   * 从前端局部状态中精确移除一批消息（id 由后端 `chat/delete_message`
   * 返回的 `deleted_ids` 决定：目标消息 + 其后所有消息）。
   */
  function removeMessages(sessionId: string, ids: string[]) {
    if (ids.length === 0) return
    const next = { ...sessionMessages.value }
    const cur = { ...(next[sessionId] || {}) }
    for (const id of ids) delete cur[id]
    next[sessionId] = cur
    sessionMessages.value = next
  }

  /**
   * 从前端局部状态中精确移除单条消息（仅本地，不调用后端）。
   *
   * 用于工具调用 resume 流程：后端广播 `Delete` 事件通知前端删掉旧的
   * pending/failed 子节点（随后会广播新的 Update/Append 写入新子节点）。
   * 与 `removeMessages` 不同，此处仅处理单条，且不触发 message_count 同步
   * ——因为新子节点会立即顶上，总数应保持不变。
   */
  function removeMessageById(sessionId: string, messageId: string) {
    if (!sessionId || !messageId) return
    const next = { ...sessionMessages.value }
    const cur = { ...(next[sessionId] || {}) }
    if (!cur[messageId]) return
    delete cur[messageId]
    next[sessionId] = cur
    sessionMessages.value = next
  }

  /**
   * 删除单条会话消息（后台落库 + 前端精确移除）。
   *
   * 后端会从已排序列表中删除目标消息及其之后所有消息，并返回被删 id 列表；
   * 前端据此精确移除本地状态，最后同步 list 的 message_count。
   */
  async function deleteMessage(sessionId: string, messageId: string): Promise<void> {
    try {
      const res = await apiDeleteMessage(sessionId, messageId)
      removeMessages(sessionId, res.deleted_ids)
    } catch (e) {
      logger.error('[sessions]', 'deleteMessage 失败', e)
      throw e
    }
    syncMessageCount(sessionId)
  }

  /**
   * 更新单条会话消息（手工编辑 / 标错重试等）。
   * - 先 patch 前端局部状态
   * - 再调用后端 `chat/update_message` 持久化
   */
  async function updateMessage(
    sessionId: string,
    message: ChatMessage
  ): Promise<void> {
    if (!message.id) return
    patchMessage(sessionId, message)
    try {
      await apiUpdateMessage(sessionId, message)
    } catch (e) {
      logger.error('[sessions]', 'updateMessage 失败', e)
      throw e
    }
  }

  /**
   * 清空当前会话的全部历史消息（保留会话本身 / 元数据）。
   * - 清空前端局部状态
   * - 调用后端 `chat/clear_messages` 持久化
   */
  async function clearMessages(sessionId: string): Promise<void> {
    const mnext = { ...sessionMessages.value }
    delete mnext[sessionId]
    sessionMessages.value = mnext
    try {
      await apiClearMessages(sessionId)
    } catch (e) {
      logger.error('[sessions]', 'clearMessages 失败', e)
      throw e
    }
    syncMessageCount(sessionId)
  }

  /**
   * 看门狗触发：把一个"卡在 working 状态但长时间无事件"的会话
   * 标记为失败，并持久化，使切换会话后仍能看到上次错误（目标 3）。
   *
   * 找到所有仍为 streaming / waiting_user_action 的消息，标记 Failed + error，
   * 调用后端 updateMessage 持久化，并复位 working 状态。
   */
  async function persistStuckFailure(
    sessionId: string,
    errorText: string
  ): Promise<void> {
    const msgs = getSessionMessages(sessionId)
    const stuck = msgs.filter(
      (m) => m.status === 'streaming' || m.status === 'waiting_user_action'
    )
    for (const m of stuck) {
      const failed: ChatMessage = {
        ...m,
        status: 'failed',
        error: errorText
      }
      try {
        await apiUpdateMessage(sessionId, failed)
      } catch (e) {
        logger.warn('[sessions]', 'persistStuckFailure updateMessage 失败', e)
      }
      patchMessage(sessionId, { ...failed })
    }
    putStatus(sessionId, {
      is_working: false,
      activity: undefined,
      last_failed: true
    })
    setWorking(sessionId, false)
  }

  /** 同步 list 中某会话的 message_count / updated_at（删除 / 清空后调用） */
  function syncMessageCount(sessionId: string) {
    const idx = list.value.findIndex((s) => s.id === sessionId)
    if (idx >= 0) {
      const count = Object.keys(sessionMessages.value[sessionId] || {}).length
      const now = Math.floor(Date.now() / 1000)
      list.value[idx] = { ...list.value[idx], message_count: count, updated_at: now }
    }
  }

  // ---- helpers ----
  function extractPreview(msg: any): string {
    const c = msg?.content
    if (typeof c === 'string') return c.slice(0, 20) + (c.length > 20 ? '...' : '')
    if (Array.isArray(c)) {
      const txt = c.filter((p: any) => p?.type === 'text').map((p: any) => p.text || '').join('')
      return txt.slice(0, 20) + (txt.length > 20 ? '...' : '')
    }
    return '新对话'
  }

  return {
    // state
    list,
    activeId,
    titles,
    loading,
    error,
    lastUsedWorkdir,
    // 多会话实时状态
    sessionMessages,
    sessionStatuses,
    // computed
    activeListItem,
    activeTitle,
    activeWorkdir,
    isActiveWorking,
    runningCount,
    // methods
    refreshList,
    createSession,
    selectSession,
    deleteSession,
    setActiveWorkdir,
    rename,
    setHeartbeat,
    setWorking,
    loadMessages,
    // 历史管理（删除 / 编辑 / 清空 / 卡死持久化）
    deleteMessage,
    updateMessage,
    clearMessages,
    persistStuckFailure,
    // 多会话实时状态 helpers
    getSessionMessages,
    getSessionStatus,
    getSessionStaleReason,
    putMessage,
    patchMessage,
    putStatus,
    dropSessionState,
    hydrateFromHistory,
    removeMessageById,
    // 运行模式（auto / interactive）
    getSessionMode,
    setSessionMode,
    // 执行风险等级（low / medium / high）
    getSessionRiskLevel,
    setSessionRiskLevel
  }
})

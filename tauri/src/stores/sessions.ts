/**
 * 会话状态管理（Pinia）
 *
 * ## 多会话缩略窗口架构
 *
 * - 每个 session 都有独立的状态空间（`sessionMessages[id]` / `sessionStatuses[id]`）
 * - 左侧列表项 = 缩略卡片：实时显示"最后一条消息预览 + 状态点"
 * - 中间主区 = 详细窗口：单一 activeId 详细渲染（与现有行为一致）
 * - 所有数据（历史 + 流式）都写入 store，组件**只读** — 消除切换赛跑
 *
 * ## 状态来自节点，不来自事件
 *
 * 会话的运行态是**会话节点的属性**（`status` + `attributes.outcome` / `.error`），
 * 经 `kind = "vdfs"` 的 `updated` 变更**带载荷**下发（零回读）。因此本 store
 * 不再订阅 `kind = "session"` 事件通道，也不再需要「防乱序」缓冲——状态是幂等
 * 的全量视图，变更可丢、可重放、可乱序。见
 * `symbio/src/plugins/session/docs/node-state-streaming.md`。
 *
 * ## 关键状态
 *
 * - `list`           : SessionListItem[]（来自后端 list + 本地状态镜像合并）
 * - `activeId`       : 当前"详细窗口"展示的会话
 * - `sessionMessages`: 实时 messages map，key 是 sessionId，value 是 `{msgId: ChatMessage}`
 *                      写入：`vdfsTranscriptSync`（VDFS 变更）与 loadMessages
 *                      读取：ModelChatPanel（详细）
 * - `sessionStatuses`: 实时状态，key 是 sessionId
 *                      写入：`applySessionNode`（会话节点变更）/ send 的乐观置位
 *                      读取：会话列表项状态展示（`.vdfs/session` 实例）
 */

import { defineStore } from 'pinia'
import { computed, ref, shallowRef } from 'vue'
import {
  listSessions,
  clearSession,
  updateSession,
  clearMessages as apiClearMessages,
  deleteMessage as apiDeleteMessage,
  updateMessage as apiUpdateMessage,
  type SessionListItem,
  type SessionMetadata
} from '@/services/session'
import { readVdfs, writeVdfs } from '@/services/vdfs'
import {
  VDFS_CHANGE_APPENDED,
  VDFS_CHANGE_CREATED,
  VDFS_CHANGE_DELETED,
  VDFS_CHANGE_UPDATED,
  VDFS_ROOT,
  VDFS_SESSION_DIR,
  VDFS_STATUS_FAILED,
  VDFS_STATUS_WORKING,
  chimeKindOfOutcome,
  isFailedStatus,
  isWorkingStatus,
  sessionRuntimeOf,
  vdfsBase,
  vdfsJoin,
  vdfsSessionAddr,
  type SessionOutcome,
  type VdfsChange,
  type VdfsNode,
} from '@/schemas/vdfs'
import { setLastWorkdir, getLastWorkdir, callPlugin } from '@/services/plugin'
import { publishVdfsChangedLocal, subscribeVdfsChanged } from '@/services/eventBus'
import { playCompletionChime } from '@/services/completionChime'
import { logger } from '@/utils/logger'
import { CHAT_ABORT } from '@/constants/pluginPaths'
import type { ChatMessage } from '@/services/model'
import { isInflightMessageStatus } from '@/schemas/chat_message'
import type { ImageAttachment } from '@/types'

/** 单个 session 的实时状态（用于缩略卡展示） */
export interface SessionLiveStatus {
  /** 会话的运行状态：与 VDFS 节点 `status` **同一词表**（`working` / `active` / …）。
   *
   * 这里不是「第二份真相」，而是**节点状态的本地镜像**：`status` 只在
   * `list` / `stat` 时可见，而 busy / idle 是事件流；两次采样之间（尤其是
   * send 的乐观置位）必须有处落脚。分页后当前会话也可能不在这一页里，
   * 按 list 取节点会落空——所以按 id 索引的这份 map 不能省。
   *
   * 读法一律走 `isWorkingStatus(status)`：**不要**再引入 `is_working` 布尔。 */
  status?: string
  /** 是否有消息处于 waiting_user_action 状态（缩略卡显示"等待审批"角标） */
  is_waiting_approval: boolean
  /**
   * 最近一次"状态写入"的本地时间（毫秒）。
   *
   * 注意：
   * - 写入来源是**节点状态**，不是事件：会话节点的 VDFS 变更（`applySessionNode`）
   *   与消息节点的变更（`vdfsTranscriptSync`），以及 send / resume 的乐观置位。
   * - `putStatus` 内部每次都会**自动更新**此字段（避免漏写）；
   *   `putMessage` 只在产生 assistant 文本预览时同步更新。
   * - 若需判断"状态是否过期"，请使用 `getSessionStaleReason()` 而不是直接读此字段。
   */
  last_event_at: number
  /** 当前活动状态文字（如 "正在思考..." / "正在调用工具 ls..."） */
  activity?: string
  /** 最后一条消息预览（assistant 的 text 内容） */
  last_preview?: string
  /**
   * 上一轮的结局（会话节点 `attributes.outcome` 的本地镜像）。
   *
   * 提示音据此选音色——它是**状态**（"上一轮怎么结束的"），不是事件：
   * 不需要靠"谁先到"来区分中止与失败。
   *
   * 注意「上一轮是否失败」**不看这里**，看 `status == 'failed'`
   * （`isFailedStatus`）——结局是过程记录，状态是当前事实，两者职责不同。
   */
  outcome?: SessionOutcome
}

export const useSessionsStore = defineStore('sessions', () => {
  // 列表（来自后端 list + 本地状态镜像）
  const list = ref<SessionListItem[]>([])
  const activeId = ref<string | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)

  // 标题缓存（id -> title）
  const titles = ref<Record<string, string>>({})

  // 最近一次使用的 workdir（用于新建会话时自动填充）
  const lastUsedWorkdir = ref<string | null>(null)

  // ===== 新建模式（无 id 详情）：首条消息发送时才真正创建会话（懒创建） =====
  // 新建态不创建节点、清单不更新；用户发送首条消息时 Session 渲染器才调
  // createSession 真正建会话，并把首条消息排队在此。ModelChatPanel 挂载后
  // 消费（按 id 匹配），完成"建会话 + 发首条消息"的闭环。
  /** 排队中的首条消息：文本 + 可选的图片附件（草稿→会话转移，object URL 所有权一并转移，不可 revoke） */
  const pendingFirstMessage = ref<{
    id: string
    text: string
    images?: ImageAttachment[]
  } | null>(null)
  /** 取出属于 sessionId 的排队首条消息（取出即清除；不匹配返回 null） */
  function consumePendingFirstMessage(
    sessionId: string
  ): { id: string; text: string; images?: ImageAttachment[] } | null {
    const p = pendingFirstMessage.value
    if (!p || p.id !== sessionId) return null
    pendingFirstMessage.value = null
    return p
  }

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
  const sessionSeq = ref<Record<string, number>>({})

  // 转写版本号：每次 `commitMessages` 提交 +1。
  //
  // 消费方（ModelChatPanel）只 watch 这一个**浅值**，不必 deep watch 整棵消息树——
  // 深 watch 在流式期间每个 token 都要遍历整棵树，是端到端 O(n²) 的主因。
  const transcriptVersion = ref(0)

  /**
   * 提交一份新的 messages 映射 —— **唯一**写入口（版本号在此统一推进）。
   *
   * 收口的意义：版本号不会漏加，也不会有第二个地方绕过它直接写。
   */
  function commitMessages(next: Record<string, Record<string, ChatMessage>>): void {
    sessionMessages.value = next
    transcriptVersion.value += 1
  }

  // 会话级错误状态（"错误是状态，不是节点"原则的前端落地）：
  // 仅用于"没有任何失败消息节点、但会话整体因错误中止"的兜底场景（如 transport 级失败、
  // send 在首帧到达前就失败），此时没有"造成中止的节点"可挂错误，错误只能作为会话级状态存在。
  // 与消息树中的 Failed Turn（后端权威失败终态，前端只在根级 Turn 渲染重试）互斥：
  // 只要存在 Failed Turn，错误就由该节点承载，sessionErrors 保持 null。
  const sessionErrors = shallowRef<Record<string, string | null>>({})

  // 兼容旧 API：返回 messages 数组形式（按 seq 排序；缺失 seq 时回退 timestamp）
  function getSessionMessages(id: string): ChatMessage[] {
    const m = sessionMessages.value[id] || {}
    return Object.values(m).sort((a, b) => {
      const sa = a.seq ?? a.timestamp ?? 0
      const sb = b.seq ?? b.timestamp ?? 0
      return sa - sb
    })
  }

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
    return sessionStatuses.value[id] || {
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
    if (isWorkingStatus(status.status)) return `已无响应 ${Math.floor(elapsed / 60000)} 分钟`
    if (status.is_waiting_approval) return `审批等待超时 ${Math.floor(elapsed / 60000)} 分钟`
    return `状态已过期 ${Math.floor(elapsed / 60000)} 分钟`
  }

  /** 写入或更新一条消息到指定 session（替换整个对象） */
  function putMessage(sessionId: string, msg: ChatMessage) {
    if (!sessionId || !msg.id) return
    const next = { ...sessionMessages.value }
    const cur = { ...(next[sessionId] || {}) }
    // 缺失 seq 时自动补一个单调序号，保证与流式 patch 的 seq 处于同一单调递增序列，
    // 否则用户消息（仅靠 timestamp ≈ epoch ms）会被排到小整数 seq 的助手节点之后。
    cur[msg.id] =
      msg.seq === undefined
        ? { ...msg, seq: nextSeq(sessionId) }
        : msg
    next[sessionId] = cur
    commitMessages(next)

    // 同步 status.last_preview（取最后一条 assistant 文本）
    if (msg.role === 'assistant' && typeof msg.content === 'string' && msg.content) {
      const preview = msg.content.length > 60 ? msg.content.slice(0, 60) + '…' : msg.content
      const snext = { ...sessionStatuses.value }
      const scur = { ...(snext[sessionId] || { is_waiting_approval: false, last_event_at: 0 }) }
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
        const scur = { ...(snext[sessionId] || { is_waiting_approval: false, last_event_at: 0 }) }
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
        seq: nextSeq(sessionId),
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
    commitMessages(next)
  }

  function nextSeq(sessionId: string): number {
    const cur = sessionSeq.value[sessionId] ?? 0
    const n = cur + 1
    sessionSeq.value = { ...sessionSeq.value, [sessionId]: n }
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
   * - `applySessionNode`（会话节点状态迁移）
   * - `vdfsTranscriptSync`（由消息节点派生活动文字 / 审批角标）
   * - `useChatConnection.send` / `resume` 的乐观置位
   * - `setSessionStatus` 同步 list.status 时
   */
  function putStatus(sessionId: string, partial: Partial<SessionLiveStatus>) {
    if (!sessionId) return
    const next = { ...sessionStatuses.value }
    const cur = next[sessionId] || {
      is_waiting_approval: false,
      last_event_at: 0
    }
    // 强制覆盖 last_event_at：调用方无需、也不应手动维护这个时间戳
    const { last_event_at: _ignored, ...rest } = partial
    next[sessionId] = { ...cur, ...rest, last_event_at: Date.now() }
    sessionStatuses.value = next
  }

  /** 读取会话级错误状态（无 Failed Turn 时的兜底错误；null = 无会话级错误） */
  function getSessionError(sessionId: string): string | null {
    return sessionErrors.value[sessionId] ?? null
  }
  /**
   * 设置/清除会话级错误状态。
   *
   * 两个来源，后者覆盖前者（服务端权威）：
   * - **节点属性**：`applySessionNode` 取会话节点的 `attributes.error`
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

  /** 清空某个 session 的实时状态（删除时调用） */
  function dropSessionState(sessionId: string) {
    const mnext = { ...sessionMessages.value }
    delete mnext[sessionId]
    commitMessages(mnext)
    const snext = { ...sessionStatuses.value }
    delete snext[sessionId]
    sessionStatuses.value = snext
  }

  /**
   * 把后端拉来的历史**合并**进 store 的实时缓存（loadMessages 调用）。
   *
   * ## 为什么是合并而不是整表替换
   *
   * 这份快照来自一次 IPC 往返，取回期间流式补丁仍在到达并写入 store。
   * 整表替换会把那些**更新的**补丁覆盖掉（经典 lost update），而它们不会被重发
   * ——结果就是消息内容倒退若干 token，或整条在途节点凭空消失。
   *
   * 规则：
   * - 快照里的节点 → 权威，整条替换（含状态与 `seq`）；
   * - 本地有、快照没有的节点 → **仅当它仍在飞行中**（`isInflightMessageStatus`）
   *   时保留。那正是 `persist_messages` 还没写盘的在途节点，也是「Turn 运行中切走
   *   再切回」时最容易被丢掉的东西；终态且不在快照里的一律丢弃（被删除或已过期的
   *   陈旧副本，保留它会让幽灵节点复活）。
   */
  function hydrateFromHistory(sessionId: string, messages: ChatMessage[]) {
    const map: Record<string, ChatMessage> = {}
    const fromSnapshot = new Set<string>()
    // 以"已分配 seq 的最大值 + 1"作为兜底游标起点，保证：
    // 1) 缺失 seq 的旧数据按后端数组顺序排在有 seq 的消息之后（不抢到前面）；
    // 2) 后续流式消息的 seq 从 max(seq) 之后继续，绝不小于任何历史 seq，
    //    避免"新流式消息被排到旧持久化消息之前"的顺序错乱。
    const maxSeq = messages.reduce(
      (mx, m) => Math.max(mx, typeof m.seq === 'number' ? m.seq : 0),
      0,
    )
    let idx = maxSeq + 1
    let waitingApproval = false
    for (const m of messages) {
      if (m.id) {
        // 后端单调序号 `seq` 即权威顺序；缺失 seq 的旧数据用递增游标兜底。
        const seq = typeof m.seq === 'number' ? m.seq : idx++
        map[m.id] = { ...m, seq }
        fromSnapshot.add(m.id)
        // 还原"等待审批"状态，使会话卡片角标在重开会话时正确显示
        if (m.status === 'waiting_user_action') waitingApproval = true
      }
    }

    // 保留本地在途节点。**重新分配 seq**：本地游标可能小于快照的最大 seq
    // （页面刚加载时游标从 0 起），沿用会让在途节点排到历史之前。
    // `waiting_user_action` 不在保留集合里，因此不会影响上面的审批角标。
    const local = sessionMessages.value[sessionId] || {}
    for (const [id, m] of Object.entries(local)) {
      if (fromSnapshot.has(id) || !isInflightMessageStatus(m.status)) continue
      map[id] = { ...m, seq: idx++ }
    }

    const next = { ...sessionMessages.value, [sessionId]: map }
    commitMessages(next)
    // 续接游标取"已分配 seq 的最大值"，保证下一轮 nextSeq 严格递增。
    const lastSeq = Object.values(map).reduce((mx, m) => Math.max(mx, m.seq ?? 0), 0)
    const snext = { ...sessionSeq.value, [sessionId]: lastSeq }
    sessionSeq.value = snext
    // 只还原"等待审批"（它由转写派生，是**消息**节点的属性）。
    // 「上一轮失败」不在此还原：它是**会话节点**的状态（`status == 'failed'`），
    // 由 `list` 快照 / 节点变更落定——从消息历史反推会造出第二份真相，
    // 且与节点状态可能不一致（历史里有失败 Turn ≠ 会话当前处于失败态）。
    const prevStatus = sessionStatuses.value[sessionId] ?? { is_waiting_approval: false, last_event_at: Date.now() }
    sessionStatuses.value = {
      ...sessionStatuses.value,
      [sessionId]: {
        ...prevStatus,
        is_waiting_approval: waitingApproval
      }
    }
  }

  /** 读取会话运行模式（默认 interactive） */
  function getSessionMode(id: string): 'auto' | 'interactive' {
    return sessionModes.value[id] || 'interactive'
  }

  /** 读取会话执行风险等级（默认 medium） */
  function getSessionRiskLevel(id: string): 'low' | 'medium' | 'high' {
    return sessionRiskLevels.value[id] || 'medium'
  }

  /**
   * 从清单项回填会话级选择（mode / risk_level）到 store map。
   *
   * 与 agent_id/provider_id 不同，mode/risk_level 的 UI 状态在 store（不在
   * 组件 ref），故需在清单刷新 / 元数据补丁时回填，使切换会话或改写后下拉态一致。
   */
  function backfillSessionMaps(items: SessionListItem[]) {
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
  }

  /**
   * 把一次 `session.metadata` 补丁镜射到本地（后端 `session/update` 的浅合并语义）。
   *
   * 供**级联选项机制**使用：选项选择统一经 `worker/session/update` 落库，
   * 本方法让前端无需等待资源变更事件往返即可收敛本地视图（`activeWorkdir`、
   * mode/risk 回退取值、最近使用目录）。它不含选项语义——只是 session/update
   * 的本地镜像，故任何带 metadata 补丁的调用方都可复用。
   */
  function applySessionMetadataPatch(id: string, patch: Record<string, unknown>) {
    const idx = list.value.findIndex((s) => s.id === id)
    if (idx < 0) return
    const cur = list.value[idx]
    const metadata = { ...(cur.metadata || {}), ...patch } as SessionMetadata
    list.value[idx] = { ...cur, metadata }
    backfillSessionMaps([list.value[idx]])
    // workdir 的既有副作用：最近使用目录（作为新建会话的默认目录）
    const wd = metadata.workdir
    if (typeof wd === 'string' && wd) {
      setLastWorkdir(wd)
      lastUsedWorkdir.value = wd
    }
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
  const isActiveWorking = computed(() => isWorkingStatus(activeListItem.value?.status))
  const runningCount = computed(() => list.value.filter(s => isWorkingStatus(s.status)).length)

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
        // 同步标题（后端 vdfs/list 的 title 已按 display_title 下发：
        // metadata.title 优先，否则从会话内容自动生成——前端不再自行拉消息推导）
        const t = (typeof it.metadata?.title === 'string' && it.metadata.title) || it.name
        if (t) {
          titles.value[it.id] = t
        }
      }
      // 回填会话级选择（mode / risk_level）到 store map（机制见 backfillSessionMaps）
      backfillSessionMaps(items)

      // 合并运行态：list 接口返回的是后端 ActiveSessionManager 的权威状态
      const liveStatuses = sessionStatuses.value
      list.value = items.map((it) => {
        const live = liveStatuses[it.id]
        // 本地镜像说「运行中」就保留：send 的乐观置位不能被一次稍早的 list
        // 快照打回（否则按钮会闪回「发送」）。其余一律以服务端 `status` 为准。
        return live && isWorkingStatus(live.status)
          ? { ...it, status: live.status }
          : it
      })

      // 关键：把后端权威 status 回填到 sessionStatuses（isLoading 的唯一来源）。
      // 否则页面重载/视图挂载后，运行中的会话在输入框显示为禁用的"发送"按钮，
      // 用户无法点击停止（stop 按钮失效 bug）。
      // 仅做 false→true 的升级：true→false 的收敛交给事件流的 idle/Abort/Error 事件，
      // 避免 list 快照与实时事件竞争时误降级。
      for (const it of items) {
        if (isWorkingStatus(it.status) && !isWorkingStatus(liveStatuses[it.id]?.status)) {
          putStatus(it.id, { status: VDFS_STATUS_WORKING, activity: '处理中…' })
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
   *
   * @param metadata 草稿态（新建会话前）累积的 metadata 补丁——由级联选项机制
   *   通用缓冲产出（workdir / agent_id / provider_id / mode / risk_level / heartbeat…），
   *   与后端 `session/update` 同一浅合并语义，故这里直接透传，前端不解释字段名。
   *   workdir 缺省时回退最近使用目录（`lastUsedWorkdir` → `getLastWorkdir`）。
   *
   * 流程：**经 VDFS 在会话挂载根上写一次**（`create: true`，不给名字）→ 后端生成
   * id 并把 metadata 一并落库 → 本地乐观插入条目（与后端写同一份 metadata，
   * 保证挂载即水合）→ 发布前端 created 事件（后端事件随后幂等收敛）。
   *
   * 「新建」在机制上就是对**目录自身**的一次 `vdfs/write`（见后端
   * `VdfsProvider::write` 的两种目标形态）：id 是 provider 的私有知识，前端不预先
   * 编造——它只需要知道「建在哪」（`.vdfs/session`），名字由后端给。
   */
  async function createSession(metadata?: Record<string, unknown>): Promise<string> {
    const meta = { created_via: 'ui', ...(metadata ?? {}) } as SessionMetadata
    // 兜底顺序：草稿显式选择 > 最近使用目录；lastWorkdir 仅作新建默认。
    if (!meta.workdir) {
      const fallback = lastUsedWorkdir.value ?? getLastWorkdir() ?? undefined
      if (fallback) meta.workdir = fallback
    }

    // 1. 后端生成 id：写会话挂载根 = 「新建一个会话，名字由 provider 定」
    const resp = await writeVdfs(
      vdfsJoin(VDFS_ROOT, VDFS_SESSION_DIR),
      JSON.stringify({ metadata: meta }),
      { create: true }
    )
    const id = vdfsBase(resp.path)
    // `vdfsBase` 对空路径返回**虚拟根**这个哨兵，因此空地址既不是 `''` 也不是
    // 合法 id——必须显式挡掉，否则会插一条 id 为 `.vdfs` 的幽灵会话。
    if (!id || id === VDFS_ROOT) throw new Error('新建会话未返回地址（provider 未给出新节点路径）')

    // 2. 立即在本地插入"未持久化"条目（与后端写同一份 meta，保证
    //    ModelChatPanel onMounted 从本地 list.metadata 同步水合时拿得到草稿选择）
    const local: SessionListItem = {
      id,
      message_count: 0,
      updated_at: Math.floor(Date.now() / 1000),
      metadata: meta
    }
    list.value = [local, ...list.value]
    titles.value[id] = '新对话'
    activeId.value = id
    // 回填会话级选择（mode / risk_level）到 store map，使选择器即时反映草稿
    backfillSessionMaps([local])

    // 初始化空 messages / status
    const mnext = { ...sessionMessages.value, [id]: {} }
    commitMessages(mnext)
    const snext = { ...sessionStatuses.value, [id]: { is_waiting_approval: false, last_event_at: Date.now() } }
    sessionStatuses.value = snext

    if (typeof meta.workdir === 'string' && meta.workdir) lastUsedWorkdir.value = meta.workdir

    // 前端模式通知：以同构载荷即时告知其他页面（工作台清单等），不等后端事件往返；
    // 后端 created 事件（`vdfs/write` 落库后广播）随后到达，各订阅方幂等收敛
    publishVdfsChangedLocal({ path: vdfsSessionAddr(id), change: VDFS_CHANGE_CREATED })

    return id
  }

  async function selectSession(id: string) {
    activeId.value = id
    // 同步最近使用目录（仅作新建默认）。
    const wd = activeWorkdir.value
    if (wd) {
      setLastWorkdir(wd)
      return
    }
    // 兜底：该会话从未绑定 workdir（早期会话）时，若存在可用上下文（最近使用目录）
    // 则自动补绑定并持久化，避免"选择已有会话卡在引导选择工作区"。
    const fallback = lastUsedWorkdir.value ?? getLastWorkdir()
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
    if (isWorkingStatus(target.status)) {
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

    removeSessionLocal(id)

    // 前端模式通知：以同构载荷即时告知其他页面（工作台清单等），不等后端事件往返；
    // 后端 deleted 事件随后到达，各订阅方幂等收敛
    publishVdfsChangedLocal({ path: vdfsSessionAddr(id), change: VDFS_CHANGE_DELETED })
  }

  /**
   * 本地移除会话（前端模式乐观更新）：清单项、标题缓存、in-memory 状态、
   * 活跃选中（被删的是当前会话时回退到列表首项）。供 deleteSession 与
   * 后端 deleted 事件订阅共用，保证两条路径行为一致。
   */
  function removeSessionLocal(id: string) {
    list.value = list.value.filter(s => s.id !== id)
    delete titles.value[id]
    // 清理 in-memory 状态
    dropSessionState(id)
    const snext = { ...sessionSeq.value }
    delete snext[id]
    sessionSeq.value = snext

    if (activeId.value === id) {
      activeId.value = list.value[0]?.id ?? null
      if (activeId.value) {
        const nextWd = activeWorkdir.value
        if (nextWd) setLastWorkdir(nextWd)
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
    // 3. 更新最近使用目录（新建默认）
    setLastWorkdir(workdir)
    lastUsedWorkdir.value = workdir
  }

  /** 读取指定会话的工作目录（编辑器发送/恢复时用它，而非"全局目录"） */
  function getSessionWorkdir(id: string): string | undefined {
    return list.value.find((s) => s.id === id)?.metadata?.workdir as string | undefined
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

  /** 主动设置会话的运行状态（来自事件通道）：同步 list 项与 sessionStatuses 两处镜像。
   *
   * 入参是**节点状态**（`VDFS_STATUS_*`）而非布尔——「忙不忙」由
   * `isWorkingStatus()` 派生，这里只负责把状态值落到两处镜像上。 */
  function setSessionStatus(id: string, status?: string) {
    const idx = list.value.findIndex(s => s.id === id)
    if (idx >= 0) {
      list.value[idx] = { ...list.value[idx], status }
    } else {
      // 列表里还没有该会话（极少见，比如刚收到状态事件），补一个
      list.value.unshift({
        id,
        message_count: 0,
        updated_at: Math.floor(Date.now() / 1000),
        status,
        metadata: {}
      })
    }
    // 同步到 sessionStatuses
    putStatus(id, { status })
  }

  /**
   * 加载（并缓存）会话消息到 store。
   *
   * ## 读入口是 VDFS，不是专用协议
   *
   * 转写是**列表**（`.vdfs/session/<id>/消息`），而会话叶子
   * `.vdfs/session/<id>` 的内容就是整份会话文档（含 `messages`）——因此
   * **一次 `vdfs/read`** 就能拿到全部历史：既省掉一条专用协议，又让前端与
   * LLM 在**同一地址**上读同一份数据（`session/get_messages` 自此不再是
   * 前端的读入口）。
   *
   * 增量由 `services/vdfsTranscriptSync` 从 `kind = "vdfs"` 变更补上——本函数
   * 只负责整份替换（切换会话 / 显式刷新）。
   *
   * 修复（CHAT_FLOW_ANALYSIS E-13）：失败时**抛出错误**而不是 swallow，
   * 让 ChatMainPanel 能显示错误状态 + 提供重试按钮。
   */
  async function loadMessages(id: string): Promise<ChatMessage[]> {
    const msgs = await fetchTranscript(id)
    hydrateFromHistory(id, msgs)
    // 同步 list message_count / updated_at
    const idx = list.value.findIndex(s => s.id === id)
    if (idx >= 0) {
      const now = Math.floor(Date.now() / 1000)
      list.value[idx] = { ...list.value[idx], message_count: msgs.length, updated_at: now }
    }
    return getSessionMessages(id)
  }

  /**
   * 经 VDFS 读取整份转写（会话叶子的内容是一份 JSON 文档）。
   *
   * 文档形状由后端 `session_content` 决定（`{ id, title, metadata, messages, updated_at }`）；
   * 这里只取 `messages`，其余字段由会话清单节点（`.vdfs/session/<id>`）承载。
   */
  async function fetchTranscript(id: string): Promise<ChatMessage[]> {
    const content = await readVdfs(vdfsSessionAddr(id))
    const text = content?.text
    if (!text) {
      throw new Error(`读取会话转写失败：${vdfsSessionAddr(id)}`)
    }
    let doc: unknown
    try {
      doc = JSON.parse(text)
    } catch (e) {
      throw new Error(`会话转写不是合法 JSON（${vdfsSessionAddr(id)}）：${e}`)
    }
    const messages = (doc as { messages?: unknown })?.messages
    return Array.isArray(messages) ? (messages as ChatMessage[]) : []
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
    commitMessages(next)
  }

  /**
   * 从某条消息起**截断到列表末尾**：移除「目标 + 其后全部」。
   *
   * ## 为什么前端要自己算这段区间
   *
   * 后端 `chat/delete_message` 的语义就是「在已排序列表里删掉目标及其之后的所有
   * 消息」（`handlers.rs::invoke_delete_message` 的 `messages.drain(i..)`）。前端若
   * 只删这一条、再等后端把被删的每一条逐个通知回来，就把一次确定的**区间删除**拆成了
   * N 次通知；而且只要漏掉其中任意一条，列表尾部就会残留一个后端已不存在的节点，
   * **没有任何机制会纠正它**（VDFS 与流式视图互不校验）。
   *
   * 判据与后端**同源**：两边都按 `seq` 升序排（后端 `ordered()` / `get_messages()`
   * 用 `seq.unwrap_or(i64::MAX)`；本 store 用 `seq ?? timestamp`），因此「取目标及其
   * 之后的全部」在两侧是同一个集合。这里按**排序后的位置**切片而不是直接比较 `seq`
   * 大小：两种写法在有 `seq` 时等价，而位置切片在缺 `seq` 的旧数据上也仍然正确
   * （不需要额外假设 `seq` 一定存在）。
   *
   * 锚点不在本地（会话没加载 / 已被别处删掉）时**什么都不删**并返回空数组——
   * 绝不拿一个并不存在的锚点去截断整个列表。
   *
   * @returns 实际被移除的消息（调用方据此回滚 / 记日志）
   */
  function removeFrom(sessionId: string, messageId: string): ChatMessage[] {
    if (!sessionId || !messageId) return []
    const cur = sessionMessages.value[sessionId]
    if (!cur || !cur[messageId]) return []
    const orderedIds = getSessionMessages(sessionId).map((m) => m.id)
    const anchorIdx = orderedIds.indexOf(messageId)
    if (anchorIdx < 0) return []

    const removed: ChatMessage[] = []
    const rest = { ...cur }
    for (const id of orderedIds.slice(anchorIdx)) {
      const m = rest[id]
      if (!m) continue
      delete rest[id]
      removed.push(m)
    }
    if (removed.length === 0) return []
    commitMessages({ ...sessionMessages.value, [sessionId]: rest })
    return removed
  }

  /** 把一批消息放回局部状态（`removeFrom` 的回滚口，仅在写后端失败时使用） */
  function restoreMessages(sessionId: string, msgs: ChatMessage[]) {
    if (msgs.length === 0) return
    const next = { ...sessionMessages.value }
    const cur = { ...(next[sessionId] || {}) }
    for (const m of msgs) cur[m.id] = m
    next[sessionId] = cur
    commitMessages(next)
  }

  /**
   * 从前端局部状态中精确移除单条消息（仅本地，不调用后端）。
   *
   * 用于**逐节点**删除：工具调用恢复时后端广播 `deleted`，前端删掉旧的
   * pending/failed 子节点（随后会广播新的 `updated` / `created` 写入新子节点）。
   * 与 `removeFrom` 的区别是它只删**这一个**节点，与顺序无关——正是 `deleted`
   * 与 `truncated` 两种变更语义的分界。
   *
   * 不触发 message_count 同步：新子节点会立即顶上，总数应保持不变。
   */
  function removeMessageById(sessionId: string, messageId: string) {
    if (!sessionId || !messageId) return
    const cur = sessionMessages.value[sessionId]
    // 存在性检查放在任何对象展开**之前**：截断删除会连带引发一串针对已删节点的
    // 冗余通知，每一次都白拷贝两份对象就太亏了（这是幂等收口的常见路径）。
    if (!cur || !cur[messageId]) return
    const next = { ...sessionMessages.value }
    const rest = { ...cur }
    delete rest[messageId]
    next[sessionId] = rest
    commitMessages(next)
  }

  /**
   * 删除单条会话消息（连同其后续所有消息）。
   *
   * 前端**自己**做这段级联（`removeFrom`，判据与后端同源），而不是等后端把被删的
   * 每一条逐个通知回来：UI 立即收敛，且不依赖任何一条通知的送达。
   * 随后用后端返回的权威 `deleted_ids` 做一次幂等对齐——本地推算若因锚点缺失等原因
   * 偏窄，这里补齐。写后端失败则回滚本地改动，保持「没落库就不显示已删除」。
   */
  async function deleteMessage(sessionId: string, messageId: string): Promise<void> {
    const removed = removeFrom(sessionId, messageId)
    syncMessageCount(sessionId)
    try {
      const res = await apiDeleteMessage(sessionId, messageId)
      removeMessages(sessionId, res.deleted_ids)
    } catch (e) {
      logger.error('[sessions]', 'deleteMessage 失败', e)
      restoreMessages(sessionId, removed)
      syncMessageCount(sessionId)
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
    commitMessages(mnext)
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
    // 与后端 persist_failure 对齐（"错误是状态、且只由造成中止的根 Turn 承载"）：
    // 仅把"根级 Turn（msg_type=turn 且 parent_id 为空）"标 Failed + error；
    // 其余仍在进行中的子节点（text / reasoning / tool_call）定稿为 Completed、
    // 绝不挂 error，避免把同一条错误刷到每条半截消息上（即原始 429 刷屏的根因）。
    let rootTurnFailed = false
    for (const m of stuck) {
      const isRootTurn = m.type === 'turn' && !m.parent_id
      if (isRootTurn) {
        const failed: ChatMessage = { ...m, status: 'failed', error: errorText }
        try {
          await apiUpdateMessage(sessionId, failed)
        } catch (e) {
          logger.warn('[sessions]', 'persistStuckFailure updateMessage 失败', e)
        }
        patchMessage(sessionId, { ...failed })
        rootTurnFailed = true
      } else {
        // 进行中的子节点：定稿为 Completed（结束流式动画），不挂 error。
        const done: ChatMessage = { ...m, status: 'completed', error: undefined }
        try {
          await apiUpdateMessage(sessionId, done)
        } catch (e) {
          logger.warn('[sessions]', 'persistStuckFailure updateMessage 失败', e)
        }
        patchMessage(sessionId, { ...done })
      }
    }
    // 没有任何根级 Turn（例如首帧到达前就卡住、连 Turn 都还没创建）：
    // 降级为会话级错误状态（不注入错误节点），保证仍能在 UI 上看到错误并可重试。
    if (!rootTurnFailed) {
      setSessionError(sessionId, errorText)
    }
    // 收敛为「以错误结束」这个**状态值**（不再是 `active` + `last_failed` 布尔）。
    // 后端随后的权威节点视图会覆盖这里——本次是本地看门狗的乐观收尾，
    // 与后端 `emit_session_state(Finished{failed})` 语义一致，故不会互相打架。
    putStatus(sessionId, {
      status: VDFS_STATUS_FAILED,
      activity: '错误',
      outcome: 'failed'
    })
    setSessionStatus(sessionId, VDFS_STATUS_FAILED)
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

  // ===== 会话节点的 VDFS 变更 → 侧栏清单同步（后端消息模式） =====
  //
  // 清单同步双模式约定：
  // - 后端消息模式（本订阅）：后端增删改节点 → `notify_change`（唯一的 `vdfs` 频道）
  //   → 此处收敛同步（跨窗口一致的唯一事实源）。载荷是**粗粒度**的（只有 path +
  //   change，不带快照），所以：deleted 本地即时移除、created 防抖重拉、
  //   updated 重读该节点。
  // - 前端模式（乐观更新）：本 store 的 createSession/deleteSession 已直接
  //   变更本地 list，并经 publishVdfsChangedLocal 以同构载荷即时通知
  //   其他页面（如工作台清单），不等事件往返；后端事件随后幂等收敛。
  // 作用域 directChildren = 只看会话叶子节点（`.vdfs/session/<id>`）：子会话
  // （`.vdfs/session/<id>/子会话/…`）与转写列表项的变更不进侧栏清单。
  let listRefreshTimer: ReturnType<typeof setTimeout> | null = null
  function scheduleListRefresh() {
    if (listRefreshTimer) clearTimeout(listRefreshTimer)
    listRefreshTimer = setTimeout(() => {
      listRefreshTimer = null
      refreshList().catch((err) => logger.warn('[sessions]', '资源变更触发清单刷新失败', err))
    }, 800)
  }

  /**
   * 应用一条**会话节点**变更（`updated`）：状态 / 结局 / 错误 / 标题就地收敛。
   *
   * ## 零回读（本函数存在的理由）
   *
   * 后端在会话节点的 `updated` 变更上附带**全量节点视图**
   * （`change.node`），所以这里**不需要**再发一次 `vdfs/stat`。
   * 一次状态迁移一次 IPC 恰恰是最不该省的那一步——状态迁移是最需要即时的路径。
   *
   * 因此 `node` 缺失时**不做回读兜底**，只记一条 warn 并放弃：
   * 那意味着后端违反了「状态变更必带节点视图」这条不变量
   * （`node-state-streaming.md` §8.2），静默回读只会把它掩盖成"看起来能用"。
   * 真丢了一次也不要紧——下一次 `list` 快照会收敛，因为状态是幂等的。
   *
   * ## 状态迁移是唯一驱动提示音的东西
   *
   * 只有 `working → 非 working` 的**迁移**才响（`refreshList` 的重复采样、
   * 标题更新、消息计数更新都不会误触发），音色取节点自述的 `outcome`。
   * 迁移判定放在 store 是因为**上一状态只有这里有**——它本来就是节点表的一部分，
   * 不是"上一条事件"。
   */
  function applySessionNode(id: string, change: VdfsChange): void {
    const node: VdfsNode | undefined = change.node
    if (!node) {
      logger.warn(
        '[sessions]',
        `会话节点变更未携带节点视图，已忽略（后端违反「状态变更必带载荷」不变量）：${change.path}`,
      )
      return
    }

    const rt = sessionRuntimeOf(node)
    const prev = sessionStatuses.value[id]
    const wasWorking = isWorkingStatus(prev?.status)
    const nowWorking = isWorkingStatus(rt.status)

    // ① 清单条目就地收敛（不整表重拉）
    const title = typeof node.title === 'string' ? node.title : ''
    const idx = list.value.findIndex((s) => s.id === id)
    if (idx >= 0) {
      const cur = list.value[idx]
      list.value[idx] = {
        ...cur,
        status: rt.status,
        updated_at: node.updated_at ?? cur.updated_at,
        // 节点自述的 message_count 随载荷下发；缺失则保留原值
        message_count:
          typeof node.message_count === 'number' ? node.message_count : cur.message_count,
        metadata: title ? { ...(cur.metadata || {}), title } : cur.metadata,
      }
    }
    if (title) titles.value[id] = title

    // ② 运行态镜像：节点状态 / 结局直通
    const patch: Partial<SessionLiveStatus> = {
      status: rt.status,
      outcome: rt.outcome,
    }
    // `activity` 只在 working ↔ 非 working **迁移**时改写：否则一次标题更新就会
    // 把消息节点派生的"正在思考…"顶掉，造成闪动。
    if (wasWorking !== nowWorking) {
      if (nowWorking) patch.is_waiting_approval = false
      patch.activity = nowWorking
        ? '处理中…'
        : rt.outcome === 'aborted'
          ? '已中止'
          : rt.outcome === 'failed'
            ? '错误'
            : undefined
    }
    putStatus(id, patch)

    // ③ 会话级错误 = 节点属性（覆盖「错误发生在任何消息节点创建之前」的场景）。
    //    新一轮开始（working）时节点不带 error ⇒ 自动清空上一轮的错误。
    setSessionError(id, rt.error ?? null)

    // ④ 提示音：状态迁移 + 结局选音色
    if (wasWorking && !nowWorking) {
      playCompletionChime(chimeKindOfOutcome(rt.outcome), id)
    }
  }

  subscribeVdfsChanged(
    { prefix: vdfsJoin(VDFS_ROOT, VDFS_SESSION_DIR), directChildren: true },
    (change) => {
      // 追加型变更只发生在转写列表项上（由 vdfsTranscriptSync 就地应用 delta），
      // 与会话清单无关——绝不能让流式的每一帧触发一次重拉。
      if (change.change === VDFS_CHANGE_APPENDED) return
      const id = vdfsBase(change.path)
      if (!id) return
      if (change.change === VDFS_CHANGE_DELETED) {
        removeSessionLocal(id)
        return
      }
      if (change.change === VDFS_CHANGE_UPDATED) {
        // 带载荷的状态迁移（含 busy / idle / 失败）——就地收敛，零回读
        applySessionNode(id, change)
        return
      }
      // created / renamed 等：本地乐观插入已覆盖同窗口场景；
      // 此处防抖重拉，收敛排序与完整字段。
      scheduleListRefresh()
    }
  )

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
    transcriptVersion,
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
    getSessionWorkdir,
    rename,
    setSessionStatus,
    isSessionWorking,
    isSessionFailed,
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
    applySessionNode,
    getSessionError,
    setSessionError,
    dropSessionState,
    hydrateFromHistory,
    removeMessageById,
    removeFrom,
    // 运行模式（auto / interactive）：写入统一走级联选项机制（metadata 补丁），
    // store 只提供读取 + 本地镜射，避免第二条写入路径。
    getSessionMode,
    // 新建模式（无 id 详情）懒创建：首条消息排队
    pendingFirstMessage,
    consumePendingFirstMessage,
    // 执行风险等级（low / medium / high）
    getSessionRiskLevel,
    // 级联选项机制：session/update 的本地镜射（metadata 浅合并）
    applySessionMetadataPatch
  }
})

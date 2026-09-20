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
 *                      读取：会话列表项状态展示（`<根>/session` 实例）
 */

import { defineStore } from 'pinia'
import { computed, ref, shallowRef } from 'vue'
import {
  listSessions,
  deleteSession as apiDeleteSession,
  updateSession,
  readSessionTranscript,
  clearMessages as apiClearMessages,
  deleteMessage as apiDeleteMessage,
  updateMessage as apiUpdateMessage,
  type SessionListItem,
  type SessionMetadata
} from '@/services/session'
import { writeVdfs } from '@/services/vdfs'
import { vdfsRoot } from '@/schemas/vdfsRoot'
import {
  VDFS_CHANGE_CREATED,
  VDFS_CHANGE_DELETED,
  VDFS_STATUS_FAILED,
  VDFS_STATUS_WORKING,
  chimeKindOfOutcome,
  isFailedStatus,
  isWorkingStatus,
  sessionRuntimeOf,
  vdfsBase,
  vdfsSessionAddr,
  type VdfsChange,
  type VdfsNode,
} from '@/schemas/vdfs'
import { setLastWorkdir, getLastWorkdir, callPlugin } from '@/services/plugin'
import { publishVdfsChangedLocal } from '@/services/eventBus'
import { ensureSessionMountDir, ensureVdfsSessionScheme } from '@/services/vdfsScheme'
import { playCompletionChime } from '@/services/completionChime'
import { logger } from '@/utils/logger'
import { CHAT_ABORT } from '@/constants/pluginPaths'
import type { ChatMessage } from '@/schemas/chat_message'
import {
  CHAT_ROLE_ASSISTANT,
  MESSAGE_STATUS_STREAMING,
  isInflightMessageStatus,
} from '@/schemas/chat_message'
import type { ImageAttachment } from '@/types'
// 消息转写规则（纯逻辑）：合并 / 水合 / 截断 / 看门狗判据
import {
  hydrateTranscript,
  mergeMessagePatch,
  previewOf,
  sortTranscript,
  stuckFailurePlanOf,
  truncateIdsFrom,
} from './sessionTranscript'
// 会话实时状态的派生规则（纯逻辑）：清单条目 / 运行态 / 选项回填
import {
  ACTIVITY_WORKING,
  listItemPatchOf,
  liveStatusPatchOf,
  mergeListWithLive,
  modeRiskBackfillOf,
  titleOf,
  workdirOf,
  workingUpgradesOf,
  type SessionLiveStatus,
  type SessionMode,
  type SessionRiskLevel,
} from './sessionLive'

// 对外类型契约保持原路径（消费方一直从 `@/stores/sessions` 取）
export type { SessionLiveStatus } from './sessionLive'

/** 单个 session 的实时状态（用于缩略卡展示）—— 定义见 `./sessionLive`，此处仅经上方 re-export 暴露 */

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
  //
  // 新建态不创建节点、清单不更新；用户发送首条消息时经
  // `createSessionWithFirstMessage` **原子地**建会话 + 把首条消息排进下面的邮箱。
  // 新会话面板（ModelChatPanel）挂载后消费（按 id 匹配），完成「建会话 + 发首条
  // 消息」的闭环。
  //
  // 为什么必须是邮箱（而不是 prop 逐级传）：生产者在 `Session`（草稿详情）里，
  // 消费者是**创建成功后才被选中、才挂载**的另一个组件——消费者此刻还不存在，
  // 没有可传的对象。故只能经由 store 这个双方共同祖先转交。
  //
  // 为什么必须原子（而不是「先 create 再赋值」两步）：队列项的 `id` 必须等于
  // 刚建出来的 `id`。两步写法下这条不变式由调用方的局部变量维持，漏第二步就是
  // 静默丢首条消息；收进 store 后此不变式无法被破坏。

  /**
   * 排队中的首条消息：文本 + 可选的图片附件。
   *
   * **所有权契约**：附件的 `thumbnailUrl` 是 `URL.createObjectURL` 出来的
   * object URL，所有权随载荷从「草稿输入区」转移到本邮箱，再转移到消费它的会话
   * 面板。转移链上的任何一环**都不得 revoke**——只有最终发送者（ModelChatPanel
   * 的 `handleSend`）在消息发出后才 revoke。提前 revoke 会让预览图变成裂图。
   */
  interface PendingFirstMessage {
    /** 队列项属于哪个会话（= 刚创建出来的会话 id） */
    id: string
    text: string
    images?: ImageAttachment[]
  }

  const pendingFirstMessage = ref<PendingFirstMessage | null>(null)

  /** 取出属于 sessionId 的排队首条消息（取出即清除；不匹配返回 null） */
  function consumePendingFirstMessage(sessionId: string): PendingFirstMessage | null {
    const p = pendingFirstMessage.value
    if (!p || p.id !== sessionId) return null
    pendingFirstMessage.value = null
    return p
  }

  // 会话运行模式（auto / interactive），按 sessionId 记忆，切换会话不丢
  // 与 agent_id/provider_id/risk_level 同级别：持久化到 session.metadata.mode
  const sessionModes = ref<Record<string, SessionMode>>({})
  // 会话执行风险等级（low / medium / high），按 sessionId 记忆，切换会话不丢
  // 与 agent_id/provider_id/mode 同级别：持久化到 session.metadata.risk_level
  const sessionRiskLevels = ref<Record<string, SessionRiskLevel>>({})

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

  /**
   * 对某个会话的消息字典做一次变更并提交 —— **唯一的变更通道**。
   *
   * 「展开外层 → 展开本会话 → 改 → 塞回 → `commitMessages`」这五步此前在六处
   * 各写一遍（写一条 / 合并 patch / 删单条 / 删一批 / 从某条起截断 / 回滚），
   * 每处都得自己记得调 `commitMessages`——漏一次就是版本号不推进、视图不刷新，
   * 而且这种漏法**不报错**（只是界面悄悄不更新）。
   *
   * 收敛到这里之后，调用方只写「这一步要改什么」：`mutate` 直接改传入的字典，
   * 或返回一份新的（返回 `undefined` 表示已在原地改完）。
   */
  function updateMessages(
    sessionId: string,
    mutate: (cur: Record<string, ChatMessage>) => Record<string, ChatMessage> | void,
  ): void {
    if (!sessionId) return
    const next = { ...sessionMessages.value }
    const cur = { ...(next[sessionId] || {}) }
    next[sessionId] = mutate(cur) ?? cur
    commitMessages(next)
  }

  // 会话级错误状态（"错误是状态，不是节点"原则的前端落地）：
  // 仅用于"没有任何失败消息节点、但会话整体因错误中止"的兜底场景（如 transport 级失败、
  // send 在首帧到达前就失败），此时没有"造成中止的节点"可挂错误，错误只能作为会话级状态存在。
  // 与消息树中的 Failed Turn（后端权威失败终态，前端只在根级 Turn 渲染重试）互斥：
  // 只要存在 Failed Turn，错误就由该节点承载，sessionErrors 保持 null。
  const sessionErrors = shallowRef<Record<string, string | null>>({})

  // 兼容旧 API：返回 messages 数组形式（按 seq 排序；缺失 seq 时回退 timestamp）
  function getSessionMessages(id: string): ChatMessage[] {
    return sortTranscript(Object.values(sessionMessages.value[id] || {}))
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

  /**
   * 写入或更新一条消息到指定 session（替换整个对象）。
   *
   * ## `seq` 的两条规则（顺序锚点不得被写入动作破坏）
   *
   * 1. **载荷带 `seq`（后端权威）**：原样采用，并把本地续接游标抬到不低于它——
   *    游标若落后于已观测到的后端序号，下一条本地消息会被排到它**前面**。
   * 2. **载荷不带 `seq`**（落库前无序号）：新消息按本地游标接在末尾；
   *    **已存在的消息保留原序号**。原先一律重新发号，会让任何一次"不带 seq 的
   *    更新"（状态迁移 / 压缩后重发）把消息顶到列表末尾——顺序因此看起来会乱。
   */
  function putMessage(sessionId: string, msg: ChatMessage) {
    if (!sessionId || !msg.id) return
    let seqReplaced = false
    updateMessages(sessionId, (cur) => {
      if (typeof msg.seq === 'number') {
        const existing = cur[msg.id]
        // 存储给的号与手里那个**不一致** ⇒ 手里那个是本地游标发的（在途期间）。
        // 这不是“更新一下号码”那么轻：它意味着**两套序号空间已经分叉**——同一
        // 条链上既有存储号又有本地号，排序随之可能错位（且只有整份回读能纠正）。
        // 因此记录下来，等会话转空闲回读一次收敛（见 `reconcileTranscript`）。
        if (existing && typeof existing.seq === 'number' && existing.seq !== msg.seq) {
          seqReplaced = true
        }
        cur[msg.id] = msg
        raiseSeqFloor(sessionId, msg.seq)
        return
      }
      const existing = cur[msg.id]
      cur[msg.id] = { ...msg, seq: existing?.seq ?? nextSeq(sessionId) }
    })
    if (seqReplaced) markTranscriptDirty(sessionId)

    // 同步 status.last_preview（取最后一条 assistant 文本；取不到则不动）
    const preview = previewOf(msg)
    // 走统一变更通道：`last_event_at` 由它推进，写入方不手动维护（与其它写点同口径）
    if (preview !== null) putStatus(sessionId, { last_preview: preview })
  }

  /**
   * 把本地续接游标抬到不低于 `seq`（只升不降）。
   *
   * 游标语义是"已分配过的最大序号"，因此收到一个更大的后端序号时必须跟上；
   * 收到更小的（压缩把序号挪到被压内容的槽位上）则**不动**——那正是"游标在前、
   * 后端序号在后"的正常情形，压回去只会让后续本地消息撞号。
   */
  function raiseSeqFloor(sessionId: string, seq: number) {
    const cur = sessionSeq.value[sessionId] ?? 0
    if (seq <= cur) return
    sessionSeq.value = { ...sessionSeq.value, [sessionId]: seq }
  }

  /**
   * 「本地号与存储号已分叉」的待对账标记（会话空闲时回读一次收敛）。
   *
   * 与 `is_waiting_approval` 一类**状态**不同，它记的是“当前这份本地转写不可全信”
   * 这个事实——所以只在触发时置位（在途节点拿到权威号），由 `reconcileTranscript`
   * 消费后清除，不参与任何渲染判定。
   */
  const transcriptDirty = ref<Record<string, boolean>>({})

  function markTranscriptDirty(sessionId: string) {
    transcriptDirty.value = { ...transcriptDirty.value, [sessionId]: true }
  }

  /** 合并 patch 到指定 session 的某条消息（不替换，只覆盖 patch 提供的字段） */
  function patchMessage(sessionId: string, patch: ChatMessage) {
    if (!sessionId || !patch.id) return
    updateMessages(sessionId, (cur) => {
      const existing = cur[patch.id]
      if (!existing) {
        cur[patch.id] = {
          content: '',
          status: MESSAGE_STATUS_STREAMING,
          role: CHAT_ROLE_ASSISTANT,
          timestamp: Date.now(),
          seq: nextSeq(sessionId),
          ...patch
        }
      } else {
        // 合并语义见 `sessionTranscript.mergeMessagePatch`（追加 vs 整条替换）
        cur[patch.id] = mergeMessagePatch(existing, patch)
      }
    })
  }

  function nextSeq(sessionId: string): number {
    const cur = sessionSeq.value[sessionId] ?? 0
    const n = cur + 1
    sessionSeq.value = { ...sessionSeq.value, [sessionId]: n }
    return n
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
   * - `applySessionNode`（会话节点状态迁移）
   * - `vdfsTranscriptSync`（由消息节点派生活动文字 / 审批角标）
   * - `useChatConnection.send` / `resume` 的乐观置位
   * - `setSessionStatus` 同步 list.status 时
   * - 消息落地路径写 `last_preview`（取到预览才写）
   */
  function putStatus(sessionId: string, partial: Partial<SessionLiveStatus>) {
    // 强制覆盖 last_event_at：调用方无需、也不应手动维护这个时间戳
    const { last_event_at: _ignored, ...rest } = partial
    updateStatus(sessionId, (cur) => ({ ...cur, ...rest, last_event_at: Date.now() }))
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
    commitStatuses(snext)
  }

  /**
   * 把后端拉来的历史**合并**进 store 的实时缓存（loadMessages 调用）。
   *
   * 合并规则（保留谁 / 丢弃谁 / `seq` 怎么分配）全部在
   * `sessionTranscript.hydrateTranscript` —— 那是「快照 vs 在途节点」的唯一判据实现。
   */
  function hydrateFromHistory(sessionId: string, messages: ChatMessage[]) {
    const { map, waitingApproval, lastSeq } = hydrateTranscript(
      messages,
      sessionMessages.value[sessionId] || {},
    )

    commitMessages({ ...sessionMessages.value, [sessionId]: map })
    // 续接游标取"已分配 seq 的最大值"，保证下一轮 nextSeq 严格递增。
    sessionSeq.value = { ...sessionSeq.value, [sessionId]: lastSeq }

    // 只还原"等待审批"（它由转写派生，是**消息**节点的属性）。
    // 「上一轮失败」不在此还原：它是**会话节点**的状态（`status == 'failed'`），
    // 由 `list` 快照 / 节点变更落定——从消息历史反推会造出第二份真相，
    // 且与节点状态可能不一致（历史里有失败 Turn ≠ 会话当前处于失败态）。
    // 缺省状态由通道给（`last_event_at` = 当前时刻）——历史里没有状态条目时，
    // 若写成 0 会被 `getSessionStaleReason` 立刻判成「状态已过期」。
    updateStatus(sessionId, (cur) => ({ ...cur, is_waiting_approval: waitingApproval }))
  }

  /** 读取会话运行模式（默认 interactive） */
  function getSessionMode(id: string): SessionMode {
    return sessionModes.value[id] || 'interactive'
  }

  /** 读取会话执行风险等级（默认 medium） */
  function getSessionRiskLevel(id: string): SessionRiskLevel {
    return sessionRiskLevels.value[id] || 'medium'
  }

  /**
   * 从清单项回填会话级选择（mode / risk_level）到 store map。
   *
   * 合法取值判定在 `sessionLive.modeRiskBackfillOf`；本函数只负责写回 ref。
   */
  function backfillSessionMaps(items: SessionListItem[]) {
    const { modes, risks } = modeRiskBackfillOf(items)
    if (Object.keys(modes).length > 0) {
      sessionModes.value = { ...sessionModes.value, ...modes }
    }
    if (Object.keys(risks).length > 0) {
      sessionRiskLevels.value = { ...sessionRiskLevels.value, ...risks }
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
      // 清单到手 ⇒ 若有会话，转写段现在推导得出来。补一次解析，关掉「引导窗口」
      // （方案未就绪时转写变更无法路由）。失败不打断清单刷新——零会话时会失败，
      // 那正是预期；等新建会话时再补。
      if (items.length > 0) {
        void ensureVdfsSessionScheme().catch((e: unknown) =>
          logger.warn('[sessions]', '转写段解析失败', e),
        )
      }
      // 同步 lastUsedWorkdir 与标题缓存（后端 vdfs/list 的 title 已按 display_title
      // 下发：metadata.title 优先，否则从会话内容自动生成——前端不再自行拉消息推导）
      for (const it of items) {
        const wd = workdirOf(it)
        if (wd) lastUsedWorkdir.value = wd
        const t = titleOf(it)
        if (t) titles.value[it.id] = t
      }
      // 回填会话级选择（mode / risk_level）到 store map（机制见 backfillSessionMaps）
      backfillSessionMaps(items)

      // 合并运行态：list 接口返回的是后端 ActiveSessionManager 的权威状态；
      // 本地镜像说「运行中」的条目保留（乐观置位不能被稍早的 list 快照打回）
      const liveStatuses = sessionStatuses.value
      list.value = mergeListWithLive(items, liveStatuses)

      // 关键：把后端权威 status 回填到 sessionStatuses（isLoading 的唯一来源）。
      // 否则页面重载/视图挂载后，运行中的会话在输入框显示为禁用的"发送"按钮，
      // 用户无法点击停止（stop 按钮失效 bug）。
      // 仅做 false→true 的升级：true→false 的收敛交给节点状态变更，
      // 避免 list 快照与实时变更竞争时误降级。
      for (const id of workingUpgradesOf(items, liveStatuses)) {
        putStatus(id, { status: VDFS_STATUS_WORKING, activity: ACTIVITY_WORKING })
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
   * 编造——它只需要知道「建在哪」（`<根>/session`），名字由后端给。
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
      await ensureSessionMountDir(),
      JSON.stringify({ metadata: meta }),
      { create: true }
    )
    const id = vdfsBase(resp.path)
    // `vdfsBase` 对空路径返回**虚拟根**这个哨兵，因此空地址既不是 `''` 也不是
    // 合法 id——必须显式挡掉，否则会插一条 id 为 `<根>` 的幽灵会话。
    if (!id || id === vdfsRoot()) throw new Error('新建会话未返回地址（provider 未给出新节点路径）')

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
    commitStatuses({ ...sessionStatuses.value, [id]: newLiveStatus() })

    if (typeof meta.workdir === 'string' && meta.workdir) lastUsedWorkdir.value = meta.workdir

    // 前端模式通知：以同构载荷即时告知其他页面（工作台清单等），不等后端事件往返；
    // 后端 created 事件（`vdfs/write` 落库后广播）随后到达，各订阅方幂等收敛
    publishVdfsChangedLocal({
      path: vdfsSessionAddr(await ensureSessionMountDir(), id),
      change: VDFS_CHANGE_CREATED,
    })

    // 新建完就有了会话 ⇒ 转写段现在推导得出来了。补一次解析，把「引导窗口」
    // （方案未就绪时转写变更无法路由）在第一次发言之前关掉。
    void ensureVdfsSessionScheme().catch((e: unknown) =>
      logger.warn('[sessions]', '转写段解析失败（将在下次清单刷新时重试）', e),
    )

    return id
  }

  /**
   * 新建会话并排队其首条消息（懒创建的唯一入口）。
   *
   * 把「建会话」与「排队首条消息」合成**一个原子操作**：队列项的 `id` 由本函数
   * 直接取自 `createSession` 的返回值，故「排队项的 id === 新建出来的 id」这条
   * 不变式不可能被调用方写错（此前是调用方先拿 id、再赋值 `pendingFirstMessage`，
   * 漏第二步即静默丢首条消息）。
   *
   * 失败语义：`createSession` 抛错时邮箱**保持原样**（不会留下指向不存在会话的
   * 半截队列项），调用方保留草稿即可重试。
   */
  async function createSessionWithFirstMessage(
    metadata: Record<string, unknown> | undefined,
    first: { text: string; images?: ImageAttachment[] }
  ): Promise<string> {
    const id = await createSession(metadata)
    pendingFirstMessage.value = { id, text: first.text, images: first.images }
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

  /**
   * 删除会话（连同其全部消息）。
   *
   * 写入入口是 **VDFS**：`delete(<根>/session/<id>)`（`services/session.ts::deleteSession`
   * 只是它的具名包装）。后端 provider 的 `delete` 与曾经的 `session/clear` 路由
   * 共用同一份实现，因此这是一次入口替换，不是第二种删除方式。
   *
   * 本地清单的移除由 `removeSessionLocal` 完成（乐观收敛），后端 `deleted` 变更
   * 随后到达、各订阅方幂等收敛——两条路径行为一致。
   */
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
      await apiDeleteSession(id)
    } catch (e) {
      logger.error('[sessions]', 'deleteSession 失败', e)
      throw e
    }

    removeSessionLocal(id)

    // 前端模式通知：以同构载荷即时告知其他页面（工作台清单等），不等后端事件往返；
    // 后端 deleted 事件随后到达，各订阅方幂等收敛
    publishVdfsChangedLocal({
      path: vdfsSessionAddr(await ensureSessionMountDir(), id),
      change: VDFS_CHANGE_DELETED,
    })
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
   * 转写是**列表**（`<根>/session/<id>/消息`），而会话叶子
   * `<根>/session/<id>` 的内容就是整份会话文档（含 `messages`）——因此
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
    // 读入口是会话域的 VDFS 门面：文档形状的解析不在 store 里
    const msgs = await readSessionTranscript(id)
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
   * 从前端局部状态中精确移除一批消息（id 由后端 `chat/delete_message`
   * 返回的 `deleted_ids` 决定：目标消息 + 其后所有消息）。
   */
  function removeMessages(sessionId: string, ids: string[]) {
    if (ids.length === 0) return
    updateMessages(sessionId, (cur) => {
      for (const id of ids) delete cur[id]
    })
  }

  /**
   * 从某条消息起**截断到列表末尾**：移除「目标 + 其后全部」。
   *
   * 区间判据在 `sessionTranscript.truncateIdsFrom`（与后端 `drain(i..)` 同源）。
   * 锚点不在本地（会话没加载 / 已被别处删掉）时**什么都不删**并返回空数组——
   * 绝不拿一个并不存在的锚点去截断整个列表。
   *
   * @returns 实际被移除的消息（调用方据此回滚 / 记日志）
   */
  function removeFrom(sessionId: string, messageId: string): ChatMessage[] {
    if (!sessionId || !messageId) return []
    const cur = sessionMessages.value[sessionId]
    if (!cur || !cur[messageId]) return []

    const ids = truncateIdsFrom(getSessionMessages(sessionId), messageId)
    if (ids.length === 0) return []

    const removed: ChatMessage[] = []
    updateMessages(sessionId, (rest) => {
      for (const id of ids) {
        const m = rest[id]
        if (!m) continue
        delete rest[id]
        removed.push(m)
      }
    })
    if (removed.length === 0) return []
    return removed
  }

  /** 把一批消息放回局部状态（`removeFrom` 的回滚口，仅在写后端失败时使用） */
  function restoreMessages(sessionId: string, msgs: ChatMessage[]) {
    if (msgs.length === 0) return
    updateMessages(sessionId, (cur) => {
      for (const m of msgs) cur[m.id] = m
    })
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
    updateMessages(sessionId, (c) => {
      delete c[messageId]
    })
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
    // 定稿口径（谁挂 error / 谁定稿为 completed）是纯逻辑，见
    // `sessionTranscript.stuckFailurePlanOf`——本函数只负责按计划落库与收敛。
    const plan = stuckFailurePlanOf(getSessionMessages(sessionId), errorText)
    for (const m of [...plan.failed, ...plan.completed]) {
      try {
        await apiUpdateMessage(sessionId, m)
      } catch (e) {
        logger.warn('[sessions]', 'persistStuckFailure updateMessage 失败', e)
      }
      patchMessage(sessionId, { ...m })
    }
    // 没有任何根级 Turn（例如首帧到达前就卡住、连 Turn 都还没创建）：
    // 降级为会话级错误状态（不注入错误节点），保证仍能在 UI 上看到错误并可重试。
    if (!plan.rootTurnFailed) {
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

  // ===== 会话节点的 VDFS 变更 → 侧栏清单同步 =====
  //
  // 订阅**不在本工厂里**：`stores/sessionNodeSync.ts` 是那条接线，由应用外壳
  // （`MainLayout`）显式 `startSessionNodeSync(store)` 启动——本 store 只提供
  // 收敛动作（`applySessionNode` / `removeSessionLocal` / `refreshList`），
  // 不自己挂监听器，因此可被独立构造与测试。

  /**
   * 转写对账：**终态补丁丢失后的自愈**（会话已空闲却仍有节点停在非终态）。
   *
   * ## 它补的是哪条不变量的客户端一半
   *
   * 后端有一条硬不变量：**不得有节点停在 `Streaming`**
   * （`session/docs/node-state-streaming.md` §8.11）——它在服务端由
   * `process_tool_calls_async` 的批尾收口、`persist_failure`、`converge_inflight`
   * 三处共同保证，且**落库结果实测干净**。
   *
   * 但「服务端状态正确」不等于「前端看到的正确」：消息状态只经
   * `kind = "vdfs"` 一条通道下发，而这条通道**不重放**——
   * `ChangeSubscriptions::notify` 在订阅表为空时直接返回（watch 登记是
   * fire-and-forget 的异步动作），消费循环也会在 `is_working` 翻转时
   * **丢掉手上那一帧**。任一处丢一次终态补丁，前端就永久停在「运行中」：
   * 没有任何机制会纠正它，因为前端只做**增量收敛**、从不整表重拉。
   *
   * 于是这里给出客户端一半：**会话已空闲却还有非终态节点 ⇒ 一定丢了一条终态**。
   *
   * ## 为什么这个判据成立（不是启发式）
   *
   * 节点补丁**恒先于**会话状态下发——正常收尾是「清在途 → 复位 `is_working`
   * → `emit_session_state`」，中止收尾是「`converge_inflight` 广播节点终态
   * → `emit_session_state(aborted)`」（`handle_abort` 里那条顺序注释写明了
   * 理由）。因此会话节点报「不忙」时，服务端的每一个节点都已是终态、
   * 且已写进存储。
   *
   * ## 为什么是**回读**而不是就地"标成已完成"
   *
   * 就地标 `completed` 是**猜**：服务端可能把它定稿成 `aborted`（用户中止）
   * 或 `failed`。猜错就把"半截"谎报成"正常结束"，并抹掉重试入口——
   * 正是 `aborted` 状态漏在状态词表里时踩过的坑。回读拿到的是权威终态，
   * 一次 IPC 换一个确定的答案。
   *
   * ## 为什么回读前后都要再看一眼 `isSessionWorking`
   *
   * 回读是异步的，其间用户可能已经发出下一条消息：此时在途节点**重新合法**，
   * 存储也不再是权威（新一轮还没落库）。丢弃本次结果即可——下一次会话
   * 转空闲时会重新触发。
   */
  async function reconcileTranscript(id: string): Promise<void> {
    if (!id || isSessionWorking(id)) return
    let msgs: ChatMessage[]
    try {
      msgs = await readSessionTranscript(id)
    } catch (err) {
      // 回读失败就保持现状：陈旧副本比"清空转写"好，下一次转空闲会再试
      // （待对账标记**不清**，正是为了让下一次还有机会）
      logger.warn('[sessions]', `转写对账回读失败，保持现状：${id}`, err)
      return
    }
    if (isSessionWorking(id)) return

    // 回读成功 ⇒ 本地转写已重新以存储为权威（含全部 `seq`），待对账标记解除。
    // 回读期间会话又跑起来了则提前返回——重入的一轮由下一次空闲再收。
    if (transcriptDirty.value[id]) {
      const next = { ...transcriptDirty.value }
      delete next[id]
      transcriptDirty.value = next
    }

    // 丢弃仍停在非终态的本地副本。会话已空闲 ⇒ 服务端不可能还有节点在跑，
    // 这一份必然是丢补丁留下的陈旧副本；它若真在存储里，下面的快照会把它
    // 以**权威终态**整条补回来（同一 id）。
    const cur = sessionMessages.value[id] || {}
    const stale = Object.values(cur).filter((m) => isInflightMessageStatus(m.status))
    if (stale.length > 0) {
      const next = { ...cur }
      for (const m of stale) delete next[m.id]
      commitMessages({ ...sessionMessages.value, [id]: next })
      logger.warn(
        '[sessions]',
        `转写对账：会话 ${id} 已空闲但仍有 ${stale.length} 个节点停在非终态，` +
          `已按存储权威状态重收敛（疑似终态补丁丢失）：${stale.map((m) => m.id).join(', ')}`,
      )
    }
    // 快照（存储）权威：终态、内容、seq 一并落定
    hydrateFromHistory(id, msgs)
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

    // ① 清单条目就地收敛（不整表重拉）：补丁口径见 `sessionLive.listItemPatchOf`
    const title = typeof node.title === 'string' ? node.title : ''
    const idx = list.value.findIndex((s) => s.id === id)
    if (idx >= 0) {
      list.value[idx] = listItemPatchOf(node, list.value[idx], rt.status)
    }
    if (title) titles.value[id] = title

    // ② 运行态镜像：节点状态 / 结局直通；activity 只在迁移时改写（见 liveStatusPatchOf）
    putStatus(id, liveStatusPatchOf(rt, wasWorking, nowWorking))

    // ③ 会话级错误 = 节点属性（覆盖「错误发生在任何消息节点创建之前」的场景）。
    //    新一轮开始（working）时节点不带 error ⇒ 自动清空上一轮的错误。
    setSessionError(id, rt.error ?? null)

    // ④ 提示音：状态迁移 + 结局选音色
    if (wasWorking && !nowWorking) {
      playCompletionChime(chimeKindOfOutcome(rt.outcome), id)
    }

    // ⑤ 终态丢失的自愈：会话已空闲却仍有节点停在非终态 ⇒ 一定丢了一条终态补丁
    //    （VDFS 变更不重放，前端只做增量收敛，没人纠正它）。
    //    判据只在**有候选**时才成立，因此正常运行路径零成本——不满足条件时
    //    连一次回读都不会发生。
    if (!nowWorking && (hasUnsettledNodes(id) || transcriptDirty.value[id])) {
      void reconcileTranscript(id)
    }
  }

  /** 本地是否还有停在非终态的节点（对账的触发条件，零 IPC） */
  function hasUnsettledNodes(id: string): boolean {
    const cur = sessionMessages.value[id]
    if (!cur) return false
    return Object.values(cur).some((m) => isInflightMessageStatus(m.status))
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
    // 后端 deleted 变更的落地口（与 deleteSession 的乐观移除同一实现，
    // 见 `sessionNodeSync`：两条路径行为一致）
    removeSessionLocal,
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
    reconcileTranscript,
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
    // 新建模式（无 id 详情）懒创建：建会话 + 排队首条消息（原子），以及取出
    // 排队消息。邮箱本体（`pendingFirstMessage`）**不对外暴露**——唯一入口是
    // `createSessionWithFirstMessage`，否则「排队项的 id === 新建出来的 id」
    // 这条不变式又散回调用方。
    createSessionWithFirstMessage,
    consumePendingFirstMessage,
    // 执行风险等级（low / medium / high）
    getSessionRiskLevel,
    // 级联选项机制：session/update 的本地镜射（metadata 浅合并）
    applySessionMetadataPatch
  }
})

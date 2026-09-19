/**
 * 会话转写规则 —— 消息级的**纯逻辑**（无响应式、无 IPC、无 store）
 *
 * ## 为什么从这里拆
 *
 * `stores/sessions.ts` 里最不能出错、也最难验证的几条规则都长在消息转写上：
 *
 * 1. **流式补丁的合并语义**（追加 vs 整条替换）——判错会让内容重复拼接或倒退；
 * 2. **历史快照与本地在途节点的合并**——判错会让在途节点凭空消失（lost update），
 *    或让已删除的陈旧副本复活成幽灵节点；
 * 3. **「目标 + 其后全部」的截断区间**——必须与后端 `drain(i..)` 判据同源，
 *    否则列表尾部会残留一个后端已不存在的节点，且**没有任何机制会纠正它**；
 * 4. **看门狗的定稿口径**——只有根级 Turn 允许挂 `error`（否则同一条错误会刷到
 *    每条半截消息上，正是原始 429 刷屏的根因）。
 *
 * 这些规则原先只能通过「造一个 Pinia store + 喂一串 patch」间接验证。抽成纯函数后
 * 每条规则都能直接断言（见 `__tests__/sessionTranscript.spec.ts`），
 * store 退化为「持有状态 + 调这些函数」。
 */

import {
  MESSAGE_STATUS_COMPLETED,
  MESSAGE_STATUS_FAILED,
  MESSAGE_STATUS_STREAMING,
  MESSAGE_STATUS_WAITING_USER_ACTION,
  MESSAGE_TYPE_TURN,
  isInflightMessageStatus,
  messageTextOf,
  type ChatMessage,
} from '@/schemas/chat_message'

/** 缩略卡预览的最大字符数（超出截断并加省略号） */
export const PREVIEW_MAX = 60

/**
 * 按 `seq` 升序排列（缺失 `seq` 时回退 `timestamp`）。
 *
 * `seq` 是**唯一的权威顺序锚点**：`timestamp` 是"业务时刻"而非"顺序"，
 * 旧数据还可能缺失，只作兜底。返回新数组，不改动入参。
 */
export function sortTranscript(messages: ChatMessage[]): ChatMessage[] {
  return [...messages].sort((a, b) => (a.seq ?? a.timestamp ?? 0) - (b.seq ?? b.timestamp ?? 0))
}

/**
 * 缩略卡预览文案（取 assistant 的文本内容，超长截断）。
 *
 * 非 assistant、或取不到任何文本时返回 `null`——调用方据此**不改动**预览
 * （而不是把预览清空：一条工具调用不该把上一句正文的预览抹掉）。
 */
export function previewOf(msg: Pick<ChatMessage, 'role' | 'content'>): string | null {
  if (msg.role !== 'assistant') return null
  // 取值走契约层的唯一实现（`messageTextOf`）：本模块是纯逻辑层，
  // 不在这里再写一份多模态形状的判定。
  const text = messageTextOf(msg.content)
  if (!text) return null
  return text.length > PREVIEW_MAX ? text.slice(0, PREVIEW_MAX) + '…' : text
}

/**
 * 把一条流式补丁合并进已有消息（**不替换，只覆盖补丁提供的字段**）。
 *
 * ## 内容合并的两种语义
 *
 * - **整条替换**：`tool_call`（参数即内容）与 `role = tool`（工具返回）。
 *   这两类的流式帧是**全量重发**，追加会把 JSON 拼成垃圾。
 * - **追加**：其余字符串内容（正文 / 思考的增量 token）。
 *
 * 注意只在 `patch.content != null` 时才动内容：终态状态补发（`content` 为
 * `null`/`undefined`）不应清空已有内容——ToolCall 的参数正是存在自身 `content` 里的，
 * 一次"仅更新状态"的补丁若把内容清了，参数面板就空了。
 *
 * `meta` 是**浅合并**（后端 flatten 的场景字段按 patch 增量下发）。
 */
export function mergeMessagePatch(existing: ChatMessage, patch: ChatMessage): ChatMessage {
  const merged: ChatMessage = { ...existing, ...patch }
  if (patch.content != null) {
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
  return merged
}

/** 合并结果：消息字典 + 审批角标 + 续接游标 */
export interface HydratedTranscript {
  map: Record<string, ChatMessage>
  /** 快照里是否存在待审批消息（缩略卡角标） */
  waitingApproval: boolean
  /** 续接游标：已分配 `seq` 的最大值，保证下一轮 `nextSeq` 严格递增 */
  lastSeq: number
}

/**
 * 把后端拉来的历史**合并**进本地实时缓存。
 *
 * ## 为什么是合并而不是整表替换
 *
 * 这份快照来自一次 IPC 往返，取回期间流式补丁仍在到达并写入本地。整表替换会把那些
 * **更新的**补丁覆盖掉（经典 lost update），而它们不会被重发——结果就是消息内容倒退
 * 若干 token，或整条在途节点凭空消失。
 *
 * ## 规则
 *
 * - 快照里的节点 → 权威，整条替换（含状态与 `seq`）；
 * - 本地有、快照没有的节点 → **仅当它仍在飞行中**（`isInflightMessageStatus`）时保留。
 *   那正是 `persist_messages` 还没写盘的在途节点，也是「Turn 运行中切走再切回」时
 *   最容易被丢掉的东西；终态且不在快照里的一律丢弃（被删除或已过期的陈旧副本，
 *   保留它会让幽灵节点复活）。
 *
 * `seq` 分配：缺失 `seq` 的旧数据按后端数组顺序排在已有 `seq` 的消息之后
 * （不抢到前面）；保留的本地在途节点**重新分配 `seq`**——本地游标可能小于快照的
 * 最大 `seq`（页面刚加载时游标从 0 起），沿用会让在途节点排到历史之前。
 */
export function hydrateTranscript(
  snapshot: ChatMessage[],
  local: Record<string, ChatMessage>,
): HydratedTranscript {
  const map: Record<string, ChatMessage> = {}
  const fromSnapshot = new Set<string>()
  const maxSeq = snapshot.reduce(
    (mx, m) => Math.max(mx, typeof m.seq === 'number' ? m.seq : 0),
    0,
  )
  let cursor = maxSeq + 1
  let waitingApproval = false

  for (const m of snapshot) {
    if (!m.id) continue
    const seq = typeof m.seq === 'number' ? m.seq : cursor++
    map[m.id] = { ...m, seq }
    fromSnapshot.add(m.id)
    // 还原「等待审批」状态，使会话卡片角标在重开会话时正确显示
    if (m.status === MESSAGE_STATUS_WAITING_USER_ACTION) waitingApproval = true
  }

  for (const [id, m] of Object.entries(local)) {
    if (fromSnapshot.has(id) || !isInflightMessageStatus(m.status)) continue
    map[id] = { ...m, seq: cursor++ }
  }

  const lastSeq = Object.values(map).reduce((mx, m) => Math.max(mx, m.seq ?? 0), 0)
  return { map, waitingApproval, lastSeq }
}

/**
 * 「目标消息 + 其后全部」的 id 序列（按给定顺序）。
 *
 * ## 为什么前端要自己算这段区间
 *
 * 后端 `chat/delete_message` 的语义就是「在已排序列表里删掉目标及其之后的所有消息」
 * （`handlers.rs::invoke_delete_message` 的 `messages.drain(i..)`）。前端若只删这一条、
 * 再等后端把被删的每一条逐个通知回来，就把一次确定的**区间删除**拆成了 N 次通知；
 * 而且只要漏掉其中任意一条，列表尾部就会残留一个后端已不存在的节点。
 *
 * 判据与后端**同源**：两边都按 `seq` 升序排（后端 `seq.unwrap_or(i64::MAX)`，
 * 本地 `seq ?? timestamp`），因此「取目标及其之后的全部」在两侧是同一个集合。
 * 这里按**排序后的位置**切片而不是直接比较 `seq` 大小：两种写法在有 `seq` 时等价，
 * 而位置切片在缺 `seq` 的旧数据上也仍然正确（不需要额外假设 `seq` 一定存在）。
 *
 * 锚点不在列表里 → 返回空数组：**绝不拿一个并不存在的锚点去截断整个列表**。
 *
 * @param ordered 已按 `sortTranscript` 排好序的消息
 */
export function truncateIdsFrom(ordered: ChatMessage[], anchorId: string): string[] {
  if (!anchorId) return []
  const anchorIdx = ordered.findIndex((m) => m.id === anchorId)
  if (anchorIdx < 0) return []
  return ordered.slice(anchorIdx).map((m) => m.id)
}

/**
 * 根级 Turn：`type = turn` 且无父节点。
 *
 * 后端 `persist_failure` 只把错误挂在**根级 Turn** 上（「错误是状态、且只由造成中止的
 * 根 Turn 承载」）；其余仍在进行中的子节点定稿为 `completed`、绝不挂 `error`。
 * 前端看门狗必须用同一判据，否则会把同一条错误刷到每条半截消息上。
 */
export function isRootTurn(msg: Pick<ChatMessage, 'type' | 'parent_id'>): boolean {
  return msg.type === MESSAGE_TYPE_TURN && !msg.parent_id
}

/** 仍在进行中的消息（看门狗据此决定哪些节点需要定稿） */
export function isInProgressMessage(msg: Pick<ChatMessage, 'status'>): boolean {
  return (
    msg.status === MESSAGE_STATUS_STREAMING ||
    msg.status === MESSAGE_STATUS_WAITING_USER_ACTION
  )
}

/**
 * 看门狗的**定稿计划**：卡在飞行中的一批消息分别该怎么收尾。
 *
 * 判定与后端 `persist_failure` 同源（见 `isRootTurn`）：错误只挂在根级 Turn 上，
 * 其余半截子节点定稿为 `completed` 且**绝不挂 error**——否则同一条错误会刷到
 * 每条消息上（原始 429 刷屏的根因）。
 *
 * 抽成纯函数的理由与同文件其他规则一样：这是「最不能出错、又最难验证」的口径，
 * 留在 store 的 async 循环里就只能靠造 store 间接测；抽出来可直接断言（见
 * `__tests__/sessionTranscript.spec.ts`）。store 退化为「按计划执行 + 落库」。
 */
export interface StuckFailurePlan {
  /** 承担错误的根级 Turn（`status = failed` + `error`） */
  failed: ChatMessage[]
  /** 进行中的子节点：定稿为 `completed`，结束流式动画，不挂 error */
  completed: ChatMessage[]
  /** 是否存在根级 Turn —— false 时调用方降级为「会话级错误」 */
  rootTurnFailed: boolean
  /** true 表示没有任何节点需要定稿（调用方可以直接收尾，不必走落库） */
  empty: boolean
}

export function stuckFailurePlanOf(
  messages: ChatMessage[],
  errorText: string,
): StuckFailurePlan {
  const failed: ChatMessage[] = []
  const completed: ChatMessage[] = []
  for (const m of messages.filter(isInProgressMessage)) {
    if (isRootTurn(m)) failed.push({ ...m, status: MESSAGE_STATUS_FAILED, error: errorText })
    else completed.push({ ...m, status: MESSAGE_STATUS_COMPLETED, error: undefined })
  }
  return {
    failed,
    completed,
    rootTurnFailed: failed.length > 0,
    empty: failed.length === 0 && completed.length === 0,
  }
}

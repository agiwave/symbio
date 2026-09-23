/**
 * 会话转写规则 —— 消息级的**纯逻辑**（无响应式、无 IPC、无 store）
 *
 * ## 为什么从这里拆
 *
 * `stores/sessions.ts` 里最不能出错、也最难验证的几条规则都长在消息转写上：
 *
 * 1. **两种落点的语义边界**（帧带 `content` 整条替换 vs 帧带 `delta` 尾部追加）——
 *    判错会让内容重复拼接或倒退。语义由**帧的字段**给出，不从节点 `type` / `role`
 *    反推；
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
  CHAT_ROLE_ASSISTANT,
  MESSAGE_STATUS_STREAMING,
  MESSAGE_STATUS_WAITING_USER_ACTION,
  MESSAGE_TYPE_REASONING,
  MESSAGE_TYPE_TEXT,
  MESSAGE_TYPE_TURN,
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
  if (msg.role !== CHAT_ROLE_ASSISTANT) return null
  // 取值走契约层的唯一实现（`messageTextOf`）：本模块是纯逻辑层，
  // 不在这里再写一份多模态形状的判定。
  const text = messageTextOf(msg.content)
  if (!text) return null
  return text.length > PREVIEW_MAX ? text.slice(0, PREVIEW_MAX) + '…' : text
}

/**
 * 把一段增量追加到消息正文尾部（**帧携带 `delta` 时**的唯一内容落点）。
 *
 * ## 为什么不再按节点类型猜"追加还是替换"
 *
 * 这里原本是 `mergeMessagePatch`：它从 `type` / `role` 反推语义——`tool_call` 与
 * `role = tool` 被判成"流式帧全量重发"因而**整条替换**，其余才追加。那条规则赖以
 * 成立的前提已经不存在：帧的语义由**字段**给出（`delta` = 尾部追加，
 * `content` = 整条替换），后端对工具参数与工具响应发的都是窄增量。
 * 继续按类型猜的代价是实测过的事故：**流式工具响应的增量被当成全量重发**，
 * 正文被最后一片覆盖——工具卡片有请求、响应是空的。
 *
 * 取值走契约层的唯一实现（`messageTextOf`）：增量帧的目标是文本正文，多模态形状
 * 不该被静默丢成空串。
 */
export function appendContent(msg: ChatMessage, delta: string): ChatMessage {
  return { ...msg, content: messageTextOf(msg.content) + delta }
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
 * 把后端拉来的历史快照装进本地消息表（**快照即权威，整表装载**）。
 *
 * 转写的权威副本在存储：`loadMessages`（切换会话 / 显式刷新）以此整表装载，
 * 增量由 `sessionTranscriptSync`（VDFS 变更消费端）持续收敛——装载与增量消费是同一条数据链路的
 * 两个入口，不存在需要"合并"的两个真相来源。
 *
 * `seq` 分配：缺失 `seq` 的旧数据按快照数组顺序续在已有最大 `seq` 之后
 * （不抢到前面），保证排序判据对新旧数据都成立。
 */
export function hydrateTranscript(
  snapshot: ChatMessage[],
  _local?: Record<string, ChatMessage>,
): HydratedTranscript {
  void _local
  const map: Record<string, ChatMessage> = {}
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
    // 还原「等待审批」状态，使会话卡片角标在重开会话时正确显示
    if (m.status === MESSAGE_STATUS_WAITING_USER_ACTION) waitingApproval = true
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
 * TurnGroupNode 的重试按钮判定也据此识别"整轮失败"这一状态。
 */
export function isRootTurn(msg: Pick<ChatMessage, 'type' | 'parent_id'>): boolean {
  return msg.type === MESSAGE_TYPE_TURN && !msg.parent_id
}

/** 仍在进行中的消息（send/resume 的乐观置位判定据此识别"已有在途节点"） */
export function isInProgressMessage(msg: Pick<ChatMessage, 'status'>): boolean {
  return (
    msg.status === MESSAGE_STATUS_STREAMING ||
    msg.status === MESSAGE_STATUS_WAITING_USER_ACTION
  )
}

/**
 * 「这条节点自身是不是一个**空壳**」——文字类节点的空内容判据。
 *
 * 流模式下后端会先发一个 `content` 为空（如 `"\n\n"`）的 Text / Reasoning 占位
 * 节点（reasoning 与 tool_call 之间），它**不落库**，但前端会短暂收到。两处需要
 * 同一条判据，且必须**同源**：
 *
 * 1. 树构建（`useChatConnection.messageTree`）：空壳叶子不渲染，否则会话流里
 *    出现空白块；
 * 2. 等待骨架的兜底（`sessionLive.needsTypingRow`）：**空壳节点不算「有东西可看」**，
 *    否则它会把「会话在跑但什么都没显示」这个本该补一条骨架的时刻判成「已有内容」——
 *    结果正是用户看到的「点完发送，什么都没有发生」。
 *
 * 两处分头写一份，就会出现「一处过滤、另一处却把它算作内容」的静默分叉：
 * 不报错、不被测试拦住，只是偶尔转一下圈。
 *
 * 只对 `text` / `reasoning` 判定——`tool_call` 的 `content` 是参数 JSON，即便为空
 * 也有一张工具卡片要显示（工具名在 `name` 上）；容器类节点（Turn / ToolCall）
 * 靠子节点呈现，不适用本条。
 */
export function isBlankContentNode(msg: Pick<ChatMessage, 'type' | 'content'>): boolean {
  const t = msg.type || MESSAGE_TYPE_TEXT
  if (t !== MESSAGE_TYPE_TEXT && t !== MESSAGE_TYPE_REASONING) return false
  // 取值走契约层的唯一实现（多模态内容 → 纯文本）
  return messageTextOf(msg.content).trim().length === 0
}

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
 * 4. **看门狗的定稿口径**——错误是**状态**、由造成中止的那个节点承载（后端把它挂在
 *    本轮根节点上），半截子节点不再各刷一份同样的错误（否则一条错误刷屏到每条半截
 *    消息，正是原始 429 刷屏的根因）。前端只按**节点自身状态**渲染错误，不假设根节点
 *    是哪种形态——一轮响应的根既可能是骨架容器，也可能直接就是一个正文节点。
 * 5. **分支收敛**（重试留下的并存两支）——未选中那一支**连同整棵子树**不渲染。
 *    与后端的 `branch_hide_unselected` 同一条判据：只藏根不藏子孙，会把旧支的
 *    工具结果留在屏幕上跟新支并列；漏认 `meta.turn_root`，`flat` 档下正是这个症状。
 *
 * 这些规则原先只能通过「造一个 Pinia store + 喂一串 patch」间接验证。抽成纯函数后
 * 每条规则都能直接断言（见 `__tests__/sessionTranscript.spec.ts`），
 * store 退化为「持有状态 + 调这些函数」。
 */

import {
  CHAT_ROLE_ASSISTANT,
  NODE_STATUS_STREAMING,
  NODE_STATUS_WAITING_USER_ACTION,
  nodeKindOf,
  messageRoleOf,
  messageTextOf,
  type ChatNode,
} from '@/schemas/chat_node'
import { isContentKind } from '@/registry/messageTypes'
import { messageBranchOf, messageTurnRoot } from '@/registry/messageFacts'

/** 缩略卡预览的最大字符数（超出截断并加省略号） */
export const PREVIEW_MAX = 60

/**
 * 按 `seq` 升序排列（缺失 `seq` 时回退 `timestamp`）。
 *
 * `seq` 是**唯一的权威顺序锚点**：`timestamp` 是"业务时刻"而非"顺序"，
 * 旧数据还可能缺失，只作兜底。返回新数组，不改动入参。
 */
export function sortTranscript(messages: ChatNode[]): ChatNode[] {
  return [...messages].sort((a, b) => (a.seq ?? a.timestamp ?? 0) - (b.seq ?? b.timestamp ?? 0))
}

/**
 * 缩略卡预览文案（取 assistant 的文本内容，超长截断）。
 *
 * 非 assistant、或取不到任何文本时返回 `null`——调用方据此**不改动**预览
 * （而不是把预览清空：一条工具调用不该把上一句正文的预览抹掉）。
 */
export function previewOf(msg: Pick<ChatNode, 'principal' | 'content'>): string | null {
  // 角色判据走契约层唯一折叠点——身份只在 `principal` 上。
  if (messageRoleOf(msg) !== CHAT_ROLE_ASSISTANT) return null
  // 取值走契约层的唯一实现（`messageTextOf`）：本模块是纯逻辑层，
  // 不在这里再写一份多模态形状的判定。
  const text = messageTextOf(msg.content)
  if (!text) return null
  return text.length > PREVIEW_MAX ? text.slice(0, PREVIEW_MAX) + '…' : text
}

/**
 * 把一段增量追加到消息正文尾部（**帧携带 `delta` 时**的唯一内容落点）。
 *
 * ## 为什么按字段而非节点类型判定「追加还是替换」
 *
 * 帧的语义由**字段**给出（`delta` = 尾部追加，`content` = 整条替换），后端对工具参数
 * 与工具响应发的都是窄增量。若反过来按 `type` / `role` 猜（把 `tool_call` /
 * `role = tool` 判成"全量重发"因而整条替换），代价是实测过的事故：**流式工具响应的
 * 增量被当成全量重发**，正文被最后一片覆盖——工具卡片有请求、响应是空的。
 *
 * 取值走契约层的唯一实现（`messageTextOf`）：增量帧的目标是文本正文，多模态形状
 * 不该被静默丢成空串。
 */
export function appendContent(msg: ChatNode, delta: string): ChatNode {
  return { ...msg, content: messageTextOf(msg.content) + delta }
}

/** 合并结果：消息字典 + 审批角标 + 续接游标 */
export interface HydratedTranscript {
  map: Record<string, ChatNode>
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
  snapshot: ChatNode[],
  _local?: Record<string, ChatNode>,
): HydratedTranscript {
  void _local
  const map: Record<string, ChatNode> = {}
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
    if (m.status === NODE_STATUS_WAITING_USER_ACTION) waitingApproval = true
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
export function truncateIdsFrom(ordered: ChatNode[], anchorId: string): string[] {
  if (!anchorId) return []
  const anchorIdx = ordered.findIndex((m) => m.id === anchorId)
  if (anchorIdx < 0) return []
  return ordered.slice(anchorIdx).map((m) => m.id)
}

/** 仍在进行中的消息（send/resume 的乐观置位判定据此识别"已有在途节点"） */
export function isInProgressMessage(msg: Pick<ChatNode, 'status'>): boolean {
  return (
    msg.status === NODE_STATUS_STREAMING ||
    msg.status === NODE_STATUS_WAITING_USER_ACTION
  )
}

/**
 * 「这条节点自身是不是一个**空壳**」——正文 / 折叠叶（`body` / `notice`）格节点的空内容判据。
 *
 * 流模式下后端会先发一个 `content` 为空（如 `"\n\n"`）的占位叶子（落地折叠为 `body` /
 * `notice`，常夹在容器子流之间），它**不落库**，但前端会短暂收到。两处需要
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
 * 只对 `body` / `notice` 判定——`skeleton` 的 `content` 是参数 JSON，即便为空
 * 也有一张容器要显示（工具名在 title 上）；容器类节点（`skeleton`）
 * 靠子节点呈现，不适用本条。
 */
export function isBlankContentNode(msg: Pick<ChatNode, 'type' | 'content'>): boolean {
  const t = nodeKindOf(msg.type)
  if (!isContentKind(t)) return false
  // 取值走契约层的唯一实现（多模态内容 → 纯文本）
  return messageTextOf(msg.content).trim().length === 0
}

/**
 * 丢掉**未选中那一支**的整棵子树（分支切换的前端半边，与后端
 * `branch_hide_unselected` 同一条判据）。
 *
 * ## 为什么前端也要做一遍
 *
 * 后端的呈现读出口**不丢**任何东西：全树带标注交出去，「看不见」与「不存在」是
 * 分开的两件事——切回去还在，而 `message.removed` 才是真没了。于是「现在读哪一支」
 * 是**呈现**判定，得有人在前端执行它，否则两支答复并排画在会话流里，用户看到的是
 * 两条互相矛盾的助手发言。
 *
 * ## 子树判据是 `parent_id ∪ meta.turn_root`，不是只看 `parent_id`
 *
 * `flat` 档下本轮响应骨架的直接子节点**升了根**（`parent_id` 被摘掉，靠
 * `meta.turn_root` 记号认回）。只按 `parent_id` 收集，那一档下旧支只剩根节点被藏掉，
 * 它的工具结果与正文全部留在屏幕上——这条缺陷不报错，只是「切了分支但没换干净」。
 *
 * 没有任何分支标注时**原样返回**（同一个数组引用）：绝大多数会话没有并存两支，
 * 这里不该为它们新建一份列表。
 */
export function dropUnselectedBranches(messages: ChatNode[]): ChatNode[] {
  const hiddenRoots = messages.filter((m) => {
    const b = messageBranchOf(m)
    return b !== null && !b.selected
  })
  if (hiddenRoots.length === 0) return messages

  // 子边 = `parent_id` 与 `turn_root` 的**并集**（与后端 `node_build::subtree_of` 同源）
  const children = new Map<string, ChatNode[]>()
  const link = (pid: string | null | undefined, child: ChatNode) => {
    if (pid == null || pid === '') return
    const arr = children.get(pid)
    if (arr) arr.push(child)
    else children.set(pid, [child])
  }
  for (const n of messages) {
    link(n.parent_id, n)
    link(messageTurnRoot(n), n)
  }

  const gone = new Set<string>()
  const queue: string[] = hiddenRoots.map((m) => m.id)
  let guard = messages.length + 1 // 环防御：合法转写是树，不会触顶
  while (queue.length > 0 && guard-- > 0) {
    const id = queue.pop()!
    if (gone.has(id)) continue
    gone.add(id)
    for (const child of children.get(id) ?? []) queue.push(child.id)
  }
  return messages.filter((m) => !gone.has(m.id))
}

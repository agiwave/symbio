/**
 * 消息呈现注册表 —— 「消息类型 / 角色 / 状态 → 呈现」的**唯一来源**
 *
 * ## 分层（与 `registry/vdfsTypes.ts` 同一套）
 *
 * - `schemas/chat_message.ts`  数据契约：取值词表（常量 + 派生类型）+ 形状，零呈现知识
 * - 本文件                     **纯 UI 映射**：图标 / 标题 / 状态标签 / 折叠策略，零组件导入
 * - `components/MessageNode.vue` 等  消费方：只做「把节点摊成 facets → 查表 → 渲染」
 *
 * 这样做的直接收益：新增一种消息类型只需在 `MESSAGE_TYPES` 加一个取值、
 * 在本文件的表里加一行；**不需要再去改渲染组件里的 if 链**。
 *
 * ## 为什么入参是 `MessageFacets` 而不是 `ChatMessage`
 *
 * 判定要用到的不只是节点自身字段，还有「父节点是什么」「子节点里有没有待审批」
 * 这类**上下文**（如 `toolResult` 要求 `parentType === 'tool_call'`）。把这些上下文
 * 显式列进 facets，而不是让本模块去读 props，好处是：
 * 每个判定都是**纯函数**，可直接单测，且不需要为了测试去搭一棵组件树。
 *
 * 组件负责把 props 摊成 facets（那一层是"读 props"，不是"做判定"）。
 */

import type { Component } from 'vue'
import {
  CHAT_ROLE_ASSISTANT,
  CHAT_ROLE_SYSTEM,
  CHAT_ROLE_TOOL,
  CHAT_ROLE_USER,
  MESSAGE_STATUS_ABORTED,
  MESSAGE_STATUS_COMPLETED,
  MESSAGE_STATUS_FAILED,
  MESSAGE_STATUS_PENDING,
  MESSAGE_STATUS_STREAMING,
  MESSAGE_STATUS_WAITING_USER_ACTION,
  MESSAGE_TYPE_COMPRESSION,
  MESSAGE_TYPE_REASONING,
  MESSAGE_TYPE_TEXT,
  MESSAGE_TYPE_TOOL_CALL,
  MESSAGE_TYPE_TURN,
  MESSAGE_TYPE_USER_PROMPT,
  type ChatMessage,
  type ChatMessageType,
  type ChatRole,
  type MessageStatus,
} from '@/schemas/chat_message'
import type { MessagePrompt } from '@/schemas/message_prompt'

// ==================== 节点 → 词表取值（缺省规则） ====================

/** 节点的角色（缺省 = 助手） */
export function messageRoleOf(node: Pick<ChatMessage, 'role'>): ChatRole {
  return (node.role as ChatRole) ?? CHAT_ROLE_ASSISTANT
}

/** 节点的类型（缺省 = 正文文本） */
export function messageTypeOf(node: Pick<ChatMessage, 'type'>): ChatMessageType {
  return (node.type as ChatMessageType) ?? MESSAGE_TYPE_TEXT
}

/** 节点的状态（缺省 = 已结束） */
export function messageStatusOf(node: Pick<ChatMessage, 'status'>): MessageStatus {
  return (node.status as MessageStatus) ?? MESSAGE_STATUS_COMPLETED
}

/**
 * 消息节点的显示名（Turn 用）。
 *
 * 优先级：节点名 > `meta.agent_name` > `meta.agent_id` > 按是否子会话取兜底名。
 * `meta` 是后端 flatten 的场景字段，这里只做取值，不解释语义。
 */
export function agentNameOf(
  node: Pick<ChatMessage, 'name' | 'meta'>,
  subSession: boolean,
): string {
  const meta = node.meta as Record<string, unknown> | undefined
  const fromMeta = meta?.agent_name || meta?.agent_id
  return (
    node.name ||
    (typeof fromMeta === 'string' ? fromMeta : '') ||
    (subSession ? '子智能体' : '助手')
  )
}

// ==================== 判定入参 ====================

/**
 * 呈现判定的**全部**输入。
 *
 * 组件把 props 摊成它；本模块的每个函数都只读它。字段一旦需要新增，
 * 会先在类型上体现出来（而不是在某个组件的 computed 里悄悄多读一个 prop）。
 */
export interface MessageFacets {
  role: ChatRole
  type: ChatMessageType
  status: MessageStatus
  /** 节点名（工具名 / 智能体名） */
  name?: string
  /** 子会话 Turn（`role = tool` 的 turn）：某工具「过程」段 */
  subSession: boolean
  /** `user_prompt` 的子类（`meta.prompt.kind`） */
  promptKind?: 'question' | 'confirm'
  /** 合成「请求」子节点（`meta.__toolRequest`，前端渲染期提升，不落存储） */
  toolRequest: boolean
  /** 工具返回（`role = tool` + `type = text` + 父节点是 `tool_call`） */
  toolResult: boolean
  /** 子节点中存在待审批（工具卡默认展开的例外之一） */
  hasWaitingChild: boolean
  /** 后端标记该失败**可恢复**（会话因它暂停，用户可就地重试） */
  recoverable: boolean
  /** 后端标记的失败类别（`meta.failure_kind`，如 `error`） */
  failureKind?: string
  /** Turn 的**直接正文子节点**：内联直排，不重复渲染折叠头部 */
  responseText: boolean
}

/**
 * 从节点 + 父节点上下文推导出全部判定入参。
 *
 * **这是「props → facets」的唯一实现**：分派器（`MessageNode`）与各个体渲染器
 * 都调它，因此同一个节点在两侧得到的判定必然一致——不存在「分派器认为是工具结果、
 * 渲染器认为不是」这种分叉。
 *
 * @param node       消息节点
 * @param parentType 父节点的消息类型（`tool_call` / `turn`；根级为 undefined）
 */
export function facetsOf(node: ChatMessage, parentType?: string): MessageFacets {
  const role = messageRoleOf(node)
  const type = messageTypeOf(node)
  return {
    role,
    type,
    status: messageStatusOf(node),
    name: node.name,
    // 子会话 Turn = 工具「过程」段（role=tool 的 turn），与根级助手 Turn 同构但无组级操作
    subSession: type === MESSAGE_TYPE_TURN && role === CHAT_ROLE_TOOL,
    promptKind: promptKindOf(node),
    // 合成「请求」子节点：渲染期把 ToolCall 自身 content 提升为独立子节点时打的标
    toolRequest: messageIsToolRequest(node),
    // 工具返回必须限定父节点是 tool_call：子会话 Turn 内部的文本子节点 role 也是 tool，
    // 误判会把它渲染成独立「响应」块（见 isResponseText）
    toolResult:
      role === CHAT_ROLE_TOOL &&
      type === MESSAGE_TYPE_TEXT &&
      parentType === MESSAGE_TYPE_TOOL_CALL,
    hasWaitingChild: (node.children || []).some(
      (c) => c.status === MESSAGE_STATUS_WAITING_USER_ACTION,
    ),
    recoverable: messageIsRecoverable(node),
    failureKind: messageFailureKind(node),
    // Turn 的直接正文子节点：内联直排（无折叠头部）。仅限 text——思考必须保留单行行头，
    // 否则流式思考会以裸 Markdown 块呈现，破坏「思考中…」动效与折叠交互。
    responseText: parentType === MESSAGE_TYPE_TURN && type === MESSAGE_TYPE_TEXT,
  }
}

// ==================== 后端 `meta` 字段 → UI 语义 ====================
//
// `meta` 是后端 flatten 出来的**场景字段袋**，字段名与含义只有后端契约知道。
// 组件里直接写 `node.meta.recoverable` 等于把后端字段名抄进了视图层——后端改一个
// 字段名，要在所有渲染器里找。因此**读取集中在这一点**，组件只问「能不能重试」，
// 不问「meta 里有没有 recoverable」。
//
// 这也是机制审计脚本（`scripts/mechanism-audit.mjs`）的判定依据：
// `components/` 下不允许出现对后端 meta 字段的直接读取。

/** 合成「请求」子节点标记（渲染期提升，不落存储） */
export function messageIsToolRequest(node: Pick<ChatMessage, 'meta'>): boolean {
  return Boolean((node.meta as Record<string, unknown> | undefined)?.__toolRequest)
}

/** 系统心跳任务自动发送的消息（后端 `trigger_heartbeat` 打标） */
export function messageIsHeartbeat(node: Pick<ChatMessage, 'meta'>): boolean {
  return Boolean((node.meta as Record<string, unknown> | undefined)?.heartbeat)
}

/**
 * 临时节点（`meta.ephemeral`）：不参与「会话里是否存在失败节点」这类会话级判定。
 *
 * 这类节点是渲染期的临时广播（不落存储），把它们的失败算成"会话失败"会让
 * 错误横幅在一个已经恢复的会话上继续挂着。
 */
export function messageIsEphemeral(node: Pick<ChatMessage, 'meta'>): boolean {
  return Boolean((node.meta as Record<string, unknown> | undefined)?.ephemeral)
}

/** 后端标记该失败**可恢复**（会话因它暂停，用户可就地重试）。只保证「存在即真」。 */
export function messageIsRecoverable(node: Pick<ChatMessage, 'meta'>): boolean {
  return Boolean((node.meta as Record<string, unknown> | undefined)?.recoverable)
}

/** 后端标记的失败类别（`error` = 可补充参数重跑；其余类别不提供补充入口） */
export function messageFailureKind(node: Pick<ChatMessage, 'meta'>): string | undefined {
  const raw = (node.meta as Record<string, unknown> | undefined)?.failure_kind
  return typeof raw === 'string' ? raw : undefined
}

/**
 * 运行时长锚点（后端在工具开始执行那一刻写入的 `meta.started_at`）。
 *
 * **锚点只取后端值，前端不自造**：用挂载时刻当锚点，切会话/重连后会把
 * 「已经跑了 3 分钟」显示成「刚刚开始」，恰好丢掉用户最需要的那条信息。
 * 缺失（旧数据）返回 `null`——宁可不说，也不给一个看起来合理但没有依据的数。
 */
export function messageStartedAt(node: Pick<ChatMessage, 'meta'>): number | null {
  const raw = (node.meta as Record<string, unknown> | undefined)?.started_at
  return typeof raw === 'number' && raw > 0 ? raw : null
}

/** 子会话锚点（子会话工具调用的恢复需路由回子会话） */
export function messageParentSessionId(node: Pick<ChatMessage, 'meta'>): string | undefined {
  const raw = (node.meta as Record<string, unknown> | undefined)?.parent_session_id
  return typeof raw === 'string' ? raw : undefined
}

// ==================== 消息级业务规则（纯函数） ====================
//
// 「这个节点能做什么」原先写在渲染组件的 computed 里，与「怎么画」混在一起。
// 抽成纯函数后：可单测、可被非渲染场景复用（如命令面板），
// 且**规则变了只改一处**。

/**
 * 工具失败能否**就地重试**（resume action=`retry`）。
 *
 * 仅「会话因该失败而暂停」的可恢复失败才可就地重试（后端 `meta.recoverable`）：
 * 会话忙碌时 resume 会被拒绝；auto 模式下运行中的工具失败是**信息性**的
 * （错误结果已喂给 LLM 继续处理），给个重试按钮只会误导。
 * 思考 / 正文 / 工具请求的失败归 **Turn 级**重试（组级错误条），不在此列。
 */
export function canRetryTool(f: MessageFacets): boolean {
  return f.type === MESSAGE_TYPE_TOOL_CALL && isFailedStatus(f.status) && f.recoverable
}

/** 工具失败能否**补充参数**后重跑（resume action=`supply`） */
export function canSupplyToolArgs(f: MessageFacets): boolean {
  return canRetryTool(f) && f.failureKind === MESSAGE_FAILURE_KIND_ERROR
}

/**
 * 失败重试的分派结果（resume 的 `action` + `targetId`）。
 *
 * 只有两个粒度，**粒度由节点类型决定**：
 * - `retry`      —— 只重跑**这一个工具**（删除该工具的失败结果 → 原参数重新执行）；
 * - `retry_turn` —— 重跑**整轮**（删除响应子树 → 重新走 LLM 请求）。
 */
export interface MessageRetryTarget {
  action: 'retry' | 'retry_turn'
  targetId: string
}

/**
 * 由失败节点推出该发哪种重试（**重试分派的唯一实现**）。
 *
 * 原先这段路由写在 `ModelChatPanel.handleRetry` 里，与面板的其它职责混在一起。
 * 抽成纯函数后可直接单测「哪种节点发哪种 action」，也避免「叶子节点要回溯到父 Turn」
 * 这条后端约束被漏掉——后端 `process_retry_turn` 要求 `target_id` 指向 Failed **Turn**，
 * 因此 Text / Reasoning 叶子失败时必须回溯到父 Turn，否则恢复会被拒绝。
 */
export function messageRetryTargetOf(
  node: Pick<ChatMessage, 'id' | 'type' | 'status' | 'parent_id'>,
): MessageRetryTarget {
  const type = messageTypeOf(node)
  // 工具调用失败 → 单工具粒度（不动整轮）
  if (type === MESSAGE_TYPE_TOOL_CALL && isFailedStatus(messageStatusOf(node))) {
    return { action: 'retry', targetId: node.id }
  }
  // 其余（Turn 本身，或 Turn 下的 Text/Reasoning 叶子）→ 整轮粒度，回溯到父 Turn
  return {
    action: 'retry_turn',
    targetId: type === MESSAGE_TYPE_TURN ? node.id : node.parent_id || node.id,
  }
}

/** 后端失败类别：可补充参数（其余类别只能原样重试） */
export const MESSAGE_FAILURE_KIND_ERROR = 'error'

/** 用户中止的交代文案（中止不是故障，文案与图标都与失败区分开） */
export const MESSAGE_ABORTED_NOTE = '已中止：本轮被提前终止'

/** 兜底错误文案（无 `error` / `meta.error` / 正文可取时） */
export const MESSAGE_FAILURE_NOTE = '执行失败'

// ==================== meta.prompt 取值 ====================

/**
 * 取节点的待响应载荷（`meta.prompt`）。
 *
 * 只做形状判定：不是本协议形状（缺 `kind`）一律返回 `null`，
 * 调用方据此走「无载荷」分支，而不是拿到半个对象去渲染。
 */
export function promptOf(node: Pick<ChatMessage, 'meta'>): MessagePrompt | null {
  const meta = node.meta as Record<string, unknown> | undefined
  const p = meta?.prompt as MessagePrompt | undefined
  if (!p || typeof p !== 'object') return null
  if (p.kind !== 'question' && p.kind !== 'confirm') return null
  return p
}

/** 待响应载荷的子形态（供 `messageTitle` 区分「提问」与「工具确认」） */
export function promptKindOf(node: Pick<ChatMessage, 'meta'>): 'question' | 'confirm' | undefined {
  return promptOf(node)?.kind
}

// ==================== 状态判定（纯函数，供组件与业务规则共用） ====================

/** 是否正在产生内容 */
export function isStreamingStatus(status: MessageStatus): boolean {
  return status === MESSAGE_STATUS_STREAMING
}

/** 是否等待用户响应（审批 / 提问） */
export function isWaitingStatus(status: MessageStatus): boolean {
  return status === MESSAGE_STATUS_WAITING_USER_ACTION
}

/** 是否以错误结束 */
export function isFailedStatus(status: MessageStatus): boolean {
  return status === MESSAGE_STATUS_FAILED
}

/** 是否用户主动终止 */
export function isAbortedStatus(status: MessageStatus): boolean {
  return status === MESSAGE_STATUS_ABORTED
}

// ==================== 文案表（角色 / 类型 / 状态 → 面向用户的词） ====================
//
// 这些是**给用户看的**词，不是机制词。消费方（如资源页的消息详情）直接查表；
// 未登记的取值由调用方决定如何兜底（通常原样显示，便于发现新取值）。

/** 角色 → 文案 */
export const MESSAGE_ROLE_LABELS: Record<string, string> = {
  [CHAT_ROLE_USER]: '用户',
  [CHAT_ROLE_ASSISTANT]: '助手',
  [CHAT_ROLE_TOOL]: '工具',
  [CHAT_ROLE_SYSTEM]: '系统',
}

/**
 * 类型 → 文案。
 *
 * **必须覆盖 `MESSAGE_TYPES` 的每一个取值**——漏一个就会把机制词
 * （如 `compression`）原样摆给用户看，与「不要把后端枚举漏出去」相悖。
 * 由单测锁死覆盖率。
 */
export const MESSAGE_TYPE_LABELS: Record<string, string> = {
  [MESSAGE_TYPE_TEXT]: '文本',
  [MESSAGE_TYPE_REASONING]: '思考',
  [MESSAGE_TYPE_TURN]: '轮次',
  [MESSAGE_TYPE_TOOL_CALL]: '工具调用',
  [MESSAGE_TYPE_USER_PROMPT]: '待响应',
  [MESSAGE_TYPE_COMPRESSION]: '上下文压缩',
}

/**
 * 状态 → 文案。
 *
 * `active` 是**旧数据别名**（历史上「已结束」与「未标注」都被映射成它），
 * 保留映射仅为兼容已落库的历史节点，新节点不会再出现该取值。
 */
export const MESSAGE_STATUS_LABELS: Record<string, string> = {
  [MESSAGE_STATUS_PENDING]: '排队中',
  [MESSAGE_STATUS_STREAMING]: '生成中',
  [MESSAGE_STATUS_WAITING_USER_ACTION]: '等待响应',
  [MESSAGE_STATUS_COMPLETED]: '已完成',
  [MESSAGE_STATUS_ABORTED]: '已中止',
  [MESSAGE_STATUS_FAILED]: '失败',
  /** 旧数据别名，见上方说明 */
  active: '正常',
}

/** 角色文案（未登记 → 原样返回） */
export function messageRoleLabel(role: string): string {
  return MESSAGE_ROLE_LABELS[role] ?? role
}

/** 类型文案（未登记 → 原样返回） */
export function messageTypeLabel(type: string): string {
  return MESSAGE_TYPE_LABELS[type] ?? type
}

/** 状态文案（未登记 → 原样返回） */
export function messageStatusLabel(status: string): string {
  return MESSAGE_STATUS_LABELS[status] ?? status
}

// ==================== 头部呈现（图标 / 标题 / 标签 / 修饰类） ====================

/**
 * 头部图标。
 *
 * 判定顺序有语义（与改造前的 if 链逐条对齐）：角色优先于类型，
 * 合成请求节点优先于它自己的 `type = text`。
 */
export function messageIcon(f: MessageFacets): string {
  if (f.role === CHAT_ROLE_USER) return '👤'
  if (f.type === MESSAGE_TYPE_USER_PROMPT) return '❓'
  if (f.type === MESSAGE_TYPE_COMPRESSION) return '🗜'
  if (f.toolRequest) return '📤'
  if (f.type === MESSAGE_TYPE_REASONING) return '💭'
  if (f.type === MESSAGE_TYPE_TOOL_CALL) return '🔧'
  if (f.type === MESSAGE_TYPE_TURN) return f.subSession ? '↳' : 'AI'
  if (f.toolResult) return '↩'
  return '💬'
}

/**
 * 头部标题。
 *
 * `agentName` 由调用方传入（Turn 的标题是「该智能体的名称」，取值逻辑见 `agentNameOf`）。
 * 判定顺序有语义，与改造前的 if 链逐条对齐。
 */
export function messageTitle(f: MessageFacets, agentName: string): string {
  if (f.role === CHAT_ROLE_USER) return '你'
  if (f.type === MESSAGE_TYPE_USER_PROMPT) {
    return f.promptKind === 'confirm' ? '工具确认' : '提问'
  }
  if (f.type === MESSAGE_TYPE_COMPRESSION) return '上下文压缩'
  if (f.toolRequest) return '请求'
  // 思考：流式中「思考中…」+ 头部动效；完成后「思考」单行
  if (f.type === MESSAGE_TYPE_REASONING) {
    return isStreamingStatus(f.status) ? '思考中…' : '思考'
  }
  if (f.type === MESSAGE_TYPE_TOOL_CALL) return f.name || '工具'
  if (f.type === MESSAGE_TYPE_TURN) return agentName
  if (f.toolResult) return '响应'
  return f.role === CHAT_ROLE_ASSISTANT ? '助手' : '消息'
}

/**
 * 状态标签 —— **折叠态下唯一的运行中信号**。
 *
 * 工具调用与压缩节点默认折叠成单行，头部就是用户能看到的全部，
 * 因此标签必须覆盖**全部非终态**，留空等于「什么都没发生」。
 * 终态（`completed`）刻意不给标签：绝大多数调用都会成功结束，
 * 给每个成功的调用挂一个「已完成」只会把真正需要注意的状态淹掉。
 */
export function messageStatusTag(f: MessageFacets): string {
  if (f.type === MESSAGE_TYPE_COMPRESSION) {
    if (isFailedStatus(f.status)) return '未完成'
    if (isAbortedStatus(f.status)) return '已中止'
    if (isStreamingStatus(f.status)) return '压缩中'
    return ''
  }
  if (f.type !== MESSAGE_TYPE_TOOL_CALL) return ''
  if (isFailedStatus(f.status)) return '失败'
  if (isAbortedStatus(f.status)) return '已中止'
  if (isWaitingStatus(f.status)) return '待确认'
  if (isStreamingStatus(f.status)) return '运行中'
  return ''
}

/** 状态标签的色调类（`VdfsCard` 风格的语义色） */
export type MessageStatusTone = 'warn' | 'sub' | 'err' | 'run'

/**
 * 状态标签色调。
 *
 * 中止**不是错误**：不给红色角标，否则用户按一次停止就会看到一片失败红。
 */
export function messageStatusTone(f: MessageFacets): MessageStatusTone {
  if (isWaitingStatus(f.status)) return 'warn'
  if (isAbortedStatus(f.status)) return 'sub'
  if (isFailedStatus(f.status)) return 'err'
  if (isStreamingStatus(f.status)) return 'run'
  return 'sub'
}

/** 头部修饰类（CSS 钩子；纯映射，不含判定） */
export interface MessageHeadModifier {
  user: boolean
  sub: boolean
  tool: boolean
  reasoning: boolean
  /** 标题呼吸动效：思考中**与工具执行中**共用——两者都是「正在进行、内容尚未成形」的行 */
  thinking: boolean
}

export function messageHeadModifier(f: MessageFacets): MessageHeadModifier {
  const running = isStreamingStatus(f.status)
  return {
    user: f.role === CHAT_ROLE_USER,
    sub: f.subSession,
    tool: f.type === MESSAGE_TYPE_TOOL_CALL,
    reasoning: f.type === MESSAGE_TYPE_REASONING,
    thinking: (f.type === MESSAGE_TYPE_REASONING || f.type === MESSAGE_TYPE_TOOL_CALL) && running,
  }
}

/**
 * 是否正在执行中的**动作**节点（工具执行 / 上下文压缩）。
 *
 * 两者都默认折叠成单行，头部就是用户能看到的全部，因此「正在跑」必须由
 * 这里派生的信号承担（脉动动效 + 已用秒数）——留空等于「什么都没发生」。
 */
export function messageIsRunningAction(f: MessageFacets): boolean {
  return (
    (f.type === MESSAGE_TYPE_TOOL_CALL || f.type === MESSAGE_TYPE_COMPRESSION) &&
    isStreamingStatus(f.status)
  )
}

/**
 * 头部「回复中…」小字是否显示。
 *
 * 只有**正文**流式时才显示：思考有「思考中…」标题、工具有「运行中」标签、
 * 压缩有「压缩中」标签，各自已经给出信号；再挂一个「回复中…」是重复噪音。
 */
export function messageShowsLiveBadge(f: MessageFacets): boolean {
  return (
    isStreamingStatus(f.status) &&
    f.type !== MESSAGE_TYPE_REASONING &&
    f.type !== MESSAGE_TYPE_TOOL_CALL &&
    f.type !== MESSAGE_TYPE_COMPRESSION
  )
}

// ==================== 折叠策略 ====================

/** 深层级（depth ≥ 此值）一律收起为单行摘要 */
export const DEEP_COLLAPSE_LEVEL = 3

/** 收起态单行摘要的最大字符数（超出截断并加省略号） */
export const MESSAGE_PREVIEW_MAX = 80

/**
 * 折叠默认态（对齐行业智能体会话流）。
 *
 * - 待审批 / 用户消息 → 展开（用户需要看到并操作）
 * - 工具调用 → **始终单行**；例外：内含待审批子节点（否则审批入口被折叠隐藏）、
 *   或可恢复失败（会话因该失败暂停，需要操作入口）
 * - 思考 → **始终单行**（流式中 = 「思考中…」动效，完成后 = 单行摘要）
 * - 正文 → 助手展开；工具结果 / 子智能体内部文本收起
 * - 其余（提问 / 压缩）→ 展开
 *
 * 用户手动点击后以 `userToggled` 为准（该状态在组件里，不在本函数）。
 */
export function messageDefaultOpen(f: MessageFacets): boolean {
  if (isWaitingStatus(f.status)) return true
  if (f.role === CHAT_ROLE_USER) return true
  if (f.type === MESSAGE_TYPE_TOOL_CALL) {
    if (f.hasWaitingChild) return true
    if (f.recoverable) return true
    return false
  }
  if (f.type === MESSAGE_TYPE_REASONING) return false
  // 正文：助手展开；工具结果 / 子智能体内部文本收起
  if (f.type === MESSAGE_TYPE_TEXT) return f.role !== CHAT_ROLE_TOOL
  // 其余类型（提问 / 压缩）一律展开
  return true
}

// ==================== 有效折叠态（含深层级与用户覆盖） ====================

/**
 * 实际是否展开。
 *
 * `userToggled` / `userOpen` 由组件持有（交互态），这里只做组合判定，
 * 保证「默认收起节点的首次点击」不会出现「点了没反应」（见 `nextOpenOf`）。
 */
export function effectiveOpenOf(
  f: MessageFacets,
  depth: number,
  userToggled: boolean,
  userOpen: boolean,
): boolean {
  if (userToggled) return userOpen
  if (depth >= DEEP_COLLAPSE_LEVEL) return false
  return messageDefaultOpen(f)
}

/**
 * 点击折叠后的展开态。
 *
 * 基于**当前实际展示态**取反，而非固定初始值：否则默认收起的节点首次点击会
 * 从初始 `userOpen = true` 翻成 `false`，展示态不变（仍收起），表现为「第一次点击无反应」。
 */
export function nextOpenOf(
  f: MessageFacets,
  depth: number,
  userToggled: boolean,
  userOpen: boolean,
): boolean {
  return !effectiveOpenOf(f, depth, userToggled, userOpen)
}

// ==================== 渲染器分派（与 VDFS 详情同一套机制） ====================
//
// 分两层，和 `vdfsTypes` / `vdfsRenderers` 完全同构：
//
//   schemas/chat_message.ts   数据契约：取值词表 + 形状
//   本文件                    **纯映射**：facets → 渲染器标识；标识 → 组件（由装配点登记）
//   registry/messageRenderers.ts  **唯一 import 组件的地方**（把标识绑到具体组件）
//
// 于是「新增一种消息类型」的完整代价是：
//   1. `MESSAGE_TYPES` 加一个取值（类型与常量同时到位）
//   2. 本文件 `messageRendererKey` 加一行分派、文案表加一行
//   3. `messageRenderers.ts` 加一行注册
// **不再需要改渲染组件里的 if 链**——这正是本次改造要买到的性质。

/**
 * 渲染器标识（机制级的呈现形态，不含场景语义）。
 *
 * - `turn`         轮次响应分组（根级助手回合与子会话回合共用）
 * - `text`         折叠式内容节点：用户气泡 / 思考 / 正文 / 工具请求 / 工具返回
 * - `tool_call`    工具调用三段式卡片（请求 / 过程 / 结果）
 * - `user_prompt`  待用户响应（提问 / 工具确认）
 * - `compression`  上下文压缩（系统动作，非对话内容）
 * - `fallback`     未登记类型的只读兜底（会话流永不空白）
 */
export type MessageRenderer =
  | 'turn'
  | 'text'
  | 'tool_call'
  | 'user_prompt'
  | 'compression'
  | 'fallback'

/**
 * 判定节点该由哪个渲染器接管（**前端选择渲染形态的唯一入口**）。
 *
 * 判定顺序有语义，与改造前的模板 `v-if` 链逐条对齐：
 * 1. `turn` 最先——任何 Turn 都走分组形态（用户消息若是 Turn 亦如此）；
 * 2. 角色优先于类型——用户消息一律走气泡（`text`），即便它的 `type` 是别的；
 * 3. 其余按类型分派；
 * 4. 未登记取值 → `fallback`（新类型上线时旧前端仍能显示内容）。
 */
export function messageRendererKey(f: MessageFacets): MessageRenderer {
  if (f.type === MESSAGE_TYPE_TURN) return 'turn'
  if (f.role === CHAT_ROLE_USER) return 'text'
  if (f.type === MESSAGE_TYPE_USER_PROMPT) return 'user_prompt'
  if (f.type === MESSAGE_TYPE_COMPRESSION) return 'compression'
  if (f.type === MESSAGE_TYPE_TOOL_CALL) return 'tool_call'
  if (f.type === MESSAGE_TYPE_TEXT || f.type === MESSAGE_TYPE_REASONING) return 'text'
  return 'fallback'
}

/** 渲染器组件注册表（由 `messageRenderers.ts` 注入；未注册由视图兜底） */
const RENDERER_COMPONENTS: Record<string, Component> = {}

/** 为某个渲染器登记组件（UI 资产，与数据契约严格分离） */
export function registerMessageRenderer(renderer: MessageRenderer, component: Component): void {
  RENDERER_COMPONENTS[renderer] = component
}

/** 取已登记的渲染器组件（未登记返回 undefined，调用方负责兜底） */
export function getMessageRenderer(renderer: MessageRenderer): Component | undefined {
  return RENDERER_COMPONENTS[renderer]
}

/**
 * 解析节点该由哪个组件渲染（**前端选择渲染形态的唯一入口**）。
 *
 * 未登记的渲染器一律回落到 `fallback`——这样「后端先行上线了新 `type`、
 * 前端还是旧版」不会把会话流渲染成空白。`fallback` 本身也没登记时返回
 * `undefined`，由视图层决定如何兜底（正常构建下不会发生）。
 */
export function resolveMessageRenderer(f: MessageFacets): Component | undefined {
  return RENDERER_COMPONENTS[messageRendererKey(f)] ?? RENDERER_COMPONENTS['fallback']
}


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
import { createRendererRegistry } from './factory'
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
  RESUME_ACTION_RETRY,
  RESUME_ACTION_RETRY_COMPACTION,
  RESUME_ACTION_RETRY_TURN,
  type ChatMessage,
  type ChatMessageType,
  type ChatRole,
  type MessageStatus,
  type ResumeAction,
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
 * 压缩失败能否**就地重试**（resume action=`retry_compaction`）。
 *
 * 压缩失败**不改动历史**（后端失败路径的硬不变式：三种失败出口都在消息里明示
 * "已保留完整历史"），因此用户看到原因后唯一能做的就是再试一次——`input_over_limit`
 * 尤其如此：换一个上下文更大的模型后，同一次压缩就能成功。所以**只要失败就给入口**，
 * 不按 `failure_kind` 分档（分档会让"换模型后重试"这条唯一出路消失）。
 *
 * 与 Turn 级重试的区别：Turn 失败删的是响应子树（对话内容），压缩失败删的是
 * **压缩节点本身**（系统动作），两者不能共用 `retry_turn`。
 */
export function canRetryCompaction(f: MessageFacets): boolean {
  return f.type === MESSAGE_TYPE_COMPRESSION && isFailedStatus(f.status)
}

/**
 * 失败重试只会用到的那三个动作。
 *
 * 写成 `Extract<ResumeAction, ...>` 而不是再抄一份字面量联合：抄一份就是多一份
 * 真相，后端改词时这里会静默失效。用常量去 `Extract` 则两头都受约束——
 * 常量名拼错是**编译错误**（import 不存在），常量值漂移是 **C 组审计红**
 * （`protocol-mirror-audit`）。
 */
export type MessageRetryAction = Extract<
  ResumeAction,
  | typeof RESUME_ACTION_RETRY
  | typeof RESUME_ACTION_RETRY_TURN
  | typeof RESUME_ACTION_RETRY_COMPACTION
>

/**
 * 失败重试的分派结果（resume 的 `action` + `targetId`）。
 *
 * 三个粒度，**粒度由节点类型决定**：
 * - `retry`            —— 只重跑**这一个工具**（删除该工具的失败结果 → 原参数重新执行）；
 * - `retry_turn`       —— 重跑**整轮**（删除响应子树 → 重新走 LLM 请求）；
 * - `retry_compaction` —— 重跑**这一次压缩**（删除失败压缩节点 → 重新压缩）。
 */
export interface MessageRetryTarget {
  action: MessageRetryAction
  targetId: string
}

/**
 * 由失败节点推出该发哪种重试（**重试分派的唯一实现**）。
 *
 * 原先这段路由写在 `ModelChatPanel.handleRetry` 里，与面板的其它职责混在一起。
 * 抽成纯函数后可直接单测「哪种节点发哪种 action」，也避免「叶子节点要回溯到父 Turn」
 * 这条后端约束被漏掉——后端 `process_retry_turn` 要求 `target_id` 指向 Failed **Turn**，
 * 因此 Text / Reasoning 叶子失败时必须回溯到父 Turn，否则恢复会被拒绝。
 *
 * 压缩节点是**根级**节点（无 `parent_id`），若落到"回溯到父 Turn"的兜底分支，
 * `target_id` 会是它自己而类型是 `Compression` —— 后端会以 NotFound 拒绝。
 * 因此它必须在工具分支之前单独分派。
 */
export function messageRetryTargetOf(
  node: Pick<ChatMessage, 'id' | 'type' | 'status' | 'parent_id'>,
): MessageRetryTarget {
  const type = messageTypeOf(node)
  // 压缩失败 → 压缩粒度（删压缩节点重跑，不动历史）
  if (type === MESSAGE_TYPE_COMPRESSION && isFailedStatus(messageStatusOf(node))) {
    return { action: RESUME_ACTION_RETRY_COMPACTION, targetId: node.id }
  }
  // 工具调用失败 → 单工具粒度（不动整轮）
  if (type === MESSAGE_TYPE_TOOL_CALL && isFailedStatus(messageStatusOf(node))) {
    return { action: RESUME_ACTION_RETRY, targetId: node.id }
  }
  // 其余（Turn 本身，或 Turn 下的 Text/Reasoning 叶子）→ 整轮粒度，回溯到父 Turn
  return {
    action: RESUME_ACTION_RETRY_TURN,
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
//
// 入参放宽到 `null | undefined`（与契约层 `isInflightMessageStatus` /
// `isUnsettledMessageStatus` 同口径）：`ChatMessage.status` 本身是可选的，
// 要求调用方先做一次存在性判断，只会把「还没标注状态」当成另一个分支去写。

/** 是否正在产生内容 */
export function isStreamingStatus(status?: MessageStatus | null): boolean {
  return status === MESSAGE_STATUS_STREAMING
}

/** 是否等待用户响应（审批 / 提问） */
export function isWaitingStatus(status?: MessageStatus | null): boolean {
  return status === MESSAGE_STATUS_WAITING_USER_ACTION
}

/** 是否以错误结束 */
export function isFailedStatus(status?: MessageStatus | null): boolean {
  return status === MESSAGE_STATUS_FAILED
}

/** 是否用户主动终止 */
export function isAbortedStatus(status?: MessageStatus | null): boolean {
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

// ==================== 类型 → 呈现（**类型知识的唯一来源**） ====================
//
// 这张表回答「这个类型长什么样」，取代原先散在 7 个函数里的 if 链。
//
// ## 它修掉的是什么
//
// 改造前 `MESSAGE_TYPE_TOOL_CALL` 在 `messageIcon` / `messageTitle` /
// `messageStatusTag` / `messageHeadModifier` / `messageIsRunningAction` /
// `messageDefaultOpen` / `messageRendererKey` 里**各出现一次**——新增一个消息类型
// 要改 7 处，而且没有任何东西保证这 7 处彼此一致（漏一处就是「新类型有图标没标题」）。
// 现在每个类型只在这里出现一次。
//
// ## 表**不表达优先级**——优先级是跨类型的规则
//
// 三条跨类型的优先级规则各写在下面取值函数里（各一行，注释说明理由）：
// 1. `role = user` 覆盖类型（用户消息一律走气泡）；
// 2. `turn` 覆盖 `role = user`（Turn 永远走分组形态，见 `messageRendererKey`）；
// 3. 合成请求 / 工具返回是**上下文标记**而非类型（二者只出现在 `type = text` 上，
//    由 `facetsOf` 保证），因此在查表前先行判定。
//
// 规则 1 与 2 的顺序**确实不同**（图标是「角色优先」，渲染器是「Turn 最先」）——
// 这是既有行为，本次重构原样保留，不合并成单一顺序。

/** 呈现面取值：字面量，或按 facets 现算（如 Turn 的图标分根级 / 子会话） */
type Facet<T> = T | ((f: MessageFacets, agentName: string) => T)

/**
 * 一个消息类型的**全部呈现面**。
 *
 * 新增一个消息类型 = 填这张表的一行 + 词表加一个取值 + 渲染器注册一行；
 * 取值函数与渲染组件都不必改。
 */
interface TypePresentation {
  /** 头部图标 */
  icon: Facet<string>
  /** 头部标题；Turn 的标题是智能体名（由调用方传入） */
  title: Facet<string>
  /** 渲染器标识（详情形态） */
  renderer: MessageRenderer
  /**
   * 头部修饰类：该类型在 CSS 上的专属钩子。
   * `user` / `sub` 由 role 与 subSession 派生（跨类型），不在此列。
   */
  head?: 'tool' | 'reasoning'
  /** 运行中时驱动标题呼吸动效（「正在进行、内容尚未成形」的行） */
  thinkingWhenRunning?: boolean
  /** 动作型节点：默认折叠成单行，运行中靠脉动 + 已用秒数给出信号 */
  runningAction?: boolean
  /**
   * 该类型**自带运行信号**（专属标题或状态标签），因此不再挂「回复中…」小字——
   * 重复信号只会变成噪音。
   */
  ownRunningSignal?: boolean
  /** 状态标签（折叠态下唯一的运行中信号）；缺省 = 该类型不显示标签 */
  statusTag?: (f: MessageFacets) => string
  /** 折叠默认态（`waiting` 与 `role = user` 两个例外由 `messageDefaultOpen` 先行处理） */
  defaultOpen: (f: MessageFacets) => boolean
}

/**
 * 类型 → 呈现。
 *
 * 用 `Record<ChatMessageType, …>` 而非 `Record<string, …>`：**漏登记一个类型是
 * 编译错误，不是运行期兜底**。运行期未知取值（后端先行上线了新 `type`）走
 * `FALLBACK_PRESENTATION`。
 */
const TYPE_PRESENTATION: Record<ChatMessageType, TypePresentation> = {
  [MESSAGE_TYPE_TEXT]: {
    icon: '💬',
    title: (f) => (f.role === CHAT_ROLE_ASSISTANT ? '助手' : '消息'),
    renderer: 'text',
    // 正文：助手展开；工具结果 / 子智能体内部文本收起
    defaultOpen: (f) => f.role !== CHAT_ROLE_TOOL,
  },

  [MESSAGE_TYPE_REASONING]: {
    icon: '💭',
    // 流式中「思考中…」+ 头部动效；完成后「思考」单行
    title: (f) => (isStreamingStatus(f.status) ? '思考中…' : '思考'),
    renderer: 'text',
    head: 'reasoning',
    thinkingWhenRunning: true,
    // 有专属标题动效，不挂「回复中…」
    ownRunningSignal: true,
    // 思考始终单行
    defaultOpen: () => false,
  },

  [MESSAGE_TYPE_TOOL_CALL]: {
    icon: '🔧',
    title: (f) => f.name || '工具',
    renderer: 'tool_call',
    head: 'tool',
    thinkingWhenRunning: true,
    runningAction: true,
    ownRunningSignal: true,
    // 折叠态下唯一的运行中信号，必须覆盖**全部非终态**；终态 completed 刻意留空
    // （绝大多数调用都会成功结束，给每个成功的调用挂「已完成」会把真正需要注意的淹掉）
    statusTag: (f) => {
      if (isFailedStatus(f.status)) return '失败'
      if (isAbortedStatus(f.status)) return '已中止'
      if (isWaitingStatus(f.status)) return '待确认'
      if (isStreamingStatus(f.status)) return '运行中'
      return ''
    },
    // 工具卡默认单行；例外：内含待审批（否则审批入口被折叠隐藏）、
    // 或可恢复失败（会话因该失败暂停，需要操作入口）
    defaultOpen: (f) => f.hasWaitingChild || f.recoverable,
  },

  [MESSAGE_TYPE_TURN]: {
    // 根级助手回合用 AI，子会话（工具「过程」段）用 ↳
    icon: (f) => (f.subSession ? '↳' : 'AI'),
    title: (_f, agentName) => agentName,
    renderer: 'turn',
    defaultOpen: () => true,
  },

  [MESSAGE_TYPE_USER_PROMPT]: {
    icon: '❓',
    title: (f) => (f.promptKind === 'confirm' ? '工具确认' : '提问'),
    renderer: 'user_prompt',
    defaultOpen: () => true,
  },

  [MESSAGE_TYPE_COMPRESSION]: {
    icon: '🗜',
    title: '上下文压缩',
    renderer: 'compression',
    // 压缩节点同样默认折叠，标签是唯一信号
    runningAction: true,
    ownRunningSignal: true,
    statusTag: (f) => {
      if (isFailedStatus(f.status)) return '未完成'
      if (isAbortedStatus(f.status)) return '已中止'
      if (isStreamingStatus(f.status)) return '压缩中'
      return ''
    },
    defaultOpen: () => true,
  },
}

/**
 * 未登记类型的兜底呈现：按**正文**呈现，只是渲染器走通用兜底
 * （不假装它是已知形态）。这样「后端先行上线新 `type`、前端还是旧版」
 * 不会把会话流渲染成空白。
 */
const FALLBACK_PRESENTATION: TypePresentation = {
  ...TYPE_PRESENTATION[MESSAGE_TYPE_TEXT],
  renderer: 'fallback',
}

/** 取类型的呈现面（未知取值 → 兜底） */
function presentationOf(type: string): TypePresentation {
  return (
    (TYPE_PRESENTATION as Record<string, TypePresentation | undefined>)[type] ??
    FALLBACK_PRESENTATION
  )
}

/** 求呈现面取值（字面量直接返回；函数则按 facets 现算） */
function facetValue<T>(v: Facet<T>, f: MessageFacets, agentName = ''): T {
  return typeof v === 'function' ? (v as (f: MessageFacets, agentName: string) => T)(f, agentName) : v
}

// ==================== 头部呈现（图标 / 标题 / 标签 / 修饰类） ====================

/**
 * 头部图标。
 *
 * 优先级（跨类型，各一行）：用户消息 > 合成请求 > 工具返回 > 类型表。
 * 请求 / 返回只出现在 `type = text` 上（`facetsOf` 保证），故在查表前判定。
 */
export function messageIcon(f: MessageFacets): string {
  if (f.role === CHAT_ROLE_USER) return '👤'
  if (f.toolRequest) return '📤'
  if (f.toolResult) return '↩'
  return facetValue(presentationOf(f.type).icon, f)
}

/**
 * 头部标题。
 *
 * `agentName` 由调用方传入（Turn 的标题是「该智能体的名称」，取值逻辑见 `agentNameOf`）。
 * 优先级与 `messageIcon` 同源（用户 > 请求 > 返回 > 类型表）。
 */
export function messageTitle(f: MessageFacets, agentName: string): string {
  if (f.role === CHAT_ROLE_USER) return '你'
  if (f.toolRequest) return '请求'
  if (f.toolResult) return '响应'
  return facetValue(presentationOf(f.type).title, f, agentName)
}

/**
 * 状态标签 —— **折叠态下唯一的运行中信号**。
 *
 * 工具调用与压缩节点默认折叠成单行，头部就是用户能看到的全部，
 * 因此标签必须覆盖**全部非终态**，留空等于「什么都没发生」。
 * 终态（`completed`）刻意不给标签：绝大多数调用都会成功结束，
 * 给每个成功的调用挂一个「已完成」只会把真正需要注意的状态淹掉。
 *
 * 具体文案由各类型在 `TYPE_PRESENTATION.statusTag` 里自持（未声明的类型无标签）。
 */
export function messageStatusTag(f: MessageFacets): string {
  return presentationOf(f.type).statusTag?.(f) ?? ''
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
  const p = presentationOf(f.type)
  return {
    user: f.role === CHAT_ROLE_USER,
    sub: f.subSession,
    tool: p.head === 'tool',
    reasoning: p.head === 'reasoning',
    // 标题呼吸动效：思考中**与工具执行中**共用——两者都是「正在进行、内容尚未成形」的行
    thinking: Boolean(p.thinkingWhenRunning) && isStreamingStatus(f.status),
  }
}

/**
 * 是否正在执行中的**动作**节点（工具执行 / 上下文压缩）。
 *
 * 两者都默认折叠成单行，头部就是用户能看到的全部，因此「正在跑」必须由
 * 这里派生的信号承担（脉动动效 + 已用秒数）——留空等于「什么都没发生」。
 * 哪些类型算「动作」由 `TYPE_PRESENTATION.runningAction` 声明。
 */
export function messageIsRunningAction(f: MessageFacets): boolean {
  return Boolean(presentationOf(f.type).runningAction) && isStreamingStatus(f.status)
}

/**
 * 头部「回复中…」小字是否显示。
 *
 * 只有**正文**流式时才显示：思考有「思考中…」标题、工具有「运行中」标签、
 * 压缩有「压缩中」标签，各自已经给出信号（`ownRunningSignal`）；
 * 再挂一个「回复中…」是重复噪音。
 */
export function messageShowsLiveBadge(f: MessageFacets): boolean {
  return isStreamingStatus(f.status) && !presentationOf(f.type).ownRunningSignal
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
 * 前两条是**跨类型**规则（任何类型的待审批 / 用户消息都展开），写在下面；
 * 其余逐类型的策略在 `TYPE_PRESENTATION.defaultOpen` 里自持。
 *
 * 用户手动点击后以 `userToggled` 为准（该状态在组件里，不在本函数）。
 */
export function messageDefaultOpen(f: MessageFacets): boolean {
  if (isWaitingStatus(f.status)) return true
  if (f.role === CHAT_ROLE_USER) return true
  return presentationOf(f.type).defaultOpen(f)
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
//   2. 本文件 `TYPE_PRESENTATION` 加一行（呈现面全在那一行里）、文案表加一行
//   3. `messageRenderers.ts` 加一行注册
// **不再需要改渲染组件里的 if 链，也不必去 7 个取值函数里各补一处**——
// 这正是本次改造要买到的性质。漏掉第 2 步会**编译报错**（`Record<ChatMessageType, …>`
// 要求穷尽），而不是运行期静默退化成兜底。

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
 * 3. 其余按类型分派（`TYPE_PRESENTATION.renderer`）；
 * 4. 未登记取值 → `fallback`（新类型上线时旧前端仍能显示内容）。
 *
 * 第 1 条**必须先于**第 2 条，因此 `turn` 在这里显式判一次，不能只靠类型表——
 * 图标 / 标题那边的顺序恰好相反（角色优先），两处顺序不同是既有行为。
 */
export function messageRendererKey(f: MessageFacets): MessageRenderer {
  if (f.type === MESSAGE_TYPE_TURN) return 'turn'
  if (f.role === CHAT_ROLE_USER) return 'text'
  return presentationOf(f.type).renderer
}

/**
 * 消息域的渲染器注册表（机制来自 `registry/factory`）。
 *
 * 本文件只声明**本域的两个约定**：标识的联合类型（`MessageRenderer`）与兜底键
 * （`fallback`）；「标识 → 组件 + 兜底」那套机制不在本文件里再实现一遍。
 */
const renderers = createRendererRegistry<MessageRenderer>('fallback')

/** 为某个渲染器登记组件（UI 资产，与数据契约严格分离） */
export const registerMessageRenderer = renderers.register

/** 取已登记的渲染器组件（未登记返回 undefined，调用方负责兜底） */
export const getMessageRenderer = renderers.get

/**
 * 解析节点该由哪个组件渲染（**前端选择渲染形态的唯一入口**）。
 *
 * 未登记的渲染器一律回落到 `fallback`——这样「后端先行上线了新 `type`、
 * 前端还是旧版」不会把会话流渲染成空白。`fallback` 本身也没登记时返回
 * `undefined`，由视图层决定如何兜底（正常构建下不会发生）。
 */
export function resolveMessageRenderer(f: MessageFacets): Component | undefined {
  return renderers.resolve(messageRendererKey(f))
}


/**
 * 消息呈现注册表 —— 「消息类型 / 角色 / 状态 → 呈现」的**唯一来源**
 *
 * ## 分层（与 `registry/vdfsTypes.ts` 同一套）
 *
 * - `schemas/chat_node.ts`  数据契约：取值词表（常量 + 派生类型）+ 形状，零呈现知识
 * - 本文件                     **纯 UI 映射**：图标 / 标题 / 状态标签 / 折叠策略，零组件导入
 * - `components/MessageNode.vue` 等  消费方：只做「把节点摊成 facets → 查表 → 渲染」
 *
 * 这样做的直接收益：新增一种消息类型只需在 `NODE_KINDS` 加一个取值、
 * 在本文件的表里加一行；**不需要再去改渲染组件里的 if 链**。
 *
 * ## 为什么入参是 `MessageFacets` 而不是 `ChatNode`
 *
 * 判定要用到的不只是节点自身字段，还有「父节点是什么」「子节点里有没有待审批」
 * 这类**上下文**（如 `toolResult` 要求 `parentType === 'skeleton'`）。把这些上下文
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
  messageRoleOf,
  NODE_STATUS_ABORTED,
  NODE_STATUS_COMPLETED,
  NODE_STATUS_FAILED,
  NODE_STATUS_PENDING,
  NODE_STATUS_REMOVED,
  NODE_STATUS_STREAMING,
  NODE_STATUS_WAITING_USER_ACTION,
  NODE_KIND_BODY,
  NODE_KIND_NOTICE,
  NODE_KIND_SKELETON,
  NODE_KIND_INTERACTION,
  NODE_KIND_TYPING,
  nodeKindOf,
  RESUME_ACTION_RETRY,
  RESUME_ACTION_RETRY_COMPACTION,
  RESUME_ACTION_RETRY_SUBTREE,
  RESUME_ACTION_SUPPLY,
  normalizePrincipal,
  principalId,
  PRINCIPAL_KIND_AGENT,
  PRINCIPAL_KIND_PERSON,
  PRINCIPAL_KIND_SYSTEM,
  PRINCIPAL_KIND_TOOL,
  type ChatNode,
  type NodeKind,
  type Principal,
  type PrincipalKind,
  type ChatRole,
  type NodeStatus,
  type ResumeAction,
} from '@/schemas/chat_node'
import type { MessagePrompt, MessagePromptKind } from '@/schemas/message_prompt'

// ==================== 节点 → 词表取值（缺省规则） ====================

/**
 * 节点的角色——**唯一读入口**（[T-147d-iii]，镜像后端 `ChatNode::node_role()`）。
 *
 * 折叠实现住在契约层（`schemas/chat_node.messageRoleOf`，纯逻辑层同源共用）；
 * 这里只转出，registry 的消费面保持稳定。
 */
export { messageRoleOf }

/**
 * 节点的**生效形态**（[T-155 B2a]，[ADR-070 §决策 3]）：wire 五值 `NodeKind`。
 *
 * 这是分派轴的**唯一读入口**。它只做一件事——把 `node.type` 归一到闭集五值（历史
 * 业务旧词经 `nodeKindOf` 一次性折叠，`run`/`tool`→`skeleton`、`reasoning`/`compression`→
 * `notice`），**不读 `meta.style_tag`**：同格业务差异靠后端下发的 `title`/`icon`/`actions`
 * 数据分呈现，不靠前端 `switch`。折叠即合流——思考与压缩进同一格、工具与整轮进同一格，
 * 前端从此不认 `Reasoning` / `ToolCall` / `Compression` 这些业务名。
 */
export function messageTypeOf(node: Pick<ChatNode, 'type'>): NodeKind {
  return nodeKindOf(node.type)
}

/** 节点的状态（缺省 = 已结束） */
export function messageStatusOf(node: Pick<ChatNode, 'status'>): NodeStatus {
  return (node.status as NodeStatus) ?? NODE_STATUS_COMPLETED
}

/**
 * 工具名（[T-147d-ii]：顶层 `name` 已退役，工具名唯一住 `meta.tool_name`）。
 *
 * 与后端 `ChatNode::tool_name()` 同一条读法；历史数据的 attrs `tool_name`
 * 已在 `sessionTranscriptSync.messageOfNode` 折进 meta，这里不必再兜旧形态。
 */
export function toolNameOf(node: Pick<ChatNode, 'meta'>): string | undefined {
  return (node.meta as { tool_name?: string } | undefined)?.tool_name
}

/**
 * 消息节点的显示名（骨架容器用）。
 *
 * 优先级：`meta.agent_name` > `meta.agent_id` > 按是否子会话取兜底名。
 * `meta` 是后端 flatten 的场景字段，这里只做取值，不解释语义。
 * （曾经的 `node.name` 首选项已随顶层字段退役删除——后端从未把智能体名写进
 * name，那条只是一条空通道，[T-147d-ii]。）
 */
export function agentNameOf(
  node: Pick<ChatNode, 'meta'>,
  subSession: boolean,
): string {
  const meta = node.meta as Record<string, unknown> | undefined
  const fromMeta = meta?.agent_name || meta?.agent_id
  return (
    (typeof fromMeta === 'string' ? fromMeta : '') ||
    (subSession ? '子智能体' : '助手')
  )
}

// ==================== principal 前缀 → 身份呈现（[T-148] 独立表） ====================
//
// 「这条发言是谁说的」的可视化——与「这条是什么形态」（`TYPE_PRESENTATION`）正交：
// 前者按**可路由地址的命名空间**（`<ns>/<name>`，[ADR-069 §决策 1]）分派，后者按
// 形态分派。命名空间比 `kind` 更细（`vdfs/write` 与 `web/fetch` 同为 tool，但图标
// / 标签不同），所以主键是命名空间、`kind` 只是兜底——「kind 不参与主键」。
//
// 形状：`icon` / `tone`（色调 CSS 钩子，具体配色由视图层样式表映射）/ `label`
// （面向用户的族名）。**未登记命名空间退回 `kind` 表，两者都缺 → `undefined`**——
// 新主体上线不判红（与 `foldNodeKind` 的默认臂同一纪律）。

/** 主体色调语义（视图层据此挂 CSS 类；不含具体色值） */
export type PrincipalTone = 'person' | 'agent' | 'tool' | 'system' | 'neutral'

/** 一个主体的身份呈现面。 */
export interface PrincipalPresentation {
  icon: string
  tone: PrincipalTone
  label: string
}

/**
 * 命名空间 → 身份呈现。键与后端 `AGENT_NAMESPACES` / `Principal::tool_principal`
 * 合成的地址段、[ADR-069] 登记的主体行对齐（`vdfs` / `web` / `local` 是插件归属，
 * `mcp` / `agent` / `legacy` 是后端合成的工具地址段，`person` / `system` 是概念命名空间，
 * `session` 是本机 agent）。
 */
const PRINCIPAL_NS_PRESENTATION: Readonly<Record<string, PrincipalPresentation>> = {
  person: { icon: '👤', tone: 'person', label: '用户' },
  session: { icon: '🤖', tone: 'agent', label: '智能体' },
  system: { icon: '⚙️', tone: 'system', label: '系统' },
  vdfs: { icon: '🗄️', tone: 'tool', label: '文件工具' },
  web: { icon: '🌐', tone: 'tool', label: '联网工具' },
  local: { icon: '🖥️', tone: 'tool', label: '本地工具' },
  mcp: { icon: '🔌', tone: 'tool', label: 'MCP 工具' },
  agent: { icon: '🧩', tone: 'tool', label: '外部智能体' },
  legacy: { icon: '🔧', tone: 'tool', label: '工具' },
}

/** 族兜底表：命名空间未登记时按 `principal.kind` 取（[ADR-069] 四族）。 */
const PRINCIPAL_KIND_PRESENTATION: Readonly<Record<PrincipalKind, PrincipalPresentation>> = {
  [PRINCIPAL_KIND_PERSON]: { icon: '👤', tone: 'person', label: '用户' },
  [PRINCIPAL_KIND_AGENT]: { icon: '🤖', tone: 'agent', label: '智能体' },
  [PRINCIPAL_KIND_TOOL]: { icon: '🔧', tone: 'tool', label: '工具' },
  [PRINCIPAL_KIND_SYSTEM]: { icon: '⚙️', tone: 'system', label: '系统' },
}

/** 取主体地址的命名空间段（`<ns>/<name>` 的 `ns`）；无斜杠 / 无主体 → `undefined`。 */
export function principalNamespace(
  value: Principal | string | undefined | null,
): string | undefined {
  const id = principalId(value)
  if (!id) return undefined
  const slash = id.indexOf('/')
  return slash > 0 ? id.slice(0, slash) : undefined
}

/**
 * principal → 身份呈现（**唯一读入口**）：命名空间精确命中优先，退回 `kind`，
 * 都缺（无主体）→ `undefined`。调用方据此决定「不显示身份徽标」，而不是替它编一个。
 */
export function principalPresentationOf(
  value: Principal | string | undefined | null,
): PrincipalPresentation | undefined {
  const normalized = normalizePrincipal(value)
  if (!normalized) return undefined
  const ns = principalNamespace(normalized)
  if (ns) {
    const hit = PRINCIPAL_NS_PRESENTATION[ns]
    if (hit) return hit
  }
  return PRINCIPAL_KIND_PRESENTATION[normalized.kind]
}

/** 节点的 principal（结构体形态，历史裸字符串一并归一）。 */
export function nodePrincipal(node: Pick<ChatNode, 'principal'>): Principal | undefined {
  return normalizePrincipal(node.principal)
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
  type: NodeKind
  status: NodeStatus
  /** 工具名（`meta.tool_name`，[T-147d-ii]） */
  name?: string
  /**
   * 后端填的头部标题（[T-155] 呈现数据）——有值即优先于 legacy 类型表。
   * 前端机制化的落点：呈现意图来自节点数据，不靠 `switch(style_tag)` 现推。
   */
  title?: string
  /** 后端填的头部图标（同上，缺省退 legacy 表）。 */
  icon?: string
  /**
   * 后端声明的**可用恢复动作**（[T-155] 呈现数据）——重试 / 补充参数 / 审批入口
   * 的首选判据；缺省（后端未填）退回按业务类型反推的 legacy 判据。
   */
  actions?: ResumeAction[]
  /** principal 身份呈现（[T-148] 独立表：命名空间优先、kind 兜底，无主体 → `undefined`） */
  principal?: PrincipalPresentation
  /** 子会话骨架（`role = tool` 的骨架）：某工具「过程」段 */
  subSession: boolean
  /** `interaction` 的子类（`meta.prompt.kind`） */
  promptKind?: MessagePromptKind
  /** 合成「请求」子节点（`meta.__toolRequest`，前端渲染期提升，不落存储） */
  toolRequest: boolean
  /** 工具返回（`role = tool` + `type = body` + 父节点是骨架 `skeleton`） */
  toolResult: boolean
  /** 子节点中存在待审批（工具卡默认展开的例外之一） */
  hasWaitingChild: boolean
  /** 后端标记该失败**可恢复**（会话因它暂停，用户可就地重试） */
  recoverable: boolean
  /** 后端标记的失败类别（`meta.failure_kind`，如 `error`） */
  failureKind?: string
  /**
   * **压缩后的历史记忆**（`meta.compacted`）。
   *
   * 后端把它写成一条 `role = user` 的消息（多数 provider 要求对话以 user 开头），
   * 但它是系统产出的记忆，不是用户说的话：不认这个标记，它就会渲染成一个
   * 右对齐的用户气泡，正文是 `[CONTEXT SNAPSHOT …]` 这类内部标记。
   */
  compacted: boolean
  /** 骨架的**直接正文子节点**：内联直排，不重复渲染折叠头部 */
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
 * @param parentType 父节点的生效形态（`skeleton`；根级为 undefined）
 */
export function facetsOf(node: ChatNode, parentType?: string): MessageFacets {
  const role = messageRoleOf(node)
  const type = messageTypeOf(node)
  return {
    role,
    type,
    status: messageStatusOf(node),
    name: toolNameOf(node),
    // 呈现数据从节点顶层直读（[T-155]）——不经 meta，是后端填的权威呈现意图。
    title: node.title,
    icon: node.icon,
    actions: node.actions,
    // principal 身份呈现（[T-148] 独立表；组件据此画徽标，缺主体则不画）
    principal: principalPresentationOf(node.principal),
    // 子会话过程段 = `role=tool` 的骨架：与根级骨架同构（都走容器递归），但无组级操作
    subSession: type === NODE_KIND_SKELETON && role === CHAT_ROLE_TOOL,
    promptKind: promptKindOf(node),
    // 合成「请求」子节点：渲染期把骨架自身 content 提升为独立子节点时打的标
    toolRequest: messageIsToolRequest(node),
    // 工具返回必须限定父节点是骨架：子会话过程段内部的文本子节点 role 也是 tool，
    // 只有「tool 角色 + body 形态 + 父是骨架」才是一次工具调用的返回体
    toolResult:
      role === CHAT_ROLE_TOOL &&
      type === NODE_KIND_BODY &&
      parentType === NODE_KIND_SKELETON,
    hasWaitingChild: (node.children || []).some(
      (c) => c.status === NODE_STATUS_WAITING_USER_ACTION,
    ),
    recoverable: messageIsRecoverable(node),
    failureKind: messageFailureKind(node),
    compacted: messageIsCompacted(node),
    // 骨架的**助手正文**子节点：内联直排（无折叠头部）。排除 tool 角色——那是工具返回
    // （有「↩ 响应」头部），不是本轮正文；两者形态都是 body 挂在骨架下，靠 role 分辨。
    responseText:
      parentType === NODE_KIND_SKELETON && type === NODE_KIND_BODY && role !== CHAT_ROLE_TOOL,
  }
}

// ==================== 组件侧判据谓词（[T-148]） ====================
//
// 「这条是不是用户消息 / 容器骨架」这类判据原先在各渲染组件里就地比较协议
// 枚举字面量（`facets.role === 'user'`、`c.type === 'skeleton'`）——每个组件都
// 得 `import` 一份枚举值，「什么算 user/骨架」的口径散在视图层。收进注册表后：
// **组件只问谓词，不再持有枚举字面量**，判据改了只动这里（与 exit 判据 3、
// 「加呈现变体不改组件」同一纪律）。facets 版读判定入参，node 版读原始节点
// （经 `messageRoleOf` / `messageTypeOf` 同一归一，供父扫子节点用）。

/** 该 facet 是用户发言（`role = user`）。 */
export const isUserFacet = (f: Pick<MessageFacets, 'role'>): boolean =>
  f.role === CHAT_ROLE_USER

/** 该 facet 是容器骨架（`type = skeleton`，一整轮或一次工具调用同构）。 */
export const isSkeletonFacet = (f: Pick<MessageFacets, 'type'>): boolean =>
  f.type === NODE_KIND_SKELETON

/** 原始节点是否用户发言（role 格先、principal.kind 兜底同一折叠）。 */
export const isUserNode = (n: Pick<ChatNode, 'role' | 'principal'>): boolean =>
  messageRoleOf(n) === CHAT_ROLE_USER

/** 原始节点折叠后是否容器骨架（父扫子节点、区分「过程段」用）。 */
export const isSkeletonNode = (n: Pick<ChatNode, 'type'>): boolean =>
  messageTypeOf(n) === NODE_KIND_SKELETON

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
export function messageIsToolRequest(node: Pick<ChatNode, 'meta'>): boolean {
  return Boolean((node.meta as Record<string, unknown> | undefined)?.__toolRequest)
}

/** 压缩后的历史记忆（后端标记 `meta.compacted`） */
export function messageIsCompacted(node: Pick<ChatNode, 'meta'>): boolean {
  return (node.meta as Record<string, unknown> | undefined)?.compacted === true
}

/**
 * 一次压缩的**结构化交代**（压缩节点 `meta`；缺字段一律 `undefined`）。
 *
 * 前端按字段渲染、**缺字段退回正文那一行**：这样同一份前端既能显示旧后端
 * （没有这些字段）的节点，也能在字段到位后自动多出「水位 / 来源 / 丢了几条」。
 *
 * 口径（与后端 `context/pipeline.rs::compression_stats` 一致）：
 * `beforeTokens` / `afterTokens` 是**内容水位**，不含请求级固定开销——
 * 与 `contextLimit` 相除得到的比例是一个**下限**。
 */
export interface MessageCompactionStats {
  /** 触发来源：`threshold`（越线自动）/ `tool`（模型主动）/ `retry`（用户重试） */
  trigger?: string
  /** 生效的上下文上限（token） */
  limit?: number
  /** 压缩前内容水位（token） */
  beforeTokens?: number
  /** 压缩后内容水位（token）——只在压缩成功时存在 */
  afterTokens?: number
  /** 这次被压掉的条数 */
  dropped?: number
}

/** 一次性读出压缩节点的统计字段（读取后端 `meta` 字段名的**唯一处**） */
export function messageCompactionStats(
  node: Pick<ChatNode, 'meta'>,
): MessageCompactionStats {
  const meta = (node.meta ?? {}) as Record<string, unknown>
  const num = (k: string): number | undefined => {
    const v = meta[k]
    return typeof v === 'number' && Number.isFinite(v) ? v : undefined
  }
  const trigger = meta.compact_trigger
  return {
    trigger: typeof trigger === 'string' && trigger ? trigger : undefined,
    limit: num('context_limit'),
    beforeTokens: num('before_tokens'),
    afterTokens: num('after_tokens'),
    dropped: num('dropped'),
  }
}

/**
 * 压缩后那条**历史记忆**自述的事实（缺字段一律 `undefined`）。
 *
 * 与 [`messageCompactionStats`] 分开：那是「压缩这一动作」的节点，这是「压缩的产物」
 * （记忆本身）的节点，后端给的字段名不同（`post_tokens` / `transcript_path`）。
 * 两者都不在组件里直读 `meta`——字段名只在本模块出现。
 */
export interface MessageMemoryStats {
  /** 压缩后的内容水位（token） */
  postTokens?: number
  /** 压缩前的完整历史是否已转存（转存路径存在即为真） */
  archived: boolean
}

export function messageMemoryStats(node: Pick<ChatNode, 'meta'>): MessageMemoryStats {
  const meta = (node.meta ?? {}) as Record<string, unknown>
  const post = meta.post_tokens
  const path = meta.transcript_path
  return {
    postTokens: typeof post === 'number' && Number.isFinite(post) ? post : undefined,
    archived: typeof path === 'string' && path.length > 0,
  }
}

/** 触发来源 → 面向用户的词（新取值原样显示，便于发现协议新增） */
export const COMPACTION_TRIGGER_LABELS: Record<string, string> = {
  threshold: '自动整理',
  tool: '模型主动',
  retry: '手动重试',
}

export function compactionTriggerLabel(trigger: string): string {
  return COMPACTION_TRIGGER_LABELS[trigger] ?? trigger
}

/**
 * token 数的紧凑写法（`128000` → `128k`、`950` → `950`）。
 *
 * 只用于**展示**：阈值比较与迟滞判断始终用后端给的原始数字，前端不拿这个值
 * 做任何判定（四舍五入过的数不能参与计算）。
 */
export function formatTokens(n: number): string {
  if (n < 1000) return String(n)
  const k = n / 1000
  return `${k >= 100 ? Math.round(k) : Math.round(k * 10) / 10}k`
}

/** 系统心跳任务自动发送的消息（后端 `trigger_heartbeat` 打标） */
export function messageIsHeartbeat(node: Pick<ChatNode, 'meta'>): boolean {
  return Boolean((node.meta as Record<string, unknown> | undefined)?.heartbeat)
}

/**
 * 临时节点（`meta.ephemeral`）：不参与「会话里是否存在失败节点」这类会话级判定。
 *
 * 这类节点是渲染期的临时广播（不落存储），把它们的失败算成"会话失败"会让
 * 错误横幅在一个已经恢复的会话上继续挂着。
 */
export function messageIsEphemeral(node: Pick<ChatNode, 'meta'>): boolean {
  return Boolean((node.meta as Record<string, unknown> | undefined)?.ephemeral)
}

/** 后端标记该失败**可恢复**（会话因它暂停，用户可就地重试）。只保证「存在即真」。 */
export function messageIsRecoverable(node: Pick<ChatNode, 'meta'>): boolean {
  return Boolean((node.meta as Record<string, unknown> | undefined)?.recoverable)
}

/** 后端标记的失败类别（`error` = 可补充参数重跑；其余类别不提供补充入口） */
export function messageFailureKind(node: Pick<ChatNode, 'meta'>): string | undefined {
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
export function messageStartedAt(node: Pick<ChatNode, 'meta'>): number | null {
  const raw = (node.meta as Record<string, unknown> | undefined)?.started_at
  return typeof raw === 'number' && raw > 0 ? raw : null
}

/** 子会话锚点（子会话工具调用的恢复需路由回子会话） */
export function messageParentSessionId(node: Pick<ChatNode, 'meta'>): string | undefined {
  const raw = (node.meta as Record<string, unknown> | undefined)?.parent_session_id
  return typeof raw === 'string' ? raw : undefined
}

// ==================== 消息级业务规则（纯函数） ====================
//
// 「这个节点能做什么」原先写在渲染组件的 computed 里，与「怎么画」混在一起。
// 抽成纯函数后：可单测、可被非渲染场景复用（如命令面板），
// 且**规则变了只改一处**。

/**
 * 节点是否携带某个可用恢复动作（[T-155 B2a] 纯数据判据的唯一读法）。
 *
 * `actions` 由后端在交互面派生（[ADR-070 §决策 3]），前端不再按业务类型 / `recoverable`
 * 反推「能不能重试」——「有对应动词即有入口，没有即不画」。`actions` 缺席（历史数据、
 * 前端合成节点）一律 `false`：合成节点本就无可重试语义。
 */
function hasAction(actions: ResumeAction[] | undefined, action: ResumeAction): boolean {
  return actions?.includes(action) ?? false
}

/**
 * 工具失败能否**就地重试**（resume action=`retry`）。
 *
 * [T-155 B2a] 判据已是**纯数据**：后端在交互面（[ADR-070 §决策 3]）把「这条节点能发
 * 哪种恢复动词」折成 `actions` 列表随节点下发，前端只读「有 `retry` 就画重试」。
 * 不再从业务类型（工具 / 可恢复标记）反推——那是本模块最后一处业务耦合。
 */
export function canRetryTool(f: MessageFacets): boolean {
  return isFailedStatus(f.status) && hasAction(f.actions, RESUME_ACTION_RETRY)
}

/** 工具失败能否**补充参数**后重跑（resume action=`supply`） */
export function canSupplyToolArgs(f: MessageFacets): boolean {
  // `actions` 里出现 `supply` 已由后端保证「失败 + failure_kind=error」——前端不再复查档位。
  return isFailedStatus(f.status) && hasAction(f.actions, RESUME_ACTION_SUPPLY)
}

/**
 * 压缩失败能否**就地重试**（resume action=`retry_compaction`）。
 *
 * 压缩失败**不改动历史**（后端失败路径的硬不变式：三种失败出口都在消息里明示
 * "已保留完整历史"），因此用户看到原因后唯一能做的就是再试一次——`input_over_limit`
 * 尤其如此：换一个上下文更大的模型后，同一次压缩就能成功。所以**只要失败就给入口**，
 * 不按 `failure_kind` 分档（分档会让"换模型后重试"这条唯一出路消失）。
 *
 * 与骨架级重试的区别：骨架失败删的是响应子树（对话内容），压缩失败删的是
 * **压缩节点本身**（系统动作），两者不能共用 `retry_subtree`。
 */
export function canRetryCompaction(f: MessageFacets): boolean {
  return isFailedStatus(f.status) && hasAction(f.actions, RESUME_ACTION_RETRY_COMPACTION)
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
  | typeof RESUME_ACTION_RETRY_SUBTREE
  | typeof RESUME_ACTION_RETRY_COMPACTION
>

/**
 * 失败重试的分派结果（resume 的 `action` + `targetId`）。
 *
 * 三个粒度，**粒度由后端下发的 `actions` 决定**（[T-155 B2a]，[ADR-070 §决策 3]）：
 * - `retry`            —— 只重跑**这一个工具**（删除该工具的失败结果 → 原参数重新执行）；
 * - `retry_subtree`       —— 重跑**整轮**（删除响应子树 → 重新走 LLM 请求）；
 * - `retry_compaction` —— 重跑**这一次压缩**（删除失败压缩节点 → 重新压缩）。
 */
export interface MessageRetryTarget {
  action: MessageRetryAction
  targetId: string
}

/**
 * 由失败节点推出该发哪种重试（**重试分派的唯一实现**）。
 *
 * [T-155 B2a]：动作种类**优先读后端下发的 `actions`**——工具节点带 `retry`、压缩节点带
 * `retry_compaction`、运行骨架带 `retry_subtree`，粒度与靶子都由数据给出，前端不再 `switch(type)`。
 * `actions` 缺席（历史数据、合成节点、或尚未走到交互面的本地视图）时退回一条**不认业务类型**
 * 的兜底：失败 / 中止一律整轮重试，叶子回溯到父骨架（后端 `process_retry_subtree` 要求
 * `target_id` 指向运行骨架，叶子自身作靶会被拒绝）——这条兜底只吃 `status` + `parent_id`。
 */
export function messageRetryTargetOf(
  node: Pick<ChatNode, 'id' | 'status' | 'parent_id' | 'actions'>,
): MessageRetryTarget {
  const actions = node.actions
  if (hasAction(actions, RESUME_ACTION_RETRY)) {
    return { action: RESUME_ACTION_RETRY, targetId: node.id }
  }
  if (hasAction(actions, RESUME_ACTION_RETRY_COMPACTION)) {
    return { action: RESUME_ACTION_RETRY_COMPACTION, targetId: node.id }
  }
  if (hasAction(actions, RESUME_ACTION_RETRY_SUBTREE)) {
    return { action: RESUME_ACTION_RETRY_SUBTREE, targetId: node.id }
  }
  // 兜底（无 `actions`）：整轮粒度，叶子回溯到父骨架，无父退回自身。
  return { action: RESUME_ACTION_RETRY_SUBTREE, targetId: node.parent_id || node.id }
}

/** 后端失败类别：可补充参数（其余类别只能原样重试） */
export const MESSAGE_FAILURE_KIND_ERROR = 'error'
/** 后端失败类别：本批剩余调用被跳过，实际未执行（见后端 `not_executed_result`） */
export const MESSAGE_FAILURE_KIND_NOT_EXECUTED = 'not_executed'
/** 后端失败类别：中止收口（用户终止，工具未跑完） */
export const MESSAGE_FAILURE_KIND_ABORTED = 'aborted'

/**
 * 「这一次调用没有结果子节点」时的兜底文案（无则返回 `null`）。
 *
 * ## 为什么需要它
 *
 * 「工具的结果」在树里是一条**子节点**，`SkeletonNode` 的响应段就按子节点渲染。
 * 后端原先有一条分支只给父节点补终态、不写结果子节点（本批剩余被跳过的调用），
 * 于是卡片有请求、无响应，而会话照常往下走——用户看到一次调用凭空消失。
 * 机制侧已修（后端 `not_executed_result`：有调用必有结果），本函数是**显示层的
 * 兜底**，服务于：
 *
 * - **历史数据**：机制修复前落下的 `not_executed` 节点，存储里真的没有结果子节点；
 * - 任何未来不守该不变量的写入方（第三方 / 未来重构）——它不报错、不被测试拦住，
 *   只是静默地少一行字。
 *
 * 只认**父节点自述的终态**（`meta.failure_kind`），不猜：
 * 无标记就不给文案，宁可留白也不凭空替工具编一份结果。
 * 口径与 `flatten_chat_messages` 的占位同源（那边是给 **LLM** 的兜底，
 * 两边同一个事实：这次调用没跑）。
 */
export function missingResultNoteOf(node: Pick<ChatNode, 'meta'>): string | null {
  switch (messageFailureKind(node)) {
    case MESSAGE_FAILURE_KIND_NOT_EXECUTED:
      return '本次调用未执行：同一批中排在它之前的工具未正常结束，同批剩余调用被一并跳过。'
    case MESSAGE_FAILURE_KIND_ABORTED:
      return '本次调用已中止，未执行。'
    default:
      return null
  }
}

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
export function promptOf(node: Pick<ChatNode, 'meta'>): MessagePrompt | null {
  const meta = node.meta as Record<string, unknown> | undefined
  const p = meta?.prompt as MessagePrompt | undefined
  if (!p || typeof p !== 'object') return null
  if (p.kind !== 'question' && p.kind !== 'confirm') return null
  return p
}

/** 待响应载荷的子形态（供 `messageTitle` 区分「提问」与「工具确认」） */
export function promptKindOf(node: Pick<ChatNode, 'meta'>): MessagePromptKind | undefined {
  return promptOf(node)?.kind
}

// ==================== 状态判定（纯函数，供组件与业务规则共用） ====================
//
// 入参放宽到 `null | undefined`（与契约层 `isInflightNodeStatus` /
// `isUnsettledNodeStatus` 同口径）：`ChatNode.status` 本身是可选的，
// 要求调用方先做一次存在性判断，只会把「还没标注状态」当成另一个分支去写。

/** 是否正在产生内容 */
export function isStreamingStatus(status?: NodeStatus | null): boolean {
  return status === NODE_STATUS_STREAMING
}

// ==================== Typing 占位合成（[T-151]，纯函数） ====================
//
// 后端**从不发** typing 帧（见 T-151 exit）；「生成中」的即时反馈由前端按一条规则
// 合成：一个**根级骨架容器**（`type = skeleton` 且无父节点）正在 streaming、且**还没有
// 任何真实子节点**时，给它挂一个 typing 占位当第一个孩子。三条状态都由这一个谓词推出：
//
// - **合成**：根级骨架 streaming 且无真实子 ⇒ 产一条 typing；
// - **替换**：它的第一个真实子到达 ⇒ 不再是「无子」，下一帧起不产 typing
//   （真实子就是那句里的「非-typing 兄弟」，到达即顶掉占位）；
// - **父收束**：根级骨架不再 streaming ⇒ 谓词不成立，typing 消失。
//
// 之所以能靠「不再合成」实现移除，是因为 typing 是**派生节点**、从不落地——它没有
// 需要在 store 里就地标 `status=removed` 的持久身；移除 ⟺ 谓词转假。合成节点带稳定
// 前缀 id（`typingIdOf`），永不与后端 id 相撞。
//
// **只认根级骨架，不认挂在父节点下的子骨架**：后者有它自己的在途信号（「运行中 / 秒数」），
// 给它挂「生成中」占位是错位的呈现；「等 LLM 吐出第一个节点」这个语义只属于会话流的根
// ——与 `SkeletonNode` 既有的等待态判据同一 scope。全程只按形状（形态 + 有无父）判定，
// 不引入任何「一整轮」的业务概念。

/** typing 占位的稳定合成 id——以宿主为键，跨帧不变（缓存/键控都靠它）。 */
export function typingIdOf(hostId: string): string {
  return `__typing__${hostId}`
}

/** 该节点是否是 typing 的宿主——正在 streaming 的**根级骨架**（`skeleton` 且无父节点）。 */
export function isTypingHost(node: ChatNode): boolean {
  // 骨架格同时容纳会话流的根与挂在父节点下的子骨架：后者有它自己的「运行中 / 秒数」信号，
  // 不该再挂「生成中」占位。用「无父」把两者分开——纯形状判据，不读 style_tag、不认业务名。
  return (
    isStreamingStatus(node.status) &&
    messageTypeOf(node) === NODE_KIND_SKELETON &&
    !node.parent_id
  )
}

/** 从可见节点扁平表合成 typing 占位子节点（宿主 = streaming 且无真实子的根级骨架）。 */
export function synthesizeTypingChildren(visible: ChatNode[]): ChatNode[] {
  // 只数**真实子**：已有的 typing 派生节点不能反过来让宿主显得「有子」。
  const parentsWithRealChild = new Set<string>()
  for (const n of visible) {
    if (n.parent_id && messageTypeOf(n) !== NODE_KIND_TYPING) {
      parentsWithRealChild.add(n.parent_id)
    }
  }
  const out: ChatNode[] = []
  for (const host of visible) {
    if (!isTypingHost(host)) continue
    if (parentsWithRealChild.has(host.id)) continue
    out.push({
      id: typingIdOf(host.id),
      parent_id: host.id,
      type: NODE_KIND_TYPING,
      status: NODE_STATUS_STREAMING,
      principal: host.principal,
      seq: host.seq,
    })
  }
  return out
}

/** 是否等待用户响应（审批 / 提问） */
export function isWaitingStatus(status?: NodeStatus | null): boolean {
  return status === NODE_STATUS_WAITING_USER_ACTION
}

/** 是否以错误结束 */
export function isFailedStatus(status?: NodeStatus | null): boolean {
  return status === NODE_STATUS_FAILED
}

/** 是否用户主动终止 */
export function isAbortedStatus(status?: NodeStatus | null): boolean {
  return status === NODE_STATUS_ABORTED
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
 * 覆盖 wire 闭集五值（`NODE_KINDS`）——[T-155 B2a] 起前端只认这五格，业务旧词在
 * 读边界经 `nodeKindOf` 归一后不再进入取值域。漏一格会把机制词原样摆给用户看，
 * 由单测锁死覆盖率。
 */
export const NODE_KIND_LABELS: Record<string, string> = {
  [NODE_KIND_BODY]: '正文',
  [NODE_KIND_NOTICE]: '系统提示',
  [NODE_KIND_SKELETON]: '组合',
  [NODE_KIND_INTERACTION]: '待响应',
  [NODE_KIND_TYPING]: '生成中',
}

/**
 * 状态 → 文案。
 *
 * `active` 是**旧数据别名**（历史上「已结束」与「未标注」都被映射成它），
 * 保留映射仅为兼容已落库的历史节点，新节点不会再出现该取值。
 */
export const NODE_STATUS_LABELS: Record<string, string> = {
  [NODE_STATUS_PENDING]: '排队中',
  [NODE_STATUS_STREAMING]: '生成中',
  [NODE_STATUS_WAITING_USER_ACTION]: '等待响应',
  [NODE_STATUS_COMPLETED]: '已完成',
  [NODE_STATUS_ABORTED]: '已中止',
  [NODE_STATUS_FAILED]: '失败',
  [NODE_STATUS_REMOVED]: '已删除',
  /** 旧数据别名，见上方说明 */
  active: '正常',
}

/** 角色文案（未登记 → 原样返回） */
export function messageRoleLabel(role: string): string {
  return MESSAGE_ROLE_LABELS[role] ?? role
}

/** 类型文案（未登记 → 原样返回） */
export function messageTypeLabel(type: string): string {
  return NODE_KIND_LABELS[type] ?? type
}

/** 状态文案（未登记 → 原样返回） */
export function messageStatusLabel(status: string): string {
  return NODE_STATUS_LABELS[status] ?? status
}

// ==================== 形态 → 呈现（**类型知识的唯一来源**） ====================
//
// 这张表回答「这个形态长什么样」，取代原先散在多个取值函数里的 if 链。
//
// ## 它修掉的是什么
//
// 改造前每个业务类型在 `messageIcon` / `messageTitle` / `messageStatusTag` /
// `messageHeadModifier` / `messageIsRunningAction` / `messageDefaultOpen` /
// `messageRendererKey` 里**各出现一次**——新增一种呈现要改 7 处，且没有任何东西
// 保证这 7 处彼此一致（漏一处就是「新类型有图标没标题」）。现在每个形态只在这里出现一次。
//
// ## [T-155 B2a]：取值域收到 wire 五值 `NodeKind`
//
// 分派轴只有五格（`body` / `notice` / `skeleton` / `interaction` / `typing`）。同格内
// 的业务差异（思考 vs 压缩、整轮 vs 工具）**不进这张表**——后端在交互面把它折成
// `title` / `icon` / `actions` **数据**，取值函数优先读数据、缺省才退到本行多数形态。
// 前端因此不再 `switch(style_tag)`，也不认任何业务名。
//
// ## 表**不表达优先级**——优先级是跨形态的规则
//
// 三条跨形态的优先级规则各写在下面取值函数里（各一行，注释说明理由）：
// 1. `role = user` 覆盖类型（用户消息一律走气泡）；
// 2. `skeleton` 覆盖 `role = user`（容器永远走分组形态，见 `messageRendererKey`）；
// 3. 合成请求 / 工具返回是**上下文标记**而非形态（二者只出现在 `type = body` 上，
//    由 `facetsOf` 保证），因此在查表前先行判定。

/** 呈现面取值：字面量，或按 facets 现算（如骨架的图标分根级 / 子会话） */
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
  /** 头部标题；骨架的标题是智能体名（由调用方传入） */
  title: Facet<string>
  /** 渲染器标识（详情形态） */
  renderer: MessageRenderer
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
 * 形态 → 呈现（[T-155 B2a]：取值域就是 wire 闭集五值 `NodeKind`）。
 *
 * 用 `Record<NodeKind, …>`：**漏登记一格是编译错误，不是运行期兜底**。同格内的业务
 * 差异（思考 vs 压缩、整轮 vs 工具）**不在此表**——由后端在交互面折成 `title`/`icon`/
 * `actions` 数据，`messageIcon` / `messageTitle` 优先取数据、缺省才退到本行的多数形态。
 */
const TYPE_PRESENTATION: Record<NodeKind, TypePresentation> = {
  // 正文叶：用户气泡 / 助手正文 / 工具请求·返回（都靠 role + 上下文标记区分，不另立格）
  [NODE_KIND_BODY]: {
    icon: '💬',
    title: (f) => (f.role === CHAT_ROLE_ASSISTANT ? '助手' : '消息'),
    renderer: 'body',
    // 正文：助手展开；工具结果 / 子智能体内部文本收起
    defaultOpen: (f) => f.role !== CHAT_ROLE_TOOL,
  },

  // 折叠叶：思考 / 压缩 / 历史记忆同在这一格——专属图标·标题由后端 title/icon 给出，
  // 本行只定义「多数形态」（默认收起、单行呼吸动效）。
  [NODE_KIND_NOTICE]: {
    icon: '💭',
    title: (f) => (isStreamingStatus(f.status) ? '处理中…' : '提示'),
    renderer: 'notice',
    thinkingWhenRunning: true,
    ownRunningSignal: true,
    // 折叠叶默认单行（记忆/思考收起；压缩的收起态由标题交待）
    defaultOpen: () => false,
  },

  // 容器：一整轮与一次工具调用结构同构——一个可折叠头部 + 一条子流。
  // 工具名 / 图标由后端 title/icon 给出；参数住骨架自身 content（SkeletonNode 当首条正文渲染）。
  [NODE_KIND_SKELETON]: {
    // 根级骨架用 AI，子会话过程段用 ↳
    icon: (f) => (f.subSession ? '↳' : 'AI'),
    title: (_f, agentName) => agentName,
    renderer: 'skeleton',
    thinkingWhenRunning: true,
    runningAction: true,
    ownRunningSignal: true,
    // 折叠态下唯一的运行中信号，覆盖全部非终态；completed 刻意留空
    statusTag: (f) => {
      if (isFailedStatus(f.status)) return '失败'
      if (isAbortedStatus(f.status)) return '已中止'
      if (isWaitingStatus(f.status)) return '待确认'
      if (isStreamingStatus(f.status)) return '运行中'
      return ''
    },
    // 骨架默认展开（整轮 = 对话主体；工具与整轮同构）。待审批 / 用户消息等跨类型
    // 例外由 `messageDefaultOpen` 先行处理，深层级收起由 `effectiveOpenOf` 兜底。
    defaultOpen: () => true,
  },

  // 待用户回应：提问 / 工具确认（子形态由 meta.prompt.kind 判别，标题据此）
  [NODE_KIND_INTERACTION]: {
    icon: '❓',
    title: (f) => (f.promptKind === 'confirm' ? '工具确认' : '提问'),
    renderer: 'interaction',
    defaultOpen: () => true,
  },

  // 进行中占位（[T-151] 前端合成）：折叠单行、只挂「生成中」信号，无正文。
  [NODE_KIND_TYPING]: {
    icon: '⋯',
    title: '生成中',
    renderer: 'fallback',
    ownRunningSignal: true,
    statusTag: () => '生成中',
    defaultOpen: () => false,
  },
}

/**
 * 未登记类型的兜底呈现：按**正文**呈现，只是渲染器走通用兜底
 * （不假装它是已知形态）。这样「后端先行上线新 `type`、前端还是旧版」
 * 不会把会话流渲染成空白。
 */
const FALLBACK_PRESENTATION: TypePresentation = {
  ...TYPE_PRESENTATION[NODE_KIND_BODY],
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
 * [T-155] 呈现数据优先：后端填了 `node.icon` 就直接用（前端不再从业务类型现推）。
 * 缺省才落 legacy 优先级链：用户消息 > 合成请求 > 工具返回 > 类型表。
 * 请求 / 返回只出现在 `type = text` 上（`facetsOf` 保证），故在查表前判定。
 */
export function messageIcon(f: MessageFacets): string {
  if (f.icon) return f.icon
  if (f.compacted) return '🧠'
  if (f.role === CHAT_ROLE_USER) return '👤'
  if (f.toolRequest) return '📤'
  if (f.toolResult) return '↩'
  return facetValue(presentationOf(f.type).icon, f)
}

/**
 * 头部标题。
 *
 * [T-155] 呈现数据优先：后端填了 `node.title` 就直接用。
 * `agentName` 由调用方传入（骨架容器的标题是「该智能体的名称」，取值逻辑见 `agentNameOf`）。
 * 缺省才落 legacy 优先级与同源（用户 > 请求 > 返回 > 类型表）。
 */
export function messageTitle(f: MessageFacets, agentName: string): string {
  if (f.title) return f.title
  if (f.compacted) return '历史记忆'
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
  skeleton: boolean
  notice: boolean
  /** 标题呼吸动效：思考中**与工具执行中**共用——两者都是「正在进行、内容尚未成形」的行 */
  thinking: boolean
}

export function messageHeadModifier(f: MessageFacets): MessageHeadModifier {
  const p = presentationOf(f.type)
  return {
    user: f.role === CHAT_ROLE_USER,
    sub: f.subSession,
    // 头部色调钩子按形态给：容器格与折叠叶各一色——纯形状，不认业务名
    skeleton: f.type === NODE_KIND_SKELETON,
    notice: f.type === NODE_KIND_NOTICE,
    // 标题呼吸动效：凡是「正在进行、内容尚未成形」的行（折叠叶思考 / 容器运行中）
    thinking:
      Boolean(p.thinkingWhenRunning || p.runningAction) && isStreamingStatus(f.status),
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
 * 收起态单行摘要取内容的**哪一端**：流式中取末端（最新），定稿后取开头（摘要）。
 *
 * 「这一行是什么」在两种状态下**是两件事**：
 *
 * - **流式中**，它是这条节点的走马灯。内容每帧都在长，只有跟着末端走这一行才会动；
 *   取开头等于把这行字钉死在最早那几个字上——界面上既看不出「还在长」还是「卡住了」，
 *   也永远追不上正文/思考已经流到的地方（用户看到的正是"折叠起来一直是开头那句"）。
 * - **定稿后**，内容不再变，这一行的职责变回**摘要**——开头才是概述
 *   （工具参数 JSON 尤其如此：取末端只剩一个 `}`）。
 *
 * 判据只取**协议字段**（`status`），不嗅探内容形态：流式中的 JSON 尚未闭合，
 * 按内容判「是不是 JSON」会在某一片恰好闭合时突然换端，比不换更乱。
 *
 * 与 `useMessageContent::messageSummaryPreviewOf` 的 `liveEdge` 参数是同一件事的
 * 两个半场——那边决定取哪一端，`NodeShell` 据此决定往哪一端裁（见其 `.node-preview`）。
 */
export function messagePreviewFollowsLiveEdge(f: MessageFacets): boolean {
  return isStreamingStatus(f.status)
}

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
  // 压缩记忆：默认**收起**。它是可以很长的一段摘要，展开会盖过对话本体；
  // 折叠态的标题（「历史记忆」）已经把它交待清楚了。这条必须排在角色规则之前
  // ——它的 `role` 是 user（见 `MessageFacets.compacted`）。
  if (f.compacted) return false
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
//   schemas/chat_node.ts   数据契约：取值词表 + 形状
//   本文件                    **纯映射**：facets → 渲染器标识；标识 → 组件（由装配点登记）
//   registry/messageRenderers.ts  **唯一 import 组件的地方**（把标识绑到具体组件）
//
// 于是「新增一种呈现形态」的完整代价是：
//   1. wire 侧：后端枚举 + 前端 `NODE_KINDS` 各加一值（闭集跨栈批次）；
//      若只是既有五值的呈现变体（思考 vs 压缩、整轮 vs 工具），**前端零改动**——
//      后端在交互面把它折成 `title` / `icon` / `actions` 数据即可
//   2. 本文件 `TYPE_PRESENTATION` 加一行（仅当新增的是第六个五值形态）、文案表加一行
//   3. `messageRenderers.ts` 加一行注册
// **不再需要改渲染组件里的 if 链**——这正是本次改造要买到的性质。漏掉第 2 步会
// **编译报错**（`Record<NodeKind, …>` 要求穷尽），而不是运行期静默退化成兜底。

/**
 * 渲染器标识（机制级的呈现形态，不含场景语义）。
 *
 * [T-155 B2a]：与 wire 五值 `NodeKind` 一一对应（`typing` 占位走 `fallback`）——
 * 前端分派轴**只有** NodeKind，不再有业务形态标识。
 *
 * - `body`        正文叶：用户气泡 / 助手正文 / 工具请求·返回
 * - `notice`      折叠叶：思考 / 上下文压缩 / 历史记忆
 * - `skeleton`    容器：一整轮助手响应 与 一次工具调用（结构同构，都递归子流）
 * - `interaction` 待用户响应（提问 / 工具确认）
 * - `fallback`    未登记形态（后端先行上线的新 type）与 typing 占位的只读兜底
 */
export type MessageRenderer =
  | 'body'
  | 'notice'
  | 'skeleton'
  | 'interaction'
  | 'fallback'

/**
 * 判定节点该由哪个渲染器接管（**前端选择渲染形态的唯一入口**）。
 *
 * 判定顺序有语义：
 * 1. `skeleton` 最先——任何容器都走分组形态（用户消息若是容器亦如此）；
 * 2. **压缩记忆先于角色**——它是 `role = user` 的系统记忆，走 `notice` 折叠叶形态
 *    （正是这一步阻止它渲染成「你说的那句话」）；
 * 3. 角色优先于类型——用户消息一律走气泡（`body`），即便它的 `type` 是别的；
 * 4. 其余按形态分派（`TYPE_PRESENTATION.renderer`）；
 * 5. 未登记取值 → `fallback`（新类型上线时旧前端仍能显示内容）。
 */
export function messageRendererKey(f: MessageFacets): MessageRenderer {
  if (f.type === NODE_KIND_SKELETON) return 'skeleton'
  if (f.compacted) return 'notice'
  if (f.role === CHAT_ROLE_USER) return 'body'
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


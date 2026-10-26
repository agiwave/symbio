/**
 * 消息节点的**摊平与谓词** —— 「节点 → 稳定读法」的唯一实现
 *
 * ## 这一层只管两件事
 *
 * - **摊平**：把 wire 上的 `ChatNode` 归一成前端内部的取值（角色 / 形态 / 状态）
 *   与一份 `MessageFacets`（呈现判定的**全部**入参）；
 * - **谓词**：把「能不能重试」「是否等待用户」「这次调用没结果」这类判定写成纯函数。
 *
 * 这一层**零文案、零渲染器**：它只回答「这个节点是什么处境」，不回答「该画成什么样」
 * （后者在 `messagePresentation.ts`，分派在 `messageRouting.ts`）。
 *
 * ## 为什么 `MessageFacets` 住在这里而不是各组件自己算
 *
 * 判定要用到的不只是节点自身字段，还有「父节点是什么形态」「子节点里有没有待审批」
 * 这类**上下文**。把它们显式列进 facets，而不是让本模块去读 props，好处是每个判定都是
 * 纯函数：可直接单测，且不需要为了测试搭一棵组件树。
 * 组件只负责把 props 交给 `facetsOf`（那一层是"读 props"，不是"做判定"）——
 * 分派器与各个体渲染器调同一个函数，因此同一个节点在两侧拿到的判定必然一致。
 *
 * ## 后端 `meta` 字段的读取为什么集中在这里
 *
 * `meta` 是后端 flatten 出来的**场景字段袋**，字段名与含义只有后端契约知道。组件里直接写
 * `node.meta.recoverable` 等于把后端字段名抄进视图层——后端改一个字段名要在所有渲染器里找。
 * 于是**读取集中在这一处**，组件只问「能不能重试」，不问「meta 里有没有 recoverable」。
 * 这也是机制审计（`scripts/mechanism-audit.mjs` M-001）的判定依据：`components/` 下
 * 不允许出现对后端 meta 字段的直接读取。
 *
 * 口径提醒：随 [D-111](../../../symbio/src/plugins/session/README.md#d-111-消息读数由交付出口装配压缩与记忆的事实行离开前端 "symbio/src/plugins/session/README")，「那几个数怎么说」
 * （压缩水位 / 触发来源 / token 单位）**不在这里**——那是交付出口的读数行 `node.stats`，
 * 本层只留「这条节点是不是压缩产物」这类形状判定。
 */

import {
  CHAT_ROLE_TOOL,
  CHAT_ROLE_USER,
  messageRoleOf,
  NODE_KIND_BODY,
  NODE_KIND_NOTICE,
  NODE_KIND_SKELETON,
  NODE_KIND_TYPING,
  NODE_STATUS_ABORTED,
  NODE_STATUS_COMPLETED,
  NODE_STATUS_FAILED,
  NODE_STATUS_STREAMING,
  NODE_STATUS_WAITING_USER_ACTION,
  nodeKindOf,
  normalizePrincipal,
  principalId,
  PRINCIPAL_KIND_AGENT,
  PRINCIPAL_KIND_PERSON,
  PRINCIPAL_KIND_SYSTEM,
  PRINCIPAL_KIND_TOOL,
  RESUME_ACTION_RETRY,
  RESUME_ACTION_RETRY_COMPACTION,
  RESUME_ACTION_RETRY_SUBTREE,
  RESUME_ACTION_SUPPLY,
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
 * 节点的角色——**唯一读入口**（镜像后端 `ChatNode::node_role`）。
 *
 * 折叠实现住在契约层（`schemas/chat_node.messageRoleOf`，纯逻辑层同源共用）；
 * 这里只转出，registry 的消费面保持稳定。
 */
export { messageRoleOf }

/**
 * 节点的**生效形态**（[D-070 §决策 3]）：wire 五值 `NodeKind`。
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

/**
 * 形态谓词：五格形状的**唯一判据**。
 *
 * 框架层（组件 / store / service / composable）只问「是不是这个形状」，
 * 不再自己写 `type === NODE_KIND_*`——分派轴住在 registry，框架零类型分支
 * （判据见 `scripts/kind-branch-audit.mjs`）。
 */
export const isSkeletonKind = (kind: NodeKind): boolean => kind === NODE_KIND_SKELETON
export const isTypingKind = (kind: NodeKind): boolean => kind === NODE_KIND_TYPING
export const isBodyKind = (kind: NodeKind): boolean => kind === NODE_KIND_BODY
export const isNoticeKind = (kind: NodeKind): boolean => kind === NODE_KIND_NOTICE
/** 正文叶：`body` / `notice` 是唯二把 `content` 当正文呈现的形态。 */
export const isContentKind = (kind: NodeKind): boolean =>
  kind === NODE_KIND_BODY || kind === NODE_KIND_NOTICE

/** 节点的状态（缺省 = 已结束） */
export function messageStatusOf(node: Pick<ChatNode, 'status'>): NodeStatus {
  return (node.status as NodeStatus) ?? NODE_STATUS_COMPLETED
}

/**
 * 工具名（顶层 `name` 已退役，工具名唯一住 `meta.tool_name`）。
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
 * 优先级：`meta.agent_id` > 按是否子会话取兜底名。
 * `meta` 是后端 flatten 的场景字段，这里只做取值，不解释语义。
 * （曾经的 `node.name` 首选项已随顶层字段退役删除——后端从未把智能体名写进
 * name，那条只是一条空通道，。`meta.agent_name` 同样从未有生产者，
 * 一并删除：读一个没人写的键只会让下一个人以为存在第二份名字来源。）
 */
export function agentNameOf(
  node: Pick<ChatNode, 'meta'>,
  subSession: boolean,
): string {
  const meta = node.meta as Record<string, unknown> | undefined
  const fromMeta = meta?.agent_id
  return (
    (typeof fromMeta === 'string' ? fromMeta : '') ||
    (subSession ? '子智能体' : '助手')
  )
}

// ==================== principal 前缀 → 身份呈现（[] 独立表） ====================
//
// 「这条发言是谁说的」的可视化——与「这条是什么形态」（`TYPE_PRESENTATION`）正交：
// 前者按**可路由地址的命名空间**（`<ns>/<name>`，[D-069 §决策 1]）分派，后者按
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
 * 合成的地址段、[D-069] 登记的主体行对齐（`vdfs` / `web` / `local` 是插件归属，
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

/** 族兜底表：命名空间未登记时按 `principal.kind` 取（[D-069] 四族）。 */
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
  /** 工具名（`meta.tool_name`，） */
  name?: string
  /**
   * 后端填的头部标题（[] 呈现数据）——有值即优先于 legacy 类型表。
   * 前端机制化的落点：呈现意图来自节点数据，不靠 `switch(style_tag)` 现推。
   */
  title?: string
  /** 后端填的头部图标（同上，缺省退 legacy 表）。 */
  icon?: string
  /**
   * 后端声明的**可用恢复动作**（[] 呈现数据）——重试 / 补充参数 / 审批入口
   * 的首选判据；缺省（后端未填）退回按业务类型反推的 legacy 判据。
   */
  actions?: ResumeAction[]
  /** principal 身份呈现（[] 独立表：命名空间优先、kind 兜底，无主体 → `undefined`） */
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
    // 呈现数据从节点顶层直读（[]）——不经 meta，是后端填的权威呈现意图。
    title: node.title,
    icon: node.icon,
    actions: node.actions,
    // principal 身份呈现（[] 独立表；组件据此画徽标，缺主体则不画）
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

// ==================== 组件侧判据谓词（[]） ====================
//
// 「这条是不是用户消息 / 容器骨架」这类判据原先在各渲染组件里就地比较协议
// 枚举字面量（`facets.role === 'user'`、`c.type === 'skeleton'`）——每个组件都
// 得 `import` 一份枚举值，「什么算 user/骨架」的口径散在视图层。收进注册表后：
// **组件只问谓词，不再持有枚举字面量**，判据改了只动这里（与 exit 判据 3、
// 「加呈现变体不改组件」同一纪律）。
//
// 只留**真有消费方**的谓词：`isSkeletonFacet` / `isUserNode` / `isSkeletonNode` 曾经
// 在这里「备用」，全仓零引用——没有消费方也没有 owner 的谓词不是契约，是第三态，
// 已删（`dead-code-audit` 的 R-003 现在会拦这种形态）。要用时再加，那时它就有了第一个
// 调用点，加的成本与删的成本一样低。

/** 该 facet 是用户发言（`role = user`）。 */
export const isUserFacet = (f: Pick<MessageFacets, 'role'>): boolean =>
  f.role === CHAT_ROLE_USER

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

/** 系统心跳任务自动发送的消息（后端 `heartbeat_message` 打标） */
export function messageIsHeartbeat(node: Pick<ChatNode, 'meta'>): boolean {
  return Boolean((node.meta as Record<string, unknown> | undefined)?.heartbeat)
}

/**
 * 后端标记该失败**可恢复**（会话因它暂停，用户可就地重试）。只保证「存在即真」。
 */
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

/**
 * 建树策略 `flat` 档下，本轮响应骨架的直接子节点**升了根**（`parent_id` 被摘掉），
 * 靠这个记号认回它属于哪一轮（后端 `node_build::turn_root_of` 的镜像）。
 *
 * 前端为什么必须认它：不认的话「这一支的子孙」在 `flat` 档下够不着，切换分支时
 * 旧支只藏掉了根、子孙全部留在屏幕上——后端为此把子树收集写成单点实现
 * （`parent_id ∪ turn_root` 一条并集），前端同一条判据不能只按 `parent_id` 走。
 */
export function messageTurnRoot(node: Pick<ChatNode, 'meta'>): string | null {
  const raw = (node.meta as Record<string, unknown> | undefined)?.turn_root
  return typeof raw === 'string' && raw !== '' ? raw : null
}

// ==================== 分支组（重试留下的并存两支）====================

/**
 * 一个节点所在**分支组**的读数（后端 `meta.branch` 的唯一读点）。
 *
 * 后端把「同一个问题有几支答复、现在读第几支」折成事实投影盖在成员根节点上
 * （[D-173](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）；
 * 这一层把它读成**数据**，组件只问「第几支 / 共几支 / 前后是谁」，
 * 不认识 `candidates` 里那些 id 的含义——切换动作把目标候选原样交回后端，
 * 后端按它认人。
 */
export interface NodeBranch {
  /** 组锚 = 第一次尝试的根节点 id（同一组的成员共享它） */
  group: string
  /** 组内**存在节点**的成员 id，按转写顺序（旧 → 新） */
  candidates: string[]
  /** 自己在 `candidates` 里的下标 */
  index: number
  /** = `candidates.length`（后端保证一致；这里不透传，缺标注就是没有分支） */
  total: number
  /** 是不是当前读的那一支 */
  selected: boolean
}

/**
 * 读这条节点的分支标注；**没有标注 ⇒ `null`**（它不在任何组里，也就没有切换器）。
 *
 * 形状不齐（字段缺失 / 类型不对）同样读成 `null`：宁可少画一个切换器，也不给一个
 * 点了不知道会切到哪里的控件。
 */
export function messageBranchOf(node: Pick<ChatNode, 'meta'>): NodeBranch | null {
  const raw = (node.meta as Record<string, unknown> | undefined)?.branch
  if (!raw || typeof raw !== 'object') return null
  const b = raw as Record<string, unknown>
  const { group, candidates, index, total, selected } = b
  if (typeof group !== 'string' || typeof selected !== 'boolean') return null
  if (typeof index !== 'number' || typeof total !== 'number') return null
  if (!Array.isArray(candidates) || candidates.length !== total) return null
  if (!candidates.every((c): c is string => typeof c === 'string')) return null
  if (index < 0 || index >= total) return null
  return { group, candidates: candidates as string[], index, total, selected }
}

/**
 * 这一组里**相邻的**另一支（`-1` = 更早的尝试，`+1` = 更晚的尝试）；走到头返回 `null`。
 *
 * 返回的是**候选 id**，不是下标：调用方拿它直接发切换动作，不必再碰 `candidates`
 * ——把数组下标运算留在组件里，等于让每个渲染器都重写一遍边界判断。
 */
export function messageBranchNeighbor(
  branch: NodeBranch,
  direction: -1 | 1,
): string | null {
  const next = branch.index + direction
  if (next < 0 || next >= branch.total) return null
  return branch.candidates[next] ?? null
}

// ==================== 消息级业务规则（纯函数） ====================
//
// 「这个节点能做什么」原先写在渲染组件的 computed 里，与「怎么画」混在一起。
// 抽成纯函数后：可单测、可被非渲染场景复用（如命令面板），
// 且**规则变了只改一处**。

/**
 * 节点是否携带某个可用恢复动作（纯数据判据的唯一读法）。
 *
 * `actions` 由后端在交互面派生（[D-070 §决策 3]），前端不再按业务类型 / `recoverable`
 * 反推「能不能重试」——「有对应动词即有入口，没有即不画」。`actions` 缺席（历史数据、
 * 前端合成节点）一律 `false`：合成节点本就无可重试语义。
 */
function hasAction(actions: ResumeAction[] | undefined, action: ResumeAction): boolean {
  return actions?.includes(action) ?? false
}

/**
 * 工具失败能否**就地重试**（resume action=`retry`）。
 *
 * 判据已是**纯数据**：后端在交互面（[D-070 §决策 3]）把「这条节点能发
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
 * 三个粒度，**粒度由后端下发的 `actions` 决定**（[D-070 §决策 3]）：
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
 * 动作种类**优先读后端下发的 `actions`**——工具节点带 `retry`、压缩节点带
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

// 后端失败类别：本批剩余调用被跳过，实际未执行（见后端 `not_executed_result`）
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

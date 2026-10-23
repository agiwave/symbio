/**
 * 消息的**呈现与文案** —— 「facets → 长什么样」的唯一来源
 *
 * ## 这一层管什么
 *
 * - **词表文案**：角色 / 形态 / 状态 → 面向用户的词（未登记取值原样返回，便于发现新取值）；
 * - **形态呈现表**：`TYPE_PRESENTATION` 一行一个形态，图标 / 标题 / 渲染器标识 /
 *   是否动作型 / 折叠默认态全在这一处；
 * - **折叠策略**：默认展开判据、深层级收拢、用户覆盖后的有效展开态与下一次点击的取值。
 *
 * 入参一律是 `MessageFacets`（由 `messageFacts.facetsOf` 摊出），不是 `ChatNode`：
 * 这一层不做取值归一，只把已经归一好的判定入参换成呈现取值。
 *
 * ## 这张表修掉的是什么
 *
 * 改造前每个业务类型在 `messageIcon` / `messageTitle` / `messageStatusTag` /
 * `messageHeadModifier` / `messageIsRunningAction` / `messageDefaultOpen` /
 * `messageRendererKey` 里**各出现一次**——新增一种呈现要改 7 处，且没有任何东西保证
 * 这 7 处彼此一致（漏一处就是「新类型有图标没标题」）。现在每个形态只在这里出现一次。
 *
 * 分派轴只有 wire 五值（`body` / `notice` / `skeleton` / `interaction` / `typing`）。
 * 同格内的业务差异（思考 vs 压缩、整轮 vs 工具）**不进这张表**——后端在交付出口把它折成
 * `title` / `icon` / `actions` **数据**（D-070 §决策 3），取值函数优先读数据、
 * 缺省才退到本行多数形态。前端因此不 `switch(style_tag)`，也不认任何业务名。
 *
 * ## 表**不表达优先级**——优先级是跨形态的规则
 *
 * 跨形态的优先级写在下面各取值函数里（各一行，注释说明理由）：用户角色覆盖类型、
 * 容器覆盖用户角色、合成请求 / 工具返回是上下文标记而非形态（查表前先行判定）。
 *
 * 零组件导入（M-004）：组件与 CSS 类名钩子由消费方按这里的取值去挂。
 */

import {
  CHAT_ROLE_ASSISTANT,
  CHAT_ROLE_SYSTEM,
  CHAT_ROLE_TOOL,
  CHAT_ROLE_USER,
  NODE_KIND_BODY,
  NODE_KIND_INTERACTION,
  NODE_KIND_NOTICE,
  NODE_KIND_SKELETON,
  NODE_KIND_TYPING,
  NODE_STATUS_ABORTED,
  NODE_STATUS_COMPLETED,
  NODE_STATUS_FAILED,
  NODE_STATUS_PENDING,
  NODE_STATUS_REMOVED,
  NODE_STATUS_STREAMING,
  NODE_STATUS_WAITING_USER_ACTION,
  type NodeKind,
} from '@/schemas/chat_node'
import {
  isAbortedStatus,
  isFailedStatus,
  isStreamingStatus,
  isWaitingStatus,
  type MessageFacets,
} from './messageFacts'


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
 * 覆盖 wire 闭集五值（`NODE_KINDS`）——起前端只认这五格，业务旧词在
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
// ## 取值域收到 wire 五值 `NodeKind`
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
 * 形态 → 呈现：取值域就是 wire 闭集五值 `NodeKind`）。
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

  // 进行中占位（[] 前端合成）：折叠单行、只挂「生成中」信号，无正文。
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
 * [] 呈现数据优先：后端填了 `node.icon` 就直接用（前端不再从业务类型现推）。
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
 * [] 呈现数据优先：后端填了 `node.title` 就直接用。
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


/**
 * 渲染器标识（机制级的呈现形态，不含场景语义）。
 *
 * 与 wire 五值 `NodeKind` 一一对应（`typing` 占位走 `fallback`）——
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

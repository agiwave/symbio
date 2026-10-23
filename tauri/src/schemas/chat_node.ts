/**
 * 对话消息的数据契约（与后端 `symbio_core` 的消息形状对齐）
 *
 * ## 为什么词表要有**运行时常量**，而不只是类型联合
 *
 * 类型联合（`type NodeStatus = 'pending' | ...`）只在编译期存在，运行时拿不到
 * 名字。于是消费方只能各写各的字面量——`'tool_call'` / `'turn'` / `'failed'`
 * 散落在组件、store、服务层，改一处漏一处。
 *
 * 这里为每个取值导出常量，**并让类型由常量数组派生**：
 * 加一个取值 = 加一行数组元素，类型与常量同时到位，不存在「类型改了常量没改」。
 * 呈现映射（图标 / 标题 / 标签）在 `registry/messageTypes.ts`；本文件零呈现知识。
 */

import type { ReadingSection } from './reading'

// ==================== 主体（Principal） ====================

/**
 * **主体族**——后端 `PrincipalKind` 的镜像（[D-069 §决策 1]，四值闭集）。
 *
 * 与曾经的 `role` 是**同一件事**：`role` 从协议顶层退役后，`kind` 是
 * 这条发言归属哪一族的唯一判据。类型由常量数组派生，加一族只加一行。
 */
export const PRINCIPAL_KIND_PERSON = 'person'
export const PRINCIPAL_KIND_AGENT = 'agent'
export const PRINCIPAL_KIND_TOOL = 'tool'
export const PRINCIPAL_KIND_SYSTEM = 'system'

export const PRINCIPAL_KINDS = [
  PRINCIPAL_KIND_PERSON,
  PRINCIPAL_KIND_AGENT,
  PRINCIPAL_KIND_TOOL,
  PRINCIPAL_KIND_SYSTEM,
] as const

export type PrincipalKind = (typeof PRINCIPAL_KINDS)[number]

/**
 * **主体地址**——后端 `symbio_core::principal::Principal` 的镜像（[D-069 §决策 1]）。
 *
 * 三个字段正交：`kind` 是一族、`id` 是可路由地址 `<namespace>/<name>`（找到 / 调用 / 查看
 * 主体都靠这个地址）、`title` 是显示名。写侧构造即定型，序列化只出对象形态；
 * 读侧兼容历史裸字符串由 [`normalizePrincipal`] 兜住。
 */
export interface Principal {
  kind: PrincipalKind;
  id: string;
  /** 显示名——工具响应节点常带、用户与主智能体常不带。 */
  title?: string;
}

/**
 * **读侧一次性映射**（[D-069 §决策 6]，）：历史 `messages.json` 里的
 * `principal` 是**裸字符串**（`"agent:main"` / `"user"` / `"session/main"`），新协议
 * 里是 `Principal` 结构体；前端消费方一律通过本函数拿到结构体形态。
 *
 * 与后端 Rust 侧 `Principal::from_legacy` **逐字一致**——同一份判据两处实现由
 * `scripts/protocol-mirror-audit.mjs` 的 C 组（词表）与本文件的 kind 常量数组守住。
 */
export function normalizePrincipal(value: Principal | string | undefined | null): Principal | undefined {
  if (value === undefined || value === null) return undefined
  if (typeof value === 'object') return value
  const s = value
  // 新形态 `<ns>/<name>`——按 namespace 派生 kind。
  const slash = s.indexOf('/')
  if (slash > 0) {
    const ns = s.slice(0, slash)
    const kind: PrincipalKind =
      ns === 'person'
        ? PRINCIPAL_KIND_PERSON
        : ns === 'system'
          ? PRINCIPAL_KIND_SYSTEM
          : // 工具命名空间：vdfs/web/local 是插件归属，mcp/agent/legacy 是后端
            // `Principal::tool_principal` 合成的地址段——与 Rust `from_legacy` 同一张表。
            ns === 'vdfs' || ns === 'web' || ns === 'local' || ns === 'mcp' || ns === 'agent' || ns === 'legacy'
            ? PRINCIPAL_KIND_TOOL
            : PRINCIPAL_KIND_AGENT
    return { kind, id: s }
  }
  // 旧形态 `<kind>:<name>`（D-069 v1）。
  const colon = s.indexOf(':')
  if (colon > 0) {
    const kindWord = s.slice(0, colon)
    const name = s.slice(colon + 1)
    switch (kindWord) {
      case 'person':
        return { kind: PRINCIPAL_KIND_PERSON, id: s }
      case 'tool':
        return { kind: PRINCIPAL_KIND_TOOL, id: `legacy/${name}` }
      case 'system':
        return { kind: PRINCIPAL_KIND_SYSTEM, id: s }
      default:
        // agent:xxx → session/xxx（本机部署的 agent 都住 session 命名空间）。
        return { kind: PRINCIPAL_KIND_AGENT, id: `session/${name}` }
    }
  }
  // 裸 `user` ⇒ Person + `person/local`；其余裸名一律按 agent 挂 session 下。
  if (s === 'user') return { kind: PRINCIPAL_KIND_PERSON, id: 'person/local' }
  return { kind: PRINCIPAL_KIND_AGENT, id: `session/${s}` }
}

/** 取主体**可路由地址**（`<ns>/<name>`），历史字符串形态一并归一。 */
export function principalId(value: Principal | string | undefined | null): string | undefined {
  return normalizePrincipal(value)?.id
}

/** 取主体**族**（渲染 / 分组判据）。 */
export function principalKind(value: Principal | string | undefined | null): PrincipalKind | undefined {
  return normalizePrincipal(value)?.kind
}

// ==================== 角色 ====================

/** 用户发言 */
export const CHAT_ROLE_USER = 'user'
/** 助手（模型）发言 */
export const CHAT_ROLE_ASSISTANT = 'assistant'
/** 工具返回（含子智能体响应的骨架） */
export const CHAT_ROLE_TOOL = 'tool'
/** 系统发言（心跳等） */
export const CHAT_ROLE_SYSTEM = 'system'

export const CHAT_ROLES = [
  CHAT_ROLE_USER,
  CHAT_ROLE_ASSISTANT,
  CHAT_ROLE_TOOL,
  CHAT_ROLE_SYSTEM,
] as const

export type ChatRole = (typeof CHAT_ROLES)[number]

/**
 * 主体族 → 角色词的**一次性折叠**（，镜像后端 `ChatNode::node_role`
 * 的 kind 兜底分支）：Person↔User / Agent↔Assistant / Tool↔Tool / System↔System。
 * 无主体 ⇒ `undefined`——缺省语义（当助手）留在调用侧折叠点，不在这里替它做主。
 */
export function roleOfPrincipal(
  value: Principal | string | undefined | null,
): ChatRole | undefined {
  switch (principalKind(value)) {
    case PRINCIPAL_KIND_PERSON:
      return CHAT_ROLE_USER
    case PRINCIPAL_KIND_AGENT:
      return CHAT_ROLE_ASSISTANT
    case PRINCIPAL_KIND_TOOL:
      return CHAT_ROLE_TOOL
    case PRINCIPAL_KIND_SYSTEM:
      return CHAT_ROLE_SYSTEM
    default:
      return undefined
  }
}

/**
 * 节点角色的**唯一折叠点**（，镜像后端 `ChatNode::node_role`）。
 *
 * 共存阶段优先级「残留 `role` 格在先、`principal.kind` 兜底」；`role` 字段退役
 * （末档）后删首行即收敛成纯 kind 折叠。缺省仍是助手——未标身份的写方按模型
 * 产出一族呈现（今天的流式帧形状，语义不变）。住在契约层（而非 registry）：
 * 纯逻辑层（`sessionTranscript`）也要用它，判据不许有第二份。
 */
export function messageRoleOf(node: Pick<ChatNode, 'role' | 'principal'>): ChatRole {
  return (node.role as ChatRole) ?? roleOfPrincipal(node.principal) ?? CHAT_ROLE_ASSISTANT
}

// ==================== 消息形态 ====================

// ---- wire 闭集五值 = 前端唯一分派轴（[ B2a]，[D-070 §决策 3]）----
//
// 这五值是**纯 UI 形状**，不含任何业务：同格内的业务差异（思考 vs 压缩、run vs tool）
// 一律由后端在交互面折成 `title` / `icon` / `actions` **数据**（[D-070 §决策 2]），
// 前端照数据显示，绝不读 `meta.style_tag` 反推。`style_tag` 就此从前端 wire 退役——
// 它仍是后端业务判据（沉 `meta`），但不是前端消费的顶层字段。

/** 展开的正文叶子（普通文本 / 用户气泡 / 工具请求·返回） */
export const NODE_KIND_BODY = 'body'
/**
 * 折叠的叶子（一个可折叠行 + `title` + `icon`，展开是正文）。
 *
 * 思考 / 压缩 / 历史记忆 / 未来的 step_summary 全在这一格——前端对它们渲染完全一样，
 * 不认「这坨叫思考还是压缩」。
 */
export const NODE_KIND_NOTICE = 'notice'
/**
 * 容器（一个骨架挂一条子流）。
 *
 * `run`（一整轮）与 `tool`（一次工具调用）与 `sub_agent` 全在这一格——结构同构，
 * 前端一律递归 `children`，不认 tool/turn。工具参数就住在骨架自身的 `content` 里，
 * 容器把它当作首条正文渲染（数据驱动，不为「工具」写特例）。
 */
export const NODE_KIND_SKELETON = 'skeleton'
/** 等待用户回应（ask_user 提问 / 工具确认） */
export const NODE_KIND_INTERACTION = 'interaction'
/** 进行中占位（前端合成，见 []） */
export const NODE_KIND_TYPING = 'typing'

export const NODE_KINDS = [
  NODE_KIND_BODY,
  NODE_KIND_NOTICE,
  NODE_KIND_SKELETON,
  NODE_KIND_INTERACTION,
  NODE_KIND_TYPING,
] as const

/** wire 闭集：`ChatNode.type` 的取值域（后端来源与前端内部表示同一个五值） */
export type NodeKind = (typeof NODE_KINDS)[number]

/**
 * 历史业务词 → wire 五值的一次性折叠（[ B2a]，读边界唯一实现）。
 *
 * 存量快照与后端旧转写把形态写成业务词（`turn` / `tool_call` / `text` / `reasoning` /
 * `compression` / `user_prompt`）；新协议一律发五值。本函数在**读取那一刻**把旧词归到
 * 五值，此后前端内部只有五值——业务词不再进入任何判定。`reasoning` 与 `compression`
 * 都归 `notice`（同格），`turn` 与 `tool_call` 都归 `skeleton`（同格），折叠即合流。
 * 已是五值 / 未知取值原样返回（交给渲染兜底，不判红）。
 */
const LEGACY_TYPE_TO_NODE_KIND: Record<string, NodeKind> = {
  text: NODE_KIND_BODY,
  reasoning: NODE_KIND_NOTICE,
  compression: NODE_KIND_NOTICE,
  turn: NODE_KIND_SKELETON,
  tool_call: NODE_KIND_SKELETON,
  sub_agent: NODE_KIND_SKELETON,
  user_prompt: NODE_KIND_INTERACTION,
}

/** 把任意来源的 `type` 归一成 wire 五值（缺省 `body`：未标形态的帧按正文处理）。 */
export function nodeKindOf(rawType?: string | null): NodeKind {
  if (!rawType) return NODE_KIND_BODY
  if ((NODE_KINDS as readonly string[]).includes(rawType)) return rawType as NodeKind
  return LEGACY_TYPE_TO_NODE_KIND[rawType] ?? NODE_KIND_BODY
}

// ==================== 消息状态 ====================

/** 未开始 */
export const NODE_STATUS_PENDING = 'pending'
/** 正在产生内容 */
export const NODE_STATUS_STREAMING = 'streaming'
/** 等待用户响应（审批 / 提问） */
export const NODE_STATUS_WAITING_USER_ACTION = 'waiting_user_action'
/** 终态：正常结束 */
export const NODE_STATUS_COMPLETED = 'completed'
/** 终态：用户主动终止（没出错，但也没跑完——与 `failed` 一样可重试） */
export const NODE_STATUS_ABORTED = 'aborted'
/** 终态：以错误结束 */
export const NODE_STATUS_FAILED = 'failed'
/**
 * 终态：节点已被删除（**就地移除，不可重试**）。
 *
 * 协议里没有 `remove` 操作——删除就是一次状态迁移，与出现、增长、完成同走一条
 * 消息帧。落到本状态的帧在接收端**就地移除**该节点（从本地视图消失），不产生
 * 任何「清空重读」。根因见后端 `symbio_core::schemas::session::chat_node::NodeStatus::Removed`。
 */
export const NODE_STATUS_REMOVED = 'removed'

export const NODE_STATUSES = [
  NODE_STATUS_PENDING,
  NODE_STATUS_STREAMING,
  NODE_STATUS_WAITING_USER_ACTION,
  NODE_STATUS_COMPLETED,
  NODE_STATUS_ABORTED,
  NODE_STATUS_FAILED,
  NODE_STATUS_REMOVED,
] as const

export type NodeStatus = (typeof NODE_STATUSES)[number]

/**
 * 是否是**已知的消息状态词**（集合判定，由 `NODE_STATUSES` 派生）。
 *
 * 存在的理由：消费方要「把后端下发的状态词原样透传、只拦未知值」，而逐个
 * `case` 枚举等于又抄一份词表——**词表加了取值而 `case` 忘了加**，那条变更就
 * 会被当成"未知状态"丢掉。实测代价见消息节点的状态透传链路（当时叫
 * `vdfsTranscriptSync`）：`aborted` 曾因此让中止后的根级骨架显示为已完成，
 * 重试入口不出现。
 *
 * 判定与词表同源，故**新增一个状态词不需要改这里**。
 */
export function isNodeStatus(value: unknown): value is NodeStatus {
  return typeof value === 'string' && (NODE_STATUSES as readonly string[]).includes(value)
}

/**
 * 消息是否**仍在飞行中**（尚未定稿）。
 *
 * 与后端 `orchestrator::failure::is_inflight` 是**同一集合**：`pending` / `streaming`，
 * 加上「还没标注状态」（流式补丁可能只带了内容，状态在后续帧才到）。
 *
 * `waiting_user_action` **不在此列**：它不是「正在跑」，而是「等用户回答」——
 * 用户仍可从审批卡片回答或忽略，把它当成在途会让审批入口在中止时被误删。
 *
 * 消费方：`hydrateFromHistory` 用它决定「快照里没有的本地节点是否该保留」。
 */
export function isInflightNodeStatus(status?: NodeStatus | null): boolean {
  return (
    status === undefined ||
    status === null ||
    status === NODE_STATUS_PENDING ||
    status === NODE_STATUS_STREAMING
  )
}

/**
 * 需要向用户交代的**非正常终态**：失败或用户中止。
 *
 * 两者都要渲染交代条并给重试入口——中止不是故障，但这一轮只跑了一半，
 * 用户通常就想重跑它。放在契约层是因为它同时被「渲染」与「业务规则」消费。
 */
export function isUnsettledNodeStatus(status?: NodeStatus | null): boolean {
  return status === NODE_STATUS_FAILED || status === NODE_STATUS_ABORTED
}

// ==================== 恢复动作 ====================

/**
 * 会话恢复动作（后端 `ResumeAction` 的**线格式词**）。
 *
 * ## 为什么它也需要一张词表
 *
 * 后端 `chat_node.rs` 的单测 `resume_action_wire_words_are_snake_case` 注释里
 * 已经点名：「前端 `ResumePayload.action` 的字面量必须与它逐字相等」。契约写明了，
 * 前端这边却只有两处**手写的联合类型**——`useChatConnection` 的载荷定义与
 * `messageTypes` 的重试分派——谁也不与后端对齐，改一个词就得靠人记得改两处。
 *
 * 处置与上面三张词表相同：常量在此、类型由常量数组派生，跨栈一致性交给
 * `scripts/protocol-mirror-audit.mjs` 的 C 组（后端枚举取值 ↔ 本词表）。
 */

/** 子树重试：删除靶子节点及其子树 → 重新走 LLM 请求（[D-070 §决策 5]） */
export const RESUME_ACTION_RETRY_SUBTREE = 'retry_subtree'
/** **单工具**重试：删除该工具的失败结果 → 用原参数重新执行 */
export const RESUME_ACTION_RETRY = 'retry'
/** 工具审批通过：删除交互节点 → 带 approved=true 重新执行 */
export const RESUME_ACTION_APPROVE = 'approve'
/** 工具审批拒绝：删除交互节点 → 生成拒绝结果 */
export const RESUME_ACTION_REJECT = 'reject'
/** 补充参数后重跑工具（与原 args 浅合并） */
export const RESUME_ACTION_SUPPLY = 'supply'
/** 回答 `ask_user` 提问 */
export const RESUME_ACTION_ANSWER = 'answer'
/** 重跑**这一次压缩**：删除失败压缩节点 → 重新压缩（不动历史） */
export const RESUME_ACTION_RETRY_COMPACTION = 'retry_compaction'

export const RESUME_ACTIONS = [
  RESUME_ACTION_RETRY_SUBTREE,
  RESUME_ACTION_RETRY,
  RESUME_ACTION_APPROVE,
  RESUME_ACTION_REJECT,
  RESUME_ACTION_SUPPLY,
  RESUME_ACTION_ANSWER,
  RESUME_ACTION_RETRY_COMPACTION,
] as const

export type ResumeAction = (typeof RESUME_ACTIONS)[number]

export interface ImageUrl {
  url: string;
  detail?: string;
}

export type ContentPart =
  | { type: 'text'; text: string }
  | { type: 'image_url'; image_url: ImageUrl };

export type MessageContent = string | ContentPart[];

/**
 * 从多模态内容里取出**纯文本**（`MessageContent` → `string` 的**唯一**实现）。
 *
 * 支持后端实际下发的四种形状：字符串 / `{ text }` / `{ parts: [{ text }] }`
 * / `ContentPart[]`。非文本分片（图片等）与其余形状一律产出空串——
 * 宁可显示空白，也不把结构化对象 `String()` 成 `[object Object]`。
 *
 * ## 为什么放在契约层
 *
 * 它紧挨着 `MessageContent` 定义，因为 store / 组合式 / 组件**都要**取正文。
 * 放在任一层里，另外两层就只能自己再写一份——此前确实出现过 5 份实现，
 * 其中 3 份漏掉了 `ContentPart[]` 这一形状，导致纯文本数组被读成空串
 * （正文凭空消失且无任何报错）。取值规则与它描述的类型必须同处一处。
 */
export function messageTextOf(content: MessageContent | undefined | null): string {
  if (!content) return ''
  if (typeof content === 'string') return content

  // 数组形态 = 契约里的多模态形状 `ContentPart[]`：只拼文本分片。
  // `input_text` / `output_text` 是部分 provider 的文本段类型名，一并认下。
  if (Array.isArray(content)) {
    return content
      .filter((p) => {
        // 类型名按 string 比较：`ContentPart` 的联合里只有 `text` / `image_url`，
        // 但 provider 变体还会发 `input_text` / `output_text`
        const t = (p as { type?: string } | undefined)?.type
        return t === 'text' || t === 'input_text' || t === 'output_text'
      })
      .map((p) => (p as { text?: string } | undefined)?.text ?? '')
      .join('\n')
  }

  // 对象形态：`{ text }`，或 `{ parts: [{ text }] }`（部分 provider 的容器形状，
  // 其分片不带 `type` 字段）
  const obj = content as unknown as Record<string, unknown>
  if (typeof obj.text === 'string') return obj.text
  const parts = obj.parts
  if (Array.isArray(parts)) {
    return parts.map((p) => (p as { text?: string } | undefined)?.text ?? '').join('\n')
  }
  return ''
}

/**
 * 会话消息（后端 `symbio_core::schemas::session::chat_node::ChatNode` 的镜像）。
 *
 * **本接口没有 `agent_id` / `prompt` 两个顶层字段**——它们曾在此声明，但后端
 * `ChatNode` 从不下发：`agent_id` 在 `meta` 里（见 `registry/messageTypes.agentNameOf`），
 * `prompt` 在后端是 `#[serde(skip_serializing)]`（内容在 `meta.prompt`）。
 * 留着它们只会让人以为能从顶层读到。字段与后端的对应由
 * `scripts/protocol-mirror-audit.mjs` 的 D 组守着。
 */
export interface ChatNode {
  id: string;
  parent_id?: string;
  /** 树形展开：前端按 `parent_id` 组装的父引用（后端只给扁平列表） */
  parent?: ChatNode;
  role?: ChatRole;
  /**
   * **这条发言是谁说的**（跨栈身份字段，后端 `ChatNode.principal` 的镜像）。
   *
   * / [D-069 §决策 1]：wire 上是**主体地址结构体** `{kind, id, title?}`——
   * `kind` 是一族（`person` / `agent` / `tool` / `system`）、`id` 是 `<ns>/<name>` 可路由地址、
   * `title` 是显示名。写侧构造即定型，序列化只出对象形态。
   *
   * 类型仍与 `string` 联合，是为了**读侧兼容**历史 `messages.json` 里遗留的裸字符串
   * （`"agent:main"` / `"user"`）——`normalizePrincipal` 是唯一入口，消费方一律通过它拿
   * 结构体形态；等历史数据一次性迁移完成后可收窄（`scripts/protocol-mirror-audit.mjs` D 组
   * 只比字段名，不比类型形态）。
   *
   * `undefined` = 尚未标身份（历史消息、未接入的写方）——可见域判据把它当**可见**：
   * 缺身份是「还没接线」，不是「不让人看」。
   */
  principal?: Principal | string;
  /**
   * 形态词：**闭集五值 `NodeKind`**（[ B2a]，[D-070 §决策 3]）——前端唯一分派轴。
   *
   * wire 与前端内部表示同一个五值；存量快照的业务旧词在**读边界**由 `nodeKindOf` 一次性
   * 归一，业务词不进入任何判定。同格业务差异走 `title`/`icon`/`actions` 数据，不读 `meta`。
   */
  type?: NodeKind;
  // 顶层 `name` 从 wire 退役——工具名住 `meta.tool_name`（读写唯一入口
  // `registry/messageTypes.toolNameOf`），存量 attrs `tool_name` 在读边界折进 meta。
  content?: MessageContent;
  /**
   * 流式**增量**：本帧新到达的那一段正文，追加到本地该节点正文尾部。
   *
   * ## 与 `content` 互斥，语义由字段本身给出
   *
   * 一帧里两者**绝不同时出现**——同帧携带即协议违例，后端唯一写入点
   * （`Transcript::apply`）报错丢弃，前端落地（`sessions.applyTranscriptMessage`）
   * 用同一条判据拒绝。因此消费端永远不必从帧的形状推断「该拼接还是该替换」：
   *
   * | 字段 | 语义 |
   * |---|---|
   * | `delta` | 追加到正文尾部（O(delta) 窄帧）——流式热路径 |
   * | `content` | **整条替换**该节点正文（幂等）——一次性节点完整体 / 编辑后的新正文 |
   *
   * ## 为什么不落存储
   *
   * 增量是**不完整的片段**，不是消息的形态：它只在出方向的帧上存在，存储里只有
   * `content`（图内累积的正文）。后端 `chat_session::ensure_durable_states` 在写入点
   * 直接拒绝携带 `delta` 的消息——「delta 永不落盘」是持久层的不变量，不是发射方的自律。
   */
  delta?: string;
  status?: NodeStatus;
  /** 失败原因（面向用户的可读短消息），仅当 status === 'failed' 时存在 */
  error?: string;
  /**
   * 头部**标题**——后端填的呈现数据（[]，[D-070 §决策 2]）。
   *
   * 机制化目标：前端按 `type`(NodeKind) + `title`/`icon`/`actions` 这些数据渲染，
   * **不再 `switch(style_tag)`**。`title`/`icon` 有值即优先用（后端是呈现权威）；
   * 缺省（旧后端 / 历史数据）才退回 `registry/messageTypes` 的 legacy 表。
   * 住在 `ChatNode` 顶层（非 meta），故由 mirror-audit D 组比字段名、G 组要求直接消费。
   */
  title?: string;
  /** 头部**图标**——后端填的呈现数据（同上，缺省退 legacy 表）。 */
  icon?: string;
  /**
   * 本节点**可用的恢复动作**（后端填的呈现数据，[D-070 §决策 2]）。
   *
   * 前端据此决定是否给重试 / 补充参数 / 审批入口，而不是按业务类型（tool / compression）
   * 反推「该给什么按钮」。缺省（后端尚未填）时退回 legacy 的类型驱动判据。
   */
  actions?: ResumeAction[];
  /**
   * 本节点的**读数行**（后端填的呈现数据，[D-111](../../../symbio/src/plugins/session/README.md#d-111-消息读数由交付出口装配压缩与记忆的事实行离开前端 "symbio/src/plugins/session/README")）：
   * 「压缩后水位 44.5k」「压掉 12 条」这类交代由后端排好版逐行下发，前端**只逐行排版**。
   *
   * 为什么这一格在顶层而不是 `meta`：判据是「不查看 `meta` 语义就能消费它」——渲染器
   * 遍历行、拼标签与值，一个业务词都不认（G 组要求它有直接消费者）。
   * 值里的单位与精度是后端定的（同一个数字两处格式化早晚不一致），前端不做数值格式化。
   *
   * 形状复用只读读数的宿主方言 `ReadingSection`（`schemas/reading.ts`，与会话读数面板
   * 同一个形状、同一批镜像守卫）；缺省 = 这个节点没有可报的数（不是「读不到」）。
   */
  stats?: ReadingSection;
  meta?: Record<string, any>;
  timestamp?: number;
  /** 树形展开：前端按 `parent_id` 组装的子列表（后端只给扁平列表） */
  children?: ChatNode[];
  /**
   * 会话内消息的**唯一权威顺序锚点**：后端写入时分配单调自增序号，
   * 父节点序号 < 子节点，按根级骨架（一轮）追加顺序递增；前端对实时流式/乐观消息
   * 也用同一字段的单调计数器续接。前端排序只用 `seq`，不再依赖 timestamp
   * （timestamp 是"业务时刻"而非"顺序"，旧数据还可能缺失，用作排序键会错乱）。
   */
  seq?: number;
}

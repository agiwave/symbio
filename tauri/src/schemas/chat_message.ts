/**
 * 对话消息的数据契约（与后端 `symbio_core` 的消息形状对齐）
 *
 * ## 为什么词表要有**运行时常量**，而不只是类型联合
 *
 * 类型联合（`type MessageStatus = 'pending' | ...`）只在编译期存在，运行时拿不到
 * 名字。于是消费方只能各写各的字面量——`'tool_call'` / `'turn'` / `'failed'`
 * 散落在组件、store、服务层，改一处漏一处。
 *
 * 这里为每个取值导出常量，**并让类型由常量数组派生**：
 * 加一个取值 = 加一行数组元素，类型与常量同时到位，不存在「类型改了常量没改」。
 * 呈现映射（图标 / 标题 / 标签）在 `registry/messageTypes.ts`；本文件零呈现知识。
 */

// ==================== 角色 ====================

/** 用户发言 */
export const CHAT_ROLE_USER = 'user'
/** 助手（模型）发言 */
export const CHAT_ROLE_ASSISTANT = 'assistant'
/** 工具返回（含子智能体响应的 Turn） */
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

// ==================== 消息类型 ====================

/** 正文文本 */
export const MESSAGE_TYPE_TEXT = 'text'
/** 思考过程（reasoning） */
export const MESSAGE_TYPE_REASONING = 'reasoning'
/** 工具调用（组合节点：请求 / 过程 / 结果） */
export const MESSAGE_TYPE_TOOL_CALL = 'tool_call'
/** 轮次分组（一轮响应的容器，根级助手回合与子会话回合共用） */
export const MESSAGE_TYPE_TURN = 'turn'
/** 待用户响应（ask_user 提问 / 工具确认） */
export const MESSAGE_TYPE_USER_PROMPT = 'user_prompt'
/** 上下文压缩（系统对历史的一次整理动作，不是对话内容） */
export const MESSAGE_TYPE_COMPRESSION = 'compression'

export const MESSAGE_TYPES = [
  MESSAGE_TYPE_TEXT,
  MESSAGE_TYPE_REASONING,
  MESSAGE_TYPE_TOOL_CALL,
  MESSAGE_TYPE_TURN,
  MESSAGE_TYPE_USER_PROMPT,
  MESSAGE_TYPE_COMPRESSION,
] as const

export type ChatMessageType = (typeof MESSAGE_TYPES)[number]

// ==================== 消息状态 ====================

/** 未开始 */
export const MESSAGE_STATUS_PENDING = 'pending'
/** 正在产生内容 */
export const MESSAGE_STATUS_STREAMING = 'streaming'
/** 等待用户响应（审批 / 提问） */
export const MESSAGE_STATUS_WAITING_USER_ACTION = 'waiting_user_action'
/** 终态：正常结束 */
export const MESSAGE_STATUS_COMPLETED = 'completed'
/** 终态：用户主动终止（没出错，但也没跑完——与 `failed` 一样可重试） */
export const MESSAGE_STATUS_ABORTED = 'aborted'
/** 终态：以错误结束 */
export const MESSAGE_STATUS_FAILED = 'failed'
/**
 * 终态：节点已被删除（**就地移除，不可重试**）。
 *
 * 协议里没有 `remove` 操作——删除就是一次状态迁移，与出现、增长、完成同走一条
 * 消息帧。落到本状态的帧在接收端**就地移除**该节点（从本地视图消失），不产生
 * 任何「清空重读」。根因见后端 `symbio_core::schemas::session::chat_message::MessageStatus::Removed`。
 */
export const MESSAGE_STATUS_REMOVED = 'removed'

export const MESSAGE_STATUSES = [
  MESSAGE_STATUS_PENDING,
  MESSAGE_STATUS_STREAMING,
  MESSAGE_STATUS_WAITING_USER_ACTION,
  MESSAGE_STATUS_COMPLETED,
  MESSAGE_STATUS_ABORTED,
  MESSAGE_STATUS_FAILED,
  MESSAGE_STATUS_REMOVED,
] as const

export type MessageStatus = (typeof MESSAGE_STATUSES)[number]

/**
 * 是否是**已知的消息状态词**（集合判定，由 `MESSAGE_STATUSES` 派生）。
 *
 * 存在的理由：消费方要「把后端下发的状态词原样透传、只拦未知值」，而逐个
 * `case` 枚举等于又抄一份词表——**词表加了取值而 `case` 忘了加**，那条变更就
 * 会被当成"未知状态"丢掉。实测代价见消息节点的状态透传链路（当时叫
 * `vdfsTranscriptSync`）：`aborted` 曾因此让中止后的 Turn 显示为已完成，
 * 重试入口不出现。
 *
 * 判定与词表同源，故**新增一个状态词不需要改这里**。
 */
export function isMessageStatus(value: unknown): value is MessageStatus {
  return typeof value === 'string' && (MESSAGE_STATUSES as readonly string[]).includes(value)
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
export function isInflightMessageStatus(status?: MessageStatus | null): boolean {
  return (
    status === undefined ||
    status === null ||
    status === MESSAGE_STATUS_PENDING ||
    status === MESSAGE_STATUS_STREAMING
  )
}

/**
 * 需要向用户交代的**非正常终态**：失败或用户中止。
 *
 * 两者都要渲染交代条并给重试入口——中止不是故障，但这一轮只跑了一半，
 * 用户通常就想重跑它。放在契约层是因为它同时被「渲染」与「业务规则」消费。
 */
export function isUnsettledMessageStatus(status?: MessageStatus | null): boolean {
  return status === MESSAGE_STATUS_FAILED || status === MESSAGE_STATUS_ABORTED
}

// ==================== 恢复动作 ====================

/**
 * 会话恢复动作（后端 `ResumeAction` 的**线格式词**）。
 *
 * ## 为什么它也需要一张词表
 *
 * 后端 `chat_message.rs` 的单测 `resume_action_wire_words_are_snake_case` 注释里
 * 已经点名：「前端 `ResumePayload.action` 的字面量必须与它逐字相等」。契约写明了，
 * 前端这边却只有两处**手写的联合类型**——`useChatConnection` 的载荷定义与
 * `messageTypes` 的重试分派——谁也不与后端对齐，改一个词就得靠人记得改两处。
 *
 * 处置与上面三张词表相同：常量在此、类型由常量数组派生，跨栈一致性交给
 * `scripts/protocol-mirror-audit.mjs` 的 C 组（后端枚举取值 ↔ 本词表）。
 */

/** 整轮重试：删除 Failed Turn 及其子树 → 重新走 LLM 请求 */
export const RESUME_ACTION_RETRY_TURN = 'retry_turn'
/** **单工具**重试：删除该工具的失败结果 → 用原参数重新执行 */
export const RESUME_ACTION_RETRY = 'retry'
/** 工具审批通过：删除 user_prompt → 带 approved=true 重新执行 */
export const RESUME_ACTION_APPROVE = 'approve'
/** 工具审批拒绝：删除 user_prompt → 生成拒绝结果 */
export const RESUME_ACTION_REJECT = 'reject'
/** 补充参数后重跑工具（与原 args 浅合并） */
export const RESUME_ACTION_SUPPLY = 'supply'
/** 回答 `ask_user` 提问 */
export const RESUME_ACTION_ANSWER = 'answer'
/** 重跑**这一次压缩**：删除失败压缩节点 → 重新压缩（不动历史） */
export const RESUME_ACTION_RETRY_COMPACTION = 'retry_compaction'

export const RESUME_ACTIONS = [
  RESUME_ACTION_RETRY_TURN,
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
 * 会话消息（后端 `symbio_core::schemas::session::chat_message::ChatMessage` 的镜像）。
 *
 * **本接口没有 `agent_id` / `prompt` 两个顶层字段**——它们曾在此声明，但后端
 * `ChatMessage` 从不下发：`agent_id` 在 `meta` 里（见 `registry/messageTypes.agentNameOf`），
 * `prompt` 在后端是 `#[serde(skip_serializing)]`（内容在 `meta.prompt`）。
 * 留着它们只会让人以为能从顶层读到。字段与后端的对应由
 * `scripts/protocol-mirror-audit.mjs` 的 D 组守着。
 */
export interface ChatMessage {
  id: string;
  parent_id?: string;
  /** 树形展开：前端按 `parent_id` 组装的父引用（后端只给扁平列表） */
  parent?: ChatMessage;
  role?: ChatRole;
  type?: ChatMessageType;
  name?: string;
  /**
   * ToolCall 节点的 **wire id**（LLM provider 返回的 `tool_call_id`）。
   *
   * 节点 `id` 在诞生时分配（会话内唯一），provider 的原始 id 存这里，仅在构建
   * LLM 请求时回传（部分协议要求原值）。历史数据可能无此字段。
   */
  tool_call_id?: string;
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
  status?: MessageStatus;
  /** 失败原因（面向用户的可读短消息），仅当 status === 'failed' 时存在 */
  error?: string;
  meta?: Record<string, any>;
  timestamp?: number;
  /** 树形展开：前端按 `parent_id` 组装的子列表（后端只给扁平列表） */
  children?: ChatMessage[];
  /**
   * 会话内消息的**唯一权威顺序锚点**：后端写入时分配单调自增序号，
   * 父节点序号 < 子节点，按 turn 追加顺序递增；前端对实时流式/乐观消息
   * 也用同一字段的单调计数器续接。前端排序只用 `seq`，不再依赖 timestamp
   * （timestamp 是"业务时刻"而非"顺序"，旧数据还可能缺失，用作排序键会错乱）。
   */
  seq?: number;
}

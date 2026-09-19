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

export const MESSAGE_STATUSES = [
  MESSAGE_STATUS_PENDING,
  MESSAGE_STATUS_STREAMING,
  MESSAGE_STATUS_WAITING_USER_ACTION,
  MESSAGE_STATUS_COMPLETED,
  MESSAGE_STATUS_ABORTED,
  MESSAGE_STATUS_FAILED,
] as const

export type MessageStatus = (typeof MESSAGE_STATUSES)[number]

/**
 * 是否是**已知的消息状态词**（集合判定，由 `MESSAGE_STATUSES` 派生）。
 *
 * 存在的理由：消费方要「把后端下发的状态词原样透传、只拦未知值」，而逐个
 * `case` 枚举等于又抄一份词表——**词表加了取值而 `case` 忘了加**，那条变更就
 * 会被当成"未知状态"丢掉。实测代价见 `services/vdfsTranscriptSync`：
 * `aborted` 曾因此让中止后的 Turn 显示为已完成，重试入口不出现。
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

export interface ChatMessage {
  id: string;
  parent_id?: string;
  parent?: ChatMessage;
  role?: ChatRole;
  type?: ChatMessageType;
  agent_id?: string;
  name?: string;
  content?: MessageContent;
  status?: MessageStatus;
  /** 失败原因（面向用户的可读短消息），仅当 status === 'failed' 时存在 */
  error?: string;
  meta?: Record<string, any>;
  timestamp?: number;
  prompt?: string;
  children?: ChatMessage[];
  /**
   * 会话内消息的**唯一权威顺序锚点**：后端写入时分配单调自增序号，
   * 父节点序号 < 子节点，按 turn 追加顺序递增；前端对实时流式/乐观消息
   * 也用同一字段的单调计数器续接。前端排序只用 `seq`，不再依赖 timestamp
   * （timestamp 是"业务时刻"而非"顺序"，旧数据还可能缺失，用作排序键会错乱）。
   */
  seq?: number;
}

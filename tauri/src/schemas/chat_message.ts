export type ChatRole = 'user' | 'assistant' | 'tool' | 'system';
export type ChatMessageType = 'text' | 'reasoning' | 'tool_call' | 'turn' | 'user_prompt';
export type MessageStatus = 'pending' | 'streaming' | 'waiting_user_action' | 'completed' | 'failed';

export interface ImageUrl {
  url: string;
  detail?: string;
}

export type ContentPart =
  | { type: 'text'; text: string }
  | { type: 'image_url'; image_url: ImageUrl };

export type MessageContent = string | ContentPart[];

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

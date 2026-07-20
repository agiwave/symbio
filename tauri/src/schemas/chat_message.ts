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
  sort_index?: number;
}

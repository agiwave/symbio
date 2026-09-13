/**
 * Model \u5bf9\u8bdd服务
 *
 * - 使用连接模式与后端通信
 * - 支持请求中止
 * - 支持断线重连
 * - 支持多模态消息（文本+图片）
 */

import { ChatMessage, ContentPart, MessageContent, ChatRole, ChatMessageType, MessageStatus } from '../schemas/chat_message'


// ==================== Chat 事件协议（原 schemas/chat_response，仅本模块使用，内联） ====================

export enum ChatEventType {
  Update = 'update',
  Error = 'error',
  Connected = 'connected',
  Disconnected = 'disconnected',
  Status = 'status',
  Abort = 'abort',
  SessionResumed = 'session_resumed',
  Delete = 'delete'
}

export type StreamEvent =
  | { type: ChatEventType.Update; message: ChatMessage }
  | { type: ChatEventType.Error; error: string }
  | { type: ChatEventType.Connected; session_id: string; is_working: boolean; messages: ChatMessage[] }
  | { type: ChatEventType.Disconnected }
  | { type: ChatEventType.Status; status: string }
  | { type: ChatEventType.Abort }
  | { type: ChatEventType.SessionResumed; session_id: string; parent_session_id: string; failed: boolean; result: string | null }
  | { type: ChatEventType.Delete; message_id: string };

export type { ChatMessage, ContentPart, MessageContent, ChatMessageType, MessageStatus, ChatRole }

// ==================== 类型定义 ====================

export interface ChatResponse {
  content?: string
  error?: string
}

/**
 * 连接事件
 */
export interface ChatEvent {
  type: ChatEventType
  request_id?: number
  message?: ChatMessage
  patch?: Partial<ChatMessage>
  error?: string
  connection_id?: string
  session_id?: string
  done?: boolean
  is_working?: boolean
  [key: string]: any
}

/**
 * Session 工作状态
 */
export interface ChatStatus {
  session_id: string
  is_working: boolean
  is_waiting_approval: boolean
}

/**
 * AI 提供商配置
 */
export interface ProviderConfig {
  provider: string
  api_base: string
  api_key: string
  model: string
  temperature?: number
  max_tokens?: number
  api_protocol?: string
}

/**
 * ChatConnection 接口定义
 */
export interface ChatConnection {
  connectionId: string
  send: (message: ChatMessage, agentId: string) => void
  abort: () => void
  close: () => void
  isConnected: () => boolean
}

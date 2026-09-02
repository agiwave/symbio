/**
 * Model \u5bf9\u8bdd服务
 *
 * - 使用连接模式与后端通信
 * - 支持请求中止
 * - 支持断线重连
 * - 支持多模态消息（文本+图片）
 */

import {
  callPlugin,
} from './plugin'
import { logger } from '@/utils/logger'

import { ChatMessage, ContentPart, MessageContent, ChatRole, ChatMessageType, MessageStatus } from '../schemas/chat_message'

import { StreamEvent, ChatEventType } from '../schemas/chat_response'

// Chat 插件路径 (V2 整合版：chat 逻辑已并入 session，使用 constants/pluginPaths 中的统一常量)
const MODEL_PATH = 'worker/model'

export type { ChatMessage, ContentPart, MessageContent, StreamEvent, ChatMessageType, MessageStatus, ChatRole }
export { ChatEventType }

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

/**
 * 获取可用的模型列表
 */
export async function listModels(): Promise<string[]> {
  try {
    const response = await callPlugin<any>(`${MODEL_PATH}/list_models`, {})
    return response.models || []
  } catch (err) {
    logger.error('model-service', 'Failed to list models:', err)
    return []
  }
}

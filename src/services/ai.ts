/**
 * AI 对话服务
 *
 * 调用后端 chat 插件进行 AI 对话
 */

import { callPlugin } from './plugin'

// Chat 插件在 agent 下，路径为 agent/chat
const CHAT_PATH = 'agent/chat'

export interface ChatMessage {
  role: 'user' | 'assistant' | 'system'
  content: string
}

export interface ProviderConfig {
  name?: string
  api_base: string
  api_key: string
  model: string
  temperature?: number
  max_tokens?: number
}

export interface ChatResponse {
  content?: string
  error?: string
}

export interface ConfigResponse {
  name: string
  api_base: string
  model: string
  temperature?: number
  max_tokens?: number
  has_api_key: boolean
  message?: string
}

/**
 * 发送消息到 AI
 */
export async function sendMessage(messages: ChatMessage[]): Promise<ChatResponse> {
  return callPlugin<ChatResponse>(CHAT_PATH, {
    action: 'send',
    messages
  })
}

/**
 * 配置 AI 提供商
 */
export async function configureProvider(config: ProviderConfig): Promise<ConfigResponse> {
  return callPlugin<ConfigResponse>(CHAT_PATH, {
    action: 'configure',
    config
  })
}

/**
 * 获取当前配置
 */
export async function getProviderConfig(): Promise<ConfigResponse> {
  return callPlugin<ConfigResponse>(CHAT_PATH, {
    action: 'get_config'
  })
}

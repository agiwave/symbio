/**
 * AI 对话服务
 *
 * - 发送消息：通过 agent/chat 调用
 * - 配置管理：直接调用 agent/@llm (openai 插件)
 */

import { callPlugin } from './plugin'

// Chat 插件路径
const CHAT_PATH = 'agent/chat'
// LLM 能力路由路径（直接操作 openai 插件）
const LLM_PATH = 'agent/@llm'

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
  success?: boolean
  config?: {
    api_base: string
    api_key_set: boolean
    model: string
    temperature?: number
    max_tokens?: number
    max_context_tokens?: number
  }
  error?: string
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
 * 配置 AI 提供商（直接操作 openai 插件）
 */
export async function configureProvider(config: ProviderConfig): Promise<{ message?: string; error?: string }> {
  return callPlugin<{ message?: string; error?: string }>(LLM_PATH, {
    action: 'configure',
    api_base: config.api_base,
    api_key: config.api_key,
    model: config.model,
    temperature: config.temperature,
    max_tokens: config.max_tokens,
  })
}

/**
 * 获取当前配置（直接从 openai 插件获取）
 */
export async function getProviderConfig(): Promise<ConfigResponse> {
  const result = await callPlugin<ConfigResponse>(LLM_PATH, {
    action: 'get_config'
  })
  return result
}
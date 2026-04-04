/**
 * AI 对话服务
 *
 * - 发送消息：通过 agent/chat 调用
 * - 配置管理：直接调用 agent/@llm (openai 插件)
 */

import { callPlugin, streamPlugin, type StreamChunk } from './plugin'

// Chat 插件路径
const CHAT_PATH = 'agent/chat'
// LLM 能力路由路径（直接操作 openai 插件）
const LLM_PATH = 'agent/@llm'

export interface ChatMessage {
  role: 'user' | 'assistant' | 'system'
  content: string
}

export interface ChatRequest {
  messages: ChatMessage[]
  session_id?: string
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
 * 发送消息到 AI（同步，等待完整结果）
 */
export async function sendMessage(messages: ChatMessage[], sessionId?: string): Promise<ChatResponse> {
  return callPlugin<ChatResponse>(CHAT_PATH, {
    messages,
    session_id: sessionId || 'default'
  })
}

/**
 * 流式发送消息到 AI（实时返回中间过程）
 *
 * @param messages 消息列表
 * @param sessionId 会话 ID
 * @param onChunk 每个 chunk 的回调
 * @returns Promise<ChatResponse> 最终结果
 */
export async function sendMessageStream(
  messages: ChatMessage[],
  sessionId: string,
  onChunk: (chunk: StreamChunk) => void
): Promise<ChatResponse> {
  let finalContent = ''
  let finalError: string | undefined

  const request: Record<string, unknown> = {
    action: 'send',
    messages,
    session_id: sessionId || 'default'
  }

  await streamPlugin(CHAT_PATH, request, (chunk) => {
    // 调用回调
    onChunk(chunk)

    // 累积内容
    if (chunk.data && typeof chunk.data === 'object') {
      const data = chunk.data as Record<string, unknown>
      if (data.content && typeof data.content === 'string') {
        finalContent = data.content as string
      }
      if (data.error && typeof data.error === 'string') {
        console.log('[ai] caught error in chunk.data:', data.error)
        finalError = data.error as string
      }
    }

    // 检查顶层错误
    if (chunk.error && typeof chunk.error === 'string') {
      console.log('[ai] caught error in chunk.error:', chunk.error)
      finalError = chunk.error
    }
  })

  if (finalError) {
    return { error: finalError }
  }

  return { content: finalContent || undefined }
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

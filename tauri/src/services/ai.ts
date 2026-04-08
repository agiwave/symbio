/**
 * AI 对话服务
 *
 * - 使用连接模式与后端通信
 * - 支持请求中止
 * - 支持断线重连
 */

import {
  connectPlugin,
  sendToConnection,
  closeConnection,
  type Connection,
  type ConnectEvent
} from './plugin'

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
 * 连接事件类型
 */
export type ChatEventType =
  | 'connected'      // 连接建立成功（包含工作状态）
  | 'request_start'  // 请求开始
  | 'chunk'          // 流式数据块
  | 'request_complete' // 请求完成
  | 'aborted'        // 请求被中止
  | 'error'          // 错误
  | 'disconnected'   // 连接断开
  | 'status'         // 状态查询响应

/**
 * 连接事件
 */
export interface ChatEvent {
  type: ChatEventType
  request_id?: number
  data?: unknown
  content?: string
  error?: string
  message?: string
  connection_id?: string
  session_id?: string
  done?: boolean
  // 工作状态
  is_working?: boolean
  current_content?: string
  reason?: string  // 中止原因
}

/**
 * Session 工作状态
 */
export interface SessionStatus {
  is_working: boolean
  current_content: string
  request_id: number
}

/**
 * 聊天连接控制器
 */
export interface ChatConnection {
  /** 连接 ID */
  connectionId: string
  /** 发送消息 */
  send: (messages: ChatMessage[], sessionId: string) => void
  /** 中止当前请求 */
  abort: () => void
  /** 关闭连接 */
  close: () => void
  /** 是否已连接 */
  isConnected: () => boolean
}

/**
 * 连接事件回调
 */
export type ChatEventCallback = (event: ChatEvent) => void

/**
 * 建立聊天连接
 *
 * @param sessionId 会话 ID
 * @param onEvent 事件回调
 * @returns 聊天连接控制器
 */
export async function createChatConnection(
  sessionId: string,
  onEvent: ChatEventCallback
): Promise<ChatConnection> {
  // 建立连接
  const conn = await connectPlugin(CHAT_PATH, { session_id: sessionId }, (event) => {
    // 处理 event 可能是 JSON 字符串的情况
    let evt = event
    if (typeof event === 'string') {
      try {
        evt = JSON.parse(event)
      } catch (e) {
        console.error('[ai] Failed to parse event:', e)
        return
      }
    }
    // evt 结构: { type: "message", data: { type: "connected", session_id: ... } }
    // 实际事件数据在 evt.data 中
    const data = (evt as ConnectEvent).data as Record<string, unknown>
    const chatEvent: ChatEvent = {
      type: (data?.type as ChatEventType) || ((evt as ConnectEvent).type as ChatEventType),
      ...data,
    }
    onEvent(chatEvent)
  })

  return {
    connectionId: conn.connectionId,

    send: (messages: ChatMessage[], sid: string) => {
      sendToConnection(conn.connectionId, {
        type: 'send',
        messages,
        session_id: sid,
      }).catch(err => {
        onEvent({ type: 'error', error: String(err) })
      })
    },

    abort: () => {
      sendToConnection(conn.connectionId, { type: 'abort' }).catch(err => {
        onEvent({ type: 'error', error: String(err) })
      })
    },

    close: () => {
      closeConnection(conn.connectionId).catch(err => {
        console.error('[ai] Failed to close connection:', err)
      })
    },

    isConnected: () => {
      // 简单检查，实际状态由后端管理
      return true
    },
  }
}

/**
 * 发送消息到 AI（同步，等待完整结果）
 * 保留向后兼容，内部使用连接模式
 */
export async function sendMessage(messages: ChatMessage[], sessionId?: string): Promise<ChatResponse> {
  return new Promise((resolve, reject) => {
    let connection: ChatConnection | null = null
    let content = ''
    let error: string | undefined

    createChatConnection(sessionId || 'default', (event) => {
      switch (event.type) {
        case 'chunk':
          if (event.data && typeof event.data === 'object') {
            const data = event.data as Record<string, unknown>
            if (data.content && typeof data.content === 'string') {
              content = data.content
            }
          }
          break
        case 'request_complete':
          connection?.close()
          resolve({ content: event.content || content })
          break
        case 'error':
          connection?.close()
          resolve({ error: event.error })
          break
        case 'aborted':
          connection?.close()
          resolve({ error: '请求已中止' })
          break
      }
    }).then(conn => {
      connection = conn
      conn.send(messages, sessionId || 'default')
    }).catch(err => {
      reject(err)
    })
  })
}

/**
 * 流式发送消息到 AI（旧接口，保持向后兼容）
 */
export async function sendMessageStream(
  messages: ChatMessage[],
  sessionId: string,
  onChunk: (chunk: { data: unknown; done: boolean; error?: string }) => void
): Promise<ChatResponse> {
  return new Promise((resolve, reject) => {
    let connection: ChatConnection | null = null
    let content = ''

    createChatConnection(sessionId, (event) => {
      console.log('[ai] sendMessageStream event:', JSON.stringify(event))
      
      switch (event.type) {
        case 'chunk':
          // 内容可能在 event.data.content 或 event.content 中
          if (event.data && typeof event.data === 'object') {
            const data = event.data as Record<string, unknown>
            if (data.content && typeof data.content === 'string') {
              content = data.content
              console.log('[ai] chunk content from data.content:', content.slice(0, 50))
            }
          }
          // 也检查 event.content
          if (event.content && typeof event.content === 'string' && !content) {
            content = event.content
            console.log('[ai] chunk content from event.content:', content.slice(0, 50))
          }
          onChunk({
            data: event.data,
            done: event.done || false,
          })
          break
        case 'request_complete':
          connection?.close()
          console.log('[ai] request_complete - event.content:', event.content, 'accumulated:', content)
          resolve({ content: event.content || content })
          break
        case 'error':
          connection?.close()
          onChunk({ data: {}, done: true, error: event.error })
          resolve({ error: event.error })
          break
        case 'aborted':
          connection?.close()
          resolve({ error: '请求已中止' })
          break
      }
    }).then(conn => {
      connection = conn
      conn.send(messages, sessionId)
    }).catch(err => {
      reject(err)
    })
  })
}

/**
 * 配置 AI 提供商（直接操作 openai 插件）
 */
export async function configureProvider(config: ProviderConfig): Promise<{ message?: string; error?: string }> {
  const { callPlugin } = await import('./plugin')
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
  const { callPlugin } = await import('./plugin')
  const result = await callPlugin<ConfigResponse>(LLM_PATH, {
    action: 'get_config'
  })
  return result
}

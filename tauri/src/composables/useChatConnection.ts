/**
 * 统一的聊天连接 Composable
 *
 * 简化版：只负责连接和发送/接收消息
 * 历史消息加载由父组件控制
 */

import { ref, onMounted, onUnmounted, watch, computed, type Ref, type ComputedRef } from 'vue'
import {
  createChatConnection,
  type ChatMessage,
  type ChatConnection,
  type ChatEvent
} from '@/services/ai'
import type { SessionMessage } from '@/services/session'

type MaybeRef<T> = Ref<T> | ComputedRef<T>

export interface UseChatConnectionOptions {
  /** 会话 ID */
  sessionId: string
  /** 消息列表（外部管理） */
  messages: MaybeRef<SessionMessage[]>
  /** 消息更新回调 */
  onUpdateMessages: (messages: SessionMessage[]) => void
  /** 发送完成回调 */
  onSendComplete?: () => void
}

export interface UseChatConnectionReturn {
  /** 是否正在加载 */
  isLoading: ComputedRef<boolean>
  /** 流式内容 */
  streamingContent: ComputedRef<string>
  /** 工具调用 */
  toolCalls: ComputedRef<Array<{ name: string; args: string; result?: string }>>
  /** 错误信息 */
  error: ComputedRef<string | null>
  /** 发送消息 */
  send: (messages: ChatMessage[], sessionId?: string) => void
  /** 中止当前请求 */
  abort: () => void
}

export function useChatConnection(options: UseChatConnectionOptions): UseChatConnectionReturn {
  const { sessionId, messages, onUpdateMessages, onSendComplete } = options

  // 连接状态
  const connection = ref<ChatConnection | null>(null)
  const isLoading = ref(false)
  const streamingContent = ref('')
  const toolCalls = ref<Array<{ name: string; args: string; result?: string }>>([])
  const error = ref<string | null>(null)

  // 处理聊天事件
  function handleChatEvent(event: ChatEvent) {
    console.log('[useChatConnection] event received:', event.type, event)
    switch (event.type) {
      case 'connected':
        error.value = null
        // 恢复工作状态（确保 current_content 是字符串）
        // 只有当后端明确报告 is_working 为 true 且 request_id 不为 0 时才恢复
        if (event.is_working === true && event.request_id && event.request_id !== 0) {
          isLoading.value = true
          const content = event.current_content
          streamingContent.value = typeof content === 'string' ? content : ''
        } else {
          // 确保不工作时状态正确
          isLoading.value = false
          streamingContent.value = ''
        }
        break

      case 'request_start':
        isLoading.value = true
        streamingContent.value = ''
        toolCalls.value = []
        break

      case 'chunk':
        if (event.data && typeof event.data === 'object') {
          const data = event.data as Record<string, unknown>
          if (data.tool_calls && Array.isArray(data.tool_calls)) {
            toolCalls.value = data.tool_calls.map((tc: any) => ({
              name: tc.function?.name || tc.name || 'unknown',
              args: tc.function?.arguments || tc.arguments || '',
              result: tc.result
            }))
          }
          if (data.content && typeof data.content === 'string') {
            streamingContent.value = data.content as string
          }
        }
        break

      case 'request_complete':
        isLoading.value = false
        if (streamingContent.value) {
          onUpdateMessages([...messages.value, {
            role: 'assistant',
            content: streamingContent.value,
            timestamp: Math.floor(Date.now() / 1000)
          }])
        }
        streamingContent.value = ''
        toolCalls.value = []
        onSendComplete?.()
        break

      case 'aborted':
        isLoading.value = false
        const abortedContent = typeof event.content === 'string' ? event.content : streamingContent.value
        if (abortedContent) {
          onUpdateMessages([...messages.value, {
            role: 'assistant',
            content: abortedContent + '\n\n*[已中止]*',
            timestamp: Math.floor(Date.now() / 1000)
          }])
        }
        streamingContent.value = ''
        toolCalls.value = []
        break

      case 'error':
        isLoading.value = false
        const errorMsg = typeof event.error === 'string' 
          ? event.error 
          : (event.error ? JSON.stringify(event.error) : '未知错误')
        error.value = errorMsg
        break
    }
  }

  // 建立连接
  async function connect(sid: string) {
    console.log('[useChatConnection] connect called, sid:', sid)
    // 关闭旧连接
    if (connection.value) {
      connection.value.close()
      connection.value = null
    }

    // 重置状态（在建立新连接前重置）
    isLoading.value = false
    streamingContent.value = ''
    toolCalls.value = []
    error.value = null

    try {
      connection.value = await createChatConnection(sid, handleChatEvent)
      console.log('[useChatConnection] connection created, id:', connection.value.connectionId)
    } catch (err) {
      console.error('[useChatConnection] Failed to connect:', err)
      error.value = `连接失败: ${err}`
    }
  }

  // 发送消息
  function send(chatMessages: ChatMessage[], sid?: string) {
    console.log('[useChatConnection] send called, sid:', sid, 'connection:', connection.value?.connectionId)
    if (!connection.value) {
      console.error('[useChatConnection] No connection')
      return
    }
    error.value = null
    isLoading.value = true
    streamingContent.value = ''
    toolCalls.value = []
    const targetSid = sid || sessionId
    console.log('[useChatConnection] sending with sessionId:', targetSid)
    connection.value.send(chatMessages, targetSid)
  }

  // 中止当前请求
  function abort() {
    if (connection.value) {
      connection.value.abort()
    }
  }

  // 生命周期
  onMounted(() => {
    connect(sessionId)
  })

  onUnmounted(() => {
    if (connection.value) {
      connection.value.close()
    }
  })

  // 监听 sessionId 变化
  watch(() => sessionId, (newId) => {
    if (newId) {
      // 切换会话时重置状态
      isLoading.value = false
      streamingContent.value = ''
      toolCalls.value = []
      error.value = null
      connect(newId)
    }
  })

  return {
    isLoading: computed(() => isLoading.value),
    streamingContent: computed(() => streamingContent.value),
    toolCalls: computed(() => toolCalls.value),
    error: computed(() => error.value),
    send,
    abort,
  }
}
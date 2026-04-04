/**
 * 统一的 AI 聊天 Composable
 *
 * 所有文件视图（Explorer、NotePage、MarkdownEditor 等）都使用这个统一的逻辑
 * 来调用 AI 助手，确保选区上下文能正确传递到 LLM。
 *
 * 自动使用全局 AI 上下文（由 useAIContext 管理）。
 */

import { ref } from 'vue'
import { sendMessageStream, type ChatMessage } from '@/services/ai'
import { buildContextualMessage, useAIContext } from '@/composables/useAIContext'

export interface UseAIChatOptions {
  /** 会话 ID */
  sessionId: string
}

export function useAIChat(options: UseAIChatOptions) {
  const { sessionId } = options

  const messages = ref<ChatMessage[]>([])
  const loading = ref(false)
  const streamingContent = ref('')

  // 使用全局 AI 上下文
  const { context } = useAIContext()

  /**
   * 发送消息到 AI
   *
   * @param userInput 用户输入的问题
   * @param onChunk 流式 chunk 回调
   */
  async function sendMessage(
    userInput: string,
    onChunk?: (chunk: any) => void
  ) {
    if (!userInput.trim() || loading.value) return

    // 使用全局上下文构建消息
    const ctx = context.value
    const finalUserInput = buildContextualMessage(userInput, ctx)

    // 添加用户消息
    messages.value.push({ role: 'user', content: finalUserInput })
    loading.value = true
    streamingContent.value = ''

    try {
      // 流式发送消息（不传递 selectionContext，所有上下文已包含在消息内容中）
      const response = await sendMessageStream(
        messages.value,
        sessionId,
        (chunk) => {
          if (chunk.data && typeof chunk.data === 'object') {
            const data = chunk.data as Record<string, unknown>
            if (data.content && typeof data.content === 'string') {
              streamingContent.value = data.content as string
            }
          }
          onChunk?.(chunk)
        }
      )

      // 流完成 - 添加助手消息
      if (response.error) {
        messages.value.push({
          role: 'assistant',
          content: `错误: ${response.error}`
        })
      } else if (streamingContent.value) {
        messages.value.push({
          role: 'assistant',
          content: streamingContent.value
        })
      } else {
        messages.value.push({
          role: 'assistant',
          content: '抱歉，无法处理请求。'
        })
      }

      return { success: true }
    } catch (error) {
      messages.value.push({
        role: 'assistant',
        content: `错误: ${error}`
      })
      return { success: false, error }
    } finally {
      loading.value = false
      streamingContent.value = ''
    }
  }

  /**
   * 清空消息历史
   */
  function clearMessages() {
    messages.value = []
    streamingContent.value = ''
  }

  return {
    messages,
    loading,
    streamingContent,
    sendMessage,
    clearMessages,
  }
}

export type AIChatReturn = ReturnType<typeof useAIChat>

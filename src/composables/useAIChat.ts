/**
 * 统一的 AI 聊天 Composable
 *
 * 所有文件视图（Explorer、NotePage、MarkdownEditor 等）都使用这个统一的逻辑
 * 来调用 AI 助手，确保选区上下文能正确传递到 LLM。
 */

import { ref, type Ref } from 'vue'
import { sendMessageStream, type ChatMessage } from '@/services/ai'

export interface AIChatContext {
  /** 文件路径 */
  filePath?: string
  /** 完整文件内容 */
  fileContent?: string
  /** 用户选中的文本 */
  selectedText?: string
  /** 选中内容的起始行号（1-based） */
  startLine?: number
  /** 选中内容的结束行号（1-based） */
  endLine?: number
}

/** Context provider function type - called at sendMessage time to get latest context */
export type AIChatContextProvider = () => AIChatContext | undefined

export interface UseAIChatOptions {
  /** 会话 ID */
  sessionId: string
  /** 上下文提供者函数（推荐），或者静态上下文对象 */
  contextProvider?: AIChatContextProvider
  /** @deprecated 使用 contextProvider 代替 */
  context?: AIChatContext
}

export function useAIChat(options: UseAIChatOptions) {
  const { sessionId, contextProvider, context } = options

  const messages = ref<ChatMessage[]>([])
  const loading = ref(false)
  const streamingContent = ref('')

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

    // 获取最新上下文（优先使用 contextProvider）
    const ctx = contextProvider ? contextProvider() : context

    // 构建带上下文的用户消息
    let finalUserInput = userInput
    if (ctx?.filePath || ctx?.selectedText) {
      const contextParts: string[] = []

      // 添加文件信息
      if (ctx.filePath) {
        const lineInfo = ctx.startLine
          ? ` (行 ${ctx.startLine}${ctx.endLine && ctx.endLine !== ctx.startLine ? '-' + ctx.endLine : ''})`
          : ''
        contextParts.push(`📄 文件: ${ctx.filePath}${lineInfo}`)
      }

      // 添加选中的内容
      if (ctx.selectedText) {
        contextParts.push(`\n**选中的内容：**\n\`\`\`\n${ctx.selectedText}\n\`\`\``)
      }

      // 添加完整文件内容（如果有）
      if (ctx.fileContent) {
        contextParts.push(`\n**完整文件内容：**\n\`\`\`\n${ctx.fileContent}\n\`\`\``)
      }

      if (contextParts.length > 0) {
        finalUserInput = `[上下文信息]\n${contextParts.join('\n')}\n\n---\n\n**问题：** ${userInput}`
      }
    }

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

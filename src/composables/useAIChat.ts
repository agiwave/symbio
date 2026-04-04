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

    // 添加用户消息
    messages.value.push({ role: 'user', content: userInput })
    loading.value = true
    streamingContent.value = ''

    try {
      // 获取最新上下文（优先使用 contextProvider）
      const ctx = contextProvider ? contextProvider() : context

      // 构建选区上下文（如果有）
      let selectionContext = undefined
      if (ctx?.filePath || ctx?.selectedText) {
        selectionContext = {
          file_path: ctx.filePath,
          file_content: ctx.fileContent,
          selected_text: ctx.selectedText,
          start_line: ctx.startLine,
          end_line: ctx.endLine,
        }
        console.log('[useAIChat] 发送选区上下文:', {
          file_path: selectionContext.file_path,
          has_file_content: !!selectionContext.file_content,
          file_content_length: selectionContext.file_content?.length || 0,
          selected_text_length: selectionContext.selected_text?.length || 0,
          start_line: selectionContext.start_line,
          end_line: selectionContext.end_line,
        })
      }

      // 流式发送消息
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
        },
        selectionContext
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

/**
 * 全局 AI 上下文 Composable
 *
 * 在 Explorer 中提供共享的 AI 上下文状态，包括：
 * - 当前文件路径
 * - 当前文件完整内容
 * - 当前选中的文本
 * - 选中内容的行号范围
 *
 * 所有组件（MarkdownEditor、AISelectionDialog、AIChatPanel）
 * 都使用这个共享上下文，确保 AI 助手始终获得最新的文件/选区信息。
 */

import { ref, computed } from 'vue'

export interface AIContext {
  /** 当前文件路径 */
  filePath?: string
  /** 当前文件完整内容 */
  fileContent?: string
  /** 用户选中的文本 */
  selectedText?: string
  /** 选中内容的起始行号（1-based） */
  startLine?: number
  /** 选中内容的结束行号（1-based） */
  endLine?: number
}

// 模块级单例状态
const globalContext = ref<AIContext>({
  filePath: undefined,
  fileContent: undefined,
  selectedText: undefined,
  startLine: undefined,
  endLine: undefined,
})

// 版本号，用于触发响应式更新
const version = ref(0)

/**
 * 更新全局 AI 上下文
 */
export function setAIContext(ctx: Partial<AIContext>) {
  globalContext.value = {
    ...globalContext.value,
    ...ctx,
  }
  version.value++
}

/**
 * 重置全局 AI 上下文
 */
export function resetAIContext() {
  globalContext.value = {
    filePath: undefined,
    fileContent: undefined,
    selectedText: undefined,
    startLine: undefined,
    endLine: undefined,
  }
  version.value++
}

/**
 * 使用全局 AI 上下文
 *
 * 返回响应式的上下文对象和版本号，当上下文更新时自动触发重新渲染。
 */
export function useAIContext() {
  // 每次调用都创建一个新的响应式引用，确保组件能跟踪变化
  const context = computed(() => ({
    ...globalContext.value,
    _version: version.value,
  }))

  return {
    context,
    version,
  }
}

/**
 * 构建格式化的用户消息（带上下文）
 *
 * 将文件路径、行号、选中内容等信息拼接到用户消息中，
 * 符合行业惯例（如 Cursor、GitHub Copilot 的实现方式）。
 */
export function buildContextualMessage(
  userInput: string,
  ctx?: AIContext
): string {
  if (!ctx || (!ctx.filePath && !ctx.selectedText)) {
    return userInput
  }

  const contextParts: string[] = []

  // 添加文件信息
  if (ctx.filePath) {
    // 只取文件名
    const fileName = ctx.filePath.split(/[\\/]/).pop() || ctx.filePath
    const lineInfo = ctx.startLine
      ? ` (行 ${ctx.startLine}${ctx.endLine && ctx.endLine !== ctx.startLine ? '-' + ctx.endLine : ''})`
      : ''
    contextParts.push(`📄 文件: ${fileName}${lineInfo}`)
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
    return `[上下文信息]\n${contextParts.join('\n')}\n\n---\n\n**问题：** ${userInput}`
  }

  return userInput
}

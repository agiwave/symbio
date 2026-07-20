/**
 * 全局 AI 上下文 Composable
 *
 * 在 Explorer 中提供共享的 AI 上下文状态，包括：
 * - 当前文件路径
 * - 当前文件完整内容
 * - 当前选中的文本
 * - 选中内容的行号范围
 *
 * 所有组件（MarkdownEditor、ModelSelectionDialog、ModelChatPanel）
 * 都使用这个共享上下文，确保 Model \u52a9\u624b始终获得最新的文件/选区信息。
 */

import { ref, computed } from 'vue'
import type { ImageAttachment } from '@/types'

export interface ModelContext {
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
  /** 附加的图片列表 */
  images?: ImageAttachment[]
  /**
   * 此上下文所属的会话 id。
   * 用于 ChatContextBar 检测"跨会话"风险：
   * 用户在 A 会话选中 → 切换到 B 会话时，context 仍存在。
   */
  sessionId?: string
}

// 模块级单例状态
const globalContext = ref<ModelContext>({
  filePath: undefined,
  fileContent: undefined,
  selectedText: undefined,
  startLine: undefined,
  endLine: undefined,
})

// 版本号，用于触发响应式更新
const version = ref(0)

/**
 * 待注入的输入文本（"AI 上下文 → 主聊天输入框"通道）
 *
 * 当外部组件（文件编辑器、资源浏览器等）想要把选区/上下文
 * 发送到当前活跃会话的 AI 输入框时，调用 enqueueInputInject。
 * ModelChatPanel 会监听 pendingInputInject 并自动填充 inputText。
 *
 * 每个 request 有递增的 id，ModelChatPanel 用 id 防止重复消费。
 */
export interface InputInjectRequest {
  /** 自增 id，ModelChatPanel 消费后用此去重 */
  id: number
  /** 目标会话 id（避免跨会话错投） */
  sessionId: string
  /** 要写入输入框的文本（已经格式化好，含上下文摘要）。
   *  留空表示只聚焦，不改写内容。 */
  text: string
  /** 是否把光标定位到开头（默认 true，AI 上下文一般放后面让用户先问） */
  focusEnd?: boolean
}

const pendingInputInject = ref<InputInjectRequest | null>(null)
let injectIdCounter = 0

/**
 * 更新全局 AI 上下文
 */
export function setModelContext(ctx: Partial<ModelContext>) {
  globalContext.value = {
    ...globalContext.value,
    ...ctx,
  }
  version.value++
}

/**
 * 重置全局 AI 上下文
 */
export function resetModelContext() {
  globalContext.value = {
    filePath: undefined,
    fileContent: undefined,
    selectedText: undefined,
    startLine: undefined,
    endLine: undefined,
    images: undefined,
    sessionId: undefined,
  }
  version.value++
}

/**
 * 把一段文本注入到指定会话的 AI 输入框
 *
 * ModelChatPanel 监听后会自动填充 + 聚焦。
 * 与 setModelContext 配套使用：通常先 setModelContext（把上下文标记为
 * "已选中的内容"），再 enqueueInputInject（把可视化摘要写到输入框）。
 */
export function enqueueInputInject(
  sessionId: string,
  text: string,
  options?: { focusEnd?: boolean }
) {
  injectIdCounter += 1
  pendingInputInject.value = {
    id: injectIdCounter,
    sessionId,
    text,
    focusEnd: options?.focusEnd ?? false
  }
  // 同时递增 version，让监听 version 的组件也能感知
  version.value++
}

/**
 * 只请求聚焦指定会话的输入框，不写入任何文本。
 * 适用于"上下文已通过 setModelContext 写入，只需要把光标定位到输入框"的场景。
 */
export function requestFocusInput(sessionId: string) {
  injectIdCounter += 1
  pendingInputInject.value = {
    id: injectIdCounter,
    sessionId,
    text: '',
    focusEnd: false
  }
  version.value++
}

/**
 * 消费（清空）pending input；ModelChatPanel 在填充完输入框后调用。
 * 返回被消费前的值（用于做去重 / 调试）。
 */
export function consumeInputInject(id: number): InputInjectRequest | null {
  if (pendingInputInject.value && pendingInputInject.value.id === id) {
    const consumed = pendingInputInject.value
    pendingInputInject.value = null
    return consumed
  }
  return null
}

/**
 * 使用全局 AI 上下文
 *
 * 返回响应式的上下文对象和版本号，当上下文更新时自动触发重新渲染。
 */
export function useModelContext() {
  // 每次调用都创建一个新的响应式引用，确保组件能跟踪变化
  const context = computed(() => ({
    ...globalContext.value,
    _version: version.value,
  }))

  return {
    context,
    version,
    pendingInputInject
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
  ctx?: ModelContext
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
  // if (ctx.fileContent) {
  //   contextParts.push(`\n**完整文件内容：**\n\`\`\`\n${ctx.fileContent}\n\`\`\``)
  // }

  if (contextParts.length > 0) {
    return `[上下文信息]\n${contextParts.join('\n')}\n\n---\n\n**问题：** ${userInput}`
  }

  return userInput
}

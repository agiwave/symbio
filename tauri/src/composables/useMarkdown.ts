/**
 * Markdown 渲染 composable
 * 
 * 提供统一的 Markdown 渲染功能，供 ModelChatPanel 和 ModelSelectionDialog 使用
 */

import { marked } from 'marked'
import { logger } from '@/utils/logger'

// 配置 marked - marked v17+ 使用对象配置
marked.setOptions({ 
  breaks: true, 
  gfm: true,
  async: false  // 禁用异步模式以获得同步返回
})

export function useMarkdown() {
  /**
   * 将 Markdown 内容渲染为 HTML
   */
  function renderMarkdown(content: string): string {
    try {
      // marked v17+ 默认返回 Promise，但设置 async: false 后可以同步使用
      const result = marked(content)
      // 检查是否为 Promise，如果是则返回原始内容（不应发生）
      if (result instanceof Promise) {
        logger.warn('useMarkdown', 'Markdown returned Promise, returning raw content')
        return content
      }
      return result as string
    } catch (error) {
      logger.error('useMarkdown', 'Failed to render markdown:', error)
      return content
    }
  }

  return {
    renderMarkdown
  }
}

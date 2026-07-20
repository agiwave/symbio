/**
 * 消息工具函数
 * 
 * 提供消息相关的工具功能
 */

import type { SessionMessage } from '@/services/session'

/**
 * 从多模态内容中提取纯文本
 */
export function extractText(content: any): string {
  if (!content) return ''
  if (typeof content === 'string') return content
  if (Array.isArray(content)) {
    return content
      .filter(p => p.type === 'text' || p.type === 'input_text' || p.type === 'output_text')
      .map(p => p.text || '')
      .join(' ')
  }
  return ''
}

/**
 * 为消息生成唯一标识
 * 用于 Vue 的 v-for key，避免使用 index 导致的重渲染问题
 */
export function getMessageKey(msg: SessionMessage, index: number): string {
  if (msg.id) return msg.id
  // 使用时间戳 + 角色 + 内容哈希 + 索引作为唯一标识
  const contentText = extractText(msg.content)
  const contentHash = simpleHash(contentText.slice(0, 50))
  return `msg-${msg.timestamp || 0}-${msg.role}-${contentHash}-${index}`
}

/**
 * 简单哈希函数
 */
function simpleHash(str: string): number {
  let hash = 0
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i)
    hash = ((hash << 5) - hash) + char
    hash = hash & hash // Convert to 32bit integer
  }
  return Math.abs(hash)
}

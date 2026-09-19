/**
 * Markdown 渲染（模块级：不依赖组件实例，缓存才有处可挂）
 *
 * 对外只有 `renderMarkdownCached` 一个入口——会话流经 `useMessageContent` 调用它，
 * 渲染结果按内容缓存（流式期间同一份正文只渲染一次）。
 */

import { marked } from 'marked'
import { logger } from '@/utils/logger'

// 配置 marked - marked v17+ 使用对象配置
marked.setOptions({ 
  breaks: true, 
  gfm: true,
  async: false  // 禁用异步模式以获得同步返回
})

/**
 * 模块级渲染：不依赖组件实例，因此可被缓存命中（H6）
 *
 * 原先它是 `useMarkdown()` 里的闭包，每次调用 `useMarkdown()` 都新建一个函数，
 * 于是缓存无处可挂——把它提到模块作用域，缓存才有意义。
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

/**
 * 渲染缓存上限（条）。流式期间正文每来一个 token 就变一次，缓存的价值在于
 * 「滚回去、切回来、重渲染」这些**重复内容**，而不是给每一帧都留一份。
 */
const CACHE_LIMIT = 150

/** 超过这个长度不进缓存：长文单次渲染，缓存它只会把小条目挤出去 */
const CACHE_MAX_CHARS = 2000

const cache = new Map<string, string>()

/** 带缓存的 Markdown 渲染（同一个 `content` 只渲染一次） */
export function renderMarkdownCached(content: string): string {
  if (!content) return ''
  if (content.length > CACHE_MAX_CHARS) return renderMarkdown(content)

  const hit = cache.get(content)
  if (hit !== undefined) return hit

  const html = renderMarkdown(content)
  if (cache.size >= CACHE_LIMIT) {
    // Map 的迭代序 = 插入序：删掉最老的一批（简单 LRU，够用）
    const oldest = cache.keys().next().value
    if (oldest !== undefined) cache.delete(oldest)
  }
  cache.set(content, html)
  return html
}


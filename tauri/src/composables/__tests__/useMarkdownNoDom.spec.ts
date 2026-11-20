/**
 * useMarkdown —— 无 DOM 宿主的安全回退
 *
 * 与 useMarkdown.spec.ts（happy-dom）互补：这里**刻意跑在 node 环境**，
 * 验证没有 DOMParser 时不放行未净化 HTML。正文来自 LLM / 工具输出，
 * 这是 XSS 防线的最后一层——没有判断能力就一律不渲染 HTML。
 */
import { describe, expect, it } from 'vitest'
import { renderMarkdownCached } from '../useMarkdown'

describe('useMarkdown：无 DOM 时不渲染 HTML', () => {
  it('内联事件处理器被转义，不放行原文', () => {
    const html = renderMarkdownCached('<img src="x" onerror="alert(1)">')
    expect(html).not.toContain('<img')
    expect(html).toContain('&lt;img')
  })

  it('script 标签被转义为纯文本', () => {
    const html = renderMarkdownCached('<script>alert(1)</script>安全正文')
    expect(html).not.toContain('<script')
    expect(html).toContain('安全正文')
  })
})

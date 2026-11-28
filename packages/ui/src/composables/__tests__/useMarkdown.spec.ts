// @vitest-environment happy-dom
/**
 * useMarkdown — 渲染管线的两条底线
 *
 * 1. **消毒**：正文来自 LLM / 工具输出，属于不可信内容；Markdown 允许内嵌 HTML，
 *    直接 `v-html` 会让一段回答在应用 webview 里执行脚本。
 * 2. **代码块语言标签**：围栏代码块的语言名是现成事实，要显示出来（不放行任何脚本）。
 */
import { describe, expect, it } from 'vitest'
import { renderMarkdownCached } from '../useMarkdown'

describe('useMarkdown：消毒', () => {
  it('剥掉内联事件处理器', () => {
    const html = renderMarkdownCached('<img src="x" onerror="alert(1)">')
    expect(html).not.toContain('onerror')
  })

  it('剥掉 script 标签但保留同一段里的正文', () => {
    const html = renderMarkdownCached('<script>alert(1)</script>安全正文')
    expect(html).not.toContain('<script')
    expect(html).toContain('安全正文')
  })

  it('保留正常排版与链接', () => {
    const html = renderMarkdownCached('**粗体** [链接](https://example.com)')
    expect(html).toContain('<strong>粗体</strong>')
    expect(html).toContain('href="https://example.com"')
  })
})

describe('useMarkdown：代码块', () => {
  it('围栏语言名进 data-lang，内容按原文转义', () => {
    const html = renderMarkdownCached('```ts\nconst a = 1 < 2\n```')
    expect(html).toContain('data-lang="ts"')
    expect(html).toContain('&lt;')
    expect(html).not.toContain('< 2')
  })

  it('未声明语言的代码块不带标签属性', () => {
    const html = renderMarkdownCached('```\nplain\n```')
    expect(html).not.toContain('data-lang')
    expect(html).toContain('plain')
  })
})

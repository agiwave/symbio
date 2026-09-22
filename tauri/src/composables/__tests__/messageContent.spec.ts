/**
 * 消息内容呈现 — 纯函数单测（node 环境）
 *
 * 覆盖两类：
 * 1. 「内容 → 显示」的取值规则：三种内容形状、失败文案优先级、JSON 判定与高亮退化、
 *    摘要截断。这些规则原先散在 `MessageNode` 的 computed 里，与渲染混在一起。
 * 2. **类名契约**：`messageHighlightedOf` 在 TS 里拼出 `class="json-key"` 一类的字符串，
 *    而样式在 `styles/markdown.css`。两者之间没有编译期约束——这条断言就是那道约束
 *    （它挡住过一次真实缺陷：这些类名曾写在组件的 `<style scoped>` 里，
 *    而 scoped 编译会给选择器补 `[data-v-xxx]`，`v-html` 注入的元素拿不到该属性，
 *    于是着色从未生效）。
 */

import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import {
  messageErrorTextOf,
  messageHighlightedOf,
  messageIsJsonContent,
  messageRenderAsJsonOf,
  messageRenderedOf,
  messageSummaryPreviewOf,
} from '../useMessageContent'
import { MESSAGE_PREVIEW_MAX } from '@/registry/messageTypes'

/**
 * `messageTextOf`（多模态内容 → 纯文本）**不在本文件测**——它在契约层
 * `schemas/chat_message.ts`，与 `MessageContent` 同处一处（store 与组件都要用，
 * 放不进这个依赖 Vue 的组合式里）。它的用例见
 * `schemas/__tests__/chat_message.spec.ts`。
 */

describe('messageErrorTextOf：优先级', () => {
  it('节点 error 字段最权威', () => {
    expect(messageErrorTextOf({ error: '请求失败', content: '正文', meta: { error: '旧' } })).toBe(
      '请求失败',
    )
  })

  it('回退顺序：meta.error → 正文 → 兜底文案', () => {
    expect(messageErrorTextOf({ content: '正文', meta: { error: '旧路径' } })).toBe('旧路径')
    expect(messageErrorTextOf({ content: '正文' })).toBe('正文')
    expect(messageErrorTextOf({})).toBe('执行失败')
  })
})

describe('JSON 判定与高亮', () => {
  it('只有完整可解析的对象 / 数组算 JSON 内容', () => {
    expect(messageIsJsonContent('{"a":1}')).toBe(true)
    expect(messageIsJsonContent('[1,2]')).toBe(true)
    expect(messageIsJsonContent('{"a":')).toBe(false) // 流式半截
    expect(messageIsJsonContent('纯文本')).toBe(false)
    expect(messageIsJsonContent('')).toBe(false)
  })

  it('合法 JSON 被 pretty-print 并逐 token 着色', () => {
    const html = messageHighlightedOf('{"a":1,"b":true,"c":null}')
    expect(html).toContain('json-key')
    expect(html).toContain('json-num')
    expect(html).toContain('json-bool')
    expect(html).toContain('json-null')
  })

  it('解析失败退化为整段字符串着色（流式期间 JSON 不完整是常态）', () => {
    const html = messageHighlightedOf('{"a":')
    expect(html).toContain('json-str')
    expect(html).toContain('{"a":') // 原文照出，不因解析失败变空白
  })

  it('HTML 特殊字符被转义（v-html 注入的安全底线）', () => {
    const html = messageHighlightedOf('<img src=x onerror=alert(1)>')
    expect(html).not.toContain('<img')
    expect(html).toContain('&lt;img')
  })

  it('工具结果走 JSON 高亮，其余走 Markdown', () => {
    expect(messageRenderedOf('{"a":1}', true)).toContain('json-key')
    expect(messageRenderedOf('# 标题', false)).toContain('<h1')
  })

  it('失败体不再重复渲染成 JSON 代码块', () => {
    expect(messageRenderAsJsonOf('{"a":1}', false, false)).toBe(true)
    expect(messageRenderAsJsonOf('{"a":1}', false, true)).toBe(false)
    expect(messageRenderAsJsonOf('{"a":1}', true, false)).toBe(false)
  })
})

describe('收起态单行摘要', () => {
  it('分组节点取首个文本 / 思考子节点的内容（容器自身没有正文）', () => {
    const node = {
      id: 'tc',
      content: '{"args":1}',
      children: [
        { id: 'r', type: 'reasoning' as const, content: '想了什么' },
        { id: 'x', type: 'text' as const, content: '正文' },
      ],
    }
    expect(messageSummaryPreviewOf(node, true, false).text).toBe('想了什么')
    expect(messageSummaryPreviewOf(node, false, false).text).toBe('{"args":1}')
  })

  it('分组节点**尚无 text/reasoning 子节点**时回落到自身正文（运行中工具的请求参数）', () => {
    // ToolCall 的参数就存在自身 content 里，且结果子节点到达之前它没有任何
    // text/reasoning 子节点。此处分组分支若只看子节点，运行中的工具行会一片空白
    // ——参数已逐帧流到本地，界面却要等结果到达才第一次显示文字。
    const running = { id: 'tc', content: '{"path":"a.rs"}', children: [] }
    expect(messageSummaryPreviewOf(running, true, true).text).toBe('{"path":"a.rs"}')
    // 子节点到达后仍以子节点为准（结果预览优先）
    const settled = {
      id: 'tc',
      content: '{"path":"a.rs"}',
      children: [{ id: 'res', type: 'text' as const, content: '文件内容' }],
    }
    expect(messageSummaryPreviewOf(settled, true, false).text).toBe('文件内容')
  })

  it('空白折叠成单空格；超长按上限截断并加省略号', () => {
    expect(messageSummaryPreviewOf({ id: 'x', content: '  a\n\n  b  ' }, false, false).text).toBe(
      'a b',
    )
    const long = 'x'.repeat(MESSAGE_PREVIEW_MAX + 50)
    const out = messageSummaryPreviewOf({ id: 'x', content: long }, false, false).text
    expect(out.length).toBe(MESSAGE_PREVIEW_MAX + 1)
    expect(out.endsWith('…')).toBe(true)
  })

  it('无内容返回空串（头部据此不显示摘要）', () => {
    expect(messageSummaryPreviewOf({ id: 'x' }, false, false).text).toBe('')
  })
})

/**
 * 摘要取**哪一端**：流式中取末端（这一行是走马灯），定稿后取开头（摘要）。
 *
 * 两种状态截断的是**相反**的一端，且返回值里的 `liveEdge` 必须与文本取自哪一端一致
 * ——渲染器据此决定往哪一端裁（`NodeShell` 的 `.node-preview`）。错配的后果是
 * 可见区里剩下**中间**那一段：取了末端却被右端省略，最新内容反而被裁掉。
 */
describe('收起态摘要取哪一端', () => {
  const OPEN = '开头：先看协议；'
  const CLOSE = '末尾：结论已定。'
  const FILLER = '甲'.repeat(MESSAGE_PREVIEW_MAX)
  const content = OPEN + FILLER + CLOSE

  it('流式中（liveEdge）取**末端**：省略号在开头，最新的内容必须在', () => {
    const p = messageSummaryPreviewOf({ id: 'x', content }, false, true)
    expect(p.liveEdge).toBe(true)
    expect(p.text.startsWith('…'), '末端截断必须前置省略号（它是半截话，不是开头）').toBe(true)
    expect(p.text).toContain(CLOSE)
    expect(p.text, '流式中不该出现开头那句话').not.toContain(OPEN)
    expect(p.text.length).toBe(MESSAGE_PREVIEW_MAX + 1)
  })

  it('定稿后取**开头**：省略号在末尾，且不含末端内容', () => {
    const p = messageSummaryPreviewOf({ id: 'x', content }, false, false)
    expect(p.liveEdge).toBe(false)
    expect(p.text.endsWith('…')).toBe(true)
    expect(p.text).toContain(OPEN)
    expect(p.text, '定稿后的摘要是概述，不该出现末端').not.toContain(CLOSE)
    expect(p.text.length).toBe(MESSAGE_PREVIEW_MAX + 1)
  })

  it('内容不超上限时两端取到的是同一串（无省略号，也不改写内容）', () => {
    const short = '短内容'
    expect(messageSummaryPreviewOf({ id: 'x', content: short }, false, true)).toEqual({
      text: short,
      liveEdge: true,
    })
    expect(messageSummaryPreviewOf({ id: 'x', content: short }, false, false)).toEqual({
      text: short,
      liveEdge: false,
    })
  })

  it('空内容返回空串，`liveEdge` 原样透传（渲染器仍需知道往哪端裁）', () => {
    expect(messageSummaryPreviewOf({ id: 'x' }, false, true)).toEqual({ text: '', liveEdge: true })
    expect(messageSummaryPreviewOf({ id: 'x' }, false, false)).toEqual({ text: '', liveEdge: false })
  })
})

describe('类名契约：高亮产出的类必须在全局样式表里有规则', () => {
  // 去注释后再断言：文件头的说明文字里就写着 `[data-v-xxx]`（解释为什么不能用 scoped），
  // 不去掉注释会把「解释」误判成「违规」。
  const css = readFileSync(new URL('../../styles/markdown.css', import.meta.url), 'utf8').replace(
    /\/\*[\s\S]*?\*\//g,
    '',
  )

  it('JSON 着色的每个类都有对应样式（且不是 scoped 选择器）', () => {
    for (const cls of ['json', 'json-key', 'json-str', 'json-num', 'json-bool', 'json-null']) {
      expect(css, `样式表缺 .${cls} 规则`).toContain(`.${cls}`)
    }
    // scoped 编译会补 [data-v-*]，而 v-html 注入的元素拿不到该属性 → 着色静默失效
    expect(css, 'markdown.css 必须是全局样式表（不得出现 scoped 属性选择器）').not.toContain(
      '[data-v-',
    )
  })

  it('实际产出的类名与样式表对得上（防止两边各改一半）', () => {
    const html = messageHighlightedOf('{"k":"v","n":1,"b":false,"z":null}')
    const produced = [...html.matchAll(/class="([^"]+)"/g)].map((m) => m[1])
    expect(produced.length).toBeGreaterThan(0)
    for (const cls of produced) {
      expect(css, `高亮产出了 .${cls}，但样式表里没有`).toContain(`.${cls}`)
    }
  })
})

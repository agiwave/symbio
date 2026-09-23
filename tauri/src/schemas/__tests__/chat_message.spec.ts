/**
 * 消息契约 — 纯函数单测（node 环境）
 *
 * 这里锁的是 `messageTextOf`：**多模态内容取纯文本的唯一实现**。
 *
 * 为什么值得单独测：它是 store / 组合式 / 组件三层的共同依赖，而且历史上出现过
 * 5 份各自实现的取值逻辑，其中 3 份漏掉了 `ContentPart[]` 这一**契约自身的形状**
 * ——纯文本数组被读成空串，正文凭空消失且不报错。下面 `ContentPart[]` 那两条
 * 就是防它回归的。
 */

import { describe, expect, it } from 'vitest'
import { MESSAGE_STATUSES, isMessageStatus, messageTextOf } from '../chat_message'
import type { MessageContent } from '../chat_message'

/**
 * `{ text }` / `{ parts }` 是**历史与变体形状**：不在当前 `MessageContent` 联合类型里，
 * 但后端与旧数据仍可能出现，取值函数必须认。这里用显式断言把它们喂进去，
 * 顺带把「类型之外的形状也要兜住」这件事固定在测试里。
 */
const asContent = (v: unknown) => v as MessageContent

describe('messageTextOf：内容形状', () => {
  it('字符串直接返回', () => {
    expect(messageTextOf('hello')).toBe('hello')
  })

  it('{ text } 取 text', () => {
    expect(messageTextOf(asContent({ text: 'hi' }))).toBe('hi')
  })

  it('{ parts } 按顺序拼接各段 text（分片不带 type）', () => {
    expect(messageTextOf(asContent({ parts: [{ text: 'a' }, { text: 'b' }] }))).toBe('a\nb')
  })

  it('ContentPart[] 只拼文本分片 —— 契约自身的形状，不可漏', () => {
    expect(messageTextOf([{ type: 'text', text: '看这张' }])).toBe('看这张')
    expect(
      messageTextOf([
        { type: 'text', text: 'a' },
        { type: 'text', text: 'b' },
      ]),
    ).toBe('a\nb')
  })

  it('ContentPart[] 里的图片段不产生文本（不能把结构化对象 String() 出来）', () => {
    expect(messageTextOf([{ type: 'image_url', image_url: { url: 'x' } }])).toBe('')
    expect(
      messageTextOf([
        { type: 'text', text: '看这张' },
        { type: 'image_url', image_url: { url: 'data:x' } },
      ]),
    ).toBe('看这张')
  })

  it('provider 变体的文本段类型名一并认下（input_text / output_text）', () => {
    expect(messageTextOf(asContent([{ type: 'input_text', text: 'in' }]))).toBe('in')
    expect(messageTextOf(asContent([{ type: 'output_text', text: 'out' }]))).toBe('out')
  })

  it('缺省 / 空 / 未知形状返回空串，而不是 String() 成 [object Object]', () => {
    expect(messageTextOf(undefined)).toBe('')
    expect(messageTextOf(null)).toBe('')
    expect(messageTextOf('')).toBe('')
    expect(messageTextOf(asContent({ text: 123 }))).toBe('')
    expect(messageTextOf(asContent({ other: 1 }))).toBe('')
  })
})

/**
 * `isMessageStatus`：集合判定**派生自词表**，不是另抄一份列表。
 *
 * 它是 VDFS 消息节点（`ext = message`）透传状态词的唯一判据。这里锁两件事：
 * 词表里每个取值都判真（派生关系没写反），未知词与非字符串一律判假
 * （判不住就会让未知词以"有状态"落进 store，或被兜底成 `completed` 而谎报成功）。
 */
describe('isMessageStatus：词表派生判定', () => {
  it('词表的每个取值都判真', () => {
    for (const s of MESSAGE_STATUSES) expect(isMessageStatus(s)).toBe(true)
  })

  it('未知词 / 旧别名 / 非字符串 / 空值一律判假', () => {
    expect(isMessageStatus('paused')).toBe(false)
    expect(isMessageStatus('active')).toBe(false) // 旧数据别名，不在消息词表内
    expect(isMessageStatus('')).toBe(false)
    expect(isMessageStatus(undefined)).toBe(false)
    expect(isMessageStatus(null)).toBe(false)
    expect(isMessageStatus(1)).toBe(false)
    expect(isMessageStatus({ status: 'failed' })).toBe(false)
  })
})

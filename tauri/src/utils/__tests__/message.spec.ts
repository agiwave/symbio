import { describe, it, expect } from 'vitest'
import { extractText, getMessageKey } from '../message'
import type { SessionMessage } from '@/services/session'

describe('extractText', () => {
  it('空值返回空字符串', () => {
    expect(extractText(null)).toBe('')
    expect(extractText(undefined)).toBe('')
    expect(extractText('')).toBe('')
  })

  it('纯字符串原样返回', () => {
    expect(extractText('hello world')).toBe('hello world')
    expect(extractText('中文内容')).toBe('中文内容')
  })

  it('多模态数组：拼接 text 片段（空格分隔）', () => {
    const content = [
      { type: 'text', text: '第一段' },
      { type: 'image_url', image_url: 'http://x' }, // 非文本片段应被过滤
      { type: 'text', text: '第二段' }
    ]
    expect(extractText(content)).toBe('第一段 第二段')
  })

  it('多模态数组：支持 input_text / output_text 类型（协议兼容）', () => {
    const content = [
      { type: 'input_text', text: 'input' },
      { type: 'output_text', text: 'output' }
    ]
    expect(extractText(content)).toBe('input output')
  })

  it('缺失 text 字段的片段按空串处理', () => {
    const content = [{ type: 'text' }, { type: 'text', text: '有值' }]
    expect(extractText(content)).toBe(' 有值')
  })

  it('其他类型（对象/数字）返回空字符串', () => {
    expect(extractText(42)).toBe('')
    expect(extractText({ type: 'text' })).toBe('')
  })
})

describe('getMessageKey', () => {
  const makeMsg = (over: Partial<SessionMessage> = {}): SessionMessage =>
    ({
      id: '',
      role: 'user',
      content: 'hello',
      timestamp: 1000,
      ...over
    }) as SessionMessage

  it('优先使用消息 id', () => {
    expect(getMessageKey(makeMsg({ id: 'abc123' }), 0)).toBe('abc123')
  })

  it('无 id 时回退到 timestamp+role+hash+index 组合键', () => {
    const key = getMessageKey(makeMsg(), 3)
    expect(key).toMatch(/^msg-1000-user-\d+-3$/)
  })

  it('同内容同位置生成稳定键（v-for key 稳定性）', () => {
    const a = getMessageKey(makeMsg(), 1)
    const b = getMessageKey(makeMsg(), 1)
    expect(a).toBe(b)
  })

  it('不同内容生成不同键', () => {
    const a = getMessageKey(makeMsg({ content: '内容A' }), 1)
    const b = getMessageKey(makeMsg({ content: '内容B' }), 1)
    expect(a).not.toBe(b)
  })

  it('长内容只取前 50 字符参与哈希（前缀相同即同键）', () => {
    const longA = 'x'.repeat(60) + 'A'
    const longB = 'x'.repeat(60) + 'B'
    // 前 50 字符相同 → 哈希相同（这是"前缀哈希"设计的已知行为）
    expect(getMessageKey(makeMsg({ content: longA }), 0)).toBe(
      getMessageKey(makeMsg({ content: longB }), 0)
    )
  })

  it('多模态内容消息也能生成键', () => {
    const key = getMessageKey(
      makeMsg({ content: [{ type: 'text', text: '多模态' }] } as any),
      0
    )
    expect(key).toMatch(/^msg-1000-user-\d+-0$/)
  })
})
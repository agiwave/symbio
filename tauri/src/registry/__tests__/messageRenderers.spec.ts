// @vitest-environment happy-dom
/**
 * 渲染器装配覆盖率 —— 「词表里有的类型，装配点必须登记了组件」
 *
 * 这是本次机制化改造的**核心保障**：新增一种消息类型时，
 * 只要忘了在 `registry/messageRenderers.ts` 登记一行，这条断言就会红。
 * 没有它，「加一个类型要改几处」这件事会重新靠记忆维持。
 *
 * 反过来也断言兜底链：未登记类型必须落到 `fallback` 且**确实有组件**，
 * 保证「后端先行上线新 type、前端还是旧版」时会话流不空白。
 */

import { describe, expect, it } from 'vitest'
import { MESSAGE_TYPES, MESSAGE_TYPE_TEXT } from '@/schemas/chat_message'
import {
  facetsOf,
  getMessageRenderer,
  messageRendererKey,
  resolveMessageRenderer,
  type MessageRenderer,
} from '../messageTypes'
// 副作用导入：装配点登记「标识 → 组件」
import '../messageRenderers'

/** 机制级渲染器全集（新增一个必须同步更新这里，否则下面的一致性断言会失败） */
const ALL_RENDERERS: MessageRenderer[] = [
  'turn',
  'text',
  'tool_call',
  'user_prompt',
  'compression',
  'fallback',
]

describe('渲染器装配：每个标识都有组件', () => {
  it.each(ALL_RENDERERS)('渲染器 %s 已登记组件', (renderer) => {
    expect(getMessageRenderer(renderer), `渲染器 ${renderer} 未登记`).toBeDefined()
  })

  it('未登记的渲染器标识返回 undefined（调用方据此兜底）', () => {
    expect(getMessageRenderer('nope' as MessageRenderer)).toBeUndefined()
  })
})

describe('渲染器装配：词表里的每个消息类型都能渲染出来', () => {
  it.each(MESSAGE_TYPES)('类型 %s 能解析到组件', (type) => {
    const renderer = resolveMessageRenderer(facetsOf({ id: 'x', type }))
    expect(renderer, `类型 ${type} 解析不到渲染器组件`).toBeDefined()
  })

  it('未登记类型回落到 fallback，而不是渲染成空白', () => {
    const f = facetsOf({ id: 'x', type: 'brand_new_type' as typeof MESSAGE_TYPE_TEXT })
    expect(messageRendererKey(f)).toBe('fallback')
    expect(resolveMessageRenderer(f)).toBeDefined()
    expect(resolveMessageRenderer(f)).toBe(getMessageRenderer('fallback'))
  })
})

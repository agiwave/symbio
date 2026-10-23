// @vitest-environment happy-dom
/**
 * 渲染器装配覆盖率 —— 「词表里有的形态，装配点必须登记了组件」
 *
 * 这是本次机制化改造的**核心保障**：新增一个 wire 五值时，
 * 只要忘了在 `registry/messageRenderers.ts` 登记一行，这条断言就会红。
 * 没有它，「加一个形态要改几处」这件事会重新靠记忆维持。
 *
 * 前端分派轴只剩 wire 五值 `NodeKind`，业务形态标识（turn / tool_call…）
 * 全部删除——渲染器标识与五值一一对应（`typing` 占位走 `fallback`）。
 *
 * 反过来也断言兜底链：未登记形态必须落到 `fallback` 且**确实有组件**，
 * 保证「后端先行上线新 type、前端还是旧版」时会话流不空白。
 */

import { describe, expect, it } from 'vitest'
import { NODE_KINDS, NODE_KIND_BODY, type NodeKind } from '@/schemas/chat_node'
import {
  facetsOf,
  getMessageRenderer,
  messageRendererKey,
  resolveMessageRenderer,
  type MessageRenderer,
} from '../messageTypes'
// 副作用导入：装配点登记「标识 → 组件」
import '../messageRenderers'

/** 机制级渲染器全集（与 wire 五值一一对应，`typing` 并入 `fallback`） */
const ALL_RENDERERS: MessageRenderer[] = [
  'body',
  'notice',
  'skeleton',
  'interaction',
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

describe('渲染器装配：词表里的每个 wire 五值都能渲染出来', () => {
  it.each(NODE_KINDS)('形态 %s 能解析到组件', (type: NodeKind) => {
    const renderer = resolveMessageRenderer(facetsOf({ id: 'x', type }))
    expect(renderer, `形态 ${type} 解析不到渲染器组件`).toBeDefined()
  })

  it('typing 占位分派到 fallback 且有兜底组件（不空白）', () => {
    const f = facetsOf({ id: 'x', type: 'typing' })
    expect(messageRendererKey(f)).toBe('fallback')
    expect(resolveMessageRenderer(f)).toBe(getMessageRenderer('fallback'))
  })

  it('后端先行上线的未知 type 折到正文、渲染成组件而非空白', () => {
    // nodeKindOf 把未知取值归一到 body——旧前端见到新 wire 词不会渲染成空白，
    // 而是按正文显示：分派轴只剩五值，未知即正文兜底）。
    const f = facetsOf({ id: 'x', type: 'brand_new_type' as typeof NODE_KIND_BODY })
    expect(f.type).toBe(NODE_KIND_BODY)
    expect(resolveMessageRenderer(f)).toBeDefined()
  })
})

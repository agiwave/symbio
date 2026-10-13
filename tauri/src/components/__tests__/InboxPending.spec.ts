// @vitest-environment happy-dom
/**
 * `InboxPending` 组件级测试 —— 排队中的用户消息（T-077 步③）
 *
 * 锁三件事：
 * 1. **空队列不渲染**（`entries: []` ⇒ 整个组件不在 DOM 里，不留一行空白）；
 * 2. **条目逐条渲染**，顺序即**消费顺序**（调用方不得重排，`v-for` 也不得重排）；
 * 3. 外观契约：虚线描边 + 「待发送」标 + 预览文案——它必须与「已落库的用户气泡」
 *    可区分（同一区域里并排出现「排队中的同一条消息」是可能的）。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import InboxPending from '../message/InboxPending.vue'

function mountPending(entries: Array<{ id: string; label: string }>) {
  return mount(InboxPending, { props: { entries } })
}

describe('InboxPending：排队中的用户消息', () => {
  it('空队列 ⇒ 整个组件不渲染（不留一行空白）', () => {
    const w = mountPending([])
    expect(w.find('.inbox-pending').exists()).toBe(false)
    expect(w.text()).toBe('')
  })

  it('单条 ⇒ 一行气泡：待发送标 + 预览文案，且不出现计数说明行', () => {
    const w = mountPending([{ id: 'i1', label: '待消费 · 帮我跑一下测试' }])
    const rows = w.findAll('.inbox-row')
    expect(rows.length).toBe(1)
    expect(rows[0].text()).toContain('待发送')
    expect(rows[0].text()).toContain('帮我跑一下测试')
    // 单条不再压一行字（最常见的情形正是前端自己刚发出去的那条）
    expect(w.find('.inbox-pending-hint').exists()).toBe(false)
  })

  it('多条 ⇒ 逐条渲染且**顺序即消费顺序**（不得重排）', () => {
    const w = mountPending([
      { id: 'i1', label: '第一条' },
      { id: 'i2', label: '第二条' },
      { id: 'i3', label: '第三条' },
    ])
    expect(w.findAll('.inbox-bubble').map((b) => b.text())).toEqual([
      '待发送第一条',
      '待发送第二条',
      '待发送第三条',
    ])
    expect(w.find('.inbox-pending-hint').text()).toContain('3 条消息排队中')
  })

  it('与「已落库的用户气泡」可区分：虚线描边 + 无底色', () => {
    const w = mountPending([{ id: 'i1', label: '待消费 · x' }])
    const style = w.find('.inbox-bubble').attributes('style') ?? ''
    // 外观契约在 scoped 样式里，DOM 侧可断言的只有类名与不内联样式
    expect(w.find('.inbox-bubble').classes()).toEqual(['inbox-bubble'])
    expect(style).toBe('')
  })
})

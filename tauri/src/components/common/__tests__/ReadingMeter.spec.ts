// @vitest-environment happy-dom
/**
 * ReadingMeter —— 「渲染器里不许有业务判据」这条纪律的判据
 *
 * 这一格是信任面最容易悄悄说谎、也最容易「顺手加一点逻辑」的地方：少一个数时最
 * 自然的写法是补 0，需要判断「快满了」时最自然的写法是写个 `0.7`。两条都会让
 * 界面与后端的账分家。所以本文件钉的全是**不发明**：
 *
 * 1. **没有这一格就整条不渲染**（不画空条）；
 * 2. **文案逐字来自产者**——用一个前端从没见过的标签也能排（组件不认业务词）；
 * 3. **几何只按 `bar` 画**：`level` 是填充、`marks` 是参考线（可多条）、
 *    `× 100` 只是 ratio→% 的单位换算；没有 `bar` 就没有条，行照常排；
 * 4. **着色只由产者的 `tone` 决定**：`level` 再高、只要语气是 normal 就不转色，
 *    因为「算不算接近动手线」的判据住在产者（阈值系数只有一处 owner）。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'

import ReadingMeter from '../ReadingMeter.vue'
import type { ReadingSection } from '@/schemas/reading'

const section = (over: Partial<ReadingSection> = {}): ReadingSection => ({
  rows: [
    { label: '已用', value: '45k / 128k' },
    { label: '自动整理', value: '89k' },
  ],
  bar: { level: 0.35, marks: [0.7], tone: 'normal' },
  ...over,
})

describe('ReadingMeter：没有读数就不出现', () => {
  it('不传 section ⇒ 整条不渲染（不画一条空条冒充「是空的」）', () => {
    const w = mount(ReadingMeter)
    expect(w.find('.reading-meter').exists()).toBe(false)
    expect(w.find('.meter-row').exists()).toBe(false)
  })

  it('只排行（没有 `bar` 这格）⇒ 有条文字、没有轨道', () => {
    const w = mount(ReadingMeter, {
      props: { section: section({ bar: undefined }) },
    })
    expect(w.findAll('.meter-row')).toHaveLength(2)
    expect(w.find('.meter-track').exists()).toBe(false)
  })
})

describe('ReadingMeter：文案逐字来自产者', () => {
  it('标签与值原样呈现，前端不拼数字、不改单位', () => {
    const w = mount(ReadingMeter, { props: { section: section() } })
    expect(
      w.findAll('.meter-row').map((r) => [r.find('.row-label').text(), r.find('.row-value').text()]),
    ).toEqual([
      ['已用', '45k / 128k'],
      ['自动整理', '89k'],
    ])
  })

  it('产者换个主题也照排（组件不认业务词，故零改动）', () => {
    const w = mount(ReadingMeter, {
      props: {
        section: {
          rows: [
            { label: '本月成本', value: '$3.20' },
            { label: '告警线', value: '$5.00' },
            { label: '口径', value: '按供应商账单折算' },
          ],
          bar: { level: 0.64, marks: [0.8, 1] },
        },
      },
    })
    expect(w.findAll('.meter-row')).toHaveLength(3)
    expect(w.findAll('.meter-tick')).toHaveLength(2)
  })

  it('`hint` 挂在行上（口径提示是产者的话，不进口头摘要）', () => {
    const w = mount(ReadingMeter, {
      props: {
        section: section({
          rows: [{ label: '已用', value: '45k / 128k', hint: '本轮请求开头的估算' }],
        }),
      },
    })
    expect(w.find('.meter-row').attributes('title')).toBe('本轮请求开头的估算')
    expect(w.find('.reading-meter').attributes('title')).toBe('已用 45k / 128k')
  })
})

describe('ReadingMeter：几何只按 bar 画', () => {
  it('level 是填充、marks 是参考线（% 只是单位换算，不是第二次除法）', () => {
    const w = mount(ReadingMeter, {
      props: { section: section({ bar: { level: 0.35, marks: [0.7, 0.9] } }) },
    })
    expect(w.find('.meter-fill').attributes('style')).toContain('width: 35%')
    expect(w.findAll('.meter-tick').map((t) => t.attributes('style'))).toEqual([
      'left: 70%;',
      'left: 90%;',
    ])
  })

  it('缺 `level`（没有分母）⇒ 条不表态，行照常排', () => {
    const w = mount(ReadingMeter, {
      props: { section: section({ bar: { marks: [0.7] } }) },
    })
    expect(w.find('.meter-fill').attributes('style')).toContain('width: 0%')
    expect(w.findAll('.meter-row')).toHaveLength(2)
  })
})

describe('ReadingMeter：着色只认产者的 tone', () => {
  it('tone=warn ⇒ 条与该行转色', () => {
    const w = mount(ReadingMeter, {
      props: {
        section: section({
          rows: [{ label: '自动整理', value: '89k', tone: 'warn' }],
          bar: { level: 0.72, marks: [0.7], tone: 'warn' },
        }),
      },
    })
    expect(w.find('.meter-fill').classes()).toContain('is-warn')
    expect(w.find('.meter-row').classes()).toContain('is-warn')
  })

  it('level 再高也不自己判断「快满了」：normal 就是不着色', () => {
    const w = mount(ReadingMeter, {
      props: { section: section({ bar: { level: 0.99, marks: [0.7], tone: 'normal' } }) },
    })
    expect(w.find('.meter-fill').classes()).not.toContain('is-warn')
    expect(w.findAll('.meter-row').every((r) => !r.classes().includes('is-warn'))).toBe(true)
  })

  it('无障碍摘要 = 逐行「标签 值」，与可见文字同一读法', () => {
    const w = mount(ReadingMeter, { props: { section: section() } })
    expect(w.find('.meter-track').attributes('aria-label')).toBe('已用 45k / 128k · 自动整理 89k')
  })
})

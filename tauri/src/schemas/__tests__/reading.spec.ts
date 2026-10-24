import { describe, expect, it } from 'vitest'
import { panelHasWarn, panelRowCount, readingSectionOf, type ReadingPanel } from '../reading'

/**
 * reading 方言的判据。
 *
 * ## 这个模块唯一的失效形态
 *
 * 它是**跨栈契约的半边**：后端 `symbio_core/schemas/reading.rs` 下发描述，本文件
 * 认它。两个访问器（行数 / 有无需留意）若与后端语义不一致，症状不是报错而是
 * **静默显示错**——抽屉标题的提示点不亮，或空态文案说错。故这里逐条钉住与后端
 * `row_count` / `has_warn` 逐字对应的形状。
 */

const row = (over: Partial<{ tone: 'normal' | 'warn' }> = {}) => ({
  label: 'L',
  value: 'V',
  ...over,
})

describe('panelRowCount', () => {
  it('数全部分区的行（嵌套不漏）', () => {
    const p: ReadingPanel = {
      title: 't',
      sections: [
        { title: 'a', rows: [row(), row()] },
        { rows: [row()] },
      ],
    }
    expect(panelRowCount(p)).toBe(3)
  })

  it('没有 sections ⇒ 0（合法空，不是错误）', () => {
    expect(panelRowCount({ title: 't' })).toBe(0)
  })
})

describe('panelHasWarn', () => {
  it('任一分区的任一行 tone=warn ⇒ 真', () => {
    const p: ReadingPanel = {
      title: 't',
      sections: [
        { title: 'a', rows: [row()] },
        { rows: [row(), row({ tone: 'warn' })] },
      ],
    }
    expect(panelHasWarn(p)).toBe(true)
  })

  it('全 normal（含 tone 缺省）⇒ 假', () => {
    const p: ReadingPanel = {
      title: 't',
      sections: [{ rows: [row(), row({ tone: 'normal' })] }],
    }
    expect(panelHasWarn(p)).toBe(false)
  })

  it('空面板 ⇒ 假', () => {
    expect(panelHasWarn({ title: 't' })).toBe(false)
  })
})

describe('readingSectionOf：任意来源的一格读数在边界收成可用形状', () => {
  it('认结构不认业务词：行 + 可选 `bar` 原样收下来', () => {
    const raw = { rows: [{ label: '已用', value: '45k / 128k' }], bar: { level: 0.35, marks: [0.7] } }
    expect(readingSectionOf(raw)).toEqual(raw)
  })

  it('产者多一行少一行、换一个主题都照收（本判据不为「水位」而写）', () => {
    expect(readingSectionOf({ rows: [{ label: '成本', value: '$3' }] })).toEqual({
      rows: [{ label: '成本', value: '$3' }],
    })
  })

  // 收口只有一条判据：`rows` 是不是数组。畸形载荷在这里就挡掉，而不是到渲染时才炸。
  it('`rows` 不成形 ⇒ undefined（不留「有条没字」的半截读数）', () => {
    expect(readingSectionOf(undefined)).toBeUndefined()
    expect(readingSectionOf(null)).toBeUndefined()
    expect(readingSectionOf('一个字符串')).toBeUndefined()
    expect(readingSectionOf({ bar: { level: 0.5 } })).toBeUndefined()
    expect(readingSectionOf({ rows: '不是数组', bar: { level: 0.5 } })).toBeUndefined()
  })
})

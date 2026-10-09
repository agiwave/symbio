import { describe, expect, it } from 'vitest'
import { panelHasWarn, panelRowCount, type ReadingPanel } from '../reading'

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

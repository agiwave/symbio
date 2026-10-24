// @vitest-environment happy-dom
/**
 * 会话级 UI 状态单测
 *
 * 钉住三条「错了也不报错」的约定：
 *
 * 1. **键是会话**：面板 A 读不到面板 B 的草稿、阅读位置与渲染窗口条数
 *    （这是它存在的全部理由）；
 * 2. **空串 = 清除**，不留空记录——否则 `forget` / 容量都要额外区分「空记录」；
 * 3. **非有限值不记**：布局中间态（`NaN` / `Infinity`）不是用户位置，记下来会让
 *    下次还原时把滚动条设成一个非法值（浏览器静默忽略 ⇒ 表现是「位置没还原」）。
 *
 * 位置记的是**离底距离**而不是 `scrollTop`：会话流的尾部在长，渲染层又只铺最近
 * 若干条根节点，从顶部量的数在这两件事上都对不上同一条消息。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useSessionUiStateStore } from '../sessionUiState'

beforeEach(() => {
  setActivePinia(createPinia())
})

describe('sessionUiState 草稿', () => {
  it('按会话隔离：互不串台', () => {
    const s = useSessionUiStateStore()
    s.setDraft('a', '写给 a 的')
    s.setDraft('b', '写给 b 的')
    expect(s.draftOf('a')).toBe('写给 a 的')
    expect(s.draftOf('b')).toBe('写给 b 的')
    expect(s.draftOf('c'), '无记录 = 空串').toBe('')
  })

  it('空串即清除（不留下一条空记录）', () => {
    const s = useSessionUiStateStore()
    s.setDraft('a', '草稿')
    s.setDraft('a', '')
    expect(s.draftOf('a')).toBe('')
    // 清除后再 forget 与直接 forget 等价 ⇒ 没有残留的空记录
    s.forget('a')
    expect(s.draftOf('a')).toBe('')
  })

  it('空 sessionId 不写（草稿态没有 id，不能污染成一条无主记录）', () => {
    const s = useSessionUiStateStore()
    s.setDraft('', '无主草稿')
    expect(s.draftOf('')).toBe('')
  })
})

describe('sessionUiState 阅读位置（离底距离）', () => {
  it('按会话隔离，无记录时给 null（调用方据此选择「贴底」）', () => {
    const s = useSessionUiStateStore()
    expect(s.scrollGapOf('a')).toBeNull()
    s.setScrollGap('a', 120)
    expect(s.scrollGapOf('a')).toBe(120)
    expect(s.scrollGapOf('b'), '另一个会话不得继承位置').toBeNull()
  })

  it('非有限值一律忽略（布局中间态不是用户位置）', () => {
    const s = useSessionUiStateStore()
    s.setScrollGap('a', Number.NaN)
    s.setScrollGap('a', Number.POSITIVE_INFINITY)
    expect(s.scrollGapOf('a')).toBeNull()
  })
})

/**
 * 窗口条数与离底距离是**一对**：窗口决定 DOM 里铺了多少内容，位置决定落在这段内容的
 * 哪一处。只存其一会让「往上翻过很远再切回来」落到窗口顶部——所以这里钉的判据是
 * 「两个都记、两个都清」。
 */
describe('sessionUiState 渲染窗口条数', () => {
  it('按会话隔离，无记录时给 null（调用方用首屏默认值）', () => {
    const s = useSessionUiStateStore()
    expect(s.windowSizeOf('a')).toBeNull()
    s.setWindowSize('a', 80)
    expect(s.windowSizeOf('a')).toBe(80)
    expect(s.windowSizeOf('b')).toBeNull()
  })

  it('非正数与非有限值不记（0 条窗口不是一个可读的位置）', () => {
    const s = useSessionUiStateStore()
    s.setWindowSize('a', 0)
    s.setWindowSize('a', -40)
    s.setWindowSize('a', Number.NaN)
    expect(s.windowSizeOf('a')).toBeNull()
  })

  it('空 sessionId 不写', () => {
    const s = useSessionUiStateStore()
    s.setWindowSize('', 80)
    expect(s.windowSizeOf('')).toBeNull()
  })
})

describe('sessionUiState forget（会话删除时的清理）', () => {
  it('草稿、阅读位置、窗口条数一起丢弃', () => {
    const s = useSessionUiStateStore()
    s.setDraft('a', '草稿')
    s.setScrollGap('a', 10)
    s.setWindowSize('a', 80)
    s.forget('a')
    expect(s.draftOf('a')).toBe('')
    expect(s.scrollGapOf('a')).toBeNull()
    expect(s.windowSizeOf('a')).toBeNull()
  })

  it('不误伤其它会话', () => {
    const s = useSessionUiStateStore()
    s.setDraft('a', 'A')
    s.setDraft('b', 'B')
    s.setScrollGap('b', 5)
    s.setWindowSize('b', 120)
    s.forget('a')
    expect(s.draftOf('b')).toBe('B')
    expect(s.scrollGapOf('b')).toBe(5)
    expect(s.windowSizeOf('b')).toBe(120)
  })
})

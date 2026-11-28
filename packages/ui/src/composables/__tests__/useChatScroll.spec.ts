/**
 * useChatScroll — 聊天滚动策略单测（happy-dom）
 *
 * 它是「消息流什么时候自动往下滚」的唯一实现。核心约定只有一条：
 * **只有用户本来就在底部附近时才自动滚动**——用户翻上去看历史时不能被新消息
 * 拽回底部。这条约定由一个布尔（`isUserAtBottom`）承载，而它由滚动事件维护，
 * 所以阈值边界必须钉死。
 *
 * 钉住：
 * 1. 阈值 50px 是**严格小于**：距底正好 50 不算在底部（否则边界上会来回抖）。
 * 2. `scrollToBottom` 落到 `scrollTop = scrollHeight`（不是 0，也不是 clientHeight）。
 * 3. 容器尚未挂载（`null`）时各函数都**不抛**，也不改变状态——组件首帧
 *    （ref 还是 null）就会走到这条。
 * 4. `smartScroll` 只在底部才真的滚；不在底部时它是**空操作**（不是滚到顶部）。
 * 5. 位置的记忆与还原都走**离底距离**这一条换算（`gapFromBottom` / `scrollToGap`），
 *    贴底判定读的是同一个数。
 */

// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { ref } from 'vue'
import { useChatScroll } from '../useChatScroll'

/** 造一个可设定 scrollHeight / clientHeight 的容器（happy-dom 里它们恒为 0） */
function container(scrollHeight: number, clientHeight: number, scrollTop = 0): HTMLElement {
  const el = document.createElement('div')
  Object.defineProperty(el, 'scrollHeight', { value: scrollHeight, configurable: true })
  Object.defineProperty(el, 'clientHeight', { value: clientHeight, configurable: true })
  el.scrollTop = scrollTop
  return el
}

describe('useChatScroll — 底部判定', () => {
  it('初始视为在底部（首屏应当自动滚到底）', () => {
    const { isUserAtBottom } = useChatScroll(ref(null))
    expect(isUserAtBottom.value).toBe(true)
  })

  it('距底小于阈值 = 在底部', () => {
    const el = container(1000, 400, 560) // 1000-560-400 = 40
    const { isUserAtBottom, handleScroll } = useChatScroll(ref(el))
    handleScroll()
    expect(isUserAtBottom.value).toBe(true)
  })

  it('阈值是严格小于：距底正好 50 不算在底部', () => {
    const el = container(1000, 400, 550) // 1000-550-400 = 50
    const { isUserAtBottom, handleScroll } = useChatScroll(ref(el))
    handleScroll()
    expect(isUserAtBottom.value).toBe(false)
  })

  it('距底大于阈值 = 不在底部', () => {
    const el = container(1000, 400, 100) // 1000-100-400 = 500
    const { isUserAtBottom, handleScroll } = useChatScroll(ref(el))
    handleScroll()
    expect(isUserAtBottom.value).toBe(false)
  })

  it('完全到底（距底 0）= 在底部', () => {
    const el = container(1000, 400, 600)
    const { isUserAtBottom, handleScroll } = useChatScroll(ref(el))
    handleScroll()
    expect(isUserAtBottom.value).toBe(true)
  })

  it('容器未挂载时不抛，也不改状态（首帧 ref 还是 null）', () => {
    const { isUserAtBottom, handleScroll, scrollToBottom, smartScroll } = useChatScroll(ref(null))
    expect(() => {
      handleScroll()
      scrollToBottom()
      smartScroll()
    }).not.toThrow()
    expect(isUserAtBottom.value).toBe(true)
  })
})

describe('useChatScroll — 滚动动作', () => {
  it('scrollToBottom 落到 scrollTop = scrollHeight', () => {
    const el = container(1000, 400, 0)
    const { scrollToBottom } = useChatScroll(ref(el))
    scrollToBottom()
    expect(el.scrollTop).toBe(1000)
  })

  it('smartScroll：在底部时才滚', () => {
    const el = container(1000, 400, 560)
    const { handleScroll, smartScroll } = useChatScroll(ref(el))
    handleScroll()
    smartScroll()
    expect(el.scrollTop).toBe(1000)
  })

  it('smartScroll：不在底部时是空操作（不把用户拽回底部）', () => {
    const el = container(1000, 400, 100)
    const { handleScroll, smartScroll } = useChatScroll(ref(el))
    handleScroll()
    smartScroll()
    expect(el.scrollTop).toBe(100)
  })

  it('用户翻上去之后再翻回底部，自动滚动随之恢复', () => {
    const el = container(1000, 400, 560)
    const { isUserAtBottom, handleScroll, smartScroll } = useChatScroll(ref(el))
    handleScroll()
    expect(isUserAtBottom.value).toBe(true)
    // 用户翻上去
    el.scrollTop = 100
    handleScroll()
    expect(isUserAtBottom.value).toBe(false)
    smartScroll()
    expect(el.scrollTop).toBe(100)
    // 再翻回底部
    el.scrollTop = 560
    handleScroll()
    smartScroll()
    expect(el.scrollTop).toBe(1000)
  })
})

/**
 * 「离底多远」是滚动位置**唯一可记忆的表示**（`scrollTop` 在渲染窗口下没有稳定含义，
 * 见本文件模块头与 `stores/sessionUiState`）。这两条把换算钉成一份实现：
 * 贴底判定与还原必须读同一个数，否则会出现「界面上算在底部、存下来却按不在底部还原」。
 */
describe('useChatScroll — 离底距离（记忆与还原）', () => {
  it('gapFromBottom 与阈值判定同源：同一个位置，两个读法一致', () => {
    const el = container(1000, 400, 100) // 离底 500
    const { gapFromBottom, handleScroll, isUserAtBottom } = useChatScroll(ref(el))
    expect(gapFromBottom()).toBe(500)
    handleScroll()
    expect(isUserAtBottom.value).toBe(false)
    el.scrollTop = 599 // 1000-599-400 = 1，离底 1
    expect(gapFromBottom()).toBe(1)
    handleScroll()
    expect(isUserAtBottom.value).toBe(true)
  })

  it('容器未挂载时给 null（那不是「离底 0」——存下来会凭空造一条在底部的记录）', () => {
    const { gapFromBottom } = useChatScroll(ref(null))
    expect(gapFromBottom()).toBeNull()
  })

  it('scrollToGap 回到离底同样远处', () => {
    const el = container(1000, 400, 0)
    const { scrollToGap, gapFromBottom } = useChatScroll(ref(el))
    scrollToGap(250)
    expect(el.scrollTop).toBe(350) // 1000-400-250
    expect(gapFromBottom()).toBe(250)
  })

  it('记下的位置比现有内容还靠上 ⇒ 落到能到的最上处，不抛也不越界', () => {
    // 会话被截断 / 消息被删之后再切回来：内容比离开时短是常态
    const el = container(600, 400, 0)
    const { scrollToGap } = useChatScroll(ref(el))
    scrollToGap(5000)
    expect(el.scrollTop).toBe(0)
  })

  it('scrollToGap(0) 即贴底（与 scrollToBottom 落在同一处）', () => {
    const el = container(1000, 400, 10)
    const { scrollToGap } = useChatScroll(ref(el))
    scrollToGap(0)
    expect(el.scrollTop).toBe(600) // 1000-400
  })

  it('容器未挂载时 scrollToGap 是空操作（不抛）', () => {
    const { scrollToGap } = useChatScroll(ref(null))
    expect(() => scrollToGap(120)).not.toThrow()
  })
})

/**
 * useToast — 全局浮动消息单测（happy-dom）
 *
 * 它是全站**唯一**的 Toast 状态源：模块级单例，任意模块 `useToast()` 都共享
 * 同一份（浮层 `Toast.vue` 挂在 MainLayout 里，只渲染一次）。单例的代价是
 * **测试之间会互相污染**，所以每条用例开头都要清场——这不是可选步骤。
 *
 * 钉住：
 * 1. 单例：两处调用拿到的是**同一个数组**（不是各持一份副本）。
 * 2. `dismiss` 不存在的 id 是**空操作**，不抛也不误删（并发关闭时很容易撞上）。
 * 3. 到期自动消失靠 `setTimeout`，且**按 id 移除**——不是「按位置移除」，
 *    否则先弹出的那条到期时会把后弹出的那条删掉。
 * 4. id 单调递增：先来的 id 小，多条并存时顺序可预期。
 */

// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useToast } from '../useToast'

/** 单例状态跨用例共享，必须清场 */
function clear() {
  const { toasts } = useToast()
  toasts.value.splice(0, toasts.value.length)
}

beforeEach(() => {
  vi.useRealTimers()
  clear()
})

describe('useToast — 单例', () => {
  it('两处调用共享同一份状态', () => {
    const a = useToast()
    const b = useToast()
    expect(b.toasts).toBe(a.toasts)
    a.showToast('info', '一条')
    expect(b.toasts.value).toHaveLength(1)
  })

  it('id 单调递增（先来的小）', () => {
    const { toasts, showToast } = useToast()
    showToast('info', '第一条')
    showToast('error', '第二条')
    expect(toasts.value[0].id).toBeLessThan(toasts.value[1].id)
  })
})

describe('useToast — 入队与移除', () => {
  it('showToast 带齐 type / text', () => {
    const { toasts, showToast } = useToast()
    showToast('success', '已保存')
    expect(toasts.value[0]).toMatchObject({ type: 'success', text: '已保存' })
  })

  it('dismiss 按 id 精确移除（多条并存时只删指定的那条）', () => {
    const { toasts, showToast, dismiss } = useToast()
    showToast('info', 'a')
    showToast('info', 'b')
    const [first, second] = toasts.value
    dismiss(second.id)
    expect(toasts.value.map((t) => t.text)).toEqual(['a'])
    dismiss(first.id)
    expect(toasts.value).toHaveLength(0)
  })

  it('dismiss 不存在的 id 是空操作：不抛、也不误删', () => {
    const { toasts, showToast, dismiss } = useToast()
    showToast('info', 'a')
    expect(() => dismiss(999999)).not.toThrow()
    expect(toasts.value).toHaveLength(1)
  })

  it('到期自动消失（默认 3000ms）', () => {
    vi.useFakeTimers()
    const { toasts, showToast } = useToast()
    showToast('info', '会自动消失')
    expect(toasts.value).toHaveLength(1)
    vi.advanceTimersByTime(2999)
    expect(toasts.value).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(toasts.value).toHaveLength(0)
    vi.useRealTimers()
  })

  it('自定义时长生效', () => {
    vi.useFakeTimers()
    const { toasts, showToast } = useToast()
    showToast('info', '短命', 100)
    vi.advanceTimersByTime(100)
    expect(toasts.value).toHaveLength(0)
    vi.useRealTimers()
  })

  it('先弹出的那条到期时，不会删掉后弹出的那条（按 id 而非按位置）', () => {
    vi.useFakeTimers()
    const { toasts, showToast } = useToast()
    showToast('info', '先', 100)
    vi.advanceTimersByTime(10)
    showToast('info', '后', 1000)
    vi.advanceTimersByTime(90) // 第一条到期
    expect(toasts.value.map((t) => t.text)).toEqual(['后'])
    vi.useRealTimers()
  })
})

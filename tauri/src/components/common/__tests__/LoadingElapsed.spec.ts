// @vitest-environment happy-dom
/**
 * LoadingElapsed — 长加载期间「已等待 N 秒」的报时
 *
 * 钉住：前 3 秒不报时（瞬时加载不该闪一下）、之后每秒递增、卸载时停表
 * （漏清 interval 会让计时器在后台一直跑）。
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import LoadingElapsed from '../LoadingElapsed.vue'

afterEach(() => {
  vi.useRealTimers()
})

describe('LoadingElapsed', () => {
  it('前 3 秒不显示（避免瞬时加载闪一下）', () => {
    vi.useFakeTimers()
    const w = mount(LoadingElapsed)
    expect(w.text()).toBe('')
    vi.advanceTimersByTime(2000)
    expect(w.text()).toBe('')
  })

  it('超过 3 秒后按秒报时', async () => {
    vi.useFakeTimers()
    const w = mount(LoadingElapsed)
    vi.advanceTimersByTime(3000)
    await w.vm.$nextTick()
    expect(w.text()).toBe('已等待 3 秒')
    vi.advanceTimersByTime(2000)
    await w.vm.$nextTick()
    expect(w.text()).toBe('已等待 5 秒')
  })

  it('卸载即停表（不留下后台计时器）', () => {
    vi.useFakeTimers()
    const w = mount(LoadingElapsed)
    w.unmount()
    expect(vi.getTimerCount()).toBe(0)
  })
})

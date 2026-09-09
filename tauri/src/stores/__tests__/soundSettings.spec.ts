// @vitest-environment happy-dom
/**
 * soundSettings store 单元测试
 *
 * 验证：默认值、localStorage 持久化恢复、持久化写入、isKindEnabled 门控
 */
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useSoundSettingsStore } from '../soundSettings'

const STORAGE_KEY = 'symbio.completion-sound'

describe('soundSettings store', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
  })

  it('默认值：总开关开、三类型全开、音量 0.6', () => {
    const s = useSoundSettingsStore()
    expect(s.enabled).toBe(true)
    expect(s.kinds).toEqual({ completed: true, aborted: true, failed: true })
    expect(s.volume).toBe(0.6)
  })

  it('isKindEnabled = 总开关 && 分类型开关', () => {
    const s = useSoundSettingsStore()
    expect(s.isKindEnabled('failed')).toBe(true)
    s.kinds.failed = false
    expect(s.isKindEnabled('failed')).toBe(false)
    s.kinds.failed = true
    s.enabled = false
    expect(s.isKindEnabled('failed')).toBe(false)
  })

  it('从 localStorage 恢复持久化设置', () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ enabled: false, kinds: { completed: false }, volume: 0.25 }),
    )
    const s = useSoundSettingsStore()
    expect(s.enabled).toBe(false)
    expect(s.kinds.completed).toBe(false)
    expect(s.kinds.aborted).toBe(true) // 未持久化的类型保持默认
    expect(s.volume).toBe(0.25)
  })

  it('修改设置会写回 localStorage', () => {
    const s = useSoundSettingsStore()
    s.enabled = false
    s.volume = 0.8
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) || '{}')
    expect(raw.enabled).toBe(false)
    expect(raw.volume).toBe(0.8)
    expect(raw.kinds).toEqual({ completed: true, aborted: true, failed: true })
  })

  it('音量写入时夹取到 [0, 1]', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ volume: 5 }))
    const s = useSoundSettingsStore()
    expect(s.volume).toBe(1)
  })

  it('损坏的持久化数据不抛错，按默认值处理', () => {
    localStorage.setItem(STORAGE_KEY, '{broken json')
    expect(() => useSoundSettingsStore()).not.toThrow()
    const s = useSoundSettingsStore()
    expect(s.enabled).toBe(true)
  })
})

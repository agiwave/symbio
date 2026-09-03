// @vitest-environment happy-dom
import { describe, it, expect, beforeEach } from 'vitest'
import { nextTick } from 'vue'
import { setActivePinia, createPinia } from 'pinia'
import { useAppearanceStore } from '../appearance'

const STORAGE_KEY = 'symbio.appearance'

describe('appearance store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
    // 重置根节点状态，避免跨用例污染
    document.documentElement.removeAttribute('data-theme')
    document.documentElement.style.removeProperty('--font-scale')
  })

  it('默认浅色 + 中号字体', () => {
    const store = useAppearanceStore()
    expect(store.theme).toBe('light')
    expect(store.fontSize).toBe('medium')
  })

  it('修改主题后写入 data-theme 并持久化', async () => {
    const store = useAppearanceStore()
    store.theme = 'dark'
    await nextTick()

    expect(document.documentElement.getAttribute('data-theme')).toBe('dark')
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)?.theme).toBe('dark')
  })

  it('修改字体后会调整 <html> 缩放系数并持久化', async () => {
    const store = useAppearanceStore()
    store.fontSize = 'large'
    await nextTick()

    expect(document.documentElement.style.getPropertyValue('--font-scale')).toBe('1.125')
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)?.fontSize).toBe('large')
  })

  it('新实例会从 localStorage 恢复已保存的外观', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ theme: 'auto', fontSize: 'small' }))
    const store = useAppearanceStore()
    expect(store.theme).toBe('auto')
    expect(store.fontSize).toBe('small')
  })
})
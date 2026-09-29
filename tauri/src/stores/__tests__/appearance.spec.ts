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

/**
 * 会话分栏开关（S5）。
 *
 * 为什么值得单独一组：它是**唯一一个不影响根节点**的外观项（由 `ModelChatPanel`
 * 直接读），因此前三条用例覆盖不到它——`apply()` 里漏写它、`watch` 里漏监听它、
 * 或恢复时漏读它，都不会让那三条变红。而它的**平凡值**（`false` = 单列，即引入
 * 分栏之前的行为）正是方案 §7.1 承诺"可被平凡值验证"的那一条：关掉分栏必须真的
 * 只显示一列，且这个选择要能活过重启。
 */
describe('appearance store：会话分栏', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
  })

  it('出厂即分栏（用户可见层面完整），且写入持久化', async () => {
    const store = useAppearanceStore()
    expect(store.dialogPanelSplit).toBe(true)
    store.apply()
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)?.dialogPanelSplit).toBe(true)
  })

  it('平凡值 false 能存下来 —— 关掉分栏是可回退的，不是一次性的', async () => {
    const store = useAppearanceStore()
    store.dialogPanelSplit = false
    await nextTick()
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)?.dialogPanelSplit).toBe(false)
  })

  it('新实例恢复已保存的 false（否则"关掉分栏"重启就失效）', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ dialogPanelSplit: false }))
    expect(useAppearanceStore().dialogPanelSplit).toBe(false)
  })

  it('旧数据缺这个键 ⇒ 落出厂值 true（不因缺字段退化成单列）', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ theme: 'dark', fontSize: 'large' }))
    const store = useAppearanceStore()
    expect(store.dialogPanelSplit).toBe(true)
    expect(store.theme).toBe('dark')
  })
})
// @vitest-environment happy-dom
/**
 * Appearance — 后端权威与首屏快照的分工（D-521）
 *
 * 三条链一断就会退回「纯前端自持」：后端配置到达要覆盖快照、用户改动要写回完整
 * 配置、后端缺席要保留首屏值。这里逐条钉住。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import Appearance from '../Appearance.vue'
import { useAppearanceStore } from '@/stores/appearance'
import { useSoundSettingsStore } from '@/stores/soundSettings'
import type { VdfsItem } from '@/schemas/vdfs'

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ showToast: vi.fn() }),
}))

vi.mock('@/services/completionChime', () => ({
  playCompletionChime: vi.fn(),
  DEFAULT_CHIME_VOLUME: 0.6,
}))

function prefNode(): VdfsItem {
  return {
    path: '@vfs/plugin_manager/appearance',
    name: 'appearance',
    title: '外观',
    kind: 'ui_prefs',
    status: '',
    access: 'rw',
    ext: 'appearance',
  }
}

function backend(overrides: Record<string, unknown> = {}): string {
  return JSON.stringify({
    theme: 'dark',
    font_size: 'large',
    sound_enabled: true,
    sound_completed: true,
    sound_aborted: true,
    sound_failed: true,
    volume: 0.6,
    ...overrides,
  })
}

function mountPanel(data: unknown) {
  const pinia = createPinia()
  setActivePinia(pinia)
  // matchMedia 在 happy-dom 里可能缺席；store 创建时就要用
  if (!window.matchMedia) {
    window.matchMedia = vi.fn().mockImplementation((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    }))
  }
  return mount(Appearance, {
    global: { plugins: [pinia] },
    props: { node: prefNode(), data },
  })
}

describe('Appearance 的后端权威与首屏快照', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('后端配置到达时覆盖首屏快照（外观与提示音都对齐）', async () => {
    const w = mountPanel(backend({ volume: 0.8, sound_enabled: false }))
    await nextTick()

    const appearance = useAppearanceStore()
    const sound = useSoundSettingsStore()
    expect(appearance.theme).toBe('dark')
    expect(appearance.fontSize).toBe('large')
    expect(sound.enabled).toBe(false)
    expect(sound.volume).toBe(0.8)

    expect(w.emitted('save')).toBeFalsy()
  })

  it('用户改动写回完整配置 JSON，而不是只交字段差量', async () => {
    const w = mountPanel(backend())
    await nextTick()

    // 主题分组的第一个按钮 = 浅色
    await w.findAll('.seg-btn')[0].trigger('click')

    const saved = w.emitted('save')
    expect(saved).toBeTruthy()
    const payload = JSON.parse(String(saved?.[0]?.[0])) as Record<string, unknown>
    expect(payload.theme).toBe('light')
    for (const key of [
      'theme',
      'font_size',
      'sound_enabled',
      'sound_completed',
      'sound_aborted',
      'sound_failed',
      'volume',
    ]) {
      expect(payload, `缺少字段 ${key}`).toHaveProperty(key)
    }
  })

  it('后端缺席时退回首屏快照，且绝不回写（避免空配置覆盖权威值）', async () => {
    localStorage.setItem(
      'symbio.appearance',
      JSON.stringify({ theme: 'dark', fontSize: 'large' }),
    )
    const w = mountPanel(undefined)
    await nextTick()

    expect(useAppearanceStore().theme).toBe('dark')
    expect(useAppearanceStore().fontSize).toBe('large')
    expect(w.emitted('save')).toBeFalsy()
  })

  it('损坏的后端正文按缺席处理，不抛错也不清空首屏值', async () => {
    localStorage.setItem('symbio.appearance', JSON.stringify({ theme: 'dark' }))
    const w = mountPanel('{not json')
    await nextTick()

    expect(useAppearanceStore().theme).toBe('dark')
    expect(w.emitted('save')).toBeFalsy()
  })
})

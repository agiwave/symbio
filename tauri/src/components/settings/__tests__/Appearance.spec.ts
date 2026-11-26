// @vitest-environment happy-dom
/**
 * Appearance — 后端权威、首屏快照与自动保存串行化的分工
 *
 * 后端配置到达要覆盖快照、用户改动要写回完整配置、后端缺席要保留首屏值；
 * 写在途的连续修改合并为最新快照，不能被单飞父层丢弃或被旧回包覆盖。
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

// 引导重开入口：只验证「按钮 → openManually」这条接线，真正的行为归 onboarding store 自己测
const openManually = vi.fn()
vi.mock('@/stores/onboarding', () => ({
  useOnboardingStore: () => ({ openManually }),
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

function mountPanel(data: unknown, saving = false) {
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
    props: { node: prefNode(), data, saving },
  })
}

function savedPayloads(w: ReturnType<typeof mountPanel>): Array<Record<string, unknown>> {
  return (w.emitted('save') ?? []).map(([payload]) => JSON.parse(String(payload)) as Record<string, unknown>)
}

async function setVolume(w: ReturnType<typeof mountPanel>, value: number): Promise<void> {
  const input = w.find('.volume-input')
  const element = input.element as HTMLInputElement
  element.value = String(value)
  await input.trigger('input')
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

describe('Appearance 自动保存的串行合并', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('第一次修改立即发出完整快照', async () => {
    const w = mountPanel(backend())
    await nextTick()

    await w.findAll('.segmented')[0].findAll('button')[0].trigger('click')

    expect(savedPayloads(w)).toEqual([JSON.parse(backend({ theme: 'light' }))])
  })

  it('在途第二次、第三次修改只合并成最新一次', async () => {
    const w = mountPanel(backend())
    await nextTick()
    await w.findAll('.segmented')[0].findAll('button')[0].trigger('click')
    await w.setProps({ saving: true })

    await w.findAll('.segmented')[0].findAll('button')[2].trigger('click')
    await w.findAll('.segmented')[1].findAll('button')[0].trigger('click')

    expect(savedPayloads(w)).toEqual([JSON.parse(backend({ theme: 'light' }))])
  })

  it('第一次保存结束后补发最新完整 payload，且旧回包不让快照倒退', async () => {
    const w = mountPanel(backend())
    await nextTick()
    await w.findAll('.segmented')[0].findAll('button')[0].trigger('click')
    await w.setProps({ saving: true })
    await w.findAll('.segmented')[0].findAll('button')[2].trigger('click')
    await w.findAll('.segmented')[1].findAll('button')[2].trigger('click')

    // 模拟父层刷新时短暂下发上一次保存的旧正文。
    await w.setProps({ data: backend() })
    expect(useAppearanceStore().theme).toBe('auto')
    expect(useAppearanceStore().fontSize).toBe('large')

    await w.setProps({ saving: false })

    expect(savedPayloads(w)).toEqual([
      JSON.parse(backend({ theme: 'light' })),
      JSON.parse(backend({ theme: 'auto', font_size: 'large' })),
    ])
  })

  it('旧回包出现后的下一次修改仍保留用户已选字段', async () => {
    const w = mountPanel(backend())
    await nextTick()
    await w.findAll('.segmented')[0].findAll('button')[0].trigger('click')
    await w.setProps({ saving: true })

    await w.setProps({ data: backend() })
    await w.findAll('.segmented')[1].findAll('button')[2].trigger('click')
    await w.setProps({ saving: false })

    expect(savedPayloads(w)[1]).toEqual(JSON.parse(backend({ theme: 'light', font_size: 'large' })))
  })

  it('无修改不重复 emit，包括 saving 初始 false 与空闲边沿', async () => {
    const w = mountPanel(backend())
    await nextTick()
    expect(savedPayloads(w)).toEqual([])

    await w.setProps({ saving: true })
    await w.setProps({ saving: false })
    expect(savedPayloads(w)).toEqual([])

    await w.findAll('.segmented')[0].findAll('button')[0].trigger('click')
    await w.setProps({ saving: true })
    await w.setProps({ saving: false })
    await w.setProps({ saving: false })

    expect(savedPayloads(w)).toEqual([JSON.parse(backend({ theme: 'light' }))])
  })

  it('saving 同一轮 true → false 也能补发队列快照', async () => {
    const w = mountPanel(backend())
    await nextTick()
    await w.findAll('.segmented')[0].findAll('button')[0].trigger('click')

    const busy = w.setProps({ saving: true })
    const edit = w.findAll('.segmented')[0].findAll('button')[2].trigger('click')
    const idle = w.setProps({ saving: false })
    await Promise.all([busy, edit, idle])

    expect(savedPayloads(w)).toEqual([
      JSON.parse(backend({ theme: 'light' })),
      JSON.parse(backend({ theme: 'auto' })),
    ])
  })

  it('volume 快速输入只保留最后值', async () => {
    const w = mountPanel(backend())
    await nextTick()
    await setVolume(w, 0.2)
    await w.setProps({ saving: true })

    await setVolume(w, 0.4)
    await setVolume(w, 0.65)
    await setVolume(w, 0.9)
    expect(savedPayloads(w)).toHaveLength(1)

    await w.setProps({ saving: false })

    expect(savedPayloads(w)).toHaveLength(2)
    expect(savedPayloads(w)[1].volume).toBe(0.9)
  })
})

describe('Appearance 的原生语义与无障碍预览', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('纯视觉代码片段不占用键盘 Tab 顺序', () => {
    const w = mountPanel(backend())
    const chip = w.find('.preview-chip')
    expect(chip.element.tagName).toBe('SPAN')
    expect(w.findAll('button').some((button) => button.text() === '代码片段')).toBe(false)
  })

  it('音量滑块保留原生 range 语义与可读百分比', () => {
    const w = mountPanel(backend({ volume: 0.65 }))
    const slider = w.find<HTMLInputElement>('.volume-input')
    expect(slider.element.type).toBe('range')
    expect(slider.attributes('aria-label')).toBe('提示音音量')
    expect(slider.attributes('aria-valuetext')).toBe('65%')
    expect(slider.element.min).toBe('0')
    expect(slider.element.max).toBe('1')
  })
})

/** 引导重开：跳过后仍能在这里找回——钉住 UI 接线（DE-020 §10 的落点从已移除的 About 迁到本面板） */
describe('Appearance 的新手引导重开入口', () => {
  beforeEach(() => {
    localStorage.clear()
    openManually.mockClear()
  })

  it('点「重新查看」调用 openManually，且不触发配置写回', async () => {
    const w = mountPanel(backend())
    await nextTick()

    const btn = w.findAll('button').find((b) => b.text().includes('重新查看'))
    expect(btn, '应有一个可见的引导重开按钮').toBeTruthy()
    await btn!.trigger('click')

    expect(openManually).toHaveBeenCalledOnce()
    expect(w.emitted('save')).toBeFalsy()
  })
})

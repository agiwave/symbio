// @vitest-environment happy-dom
/**
 * completionChime 单元测试
 *
 * 核心验证：
 * 1. 三种结束类型（completed/aborted/failed）音色参数互不相同
 * 2. 总开关 / 分类型开关 / force 的门控逻辑
 * 3. 同会话去重窗口（Error 后紧跟 Abort 只响一次）
 *
 * 音频部分用桩 AudioContext，不真实发声。
 * 发声次数以 oscillator.start() 调用数度量：每种提示音 = 2 个音 = 2 次 start。
 */
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import {
  playCompletionChime,
  setChimeSettingsSource,
  _resetChimeForTest,
  CHIME_TONES,
} from '../completionChime'
// service 自身不认识 store —— 设置来源在这里注入（生产侧由 MainLayout 注入），
// 因此「改开关 → 不响」这条链路照旧可测，只是接线权在调用方。
import { useSoundSettingsStore } from '@/stores/soundSettings'

class FakeAudioContext {
  state = 'running'
  currentTime = 10
  destination = {}
  createOscillator() {
    // connect 需可链式：osc.connect(gain).connect(destination)
    return {
      type: 'sine',
      frequency: { value: 0 },
      connect: vi.fn((t: unknown) => t),
      start: vi.fn(() => FakeAudioContext.starts++),
      stop: vi.fn(),
    }
  }
  createGain() {
    return {
      gain: { setValueAtTime: vi.fn(), linearRampToValueAtTime: vi.fn(), exponentialRampToValueAtTime: vi.fn() },
      connect: vi.fn((t: unknown) => t),
    }
  }
  resume() {
    return Promise.resolve()
  }
  /** oscillator.start 累计调用数（= 已发出的音数；每次 chime 播 2 个音） */
  static starts = 0
}

/** 已播放的音数（2 音/次 chime → starts/2 = chime 次数） */
function starts() {
  return FakeAudioContext.starts
}

describe('completionChime', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
    _resetChimeForTest()
    FakeAudioContext.starts = 0
    vi.stubGlobal('AudioContext', FakeAudioContext as unknown as typeof AudioContext)
    setChimeSettingsSource(() => {
      const s = useSoundSettingsStore()
      return { enabled: (kind) => s.isKindEnabled(kind), volume: s.volume }
    })
  })

  it('未注入设置来源时按「全开 + 默认音量」兜底发声（service 不依赖 store 也自洽）', () => {
    _resetChimeForTest() // 清空上一用例注入的来源
    expect(playCompletionChime('completed', 'sess-nosrc')).toBe(true)
  })

  it('三种结束类型使用互不相同的音色参数', () => {
    const keys = (['completed', 'aborted', 'failed'] as const).map((k) => JSON.stringify(CHIME_TONES[k]))
    expect(new Set(keys).size).toBe(3)
  })

  it('正常结束为上行双音，中止为下行双音，失败为低频双响', () => {
    expect(CHIME_TONES.completed.freq2).toBeGreaterThan(CHIME_TONES.completed.freq)
    expect(CHIME_TONES.aborted.freq2).toBeLessThan(CHIME_TONES.aborted.freq)
    expect(CHIME_TONES.failed.freq2).toBeLessThan(CHIME_TONES.failed.freq)
    expect(CHIME_TONES.failed.freq).toBeLessThan(CHIME_TONES.completed.freq)
  })

  it('总开关关闭时不播放', () => {
    const settings = useSoundSettingsStore()
    settings.enabled = false
    expect(playCompletionChime('completed', 'sess-1')).toBe(false)
    expect(starts()).toBe(0)
  })

  it('分类型关闭时不播放该类型', () => {
    const settings = useSoundSettingsStore()
    settings.kinds.completed = false
    expect(playCompletionChime('completed', 'sess-2')).toBe(false)
    expect(starts()).toBe(0)
    // 其他类型不受影响
    expect(playCompletionChime('failed', 'sess-2b')).toBe(true)
    expect(starts()).toBe(2)
  })

  it('同一会话一轮结束（Error 后紧跟 Abort）只响一次', () => {
    expect(playCompletionChime('failed', 'sess-dedup')).toBe(true)
    expect(playCompletionChime('aborted', 'sess-dedup')).toBe(false)
    expect(starts()).toBe(2)
  })

  it('不同会话互不影响去重', () => {
    expect(playCompletionChime('failed', 'sess-a')).toBe(true)
    expect(playCompletionChime('aborted', 'sess-b')).toBe(true)
    expect(starts()).toBe(4)
  })

  it('force 绕过门控与去重（设置页试听）', () => {
    const settings = useSoundSettingsStore()
    settings.enabled = false
    // enabled=false 不响、不占去重窗口
    expect(playCompletionChime('failed', 'sess-force')).toBe(false)
    expect(playCompletionChime('failed', 'sess-force', { force: true })).toBe(true)
    expect(starts()).toBe(2)
  })

  it('AudioContext 不可用时静默跳过并返回 false', () => {
    vi.stubGlobal('AudioContext', undefined)
    expect(playCompletionChime('completed', 'sess-noctx')).toBe(false)
    expect(starts()).toBe(0)
  })

  it('试听（无 sessionId）不去重', () => {
    expect(playCompletionChime('completed')).toBe(true)
    expect(playCompletionChime('completed')).toBe(true)
    expect(starts()).toBe(4)
  })
})

/**
 * completionChime — 会话结束提示音服务
 *
 * ## 目标
 *
 * 任何会话收到结束事件（正常结束 / 用户中止 / 异常失败）时播放一段短提示音，
 * 三种终态使用不同音色，便于用户在不盯着界面的情况下分辨会话结局：
 *
 * - `completed`：两音上行（C5→G5），明亮 = 「干完了」
 * - `aborted`：双音下行（E5→C5），平缓 = 「停了」
 * - `failed`：低频短促双响（A4→F4），低沉 = 「出错了」
 *
 * ## 实现选择
 *
 * 用 Web Audio API 实时合成（振荡器 + 包络），**不引入音频资源文件**：
 * 零资源开销、体积零增加、免资源加载时序问题（Tauri WebView 下 audio 文件
 * 加载偶发延迟，合成音频没有这个问题）。
 *
 * ## 防抖
 *
 * 后端一轮交互结束可能连发多条终态事件（如 Abort 前已广播 Error，或 status
 * 与事件流各广播一次），用 (sessionId, kind) 去重窗口（1.5s）保证一个会话的
 * 一轮结束只响一次。
 */

import type { CompletionKind } from '@/schemas/vdfs'
import { logger } from '@/utils/logger'

const MODULE_TAG = 'completion-chime'
const DEDUP_WINDOW_MS = 1500

/**
 * 默认音量（0~1）。
 *
 * `stores/soundSettings` 的初值也取自这里——「默认多响」只应有一个答案，
 * 两处各写一个数迟早会漂移。
 */
export const DEFAULT_CHIME_VOLUME = 0.6

/**
 * 提示音设置（**依赖倒置**：本模块是 service，不反向依赖 Pinia store）。
 *
 * 「该不该响」与「响多大声」由**设置来源**回答，来源由应用外壳（`MainLayout`）
 * 注入——本模块只管发声，因此可脱离 Pinia 单独测试。
 */
export interface ChimeSettings {
  /** 某类型当前是否应当响铃（总开关 × 分类型开关） */
  enabled(kind: CompletionKind): boolean
  /** 音量 0~1 */
  volume: number
}

export interface ChimeOptions {
  /** 试听：绕过设置门控与去重（设置页专用） */
  force?: boolean
}

/** 未注入来源时的兜底：全部开启 + 默认音量（保证未接线的场景仍会响） */
const FALLBACK_SETTINGS: ChimeSettings = { enabled: () => true, volume: DEFAULT_CHIME_VOLUME }

let _settingsSource: () => ChimeSettings = () => FALLBACK_SETTINGS

/**
 * 注册设置来源（应用外壳调用一次）。
 *
 * 传函数而非快照：用户随时可能在设置页改开关与音量，每次发声都要读到最新值。
 */
export function setChimeSettingsSource(fn: () => ChimeSettings): void {
  _settingsSource = fn
}

/** 音色参数：与 CompletionKind 一一对应（导出供设置 UI 文案与测试断言） */
export const CHIME_TONES: Record<CompletionKind, { freq: number; freq2?: number; duration: number; type: OscillatorType; gap: number }> = {
  // 两音上行，明亮收敛
  completed: { freq: 523.25, freq2: 783.99, duration: 0.16, type: 'sine', gap: 0.1 },
  // 双音下行，平缓柔和
  aborted: { freq: 659.25, freq2: 523.25, duration: 0.2, type: 'sine', gap: 0.12 },
  // 低频短促双响，警示但不刺耳
  failed: { freq: 440.0, freq2: 349.23, duration: 0.14, type: 'triangle', gap: 0.08 },
}

/** 复用全局 AudioContext（浏览器限制每个页面可创建的 context 数量） */
let _ctx: AudioContext | null = null
function getCtx(): AudioContext | null {
  try {
    if (!_ctx) {
      const Ctor: typeof AudioContext | undefined =
        (globalThis as { AudioContext?: typeof AudioContext; webkitAudioContext?: typeof AudioContext }).AudioContext ||
        (globalThis as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
      if (!Ctor) return null
      _ctx = new Ctor()
    }
    // 自动播放策略：context 可能处于 suspended，恢复后才有声音
    if (_ctx.state === 'suspended') void _ctx.resume().catch(() => {})
    return _ctx
  } catch {
    return null
  }
}

/** 去重窗口记录：同一会话一轮结束（可能连发多种终态事件）只响一次 */
const _recentBy = new Map<string, number>()

/** 供测试注入（清空去重状态、音频上下文缓存与设置来源） */
export function _resetChimeForTest(): void {
  _recentBy.clear()
  _ctx = null
  _settingsSource = () => FALLBACK_SETTINGS
}

/** 单音播放（osc + gain 包络：attack 短促起音、exponential 衰减收尾，无爆音） */
function playTone(
  ctx: AudioContext,
  freq: number,
  startAt: number,
  duration: number,
  type: OscillatorType,
  volume: number,
): void {
  const osc = ctx.createOscillator()
  const gain = ctx.createGain()
  osc.type = type
  osc.frequency.value = freq
  const attack = 0.008
  gain.gain.setValueAtTime(0, startAt)
  gain.gain.linearRampToValueAtTime(volume, startAt + attack)
  gain.gain.exponentialRampToValueAtTime(0.0001, startAt + duration)
  osc.connect(gain).connect(ctx.destination)
  osc.start(startAt)
  osc.stop(startAt + duration + 0.02)
}

/**
 * 播放会话结束提示音
 *
 * @param kind    结束类型（completed / aborted / failed）
 * @param sessionId 触发结束的会话 id，用于去重；传 undefined 表示不去重（如试听）
 * @param opts.force 为 true 时绕过设置门控与去重（设置页"试听"专用）
 * @returns 本次是否实际发出了声音（供测试断言）
 */
export function playCompletionChime(
  kind: CompletionKind,
  sessionId?: string,
  opts?: ChimeOptions,
): boolean {
  const settings = _settingsSource()

  // 去重：同一会话一轮结束（无论先 Error 后 Abort，还是连续多条 idle）只响一次。
  // 按 sessionId 记录而非 (sessionId, kind)，否则 Abort 紧跟 Error 时会响两声。
  if (sessionId && !opts?.force) {
    const now = Date.now()
    const last = _recentBy.get(sessionId)
    if (last !== undefined && now - last < DEDUP_WINDOW_MS) return false
    _recentBy.set(sessionId, now)
    // 顺带清理过期项，防止长期驻留缓慢膨胀
    if (_recentBy.size > 256) {
      for (const [key, ts] of _recentBy) {
        if (now - ts >= DEDUP_WINDOW_MS) _recentBy.delete(key)
      }
    }
  }

  // 设置门控：总开关或对应类型被关闭时不响（force 绕过，试听用）
  if (!opts?.force && !settings.enabled(kind)) return false

  const ctx = getCtx()
  if (!ctx) {
    logger.warn(MODULE_TAG, 'AudioContext unavailable, chime skipped')
    return false
  }

  try {
    const tone = CHIME_TONES[kind]
    const volume = Math.max(0, Math.min(1, settings.volume)) * 0.25 // 峰值增益封顶，避免刺耳
    const t0 = ctx.currentTime + 0.01
    playTone(ctx, tone.freq, t0, tone.duration, tone.type, volume)
    if (tone.freq2) {
      playTone(ctx, tone.freq2, t0 + tone.duration + tone.gap, tone.duration, tone.type, volume)
    }
    return true
  } catch (err) {
    logger.warn(MODULE_TAG, 'play chime failed', err)
    return false
  }
}

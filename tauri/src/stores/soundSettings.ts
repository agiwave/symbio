/**
 * 提示音设置 store — 会话结束提示音的开关与持久化
 *
 * 设计要点：
 * - 仅前端本地设置（与 appearance 同类）：localStorage 持久化，不经后端
 * - 总开关 + 分类型开关（正常结束 / 中止 / 失败），不同类型的提示音可独立关闭
 * - 音量 0~1，全局共用（与"结束提示音"这一功能域绑定，不按类型分别调音量）
 */

import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

const STORAGE_KEY = 'symbio.completion-sound'

/** 会话结束类型（与 sessionBusWatcher 的三个终态分支一一对应） */
export type CompletionKind = 'completed' | 'aborted' | 'failed'

interface PersistedSoundSettings {
  enabled?: boolean
  kinds?: Partial<Record<CompletionKind, boolean>>
  volume?: number
}

export const useSoundSettingsStore = defineStore('soundSettings', () => {
  /** 总开关：关闭后任何结束都不响铃（默认开启） */
  const enabled = ref(true)
  /** 分类型开关：completed=正常结束 / aborted=用户中止 / failed=异常结束 */
  const kinds = ref<Record<CompletionKind, boolean>>({
    completed: true,
    aborted: true,
    failed: true,
  })
  /** 音量 0~1 */
  const volume = ref(0.6)

  // ---- 持久化恢复（惰性单例，仅首次创建 store 时执行一次） ----
  let hydrated = false
  function hydrate() {
    if (hydrated) return
    hydrated = true
    try {
      const raw = localStorage.getItem(STORAGE_KEY)
      if (!raw) return
      const saved = JSON.parse(raw) as PersistedSoundSettings
      if (typeof saved.enabled === 'boolean') enabled.value = saved.enabled
      if (saved.kinds) {
        for (const k of Object.keys(kinds.value) as CompletionKind[]) {
          if (typeof saved.kinds[k] === 'boolean') kinds.value[k] = saved.kinds[k] as boolean
        }
      }
      if (typeof saved.volume === 'number' && Number.isFinite(saved.volume)) {
        volume.value = Math.min(1, Math.max(0, saved.volume))
      }
    } catch {
      // 损坏的持久化数据按默认值处理，不阻断启动
    }
  }
  hydrate()

  // ---- 持久化写入（任一设置变化即保存，同步 flush 保证写入即时生效） ----
  watch(
    [enabled, kinds, volume],
    () => {
      try {
        localStorage.setItem(
          STORAGE_KEY,
          JSON.stringify({ enabled: enabled.value, kinds: { ...kinds.value }, volume: volume.value } satisfies PersistedSoundSettings),
        )
      } catch {
        // 隐私模式等写入失败时静默忽略（仅影响下次启动的记忆）
      }
    },
    { deep: true, flush: 'sync' },
  )

  /** 某结束类型当前是否应当响铃（总开关与分类型开关取与） */
  function isKindEnabled(kind: CompletionKind): boolean {
    return enabled.value && kinds.value[kind] === true
  }

  return { enabled, kinds, volume, isKindEnabled }
})

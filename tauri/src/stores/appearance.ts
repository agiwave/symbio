/**
 * 外观设置 Store
 *
 * 统一管理主题（浅色 / 深色 / 跟随系统）与字体大小（小 / 中 / 大）。
 *
 * - 持久化：写入 localStorage（`symbio.appearance`），应用重启后自动恢复。
 * - 即时生效：修改 theme / fontSize 后立即应用到 `<html>` 根节点。
 * - 跟随系统：theme 为 `auto` 时，监听系统深色模式变化并实时切换。
 *
 * 应用方式说明：
 * - 主题：在 `<html data-theme=...>` 上设置 `light` / `dark`，
 *   各组件通过 `:root[data-theme="dark"]` 变量（见 App.vue）切换配色。
 * - 字体：通过修改 `<html>` 的 font-size 缩放全局 `rem` 尺寸，
 *   `medium` 对应 16px（浏览器默认），与改动前的视觉保持一致。
 */

import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

export type ThemeMode = 'light' | 'dark' | 'auto'
export type FontSize = 'small' | 'medium' | 'large'

/** localStorage 存储键 */
const STORAGE_KEY = 'symbio.appearance'

/** 各字体档位对应的<html>根字号（rem 基准），medium=16px 保持既有观感不变 */
const FONT_SIZE_MAP: Record<FontSize, number> = {
  small: 14,
  medium: 16,
  large: 18,
}

interface PersistedAppearance {
  theme?: ThemeMode
  fontSize?: FontSize
}

function loadPersisted(): PersistedAppearance {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return raw ? (JSON.parse(raw) as PersistedAppearance) : {}
  } catch {
    return {}
  }
}

export const useAppearanceStore = defineStore('appearance', () => {
  const saved = loadPersisted()
  const theme = ref<ThemeMode>(saved.theme ?? 'light')
  const fontSize = ref<FontSize>(saved.fontSize ?? 'medium')

  const darkMedia = window.matchMedia('(prefers-color-scheme: dark)')

  function resolveTheme(): 'light' | 'dark' {
    return theme.value === 'auto' ? (darkMedia.matches ? 'dark' : 'light') : theme.value
  }

  /** 将当前配置应用到 <html> 根节点，并持久化 */
  function apply() {
    document.documentElement.setAttribute('data-theme', resolveTheme())
    document.documentElement.style.fontSize = `${FONT_SIZE_MAP[fontSize.value]}px`
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ theme: theme.value, fontSize: fontSize.value })
    )
  }

  // 系统深色模式变化时，仅当处于"跟随系统"才需要重新解析
  darkMedia.addEventListener('change', () => {
    if (theme.value === 'auto') apply()
  })

  // 主题 / 字体改变 → 即时应用并持久化
  watch([theme, fontSize], () => apply())

  return { theme, fontSize, apply }
})
/**
 * useToast — 全局浮动消息（Toast）单例
 *
 * 取代原先散落在 AgentView / McpView / ModelProvidersView / SkillView 的 4 套
 * 本地 toast 实现。全站唯一的 Toast 状态源：任意模块 import useToast() 即共享
 * 同一条浮层（Toast.vue 挂在 MainLayout 中，仅渲染一次）。
 *
 * 视觉令牌化见 Toast.vue 的 scoped style（只消费 var(--*) 令牌）。
 */

import { ref } from 'vue'

export type ToastType = 'success' | 'error' | 'info'

export interface ToastItem {
  /** 唯一 id，用于单独移除 */
  id: number
  type: ToastType
  text: string
}

// ── 模块级单例状态（所有调用方共享同一份）─────────────────────────
const toasts = ref<ToastItem[]>([])
let seq = 0

/** 按 id 移除一条 toast */
function dismiss(id: number): void {
  const idx = toasts.value.findIndex((t) => t.id === id)
  if (idx !== -1) {
    toasts.value.splice(idx, 1)
  }
}

/**
 * 显示一条浮动消息
 * @param type  语义类型（success / error / info）
 * @param text  消息文本
 * @param duration 自动消失时长（ms），默认 3000
 */
function showToast(type: ToastType, text: string, duration = 3000): void {
  const id = ++seq
  toasts.value.push({ id, type, text })
  window.setTimeout(() => dismiss(id), duration)
}

export function useToast() {
  return { toasts, showToast, dismiss }
}

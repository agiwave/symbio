/**
 * useToast — 全局浮动消息（Toast）单例

 * 全站唯一的 Toast 状态源：任意模块 import useToast() 即共享同一条浮层
 * （Toast.vue 挂在 MainLayout 中，仅渲染一次）。
 /
 * 视觉令牌化见 Toast.vue 的 scoped style（只消费 var(--*) 令牌）。
 *
 * 计时可由消费方暂停 / 恢复（Toast.vue 在 hover / focus 时调用）：错误提示
 * 往往需要读两遍，3 秒自动消失对长文案不够用，而"手一移到上面就消失"更糟。
 */

import { ref } from 'vue'

export type ToastType = 'success' | 'error' | 'info'

/**
 * 提示里的一个可执行动作（如发送失败时的「重试」）。
 *
 * 只在提示文字本身不足以让用户完成下一步时使用：一句「可重试」而没有可点的重试，
 * 用户还得自己去找入口。动作是**可选的**——绝大多数提示只陈述事实。
 */
export interface ToastAction {
  label: string
  run: () => void
}

export interface ToastItem {
  /** 唯一 id，用于单独移除 */
  id: number
  type: ToastType
  text: string
  /** 可选的内联动作（按钮文案 + 回调） */
  action?: ToastAction
}

// ── 模块级单例状态（所有调用方共享同一份）─────────────────────────
const toasts = ref<ToastItem[]>([])
let seq = 0

interface TimerState {
  handle: ReturnType<typeof setTimeout>
  /** 本轮计时开始时刻 */
  startedAt: number
  /** 未走完的时长（暂停时累计下来） */
  remaining: number
}

const timers = new Map<number, TimerState>()

/** 默认停留时长：错误信息更长，给用户足够时间读完 */
const DEFAULT_DURATION: Record<ToastType, number> = {
  success: 3000,
  info: 3000,
  error: 6000,
}

/** 按 id 移除一条 toast */
function dismiss(id: number): void {
  const timer = timers.get(id)
  if (timer) {
    clearTimeout(timer.handle)
    timers.delete(id)
  }
  const idx = toasts.value.findIndex((t) => t.id === id)
  if (idx !== -1) {
    toasts.value.splice(idx, 1)
  }
}

function schedule(id: number, ms: number): void {
  timers.set(id, {
    handle: setTimeout(() => dismiss(id), ms),
    startedAt: Date.now(),
    remaining: ms,
  })
}

/** 暂停自动消失（鼠标悬停 / 键盘焦点进入时） */
function pause(id: number): void {
  const timer = timers.get(id)
  if (!timer) return
  clearTimeout(timer.handle)
  timer.remaining = Math.max(0, timer.remaining - (Date.now() - timer.startedAt))
}

/** 恢复自动消失（离开 / 失焦时） */
function resume(id: number): void {
  const timer = timers.get(id)
  if (!timer) return
  if (timer.remaining <= 0) {
    dismiss(id)
    return
  }
  timer.startedAt = Date.now()
  timer.handle = setTimeout(() => dismiss(id), timer.remaining)
}

/**
 * 显示一条浮动消息
 * @param type  语义类型（success / error / info）
 * @param text  消息文本
 * @param duration 自动消失时长（ms）；缺省按类型取默认值
 * @param action 可选内联动作（按钮），点击后先移除本条再执行
 */
function showToast(type: ToastType, text: string, duration?: number, action?: ToastAction): void {
  const id = ++seq
  toasts.value.push({ id, type, text, action })
  schedule(id, duration ?? DEFAULT_DURATION[type])
}

export function useToast() {
  return { toasts, showToast, dismiss, pause, resume }
}

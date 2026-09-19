/**
 * Toast — 全局浮层渲染单测（happy-dom）
 *
 * 浮层挂在 MainLayout 里、全站只渲染一次，消费 `useToast()` 的**模块级单例**。
 * 因此这里不断言「谁触发了它」，只断言浮层本身的三条约定：
 *
 * 1. 它是 **Teleport 到 body** 的：浮层不能被父级的 `overflow: hidden` 或
 *    层叠上下文裁掉——这是它挂在应用根之外的原因。
 * 2. 语义落到 class（`toast--success` / `error` / `info`），**颜色不由测试断言**
 *    （值是令牌，归 CSS）。
 * 3. 点击浮层可提前关闭（不必等 3 秒到期）；`aria-live` 与 `role=status`
 *    让读屏能播报。
 *
 * ⚠️ 单例状态跨用例共享，每条用例前必须清场。Teleport 的内容不在 wrapper 里，
 *    直接查 `document`。
 */

// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import Toast from '../Toast.vue'
import { useToast } from '@/composables/useToast'

function clear() {
  const { toasts } = useToast()
  toasts.value.splice(0, toasts.value.length)
}

const layer = () => document.querySelectorAll('.toast')

beforeEach(() => {
  vi.useRealTimers()
  clear()
  document.body.innerHTML = ''
})

describe('Toast — 浮层', () => {
  it('无 toast 时只有空容器（不渲染残留节点）', () => {
    mount(Toast, { attachTo: document.body })
    expect(document.querySelector('.toast-layer')).toBeTruthy()
    expect(layer()).toHaveLength(0)
  })

  it('渲染到 body（Teleport 出应用根，不被父级裁切）', () => {
    const { showToast } = useToast()
    showToast('info', '一条通知')
    mount(Toast, { attachTo: document.body })
    expect(layer()).toHaveLength(1)
    expect(document.querySelector('.toast-layer')?.parentElement).toBe(document.body)
  })

  it('语义类型落到 class', () => {
    const { showToast } = useToast()
    showToast('success', '成功')
    showToast('error', '失败')
    showToast('info', '提示')
    mount(Toast, { attachTo: document.body })
    const classes = Array.from(layer()).map((el) => el.className)
    expect(classes.some((c) => c.includes('toast--success'))).toBe(true)
    expect(classes.some((c) => c.includes('toast--error'))).toBe(true)
    expect(classes.some((c) => c.includes('toast--info'))).toBe(true)
  })

  it('文本原样展示', () => {
    const { showToast } = useToast()
    showToast('error', '保存失败：IPC 断了')
    mount(Toast, { attachTo: document.body })
    expect(document.querySelector('.toast__text')?.textContent).toBe('保存失败：IPC 断了')
  })

  it('点击浮层提前关闭（不用等到期）', async () => {
    const { showToast, toasts } = useToast()
    showToast('info', '点我关闭')
    mount(Toast, { attachTo: document.body })
    const el = layer()[0] as HTMLElement
    el.click()
    await Promise.resolve()
    expect(toasts.value).toHaveLength(0)
  })

  it('aria-live 让读屏能播报', () => {
    mount(Toast, { attachTo: document.body })
    const region = document.querySelector('.toast-layer')
    expect(region?.getAttribute('aria-live')).toBe('polite')
    expect(region?.getAttribute('role')).toBe('region')
  })
})

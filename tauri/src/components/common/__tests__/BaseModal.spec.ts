/**
 * BaseModal — 弹窗外壳单测（happy-dom）
 *
 * 它是 4 处弹窗（`ConfirmDialog` / `OptionFormDialog` / `HomedirSwitcher` /
 * `ModelChatPanel` 的编辑浮层）的**共同外壳**，而其中三处**只有**这一层提供
 * ESC 与焦点陷阱。所以下面的断言是那三处的可访问性护栏，不只是"公共组件的
 * 自测"——删掉这里任何一条，那三处就会静默退回"键盘关不掉"。
 *
 * 钉住四件事：
 * 1. **显隐**：`visible=false` 整个遮罩不渲染（`v-if`，不是隐藏）；
 * 2. **两条关闭途径**：ESC 与点遮罩都走 `close`；`dismissable=false` 时两条
 *    都要关掉（确认框 loading 期间不许被关走）；
 * 3. **焦点陷阱**：打开即聚焦首个可交互元素；Tab / Shift+Tab 在面板内循环；
 *    面板内没有可聚焦元素时 Tab 被拦下，而不是让焦点逃到遮罩后的页面；
 * 4. **类名契约**：机制类 `modal-mask` / `modal-panel` 恒在，语义类由
 *    `panelClass` 追加（测试与特化样式靠它定位）。
 */

// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import BaseModal from '../BaseModal.vue'

const SLOT = '<button class="first">一</button><button class="last">二</button>'

function mountModal(props: Record<string, unknown> = {}, slots: Record<string, string> = {}) {
  return mount(BaseModal, {
    props: { visible: true, ...props },
    slots: { default: SLOT, ...slots },
    attachTo: document.body,
  })
}

/** 在遮罩上派发一次键盘事件并返回它（用于检查是否被 preventDefault） */
function pressKey(w: ReturnType<typeof mountModal>, init: KeyboardEventInit) {
  const ev = new KeyboardEvent('keydown', { cancelable: true, bubbles: true, ...init })
  w.find('.modal-mask').element.dispatchEvent(ev)
  return ev
}

describe('BaseModal — 显隐与类名契约', () => {
  it('visible=false 时整个遮罩不渲染（不是隐藏）', () => {
    expect(mountModal({ visible: false }).find('.modal-mask').exists()).toBe(false)
  })

  it('机制类恒在，语义类由 panelClass 追加', () => {
    const w = mountModal({ panelClass: 'confirm-dialog' })
    expect(w.find('.modal-mask').exists()).toBe(true)
    expect(w.find('.modal-panel').classes()).toContain('confirm-dialog')
  })

  it('role 可指定（确认类用 alertdialog），且恒带 aria-modal', () => {
    const w = mountModal({ role: 'alertdialog', labelledby: 't1', describedby: 'd1' })
    const panel = w.find('.modal-panel')
    expect(panel.attributes('role')).toBe('alertdialog')
    expect(panel.attributes('aria-modal')).toBe('true')
    expect(panel.attributes('aria-labelledby')).toBe('t1')
    expect(panel.attributes('aria-describedby')).toBe('d1')
  })
})

describe('BaseModal — 两条关闭途径', () => {
  it('ESC 走 close', async () => {
    const w = mountModal()
    await w.find('.modal-mask').trigger('keydown', { key: 'Escape' })
    expect(w.emitted('close')).toHaveLength(1)
  })

  it('点遮罩走 close', async () => {
    const w = mountModal()
    await w.find('.modal-mask').trigger('click')
    expect(w.emitted('close')).toHaveLength(1)
  })

  it('dismissable=false 时两条都关不掉（如确认框 loading 中）', async () => {
    const w = mountModal({ dismissable: false })
    await w.find('.modal-mask').trigger('keydown', { key: 'Escape' })
    await w.find('.modal-mask').trigger('click')
    expect(w.emitted('close')).toBeUndefined()
  })
})

describe('BaseModal — 焦点陷阱', () => {
  it('打开即聚焦首个可交互元素', async () => {
    const w = mountModal()
    await flushPromises()
    expect(document.activeElement).toBe(w.find('.first').element)
  })

  it('Tab 在最后一个元素上时回到第一个（不逃出遮罩）', async () => {
    const w = mountModal()
    await flushPromises()
    ;(w.find('.last').element as HTMLElement).focus()
    const ev = pressKey(w, { key: 'Tab' })
    expect(ev.defaultPrevented).toBe(true)
    expect(document.activeElement).toBe(w.find('.first').element)
  })

  it('Shift+Tab 在第一个元素上时回到最后一个', async () => {
    const w = mountModal()
    await flushPromises()
    ;(w.find('.first').element as HTMLElement).focus()
    const ev = pressKey(w, { key: 'Tab', shiftKey: true })
    expect(ev.defaultPrevented).toBe(true)
    expect(document.activeElement).toBe(w.find('.last').element)
  })

  it('面板内无可聚焦元素时 Tab 被拦下（否则焦点会落到遮罩后的页面）', async () => {
    const w = mountModal({}, { default: '<span>纯文本，没有可聚焦元素</span>' })
    await flushPromises()
    const ev = pressKey(w, { key: 'Tab' })
    expect(ev.defaultPrevented).toBe(true)
  })

  it('非 Tab / ESC 的按键不干预（不吞掉正常输入）', async () => {
    const w = mountModal()
    await flushPromises()
    const ev = pressKey(w, { key: 'a' })
    expect(ev.defaultPrevented).toBe(false)
    expect(w.emitted('close')).toBeUndefined()
  })
})

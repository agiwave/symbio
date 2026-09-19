/**
 * ConfirmDialog — 二次确认对话框单测（happy-dom）
 *
 * 它替代浏览器原生 `confirm`，是**破坏性操作**（删除 / 切换系统目录）的最后一道
 * 闸门，因此这里钉的是「关不掉」和「误触发」两类回归：
 *
 * 1. **关闭途径完备**：取消按钮、点遮罩、ESC 三条都走 `cancel` + `update:visible`
 *    ——少一条就是「弹出来了关不掉」。
 * 2. **确认不自动关闭**：只 `emit('confirm')`，关不关由调用方决定（异步操作要等
 *    结果，不能先关再失败）。
 * 3. **loading 是硬闸门**：按钮禁用，且**点了也不 emit**（不是只靠 disabled 挡
 *    ——键盘与程序化触发仍可能进来）。
 * 4. **焦点陷阱**：打开即聚焦首个可交互元素；Tab 到最后一个时回到第一个。
 * 5. **可访问性**：`role=alertdialog` + `aria-labelledby` / `aria-describedby`。
 *
 * 注：`visible=false` 时整个遮罩不渲染（`v-if`，不是隐藏）。
 */

// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import ConfirmDialog from '../ConfirmDialog.vue'

function mountDialog(props: Record<string, unknown> = {}, slots: Record<string, string> = {}) {
  return mount(ConfirmDialog, {
    props: { visible: true, message: '确定吗？', ...props },
    slots,
    attachTo: document.body,
  })
}

describe('ConfirmDialog — 显隐与结构', () => {
  it('不可见时整个遮罩不渲染（不是隐藏）', () => {
    expect(mountDialog({ visible: false }).find('.confirm-overlay').exists()).toBe(false)
  })

  it('role=alertdialog 且标题/正文都挂上 aria 关联', () => {
    const w = mountDialog({ title: '删除会话' })
    const dialog = w.find('.confirm-dialog')
    expect(dialog.attributes('role')).toBe('alertdialog')
    expect(dialog.attributes('aria-labelledby')).toBeTruthy()
    expect(dialog.attributes('aria-describedby')).toBeTruthy()
    // 关联必须真的指向存在的节点，否则 aria 是空引用
    expect(w.find(`#${dialog.attributes('aria-labelledby')}`).exists()).toBe(true)
    expect(w.find(`#${dialog.attributes('aria-describedby')}`).exists()).toBe(true)
  })

  it('无标题时不渲染 header（不留一个空条）', () => {
    expect(mountDialog().find('.confirm-header').exists()).toBe(false)
    expect(mountDialog({ title: 'x' }).find('.confirm-header').exists()).toBe(true)
  })

  it('插槽优先于 message', () => {
    const w = mountDialog({ message: '默认文案' }, { default: '<span class="s">自定义</span>' })
    expect(w.find('.s').text()).toBe('自定义')
    expect(w.html()).not.toContain('默认文案')
  })
})

describe('ConfirmDialog — 三条关闭途径', () => {
  it('取消按钮 → cancel + update:visible(false)', async () => {
    const w = mountDialog()
    await w.find('.confirm-btn.cancel').trigger('click')
    expect(w.emitted('cancel')).toHaveLength(1)
    expect(w.emitted('update:visible')).toEqual([[false]])
  })

  it('点遮罩（不含对话框本体）→ 取消', async () => {
    const w = mountDialog()
    await w.find('.confirm-overlay').trigger('click')
    expect(w.emitted('cancel')).toHaveLength(1)
  })

  it('点对话框本体**不**取消（冒泡不能穿出）', async () => {
    const w = mountDialog()
    await w.find('.confirm-dialog').trigger('click')
    expect(w.emitted('cancel')).toBeUndefined()
  })

  it('ESC 键 → 取消', async () => {
    const w = mountDialog()
    await w.find('.confirm-overlay').trigger('keydown', { key: 'Escape' })
    expect(w.emitted('cancel')).toHaveLength(1)
  })
})

describe('ConfirmDialog — 确认与危险态', () => {
  it('确认只 emit confirm，**不自动关闭**（异步结果由调用方决定）', async () => {
    const w = mountDialog()
    await w.find('.confirm-btn.primary').trigger('click')
    expect(w.emitted('confirm')).toHaveLength(1)
    expect(w.emitted('update:visible')).toBeUndefined()
  })

  it('danger 让主按钮带 danger 类（颜色归 CSS，不断言值）', () => {
    expect(mountDialog({ danger: true }).find('.confirm-btn.primary').classes()).toContain('danger')
    expect(mountDialog().find('.confirm-btn.primary').classes()).not.toContain('danger')
  })

  it('按钮文案可覆盖，缺省「确认 / 取消」', () => {
    const w = mountDialog({ confirmText: '删除', cancelText: '再想想' })
    expect(w.find('.confirm-btn.primary').text()).toBe('删除')
    expect(w.find('.confirm-btn.cancel').text()).toBe('再想想')
  })
})

describe('ConfirmDialog — loading 是硬闸门', () => {
  it('按钮禁用', () => {
    const w = mountDialog({ loading: true })
    expect(w.find('.confirm-btn.primary').attributes('disabled')).toBeDefined()
    expect(w.find('.confirm-btn.cancel').attributes('disabled')).toBeDefined()
  })

  it('主按钮显示「处理中…」', () => {
    expect(mountDialog({ loading: true }).find('.confirm-btn.primary').text()).toBe('处理中…')
  })

  it('点了也不 emit：确认与取消都不放行（不只是靠 disabled 挡）', async () => {
    const w = mountDialog({ loading: true })
    await w.find('.confirm-btn.primary').trigger('click')
    await w.find('.confirm-btn.cancel').trigger('click')
    await w.find('.confirm-overlay').trigger('keydown', { key: 'Escape' })
    expect(w.emitted('confirm')).toBeUndefined()
    expect(w.emitted('cancel')).toBeUndefined()
  })
})

describe('ConfirmDialog — 焦点陷阱', () => {
  it('打开（false→true 跃迁）即聚焦首个可交互元素', async () => {
    // 注意：自动聚焦挂在 watch 上（非 immediate），因此只在「打开」这一跃迁发生。
    // 初次就以 visible=true 挂载不触发——实际用法是常驻挂载、靠 visible 开合。
    const w = mount(ConfirmDialog, { props: { visible: false, message: 'x' }, attachTo: document.body })
    await w.setProps({ visible: true })
    await new Promise((r) => setTimeout(r, 0))
    expect(document.activeElement?.className).toContain('confirm-btn')
  })

  it('Tab 到最后一个时回到第一个（焦点出不去）', async () => {
    const w = mountDialog()
    await new Promise((r) => setTimeout(r, 0))
    const buttons = w.findAll('.confirm-btn')
    const last = buttons[buttons.length - 1].element as HTMLElement
    last.focus()
    const ev = new KeyboardEvent('keydown', { key: 'Tab', cancelable: true, bubbles: true })
    last.dispatchEvent(ev)
    expect(ev.defaultPrevented).toBe(true)
    expect(document.activeElement).toBe(buttons[0].element)
  })

  it('Shift+Tab 从第一个回到最后一个', async () => {
    const w = mountDialog()
    await new Promise((r) => setTimeout(r, 0))
    const buttons = w.findAll('.confirm-btn')
    const first = buttons[0].element as HTMLElement
    first.focus()
    const ev = new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, cancelable: true, bubbles: true })
    first.dispatchEvent(ev)
    expect(ev.defaultPrevented).toBe(true)
    expect(document.activeElement).toBe(buttons[buttons.length - 1].element)
  })
})

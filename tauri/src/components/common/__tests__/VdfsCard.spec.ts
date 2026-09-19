/**
 * VdfsCard — 通用「列表项」卡片单测（happy-dom）
 *
 * 工作台中栏列表里每一张卡都是它（模型 / MCP / 技能 / 智能体 / 设置分区），
 * 卡片本身**不认识资源类型**——类型知识在 registry 与调用方。因此这里测的是
 * 它与调用方之间的契约，而不是任何具体类型。
 *
 * 钉住的三条约定：
 *
 * 1. **两种激活方式等价**：鼠标点击与键盘 Enter / Space 都 emit `click`。
 *    它是 `role="option"`，键盘可达不是加分项而是前提——空格还要 `.prevent`
 *    挡掉页面滚动。
 * 2. **状态点是可选件**：`showStatus=false` 时整块 `.status-area` 不渲染
 *    （无状态语义的类型由调用方关掉），而不是渲染一个灰点。
 * 3. **元信息行按需出现**：只有 `meta` 插槽或 `tags` 非空时才渲染，
 *    否则不留一条空的横条。
 *
 * ⚠️ 已知争议（本次**只记录不改**）：`disabled` 只影响 `tabindex` /
 * `aria-disabled` / 视觉，点击与键盘**仍会 emit click**——拦截应在调用方或
 * 本组件补一层判断，属行为变更，需确认范围后再动。故此处不断言该组合。
 */

// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent, h, markRaw, nextTick } from 'vue'
import VdfsCard from '../VdfsCard.vue'

const ICON_STUB = markRaw(
  defineComponent({
    name: 'CardIconStub',
    render: () => h('svg', { class: 'icon-stub' }),
  })
)

function mountCard(props: Record<string, unknown> = {}, slots: Record<string, string> = {}) {
  return mount(VdfsCard, { props: { title: '一张卡', ...props }, slots })
}

describe('VdfsCard — 激活', () => {
  it('鼠标点击 emit click', async () => {
    const w = mountCard()
    await w.trigger('click')
    expect(w.emitted('click')).toHaveLength(1)
  })

  it('键盘 Enter 与点击等价', async () => {
    const w = mountCard()
    await w.trigger('keydown.enter')
    expect(w.emitted('click')).toHaveLength(1)
  })

  it('键盘 Space 与点击等价（且 preventDefault 挡掉页面滚动）', async () => {
    const w = mountCard()
    // 用原生事件派发：`trigger()` 不返回事件对象，验不到 defaultPrevented。
    const ev = new KeyboardEvent('keydown', { key: ' ', cancelable: true, bubbles: true })
    w.element.dispatchEvent(ev)
    await nextTick()
    expect(w.emitted('click')).toHaveLength(1)
    expect(ev.defaultPrevented).toBe(true)
  })

  it('role=option + aria-selected 跟随 isActive', () => {
    const on = mountCard({ isActive: true })
    const off = mountCard({ isActive: false })
    expect(on.attributes('role')).toBe('option')
    expect(on.attributes('aria-selected')).toBe('true')
    expect(off.attributes('aria-selected')).toBe('false')
  })

  it('禁用态移出 Tab 序（tabindex=-1）但仍保留 role', () => {
    const w = mountCard({ disabled: true })
    expect(w.attributes('tabindex')).toBe('-1')
    expect(w.attributes('aria-disabled')).toBe('true')
    expect(w.classes()).toContain('disabled')
  })
})

describe('VdfsCard — 状态点与图标', () => {
  it('status 落到 class，title 即 statusTitle', () => {
    const w = mountCard({ status: 'error', statusTitle: '已失效' })
    const dot = w.find('.status-dot')
    expect(dot.classes()).toContain('status-error')
    expect(dot.attributes('title')).toBe('已失效')
  })

  it('showStatus=false 时状态点整块不渲染（不是渲染一个灰点）', () => {
    expect(mountCard({ showStatus: true }).find('.status-area').exists()).toBe(true)
    expect(mountCard({ showStatus: false }).find('.status-area').exists()).toBe(false)
  })

  it('图标是可选的：给了才渲染，且带 .card-icon（尺寸归 CSS）', () => {
    expect(mountCard().find('.card-icon').exists()).toBe(false)
    const w = mountCard({ icon: ICON_STUB })
    expect(w.find('.card-icon.icon-stub').exists()).toBe(true)
  })
})

describe('VdfsCard — 文本与标签', () => {
  it('标题渲染且作 aria-label（读屏读的是卡片名）', () => {
    const w = mountCard({ title: '会话 abc' })
    expect(w.find('.title-text').text()).toBe('会话 abc')
    expect(w.attributes('aria-label')).toBe('会话 abc')
  })

  it('副标题可选：给了才渲染 .card-preview', () => {
    expect(mountCard().find('.card-preview').exists()).toBe(false)
    expect(mountCard({ subtitle: '描述' }).find('.preview-text').text()).toBe('描述')
  })

  it('badge 的 kind 落到 class，缺省 default', () => {
    const w = mountCard({ badge: '3 条', badgeKind: 'warn' })
    const badge = w.find('.activity-text')
    expect(badge.text()).toBe('3 条')
    expect(badge.classes()).toContain('kind-warn')
    expect(mountCard({ badge: 'x' }).find('.activity-text').classes()).toContain('kind-default')
  })

  it('tags 逐条渲染，kind 缺省 default', () => {
    const w = mountCard({ tags: [{ label: '本地' }, { label: '启用', kind: 'success' }] })
    const tags = w.findAll('.tag')
    expect(tags).toHaveLength(2)
    expect(tags[0].classes()).toContain('tag-default')
    expect(tags[1].classes()).toContain('tag-success')
  })
})

describe('VdfsCard — 元信息行', () => {
  it('有 tags 才出现', () => {
    expect(mountCard().find('.card-meta').exists()).toBe(false)
    expect(mountCard({ tags: [{ label: '本地' }] }).find('.card-meta').exists()).toBe(true)
  })

  it('有 meta 插槽也出现（插槽与 tags 可共存）', () => {
    const w = mountCard({ tags: [{ label: '本地' }] }, { meta: '<span class="m">来源：本地</span>' })
    expect(w.find('.card-meta .m').exists()).toBe(true)
    expect(w.findAll('.tag')).toHaveLength(1)
  })
})

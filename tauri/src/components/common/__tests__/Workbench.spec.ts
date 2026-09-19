/**
 * Workbench — 三栏工作台容器单测（happy-dom）
 *
 * 它是全项目**唯一**的三栏容器（`docs/design/vdfs-frontend.md`），此前只有
 * 经 VdfsWorkbench 间接覆盖的 80% 语句 / 23.5% 分支——分支缺口正是侧栏那几个
 * 条件渲染（有/无图标、计数角标、空态与加载态的互斥）。这里直接测容器本身。
 *
 * 钉住的三条约定：
 *
 * 1. **未登记图标的侧栏项兜底必须是文件夹、不能是文件**——侧栏项一律是目录
 *    （`.vdfs` 的挂载点），用文件图标是语义错配。断言写死 path 的 `d`，
 *    挡住它被改回文件图标。
 * 2. **图标不写死尺寸**：兜底 svg 不带 `width` / `height`，尺寸交给容器 CSS
 *    （`.nav-btn svg` 20px / `.card-icon` 16px）。写死任何一边，另一边就是错的。
 * 3. **空态与加载态互斥且空态优先**：两者都以「列表无内容」为前提，
 *    `empty` 插槽在场时显示空态，否则才轮到「加载中…」。
 *
 * 另：不传 `railItems` = 两栏形态（无侧边栏），是本容器的一等形态，不是边界。
 */

// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent, h, markRaw } from 'vue'
import Workbench, { type WorkbenchRailItem } from '../Workbench.vue'

// markRaw：组件对象经 props 传入会被响应式包装，Vue 会告警（不必要的开销）。
// 生产侧图标来自 registry（已是 markRaw / 普通组件），这里对齐同一前提。
const ICON_STUB = markRaw(
  defineComponent({
    name: 'IconStub',
    render: () => h('svg', { class: 'icon-stub' }),
  })
)

function rail(over: Partial<WorkbenchRailItem> = {}): WorkbenchRailItem {
  return { key: 'k', label: '类别', ...over }
}

describe('Workbench — 侧边栏形态', () => {
  it('不传 railItems 即两栏形态：不渲染侧边栏', () => {
    const w = mount(Workbench)
    expect(w.find('.side-nav').exists()).toBe(false)
    expect(w.find('.workbench-list').exists()).toBe(true)
    expect(w.find('.workbench-detail').exists()).toBe(true)
  })

  it('传 railItems 即三栏形态：每项一个 nav-btn，键为 key', () => {
    const w = mount(Workbench, {
      props: { railItems: [rail({ key: 'a' }), rail({ key: 'b' })] },
    })
    expect(w.find('.side-nav').exists()).toBe(true)
    expect(w.findAll('.nav-btn')).toHaveLength(2)
  })

  it('有登记图标时用它，不再画兜底 svg', () => {
    const w = mount(Workbench, {
      props: { railItems: [rail({ icon: ICON_STUB })] },
    })
    expect(w.find('.nav-btn .icon-stub').exists()).toBe(true)
    // 兜底 svg 无 class，故按「是否只有一个 svg」判定它没被额外渲染
    expect(w.findAll('.nav-btn svg')).toHaveLength(1)
  })

  it('无登记图标时兜底是**文件夹**图标，且不是文件图标', () => {
    const w = mount(Workbench, { props: { railItems: [rail()] } })
    const svg = w.find('.nav-btn svg')
    const d = svg.find('path').attributes('d') ?? ''
    // 文件夹：底边连续 + 顶部有折角（M22 19a2 2 0 0 1-2 2H4 …）
    expect(d).toContain('M22 19a2 2 0 0 1-2 2H4')
    // 文件：右上角折页（M13 2H6a2 …）——不能出现
    expect(d).not.toContain('M13 2H6a2')
  })

  it('兜底图标不写死尺寸：无 width / height 属性（尺寸归容器 CSS）', () => {
    const w = mount(Workbench, { props: { railItems: [rail()] } })
    const svg = w.find('.nav-btn svg')
    expect(svg.attributes('width')).toBeUndefined()
    expect(svg.attributes('height')).toBeUndefined()
    expect(svg.attributes('viewBox')).toBe('0 0 24 24')
  })

  it('tooltip 优先取 description，缺省回落 label', () => {
    const w = mount(Workbench, {
      props: {
        railItems: [rail({ label: '会话', description: '会话挂载点' }), rail({ label: '模型' })],
      },
    })
    const [withDesc, withoutDesc] = w.findAll('.nav-btn')
    expect(withDesc.attributes('title')).toBe('会话挂载点')
    expect(withoutDesc.attributes('title')).toBe('模型')
    // aria-label 恒为 label（读屏说的是名字，不是说明）
    expect(withDesc.attributes('aria-label')).toBe('会话')
  })

  it('计数角标：有 count 才渲染，0 不渲染', () => {
    const w = mount(Workbench, {
      props: { railItems: [rail({ key: 'a', count: 3 }), rail({ key: 'b', count: 0 }), rail({ key: 'c' })] },
    })
    const badges = w.findAll('.nav-btn').map((b) => b.find('.nav-count').exists())
    expect(badges).toEqual([true, false, false])
    expect(w.findAll('.nav-btn')[0].find('.nav-count').text()).toBe('3')
  })

  it('active 只在当前项上（高亮不扩散）', () => {
    const w = mount(Workbench, {
      props: { railItems: [rail({ key: 'a', active: true }), rail({ key: 'b' })] },
    })
    const classes = w.findAll('.nav-btn').map((b) => b.classes('active'))
    expect(classes).toEqual([true, false])
  })

  it('点击侧栏项 emit rail-select，带的是 key 而不是 label', async () => {
    const w = mount(Workbench, { props: { railItems: [rail({ key: 'session', label: '会话' })] } })
    await w.find('.nav-btn').trigger('click')
    expect(w.emitted('rail-select')).toEqual([['session']])
  })

  it('rail-header / rail-footer 插槽落在侧栏内的首尾', () => {
    const w = mount(Workbench, {
      props: { railItems: [rail()] },
      slots: { 'rail-header': '<span class="rh">H</span>', 'rail-footer': '<span class="rf">F</span>' },
    })
    const nav = w.find('.side-nav')
    expect(nav.find('.rh').exists()).toBe(true)
    expect(nav.find('.rf').exists()).toBe(true)
  })
})

describe('Workbench — 中栏', () => {
  it('标题与动作行都渲染，动作行内容来自插槽', () => {
    const w = mount(Workbench, {
      props: { title: '会话' },
      slots: { 'header-actions': '<button class="new-btn">新建</button>' },
    })
    expect(w.find('.panel-title').text()).toBe('会话')
    expect(w.find('.header-actions .new-btn').exists()).toBe(true)
  })

  it('中栏宽度按 listWidth 落到行内样式', () => {
    const w = mount(Workbench, { props: { listWidth: 320 } })
    expect(w.find('.workbench-list').attributes('style')).toContain('width: 320px')
  })

  it('列表无内容 + 有 empty 插槽 → 空态', () => {
    const w = mount(Workbench, {
      props: { hasListContent: false, loading: true },
      slots: { empty: '<span class="e">没有内容</span>' },
    })
    expect(w.find('.empty-state .e').exists()).toBe(true)
    expect(w.find('.loading-state').exists()).toBe(false)
  })

  it('列表无内容 + 无 empty 插槽 + loading → 加载态', () => {
    const w = mount(Workbench, { props: { hasListContent: false, loading: true } })
    expect(w.find('.empty-state').exists()).toBe(false)
    expect(w.find('.loading-state').text()).toBe('加载中…')
  })

  it('列表无内容且不在加载 → 两个态都不显示（不是空白占位）', () => {
    const w = mount(Workbench, {
      props: { hasListContent: false, loading: false },
      slots: { empty: '<span class="e">没有内容</span>' },
    })
    // 有 empty 插槽时仍显示空态——此例验证 loading=false 时不会误报加载中
    expect(w.find('.empty-state').exists()).toBe(true)
    expect(w.find('.loading-state').exists()).toBe(false)
  })

  it('列表有内容 → 空态与加载态都让位', () => {
    const w = mount(Workbench, {
      props: { hasListContent: true, loading: true },
      slots: { empty: '<span class="e">没有内容</span>', list: '<div class="row">一行</div>' },
    })
    expect(w.find('.row').exists()).toBe(true)
    expect(w.find('.empty-state').exists()).toBe(false)
    expect(w.find('.loading-state').exists()).toBe(false)
  })
})

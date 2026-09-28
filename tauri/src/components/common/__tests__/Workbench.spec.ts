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
 *    （`@vfs` 的挂载点），用文件图标是语义错配。断言写死 path 的 `d`，
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
    expect(w.find('.list-skeleton').exists()).toBe(false)
  })

  it('列表无内容 + 无 empty 插槽 + loading → 骨架卡加载态', () => {
    const w = mount(Workbench, { props: { hasListContent: false, loading: true } })
    expect(w.find('.empty-state').exists()).toBe(false)
    // 骨架而非纯文字：形状与真实卡片一致，数据到达时不重排
    expect(w.find('.list-skeleton').exists()).toBe(true)
    expect(w.findAll('.vdfs-card-skeleton').length).toBeGreaterThan(1)
    expect(w.find('.list-skeleton').attributes('aria-busy')).toBe('true')
  })

  it('列表无内容且不在加载 → 两个态都不显示（不是空白占位）', () => {
    const w = mount(Workbench, {
      props: { hasListContent: false, loading: false },
      slots: { empty: '<span class="e">没有内容</span>' },
    })
    // 有 empty 插槽时仍显示空态——此例验证 loading=false 时不会误报加载中
    expect(w.find('.empty-state').exists()).toBe(true)
    expect(w.find('.list-skeleton').exists()).toBe(false)
  })

  it('列表有内容 → 空态与加载态都让位', () => {
    const w = mount(Workbench, {
      props: { hasListContent: true, loading: true },
      slots: { empty: '<span class="e">没有内容</span>', list: '<div class="row">一行</div>' },
    })
    expect(w.find('.row').exists()).toBe(true)
    expect(w.find('.empty-state').exists()).toBe(false)
    expect(w.find('.list-skeleton').exists()).toBe(false)
  })
})

/**
 * 中栏收起（列表为空且右栏是新建详情）。
 *
 * 这是用户报的「三栏里的列表栏为空时，应该直接显示详情（新建）」。收起条件
 * 是**四条的合取**，所以逐条钉住"少一个就不能收"——只测最理想的那一路等于
 * 没测：任何一条写反，用户看到的都是「栏位乱闪」或「该说的话说不出来」。
 *
 * 特别记一条**被否掉的判据**：第一版写的是"宿主没提供 empty 插槽才收起"，
 * 而 `VdfsWorkbench` 无条件声明该插槽 ⇒ 那条判据恒为假、收起永不发生。
 * 所以下面有一条专门钉「有 empty 插槽也照样能收」。
 *
 * 另记一条**被否掉的写法**：收起时给根元素挂个 `.list-hidden` 标记类。它没有任何
 * CSS 定义（`style-audit` 因此报错），且没有消费方——收起这件事已经由
 * `.workbench-list` **不存在**表达完了，标记类是纯噪声。断言一律查中栏本身。
 */
describe('Workbench — 列表为空时收起中栏', () => {
  it('空列表 + 草稿详情 + 可新建 → 中栏整个不渲染，详情铺满', () => {
    const w = mount(Workbench, {
      props: { hasListContent: false, loading: false, hasDetail: true, hasDraft: true, canCreate: true },
      slots: { detail: '<div class="d">详情</div>' },
    })
    expect(w.find('.workbench-list').exists()).toBe(false)
    // 详情仍在（收的是列表，不是详情）
    expect(w.find('.workbench-detail .d').exists()).toBe(true)
  })

  it('★ 有 empty 插槽**照样**收起（被否掉的旧判据：该插槽在本项目恒存在）', () => {
    const w = mount(Workbench, {
      props: { hasListContent: false, hasDetail: true, hasDraft: true, canCreate: true },
      slots: { empty: '<span class="e">暂无子目录</span>', detail: '<div class="d">详情</div>' },
    })
    expect(w.find('.workbench-list').exists()).toBe(false)
  })

  it('右栏不是草稿（选了某个已有条目）→ 不收起，空目录的解释必须留着', () => {
    const w = mount(Workbench, {
      props: { hasListContent: false, hasDetail: true, hasDraft: false, canCreate: true },
      slots: { empty: '<span class="e">此目录为空</span>', detail: '<div class="d">详情</div>' },
    })
    expect(w.find('.workbench-list').exists()).toBe(true)
    expect(w.find('.empty-state .e').exists()).toBe(true)
  })

  it('不可新建（目录由系统管理）→ 不收起，那句说明是唯一的解释', () => {
    const w = mount(Workbench, {
      props: { hasListContent: false, hasDetail: true, hasDraft: true, canCreate: false },
      slots: { empty: '<span class="e">该目录由系统管理</span>', detail: '<div class="d">详情</div>' },
    })
    expect(w.find('.workbench-list').exists()).toBe(true)
  })

  it('加载中 → 不收起（否则「正在取」会被读成「什么都没有」，数据到了整栏弹回）', () => {
    const w = mount(Workbench, {
      props: { hasListContent: false, loading: true, hasDetail: true, hasDraft: true, canCreate: true },
      slots: { detail: '<div class="d">详情</div>' },
    })
    expect(w.find('.workbench-list').exists()).toBe(true)
    expect(w.find('.list-skeleton').exists()).toBe(true)
  })

  it('右栏也没内容（hasDetail=false）→ 不收起（两边都空时收起 = 整屏空白）', () => {
    const w = mount(Workbench, {
      props: { hasListContent: false, loading: false, hasDetail: false, hasDraft: true, canCreate: true },
    })
    expect(w.find('.workbench-list').exists()).toBe(true)
  })

  it('hasDetail / hasDraft 缺省为 false ⇒ 不声明它们的宿主一律不收栏（默认必须保守）', () => {
    const w = mount(Workbench, { props: { hasListContent: false } })
    expect(w.find('.workbench-list').exists()).toBe(true)
    // 只声明 hasDetail（右栏有内容）也不够 —— 还得右栏那张是草稿
    const w2 = mount(Workbench, { props: { hasListContent: false, hasDetail: true } })
    expect(w2.find('.workbench-list').exists()).toBe(true)
  })

  it('列表有内容 → 永远不收起', () => {
    const w = mount(Workbench, {
      props: { hasListContent: true, hasDetail: true, hasDraft: true, canCreate: true, loading: true },
      slots: { list: '<div class="row">一行</div>' },
    })
    expect(w.find('.workbench-list').exists()).toBe(true)
  })
})

// @vitest-environment happy-dom
/**
 * ChatOptionBar —— 会话选项栏（紧凑渲染形态）单测
 *
 * 本组件是 `DetailDefinition` 的**第二种渲染形态**（纵向表单是第一种）。这里钉住
 * 的是「定义 → 交互 → 落库」这条链，按 `DetailField.widget` 分派：
 *
 * - `select`：展开候选菜单（候选 = `field.options`，说明 = `DetailOption.description`），
 *   选中后把 `{ <field.key>: <候选 value> }` 写进会话 metadata；
 * - `path`  ：经原生取值原语取值后直接写入；`disabled_when` 成立时不可点；
 * - `form`  ：弹出子表单，保存整份子对象；
 * - 草稿态（无 `sessionId`）：不落库，缓冲在 `getDraftMetadata()` 里。
 *
 * 三条**不变量**值得单独说，它们错了都不会报错、只会静默走样：
 *
 * 1. **前端零业务字段名**——写入的键来自定义（`field.key`），不是组件里写死的
 *    字符串。测试因此用一组**虚构字段名**（`alpha` / `beta`）验证这一点：真实
 *    会话字段名在组件里出现一次都算违约；
 * 2. **按钮文本与候选选中态必须同源**（都按「生效值」= 值 ?? `default` 算），
 *    否则会出现「按钮写着中风险、菜单里却一个都没勾」；
 * 3. **未设置时的按钮文本来自 `options` 的值→标签表**（`path` / `form` 没有候选
 *    菜单，`options` 退化为查表），这是「外观不变」的关键一格。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'

const hoisted = vi.hoisted(() => ({
  updateSession: vi.fn(async () => {}),
  pickNative: vi.fn(async () => null as string | null),
  applySessionMetadataPatch: vi.fn(),
}))

vi.mock('@/services/session', () => ({ updateSession: hoisted.updateSession }))
vi.mock('@/services/nativePick', () => ({ pickNative: hoisted.pickNative }))
vi.mock('@/stores/sessions', () => ({
  useSessionsStore: () => ({ applySessionMetadataPatch: hoisted.applySessionMetadataPatch }),
}))

import ChatOptionBar from '../ChatOptionBar.vue'
import OptionFormDialog from '../OptionFormDialog.vue'
import DetailForm from '@/components/vdfs/DetailForm.vue'
import type { DetailDefinition } from '@/schemas/vdfs'

/** 一组虚构字段：组件不该认识任何真实业务字段名 */
function definition(): DetailDefinition {
  return {
    binding: 'option',
    title_fallback: '会话选项',
    sections: [
      {
        collapsed: false,
        fields: [
          {
            key: 'alpha',
            label: '甲',
            description: '甲说明',
            widget: 'path',
            icon: 'folder',
            pick: 'directory',
            options: [{ value: '', label: '未选择' }],
            // 与后端 `workdir_field` 同形：`truthy` 而不是 `not_equals: 0`，
            // 使草稿节点（没有 `message_count` 键）不被判成「有历史」
            disabled_when: {
              all: [
                { key: 'message_count', truthy: true },
                { key: 'alpha', truthy: true },
              ],
            },
          },
          {
            key: 'beta',
            label: '乙',
            widget: 'select',
            icon: 'risk',
            default: 'b',
            options: [
              { value: 'a', label: '甲选', description: '甲选说明' },
              { value: 'b', label: '乙选' },
              { value: 'c', label: '丙选' },
            ],
          },
          {
            key: 'gamma',
            label: '丙',
            widget: 'form',
            icon: 'heartbeat',
            options: [
              { value: 'true', label: '已开启' },
              { value: 'false', label: '未开启' },
            ],
            // 子对象缺省值：未配置过时按钮显示「未开启」，子表单据此预填
            default: { enabled: false, interval_seconds: 300 },
            form: {
              binding: 'option',
              title_from: ['enabled'],
              title_fallback: '丙',
              sections: [{ collapsed: false, fields: [{ key: 'enabled', label: '开关', widget: 'toggle' }] }],
              actions: [{ id: 'save', label: '保存', style: 'primary' }],
            },
          },
        ],
      },
    ],
  }
}

function mountBar(props: Record<string, unknown> = {}) {
  return mount(ChatOptionBar, { props: { definition: definition(), ...props } })
}

/** 第 i 个字段按钮（顺序 = 定义顺序） */
function btn(w: ReturnType<typeof mount>, i: number) {
  return w.findAll('button.option-btn')[i]
}

beforeEach(() => {
  hoisted.updateSession.mockClear()
  hoisted.pickNative.mockClear()
  hoisted.applySessionMetadataPatch.mockClear()
  hoisted.pickNative.mockResolvedValue(null)
})

describe('ChatOptionBar：按钮文本（紧凑形态的取值规则）', () => {
  it('select 未设置时显示定义缺省值的标签（default 查表）', () => {
    const w = mountBar()
    expect(btn(w, 1).text()).toContain('乙选')
  })

  it('select 已设置时显示当前值的标签（值→标签查表，前端不硬编码取值）', () => {
    const w = mountBar({ values: { beta: 'c' } })
    expect(btn(w, 1).text()).toContain('丙选')
  })

  it('path 显示路径末段；未设置时显示 options 里 value="" 的标签', () => {
    const w = mountBar({ values: { alpha: 'D:\\work\\proj' } })
    expect(btn(w, 0).text()).toContain('proj')

    const empty = mountBar()
    expect(btn(empty, 0).text()).toContain('未选择')
  })

  it('form 先按子定义 title_from 取代表值再查表（开 / 关）', () => {
    const off = mountBar({ values: { gamma: { enabled: false } } })
    expect(btn(off, 2).text()).toContain('未开启')

    const on = mountBar({ values: { gamma: { enabled: true } } })
    expect(btn(on, 2).text()).toContain('已开启')
  })

  it('form 未配置过时按字段 default 里的代表值显示（不是字段名）', () => {
    const w = mountBar()
    expect(btn(w, 2).text()).toContain('未开启')
    expect(btn(w, 2).text()).not.toContain('丙')
  })

  it('visible_when 不成立的字段整条不渲染', () => {
    const def = definition()
    def.sections[0].fields[1].visible_when = { key: 'beta', equals: 'never' }
    const w = mount(ChatOptionBar, { props: { definition: def } })
    expect(w.findAll('button.option-btn')).toHaveLength(2)
  })
})

describe('ChatOptionBar：select 分派（菜单 → 落库）', () => {
  it('点击展开候选菜单，候选说明照常显示', async () => {
    const w = mountBar({ sessionId: 's1' })
    await btn(w, 1).trigger('click')

    const items = w.findAll('.menu-item')
    expect(items).toHaveLength(3)
    expect(items[0].text()).toContain('甲选说明')
  })

  it('当前值对应的候选标记为选中（选中态与按钮文本同源）', async () => {
    const w = mountBar({ sessionId: 's1', values: { beta: 'c' } })
    await btn(w, 1).trigger('click')

    const items = w.findAll('.menu-item')
    expect(items[2].classes()).toContain('active')
    expect(items[0].classes()).not.toContain('active')
  })

  it('选中候选 ⇒ 以 field.key 为键写入会话 metadata，并本地镜射', async () => {
    const w = mountBar({ sessionId: 's1' })
    await btn(w, 1).trigger('click')
    await w.findAll('.menu-item')[0].trigger('click')
    await flushPromises()

    expect(hoisted.updateSession).toHaveBeenCalledWith('s1', { beta: 'a' })
    expect(hoisted.applySessionMetadataPatch).toHaveBeenCalledWith('s1', { beta: 'a' })
    // 落库后菜单收起
    expect(w.find('.opt-menu').exists()).toBe(false)
  })

  it('再次点击同一按钮收起菜单（不落库）', async () => {
    const w = mountBar({ sessionId: 's1' })
    await btn(w, 1).trigger('click')
    await btn(w, 1).trigger('click')

    expect(w.find('.opt-menu').exists()).toBe(false)
    expect(hoisted.updateSession).not.toHaveBeenCalled()
  })
})

describe('ChatOptionBar：path 分派（原生取值 → 落库）', () => {
  it('点击唤起原生取值原语，取值即写入（无候选菜单）', async () => {
    hoisted.pickNative.mockResolvedValue('/tmp/picked')
    const w = mountBar({ sessionId: 's1' })

    await btn(w, 0).trigger('click')
    await flushPromises()

    expect(hoisted.pickNative).toHaveBeenCalledWith('directory')
    expect(hoisted.updateSession).toHaveBeenCalledWith('s1', { alpha: '/tmp/picked' })
  })

  it('用户取消取值 ⇒ 不落库', async () => {
    hoisted.pickNative.mockResolvedValue(null)
    const w = mountBar({ sessionId: 's1' })

    await btn(w, 0).trigger('click')
    await flushPromises()

    expect(hoisted.updateSession).not.toHaveBeenCalled()
  })

  it('disabled_when 成立 ⇒ 按钮禁用且点击无效（作用域含节点属性）', async () => {
    const w = mountBar({ sessionId: 's1', values: { alpha: '/a' }, scope: { message_count: 2 } })
    expect(btn(w, 0).classes()).toContain('is-disabled')

    await btn(w, 0).trigger('click')
    await flushPromises()
    expect(hoisted.pickNative).not.toHaveBeenCalled()

    // 同样的值，但会话还没有历史 ⇒ 不锁
    const free = mountBar({ sessionId: 's1', values: { alpha: '/a' }, scope: { message_count: 0 } })
    expect(btn(free, 0).classes()).not.toContain('is-disabled')
  })

  it('草稿态（作用域里没有 message_count）⇒ 不锁：缺席的键按「没有历史」处理', () => {
    // 草稿节点没有任何属性；锁定条件若写成 `not_equals: 0`，缺席键会被判成
    // 「有历史」，新建会话一上来工作目录就锁死。这条防的就是那个回归。
    const w = mountBar()
    expect(btn(w, 0).classes()).not.toContain('is-disabled')
  })
})

describe('ChatOptionBar：form 分派（子表单 → 落库）', () => {
  it('点击弹出子表单，子对象当前值原样预填', async () => {
    const w = mountBar({ sessionId: 's1', values: { gamma: { enabled: true } } })
    await btn(w, 2).trigger('click')

    const dialog = w.findComponent(OptionFormDialog)
    expect(dialog.exists()).toBe(true)
    expect(dialog.findComponent(DetailForm).props('values')).toEqual({ enabled: true })
  })

  it('子对象未配置过 ⇒ 子表单按字段 default 预填（缺省配置而非空表）', async () => {
    const w = mountBar({ sessionId: 's1' })
    await btn(w, 2).trigger('click')

    expect(w.findComponent(OptionFormDialog).findComponent(DetailForm).props('values')).toEqual({
      enabled: false,
      interval_seconds: 300,
    })
  })

  it('子表单保存 ⇒ 整份子对象写入该字段（键来自定义）', async () => {
    const w = mountBar({ sessionId: 's1' })
    await btn(w, 2).trigger('click')

    const payload = { enabled: true, interval_seconds: 60 }
    w.findComponent(OptionFormDialog).findComponent(DetailForm).vm.$emit('save', payload)
    await flushPromises()

    expect(hoisted.updateSession).toHaveBeenCalledWith('s1', { gamma: payload })
    expect(w.findComponent(OptionFormDialog).exists()).toBe(false)
  })

  it('落库失败 ⇒ 上抛提示且不静默（子表单仍关闭）', async () => {
    hoisted.updateSession.mockRejectedValueOnce(new Error('boom'))
    const w = mountBar({ sessionId: 's1' })
    await btn(w, 2).trigger('click')

    w.findComponent(OptionFormDialog).findComponent(DetailForm).vm.$emit('save', { enabled: true })
    await flushPromises()

    // 失败不得被吞掉：按钮文本不因失败而变，且不留下半开的弹窗
    expect(w.findComponent(OptionFormDialog).exists()).toBe(false)
    expect(btn(w, 2).text()).toContain('未开启')
  })
})

describe('ChatOptionBar：草稿态（新建会话前）', () => {
  it('无 sessionId ⇒ 不落库，缓冲进 getDraftMetadata()', async () => {
    const w = mountBar()
    await btn(w, 1).trigger('click')
    await w.findAll('.menu-item')[2].trigger('click')
    await flushPromises()

    expect(hoisted.updateSession).not.toHaveBeenCalled()
    expect((w.vm as unknown as { getDraftMetadata: () => unknown }).getDraftMetadata()).toEqual({
      beta: 'c',
    })
  })

  it('草稿缓冲立即回显（按钮文本与选中态都按缓冲值算）', async () => {
    const w = mountBar()
    await btn(w, 1).trigger('click')
    await w.findAll('.menu-item')[2].trigger('click')
    await flushPromises()

    expect(btn(w, 1).text()).toContain('丙选')

    await btn(w, 1).trigger('click')
    expect(w.findAll('.menu-item')[2].classes()).toContain('active')
  })
})

describe('ChatOptionBar：定义缺席', () => {
  it('没有定义 ⇒ 不渲染任何按钮（会话页其余部分照常）', () => {
    const w = mount(ChatOptionBar, { props: { definition: null } })
    expect(w.findAll('button.option-btn')).toHaveLength(0)
    expect(w.find('.chat-option-bar').exists()).toBe(true)
  })
})

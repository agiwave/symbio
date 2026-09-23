/**
 * VDFS `form` 方言的**动作区装配**纯逻辑单测（node 环境）
 *
 * 覆盖 `mergeDetailActions`：把「渲染器自有动作」（save / test / open-container…）
 * 与「机制级默认动作」（重命名 / 删除，页面单点算好）装配成一行动作区。
 *
 * 这两段此前在 DetailForm / VdfsFormDetail / 各渲染器里**各算了一遍**（G1），
 * 同一份去重规则散在三处，改一处漏两处的概率不为零。现在是唯一实现，故直接
 * 对它下断言——规则只有两条，且都是「错了不报错、只多画或少画一个按钮」的类型：
 *
 * 1. 两段之间插一个 divider（视觉分组；缺了会把两段读成同一组）；
 * 2. **同 id 时渲染器声明的那一份胜出** —— 否则「详情定义自带 delete」与
 *    「机制兜底的 delete」会渲染成两个删除按钮（重复入口，点哪个都说不清）。
 *
 * 另有三条排序/对齐约定：自有动作在前、机制动作用单个 id 表达忙态、返回的三个
 * 数组等长（`VdfsActions` 的入参形状）。
 *
 * 第二个 describe 覆盖**预设联动**（同一模块的另一组规则）。这段原先只存在于
 * `DetailForm.vue` 的 computed 与 `applyPreset` 里，只能经组件挂载间接覆盖——
 * 而它是本层最绕的一段（`fill` 策略 / `set_always` / 动态候选 / 建议合并），
 * 恰是最该被直接钉住的。下沉为纯函数后在此逐条下断言。
 */

import { describe, expect, it } from 'vitest'
import {
  compactFieldText,
  detailPresetFieldOptions,
  detailPresetFieldSuggestions,
  detailPresetOf,
  detailPresetPatch,
  evalDetailCondition,
  mergeDetailActions,
} from '../vdfs-form'
import type { DetailAction, DetailField, DetailPreset, DetailPresetSpec } from '../vdfs'

const save: DetailAction = { id: 'save', label: '保存', style: 'primary' }
const test: DetailAction = { id: 'test', label: '测试', style: 'secondary' }
const rename: DetailAction = { id: 'rename', label: '重命名', style: 'secondary' }
const del = (label: string): DetailAction => ({ id: 'delete', label, style: 'danger' })

describe('mergeDetailActions 装配与去重', () => {
  it('自有动作在前、机制动作经 divider 注入在后，三数组等长且按索引对齐', () => {
    const r = mergeDetailActions([save], [rename, del('删除')])

    expect(r.actions.map((a) => a.id)).toEqual(['save', 'divider', 'rename', 'delete'])
    expect(r.actions[1].style).toBe('divider')
    // `VdfsActions` 的入参形状：busy / disabled 与 actions 等长
    expect(r.busy).toHaveLength(r.actions.length)
    expect(r.disabled).toHaveLength(r.actions.length)
  })

  it('同 id 时定义声明的那一份胜出：不出现两个 delete 按钮，也不插多余的 divider', () => {
    // 详情定义已自带「删除 Provider」——机制兜底的 delete 不该再注入一份
    const r = mergeDetailActions([save, del('删除 Provider')], [del('删除')])

    const deletes = r.actions.filter((a) => a.id === 'delete')
    expect(deletes, '重复入口是这次装配要消灭的东西').toHaveLength(1)
    expect(deletes[0].label, '更具体的那份文案来自定义').toBe('删除 Provider')
    // 机制那一段被去重掉 ⇒ 没有可注入的动作 ⇒ 不插 divider
    expect(r.actions.some((a) => a.id === 'divider')).toBe(false)
  })

  it('机制动作忙态按 id 判定（同时最多一个在跑），自有动作不受影响', () => {
    const r = mergeDetailActions([save], [rename, del('删除')], [], [], 'delete')

    const busyById = Object.fromEntries(r.actions.map((a, i) => [a.id, r.busy[i]]))
    expect(busyById.delete).toBe(true)
    expect(busyById.rename).toBe(false)
    expect(busyById.save).toBe(false)
  })

  it('自有动作的忙 / 禁用按索引透传；机制动作一律不禁用（忙态已挡住重复点击）', () => {
    const r = mergeDetailActions([save, test], [del('删除')], [true, false], [false, true])

    expect(r.busy.slice(0, 2)).toEqual([true, false])
    expect(r.disabled.slice(0, 2)).toEqual([false, true])
    expect(r.disabled[r.actions.length - 1], '机制动作不参与 disabled_when').toBe(false)
  })

  it('无自有动作时不插 divider（仅机制动作）——消息 / 只读详情即此形态', () => {
    const r = mergeDetailActions([], [rename, del('删除')])

    expect(r.actions.map((a) => a.id)).toEqual(['rename', 'delete'])
    expect(r.actions.some((a) => a.id === 'divider')).toBe(false)
  })

  it('机制动作为空时只有自有动作——新建态 / 只读节点的形态', () => {
    const r = mergeDetailActions([save], [])

    expect(r.actions.map((a) => a.id)).toEqual(['save'])
    expect(r.actions.some((a) => a.id === 'divider')).toBe(false)
  })
})

// ==================== 预设联动 ====================

const spec = (fill: string, presets: DetailPreset[]): DetailPresetSpec => ({
  field: 'provider',
  fill,
  presets,
})

const field = (over: Partial<DetailField> = {}): DetailField => ({
  key: 'model',
  label: '模型',
  widget: 'select',
  ...over,
})

describe('detailPresetOf 预设命中', () => {
  it('无规格 ⇒ null；未命中取值 ⇒ null', () => {
    expect(detailPresetOf(null, 'openai')).toBeNull()
    expect(detailPresetOf(spec('if_empty', [{ value: 'openai', label: 'OpenAI' }]), 'anthropic')).toBeNull()
  })

  it('命中取值 ⇒ 该预设（label 与 set 一并带出）', () => {
    const openai: DetailPreset = { value: 'openai', label: 'OpenAI', set: { base_url: 'https://api.openai.com' } }
    expect(detailPresetOf(spec('if_empty', [openai]), 'openai')).toBe(openai)
  })
})

describe('detailPresetFieldOptions 字段候选', () => {
  it('静态字段：无论有没有预设，都用自身 options', () => {
    const f = field({ options: [{ value: 'a', label: 'A' }] })
    expect(detailPresetFieldOptions(f, null)).toEqual([{ value: 'a', label: 'A' }])
    expect(detailPresetFieldOptions(f, { value: 'p', label: 'P', options: { model: ['x'] } })).toEqual([
      { value: 'a', label: 'A' },
    ])
  })

  it('options_from_preset：取当前预设注入的候选，值即标签', () => {
    const f = field({ options_from_preset: true })
    const preset: DetailPreset = { value: 'p', label: 'P', options: { model: ['gpt-4o', 'gpt-4o-mini'] } }
    expect(detailPresetFieldOptions(f, preset)).toEqual([
      { value: 'gpt-4o', label: 'gpt-4o' },
      { value: 'gpt-4o-mini', label: 'gpt-4o-mini' },
    ])
  })

  it('options_from_preset 但预设为空 / 该字段未被注入 ⇒ 空候选（不是退回静态）', () => {
    const f = field({ options_from_preset: true, options: [{ value: 'a', label: 'A' }] })
    expect(detailPresetFieldOptions(f, null)).toEqual([])
    expect(detailPresetFieldOptions(f, { value: 'p', label: 'P', options: {} })).toEqual([])
  })
})

describe('detailPresetFieldSuggestions datalist 建议', () => {
  it('非 preset 来源：只给静态建议', () => {
    const f = field({ suggestions: ['gpt-4o'] })
    expect(detailPresetFieldSuggestions(f, { value: 'p', label: 'P', options: { model: ['x'] } })).toEqual([
      'gpt-4o',
    ])
  })

  it('preset 来源：静态在前、动态在后，且去掉与静态重复的', () => {
    const f = field({ suggestions_from_preset: true, suggestions: ['gpt-4o', 'o3'] })
    const preset: DetailPreset = { value: 'p', label: 'P', options: { model: ['gpt-4o', 'gpt-4.1'] } }
    expect(detailPresetFieldSuggestions(f, preset)).toEqual(['gpt-4o', 'o3', 'gpt-4.1'])
  })
})

describe('detailPresetPatch 字段补丁', () => {
  const preset: DetailPreset = {
    value: 'openai',
    label: 'OpenAI',
    set: { base_url: 'https://api.openai.com', api_key_env: 'OPENAI_API_KEY' },
    set_always: { protocol: 'openai' },
  }
  const s = spec('if_empty', [preset])

  it('fill = if_empty：只补空字段，用户已填的不覆盖', () => {
    const cur: Record<string, unknown> = { base_url: 'https://mine.example', api_key_env: '' }
    expect(detailPresetPatch(s, preset, (k) => cur[k], true)).toEqual({
      api_key_env: 'OPENAI_API_KEY', // 空 ⇒ 补
      protocol: 'openai', // set_always 总是写
    })
  })

  it('fill = always：一律覆盖 set 值', () => {
    const cur: Record<string, unknown> = { base_url: 'https://mine.example' }
    expect(detailPresetPatch(spec('always', [preset]), preset, (k) => cur[k], true)).toEqual({
      base_url: 'https://api.openai.com',
      api_key_env: 'OPENAI_API_KEY',
      protocol: 'openai',
    })
  })

  it('applySet = false（编辑态预填）：跳过 set，但 set_always 仍生效', () => {
    const cur: Record<string, unknown> = {}
    expect(detailPresetPatch(s, preset, (k) => cur[k], false)).toEqual({ protocol: 'openai' })
  })

  it('同一键同时在 set 与 set_always ⇒ 以 set_always 为准（后写胜出）', () => {
    const both: DetailPreset = {
      value: 'p',
      label: 'P',
      set: { protocol: 'from-set' },
      set_always: { protocol: 'from-set-always' },
    }
    const p = detailPresetPatch(spec('always', [both]), both, () => undefined, true)
    expect(p.protocol).toBe('from-set-always')
  })

  it('数组按「空数组即空」判定（if_empty 下会补）', () => {
    const listPreset: DetailPreset = { value: 'p', label: 'P', set: { models: ['a'] } }
    const p = detailPresetPatch(spec('if_empty', [listPreset]), listPreset, () => [], true)
    expect(p.models).toEqual(['a'])
  })

  it('无规格 / 无命中预设 ⇒ 空补丁（不是崩，也不是全量写回）', () => {
    expect(detailPresetPatch(null, null, () => undefined, true)).toEqual({})
    expect(detailPresetPatch(s, null, () => undefined, true)).toEqual({})
  })
})

// ==================== 条件求值 + 紧凑渲染形态取值规则 ====================
//
// 这两组规则是**共享的**：纵向表单（DetailForm）与紧凑选项栏（ChatOptionBar）
// 都要判 `disabled_when`、都要把值显示成人能读的一句话。各写一份必然漂移，
// 且漂移的表现是静默的（同一份定义在两种形态下给出不同答案），故在此直接钉住。

describe('evalDetailCondition（作用域由调用方给）', () => {
  const scopeOf = (scope: Record<string, unknown>) => (k: string) => scope[k]

  it('无条件 ⇒ 成立（服务于 when / visible_when 的「缺省即显示」）', () => {
    expect(evalDetailCondition(null, scopeOf({}))).toBe(true)
    expect(evalDetailCondition(undefined, scopeOf({}))).toBe(true)
    expect(evalDetailCondition({}, scopeOf({}))).toBe(true)
  })

  it('equals / not_equals / truthy 三条谓词', () => {
    expect(evalDetailCondition({ key: 'a', equals: 1 }, scopeOf({ a: 1 }))).toBe(true)
    expect(evalDetailCondition({ key: 'a', equals: 1 }, scopeOf({ a: 2 }))).toBe(false)
    expect(evalDetailCondition({ key: 'a', not_equals: 0 }, scopeOf({ a: 2 }))).toBe(true)
    expect(evalDetailCondition({ key: 'a', truthy: true }, scopeOf({ a: 0 }))).toBe(false)
    expect(evalDetailCondition({ key: 'a', truthy: true }, scopeOf({ a: 3 }))).toBe(true)
  })

  it('all 是 AND，且递归求值', () => {
    const c = { all: [{ key: 'a', truthy: true }, { key: 'b', equals: 'x' }] }
    expect(evalDetailCondition(c, scopeOf({ a: 1, b: 'x' }))).toBe(true)
    expect(evalDetailCondition(c, scopeOf({ a: 1, b: 'y' }))).toBe(false)
    expect(evalDetailCondition(c, scopeOf({ a: 0, b: 'x' }))).toBe(false)
  })

  it('缺席的键按 undefined 求值：truthy 判 false、not_equals 判 true', () => {
    // 这条是「草稿节点没有 message_count」那类场景的判据来源，两边差异必须显式
    expect(evalDetailCondition({ key: 'missing', truthy: true }, scopeOf({}))).toBe(false)
    expect(evalDetailCondition({ key: 'missing', not_equals: 0 }, scopeOf({}))).toBe(true)
  })

  it('equals 用 JSON 形状比较（数组 / 对象按内容判等）', () => {
    expect(evalDetailCondition({ key: 'a', equals: ['x'] }, scopeOf({ a: ['x'] }))).toBe(true)
    expect(evalDetailCondition({ key: 'a', equals: ['x'] }, scopeOf({ a: ['y'] }))).toBe(false)
  })
})

describe('compactFieldText（选项栏按钮文本的唯一规则）', () => {
  const select = (partial: Partial<DetailField>): DetailField => ({
    key: 'k',
    label: '字段名',
    widget: 'select',
    options: [
      { value: 'a', label: '甲' },
      { value: 'b', label: '乙' },
    ],
    ...partial,
  })

  it('有值时按 options 值→标签查表（前端不硬编码取值）', () => {
    expect(compactFieldText(select({}), 'b')).toBe('乙')
  })

  it('无值时按字段 default 查表；default 也没有 ⇒ 回落字段 label', () => {
    expect(compactFieldText(select({ default: 'a' }), undefined)).toBe('甲')
    expect(compactFieldText(select({}), undefined)).toBe('字段名')
  })

  it('值不在候选里 ⇒ 原样显示值（陈旧取值仍可见，不是空白）', () => {
    expect(compactFieldText(select({}), 'gone')).toBe('gone')
  })

  it('path 取路径末段（两种分隔符都认）', () => {
    const f = select({ widget: 'path', options: [{ value: '', label: '未选择' }] })
    const win = ['D:', 'work', 'proj'].join(String.fromCharCode(92))
    expect(compactFieldText(f, win)).toBe('proj')
    expect(compactFieldText(f, '/home/u/proj/')).toBe('proj')
  })

  it('path 未设置 ⇒ options 里 value="" 的标签（「未选择目录」）', () => {
    const f = select({ widget: 'path', options: [{ value: '', label: '未选择' }] })
    expect(compactFieldText(f, undefined)).toBe('未选择')
  })

  it('form 先按子定义 title_from 取代表值再查表；无值则用字段 default', () => {
    const f = select({
      widget: 'form',
      options: [
        { value: 'true', label: '已开启' },
        { value: 'false', label: '未开启' },
      ],
      default: { enabled: false },
      form: {
        binding: 'option',
        title_from: ['enabled'],
        sections: [{ fields: [{ key: 'enabled', label: '开关', widget: 'toggle' }] }],
      },
    })
    expect(compactFieldText(f, { enabled: true })).toBe('已开启')
    expect(compactFieldText(f, { enabled: false })).toBe('未开启')
    expect(compactFieldText(f, undefined)).toBe('未开启')
  })

  it('form 的值不是对象（后端下发的形状未必守约）⇒ 不崩，回落 label', () => {
    const f = select({
      widget: 'form',
      form: { binding: 'option', title_from: ['enabled'], sections: [] },
    })
    expect(compactFieldText(f, 'oops')).toBe('字段名')
  })
})

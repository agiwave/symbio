/**
 * 表单 widget 表 —— 机制层的纯逻辑单测（node 环境）
 *
 * 这张表是「widget 名 → 呈现 + 编解码」的唯一来源，DetailForm 不再有任何按
 * widget 名的分支。故直接对它下断言，锁住六件事：
 *
 * 1. **未登记回落** `text`（页面永不空白，与消息域 / VDFS 域同一条兜底约定）；
 * 2. **呈现组归并**——10 个 widget 只对应 5 种呈现形态，同组的行为必须一致；
 * 3. **初始值**——开关是 `false`、其余是 `''`，且字段 `default` 优先；
 * 4. **编解码往返**——list / map 的编辑态文本与存储态结构能来回转换；
 * 5. **整行与只读**——结构化 widget 天然整行，`static` 唯一只读（不参与保存）；
 * 6. 占位提示的优先级（字段显式 > widget 缺省）。
 */

import { describe, expect, it } from 'vitest'
import type { DetailField } from '@/schemas/vdfs'
import {
  widgetFromEdit,
  widgetInitialOf,
  widgetIsFullWidth,
  widgetIsReadonly,
  widgetPlaceholderOf,
  widgetSpecOf,
  widgetToEdit,
} from '../formWidgets'

/** 造一个字段定义（只填判定要用到的键） */
function field(partial: Partial<DetailField> & { widget: string }): DetailField {
  return { key: 'k', label: 'K', ...partial }
}

describe('formWidgets：未登记回落', () => {
  it('未知 widget 名回落到 text（页面永不空白）', () => {
    const spec = widgetSpecOf('不存在的 widget')
    expect(spec.tag).toBe('input')
    expect(spec.inputType).toBe('text')
    expect(spec.readonly).toBe(false)
  })

  it('undefined 也回落（字段缺失 widget 时不炸）', () => {
    expect(widgetSpecOf(undefined).tag).toBe('input')
  })
})

describe('formWidgets：呈现组归并', () => {
  it('多行组：textarea / list / map 共用 textarea 形态', () => {
    for (const w of ['textarea', 'list', 'map']) {
      expect(widgetSpecOf(w).tag).toBe('textarea')
    }
  })

  it('单行组：text / password / number / datalist 共用 input 形态', () => {
    for (const w of ['text', 'password', 'number', 'datalist']) {
      expect(widgetSpecOf(w).tag).toBe('input')
    }
  })

  it('input 组各自的 type 与附加能力', () => {
    expect(widgetSpecOf('password').inputType).toBe('password')
    expect(widgetSpecOf('password').revealable).toBe(true)
    expect(widgetSpecOf('number').inputType).toBe('number')
    expect(widgetSpecOf('datalist').datalist).toBe(true)
    // 普通 text 不带这两种附加件
    expect(widgetSpecOf('text').revealable).toBeUndefined()
    expect(widgetSpecOf('text').datalist).toBeUndefined()
  })

  it('select / toggle / static 各自成组', () => {
    expect(widgetSpecOf('select').tag).toBe('select')
    expect(widgetSpecOf('toggle').tag).toBe('toggle')
    expect(widgetSpecOf('static').tag).toBe('static')
  })
})

describe('formWidgets：初始值', () => {
  it('开关是 false，文本类是空串（缺省值不能是 undefined，否则 v-model 失控）', () => {
    expect(widgetInitialOf(field({ widget: 'toggle' }))).toBe(false)
    expect(widgetInitialOf(field({ widget: 'text' }))).toBe('')
    expect(widgetInitialOf(field({ widget: 'list' }))).toBe('')
  })

  it('字段显式 default 优先于 widget 缺省', () => {
    expect(widgetInitialOf(field({ widget: 'text', default: 'abc' }))).toBe('abc')
    // 显式 false 也要保住（不能被 `??` 当成"没给"而换成 ''）
    expect(widgetInitialOf(field({ widget: 'toggle', default: false }))).toBe(false)
    expect(widgetInitialOf(field({ widget: 'toggle', default: true }))).toBe(true)
  })
})

describe('formWidgets：结构化编解码往返', () => {
  it('list：数组 ↔ 每行一项（去空行与首尾空白）', () => {
    expect(widgetFromEdit('list', 'x\n y \n\nz')).toEqual(['x', 'y', 'z'])
    // 存储态 → 编辑态 → 存储态 必须回到原值
    expect(widgetToEdit('list', ['a', 'b'])).toBe('a\nb')
    expect(widgetFromEdit('list', widgetToEdit('list', ['a', 'b']))).toEqual(['a', 'b'])
  })

  it('map：对象 ↔ 每行 KEY=VALUE（无 = 视为空值键）', () => {
    expect(widgetFromEdit('map', 'K=V\nE=')).toEqual({ K: 'V', E: '' })
    expect(widgetFromEdit('map', 'A=1')).toEqual({ A: '1' })
  })

  it('编解码只作用于结构化 widget，其余原样透传', () => {
    expect(widgetFromEdit('text', 'hello')).toBe('hello')
    expect(widgetFromEdit('text', undefined)).toBe('')
    // 非数组 / 非对象输入不炸，退化为空
    expect(widgetFromEdit('list', 42)).toEqual([])
    expect(widgetFromEdit('map', null)).toEqual({})
  })
})

describe('formWidgets：整行 / 只读 / 占位', () => {
  it('整行：textarea / list / map 天然整行，字段显式声明也生效', () => {
    expect(widgetIsFullWidth(field({ widget: 'text' }))).toBe(false)
    expect(widgetIsFullWidth(field({ widget: 'textarea' }))).toBe(true)
    expect(widgetIsFullWidth(field({ widget: 'list' }))).toBe(true)
    expect(widgetIsFullWidth(field({ widget: 'text', full_width: true }))).toBe(true)
  })

  it('只读：只有 static 不参与保存', () => {
    expect(widgetIsReadonly('static')).toBe(true)
    for (const w of ['text', 'select', 'toggle', 'list', 'map']) {
      expect(widgetIsReadonly(w)).toBe(false)
    }
  })

  it('占位：字段显式 > widget 缺省 > 空', () => {
    expect(widgetPlaceholderOf(field({ widget: 'list' }))).toBe('每行一项')
    expect(widgetPlaceholderOf(field({ widget: 'map' }))).toBe('每行一项：KEY=VALUE')
    expect(widgetPlaceholderOf(field({ widget: 'list', placeholder: '自定义' }))).toBe('自定义')
    expect(widgetPlaceholderOf(field({ widget: 'text' }))).toBe('')
  })
})

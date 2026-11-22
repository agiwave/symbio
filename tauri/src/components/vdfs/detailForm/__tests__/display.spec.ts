/**
 * 详情表单只读展示派生的单测。
 *
 * 这些函数原本埋在 `DetailForm.vue` 里，只能借 mount 组件覆盖；抽出后把
 * 「选项映射 / 子对象摘要 / 缺省动作补齐」的边界逐条钉住，防止拆分后语义漂移。
 */

import { describe, expect, it } from 'vitest'
import { staticDisplayOf, subDefinitionOf, subLabelOf, subSummaryOf } from '../display'
import type { DetailField } from '@/schemas/vdfs'

function field(extra: Partial<DetailField> = {}): DetailField {
  return { key: 'k', label: '字段', widget: 'text', ...extra }
}

describe('staticDisplayOf —— static 字段只读展示', () => {
  const f = field({
    options: [
      { value: 'a', label: '甲' },
      { value: 'b', label: '乙' },
    ],
  })

  it.each([
    [null, '—'],
    [undefined, '—'],
    ['', '—'],
    [[], '—'],
    [{}, '—'],
  ])('空值 %j 显示为占位符', (value, expected) => {
    expect(staticDisplayOf(f, value)).toBe(expected)
  })

  it('option 值映射为标签', () => {
    expect(staticDisplayOf(f, 'a')).toBe('甲')
  })

  it('数组按「、」展开，对象按 key value 展开', () => {
    expect(staticDisplayOf(f, ['x', 'y'])).toBe('x、y')
    expect(staticDisplayOf(f, { host: 'localhost', port: 8080 })).toBe(
      'host localhost、port 8080'
    )
  })

  it('未登记的字面值原样输出', () => {
    expect(staticDisplayOf(f, 'z')).toBe('z')
  })
})

describe('subLabelOf —— 子字段展示名', () => {
  const f = field({
    form: {
      binding: 'option',
      sections: [{ fields: [{ key: 'host', label: '地址', widget: 'text' }] }],
    },
  })

  it('命中子定义返回 label，未命中回退 key', () => {
    expect(subLabelOf(f, 'host')).toBe('地址')
    expect(subLabelOf(f, 'port')).toBe('port')
  })

  it('没有子定义时回退 key', () => {
    expect(subLabelOf(field(), 'host')).toBe('host')
  })
})

describe('subSummaryOf —— 子对象一行摘要', () => {
  const f = field({
    form: {
      binding: 'option',
      sections: [{ fields: [{ key: 'host', label: '地址', widget: 'text' }] }],
    },
  })

  it('过滤空值并按子定义 label 展示', () => {
    expect(subSummaryOf(f, { host: 'localhost', port: 8080, note: '', other: null })).toBe(
      '地址=localhost，port=8080'
    )
  })

  it('非对象 / 空对象显示「未配置」', () => {
    expect(subSummaryOf(f, 'oops')).toBe('未配置')
    expect(subSummaryOf(f, {})).toBe('未配置')
  })
})

describe('subDefinitionOf —— 子表单定义兜底', () => {
  it('补 binding 兜底、标题回落与缺省的取消/确定动作', () => {
    const def = subDefinitionOf(field({ label: '连接设置' }))
    expect(def.binding).toBe('option')
    expect(def.title_fallback).toBe('连接设置')
    expect(def.actions?.map((a) => a.id)).toEqual(['cancel', 'save'])
  })

  it('已有同 id 动作不被缺省动作覆盖', () => {
    const f = field({
      form: {
        binding: 'upload',
        sections: [],
        actions: [{ id: 'save', label: '保存并测试', style: 'primary' }],
      },
    })
    const def = subDefinitionOf(f)
    expect(def.binding).toBe('upload')
    expect(def.actions?.map((a) => a.label)).toEqual(['保存并测试', '取消'])
  })
})

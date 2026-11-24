/**
 * 详情表单的**只读展示派生**（从 `DetailForm.vue` 按职责拆出）。
 *
 * 与同目录的 `save.ts` 同一手法：纯函数，只吃字段定义与当前值，不碰组件的
 * 响应式状态——于是可以直接单测，组件只负责把 `form[f.key]` 传进来。
 *
 * 依赖方向不变：本文件在 components/vdfs 层，只向下依赖 schemas（契约层）；
 * 组件 → helper 是本模块内部新增的一条引用，不是跨层依赖。
 */

import type { DetailAction, DetailDefinition, DetailField } from '@/schemas/vdfs'

/** 子表单固定的两个动作（子定义未声明时补上，声明了则以子定义为准） */
const SUB_FORM_ACTIONS: DetailAction[] = [
  { id: 'cancel', label: '取消', style: 'default' },
  { id: 'save', label: '确定', style: 'primary' },
]

/** 子字段的展示名：在子定义里按 key 找 label，找不到就原样回显 key */
export function subLabelOf(f: DetailField, key: string): string {
  for (const sec of f.form?.sections ?? []) {
    const hit = sec.fields.find((x) => x.key === key)
    if (hit) return hit.label
  }
  return key
}

/** 子对象摘要（一行；避免把整棵子树摊进父表单） */
export function subSummaryOf(f: DetailField, value: unknown): string {
  if (!value || typeof value !== 'object') return '未配置'
  const entries = Object.entries(value as Record<string, unknown>).filter(
    ([, val]) => val !== '' && val !== null && val !== undefined
  )
  if (!entries.length) return '未配置'
  return entries.map(([k, val]) => `${subLabelOf(f, k)}=${String(val)}`).join('，')
}

/** 子定义：补 `binding` 兜底、标题回落、以及缺省的两个动作 */
export function subDefinitionOf(f: DetailField): DetailDefinition {
  const def: DetailDefinition = f.form ?? { binding: 'option', sections: [] }
  const declared = new Set((def.actions ?? []).map((a) => a.id))
  return {
    ...def,
    binding: def.binding || 'option',
    title_fallback: def.title_fallback || f.label,
    actions: [...(def.actions ?? []), ...SUB_FORM_ACTIONS.filter((a) => !declared.has(a.id))],
  }
}

/** static 只读展示：options 值→标签映射，数组/对象友好展开 */
export function staticDisplayOf(f: DetailField, value: unknown): string {
  if (value == null || value === '') {
    if (f.default != null && f.default !== '') return String(f.default)
    return '—'
  }
  const opt = (f.options ?? []).find((o) => o.value === value)
  if (opt) return opt.label
  if (Array.isArray(value)) return value.length ? value.join('、') : '—'
  if (typeof value === 'object') {
    const entries = Object.entries(value as Record<string, unknown>)
    return entries.length ? entries.map(([k, val]) => `${k} ${val}`).join('、') : '—'
  }
  return String(value)
}

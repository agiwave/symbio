/**
 * 详情表单的保存载荷派生（从 `DetailForm.vue` 按职责拆出）。
 *
 * 这一层是**纯函数**：字段序列化（list → string[]、map → 对象）、id 派生、
 * name 回落链，只看传入的 `SaveContext`，不认识组件的响应式状态——
 * 于是它可以直接单测，组件只负责把当前 props/form 装成 context。
 *
 * `isVisible`（`visible_when` 求值）与 `idFrom` / `nameFrom`（回落链首值）由组件
 * 传入——它们依赖表单模型与 `valueOf`，取值口径留在组件里，本模块只决定
 * 「哪些字段参与保存 / id 怎么去重 / name 怎么补」。
 */

import type { DetailDefinition, DetailField, VdfsNode } from '@/schemas/vdfs'
import { widgetFromEdit, widgetIsReadonly, widgetOmitsWhenEmpty } from '@/registry/formWidgets'

/** 保存载荷派生所需的全部输入（组件一侧的当前值快照）。 */
export interface SaveContext {
  definition: DetailDefinition
  form: Record<string, unknown>
  /** `visible_when` 求值（住在组件里）；隐藏字段不参与保存 */
  isVisible: (f: DetailField) => boolean
  isUpload: boolean
  node: VdfsNode | null
  existingIds: string[]
  /** `id_from` 回落链的**首值**（组件已用 `valueOf` 解析） */
  idFrom: string
  /** `name_from` 回落链的**首值** */
  nameFrom: string
}

export function isEmpty(v: unknown): boolean {
  return v == null || v === '' || (Array.isArray(v) && v.length === 0)
}

/** 可读 slug + `-2` 递增去重（机制内唯一实现，新建态 id 派生用） */
export function generateId(base: string, existingIds: string[]): string {
  const used = new Set(existingIds)
  const slug =
    base
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9-_]+/g, '-')
      .replace(/^-+|-+$/g, '') || 'resource'
  let id = slug
  let counter = 2
  while (used.has(id)) {
    id = `${slug}-${counter}`
    counter++
  }
  return id
}

/**
 * 字段序列化（机制唯一实现）：list → string[]、map → 对象；static 只读。
 *
 * `visible_when` 不满足的字段不参与保存（如 mcp 的 stdio/http 互斥字段），
 * 也不参与本地必填与 JSON 语法预检。
 */
function buildValues(ctx: SaveContext): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const sec of ctx.definition.sections) {
    for (const f of sec.fields) {
      if (widgetIsReadonly(f.widget)) continue
      if (!ctx.isVisible(f)) continue
      // 数字字段的编辑态空串 = 没填：省略键，让后端的 `serde(default)` 决定
      // 「不限制 / 取默认」。提交空串会让 `Option<u32>` / `f64` 反序列化失败。
      if (widgetOmitsWhenEmpty(f.widget) && isEmpty(ctx.form[f.key])) continue
      out[f.key] = widgetFromEdit(f.widget, ctx.form[f.key])
    }
  }
  return out
}

/**
 * 保存载荷（机制唯一入口）：
 * - 非 upload 绑定：纯字段值，由页面写回它自己的通道；
 * - upload 绑定：带上 id（既有节点用 `node.name`，新建态按定义派生），
 *   并补齐 name 回落链（首个非空字段补 name，不覆盖用户已填的 name）。
 */
export function buildSave(ctx: SaveContext): Record<string, unknown> {
  if (!ctx.isUpload) return buildValues(ctx)
  const id = ctx.node?.name || generateId(ctx.idFrom || 'resource', ctx.existingIds)
  const values = buildValues(ctx)
  const manifest: Record<string, unknown> = { id, ...values }
  const nameKey = ctx.definition.name_from?.[0]
  if (nameKey && ctx.nameFrom && isEmpty(ctx.form[nameKey])) {
    manifest[nameKey] = ctx.nameFrom
  }
  return manifest
}

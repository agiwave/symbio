/**
 * 表单 widget 表 —— 「widget 名 → 呈现形态 + 编解码行为」的**唯一来源**
 *
 * ## 分层（与 `registry/messageTypes.ts` 同一套）
 *
 * - `schemas/vdfs-form.ts`  数据契约：字段定义 + 结构化取值的编解码纯函数
 * - 本文件                  **纯 UI 映射**：widget 名 → 呈现组 / 初始值 / 编解码 /
 *                           整行与否 / 是否只读，零组件导入
 * - `components/vdfs/DetailForm.vue`  消费方：只做「查表 → 按 `tag` 渲染五个呈现组」
 *
 * ## 为什么是「表 + 呈现组」而不是每个 widget 一个组件
 *
 * 10 个 widget 里，真正不同的只是**行为**（怎么把存储值变成编辑值、怎么序列化
 * 回去、占不占整行），而**呈现**只有五种形态（只读展示 / 开关 / 下拉 / 多行框 /
 * 单行输入框）——`textarea`/`list`/`map` 共用多行框，`text`/`password`/`number`/
 * `datalist` 共用单行输入框。因此本表同时给出这两样东西：
 *
 * - `tag`     → 模板按它分派五个呈现组（不再是按 widget 名的 14 分支 v-if 链）；
 * - 其余字段 → 该 widget 独有的行为。
 *
 * 于是「新增一种 widget」= 在此加一行；**无需改动渲染器**。未登记的 widget 名
 * 回落到 `text`（与消息域 / VDFS 域「未登记 → 兜底」同一条约定：页面永不空白）。
 *
 * ## 此前这份知识散在 DetailForm 的 8 处
 *
 * 初始值（`initForm`）、整行判定（`isFullWidth`）、占位提示
 * （`structuredPlaceholder`）、预填转换（`toEditValue`）、保存序列化
 * （`buildValues`）、只读过滤（`f.widget === 'static'`）各写一遍，且模板里还有
 * 一条按 widget 名的 v-if 链。收敛到本表后，那条链只剩五个呈现组。
 */

import type { DetailField } from '@/schemas/vdfs'
import { editTextOf, parseListValue, parseMapValue } from '@/schemas/vdfs-form'

/** widget 名（由后端 `DetailDefinition` 下发；前端不发明新取值） */
export type FormWidget =
  | 'text'
  | 'password'
  | 'number'
  | 'datalist'
  | 'textarea'
  | 'list'
  | 'map'
  | 'select'
  | 'toggle'
  | 'static'

/**
 * 呈现组 —— 模板真正分派的东西。
 *
 * - `static`   只读展示（值→标签映射），不入表单模型
 * - `toggle`   复选框
 * - `select`   下拉
 * - `textarea` 多行文本（textarea / list / map 共用）
 * - `input`    单行输入（text / password / number / datalist 共用）
 */
export type FormWidgetTag = 'static' | 'toggle' | 'select' | 'textarea' | 'input'

export interface FormWidgetSpec {
  tag: FormWidgetTag
  /** 编辑态初始值（字段未声明 `default` 时用） */
  initial: unknown
  /** 存储态 → 编辑态 */
  toEdit(v: unknown): unknown
  /** 编辑态 → 存储态（保存序列化） */
  fromEdit(v: unknown): unknown
  /** 天然占整行（显式 `full_width` 之外的原因） */
  fullWidth: boolean
  /** 只读展示：不参与保存 */
  readonly: boolean
  /** 未显式给 `placeholder` 时的格式提示 */
  placeholder?: string
  /** `input` 组的 `type`（明文切换时由渲染器改写成 `text`） */
  inputType?: string
  /** 密码：带显隐切换按钮 */
  revealable?: boolean
  /** 附带 `<datalist>` 建议 */
  datalist?: boolean
}

/** 透传：编辑态与存储态同形（多数 widget 如此） */
const identity = (v: unknown): unknown => v ?? ''

/** widget 名 → 行为（**唯一的硬编码表**，纯 UI 约定） */
const WIDGET_SPECS: Record<string, FormWidgetSpec> = {
  text: { tag: 'input', initial: '', toEdit: identity, fromEdit: identity, fullWidth: false, readonly: false, inputType: 'text' },
  password: {
    tag: 'input', initial: '', toEdit: identity, fromEdit: identity,
    fullWidth: false, readonly: false, inputType: 'password', revealable: true,
  },
  number: { tag: 'input', initial: '', toEdit: identity, fromEdit: identity, fullWidth: false, readonly: false, inputType: 'number' },
  datalist: {
    tag: 'input', initial: '', toEdit: identity, fromEdit: identity,
    fullWidth: false, readonly: false, inputType: 'text', datalist: true,
  },
  textarea: { tag: 'textarea', initial: '', toEdit: identity, fromEdit: identity, fullWidth: true, readonly: false },
  // 结构化 widget：编辑态是多行文本，存储态是数组 / 对象（两侧约定见 schemas/vdfs-form）
  list: {
    tag: 'textarea', initial: '',
    toEdit: (v) => editTextOf('list', v), fromEdit: parseListValue,
    fullWidth: true, readonly: false, placeholder: '每行一项',
  },
  map: {
    tag: 'textarea', initial: '',
    toEdit: (v) => editTextOf('map', v), fromEdit: parseMapValue,
    fullWidth: true, readonly: false, placeholder: '每行一项：KEY=VALUE',
  },
  select: { tag: 'select', initial: '', toEdit: identity, fromEdit: identity, fullWidth: false, readonly: false },
  toggle: { tag: 'toggle', initial: false, toEdit: identity, fromEdit: identity, fullWidth: false, readonly: false },
  static: { tag: 'static', initial: '', toEdit: identity, fromEdit: identity, fullWidth: false, readonly: true },
}

/** 未登记 widget 名的回落（与消息域 / VDFS 域同一条兜底约定） */
const FALLBACK: FormWidgetSpec = WIDGET_SPECS.text!

/** 查表；未登记回落 `text`（页面永不空白） */
export function widgetSpecOf(widget: string | undefined): FormWidgetSpec {
  return (widget && WIDGET_SPECS[widget]) || FALLBACK
}

/** 编辑态初始值（字段 `default` 优先，否则取 widget 的） */
export function widgetInitialOf(f: DetailField): unknown {
  return f.default ?? widgetSpecOf(f.widget).initial
}

/** 存储态 → 编辑态（预填入口） */
export function widgetToEdit(widget: string | undefined, v: unknown): unknown {
  return widgetSpecOf(widget).toEdit(v)
}

/** 编辑态 → 存储态（保存序列化入口） */
export function widgetFromEdit(widget: string | undefined, v: unknown): unknown {
  return widgetSpecOf(widget).fromEdit(v)
}

/** 该 widget 是否只读展示（只读字段不参与保存） */
export function widgetIsReadonly(widget: string | undefined): boolean {
  return widgetSpecOf(widget).readonly
}

/** 整行布局：字段显式声明，或该 widget 天然占整行 */
export function widgetIsFullWidth(f: DetailField): boolean {
  return Boolean(f.full_width) || widgetSpecOf(f.widget).fullWidth
}

/** 占位提示：字段显式声明优先，否则取 widget 的格式提示 */
export function widgetPlaceholderOf(f: DetailField): string {
  return f.placeholder ?? widgetSpecOf(f.widget).placeholder ?? ''
}

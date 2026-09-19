/**
 * VDFS `ext = form` 的**宿主方言**（前端侧类型契约）
 *
 * 与后端对齐：详情定义由 provider 的 `detail_definition` 产出，随 `VdfsNode.schema`
 * **原样透传**到前端（VDFS 不解释其内容）。规范：docs/design/vdfs.md。
 *
 * **本文件是数据契约层：零组件知识**（不得导入任何 Vue 组件）。
 * 渲染它的是 `components/vdfs/DetailForm.vue`。
 *
 * 表单的**取值输入**与定义分开：定义说「有哪些字段」，字段当前值来自
 * `vdfs/read` 返回的 `VdfsContent.text`（JSON 文本，前端 parse 成对象后
 * 作为显式入参交给渲染器）。节点上**没有** config / extra 这类携带正文的字段。
 */

/**
 * 表单渲染器的能力位（`cap.<name>` 条件求值来源）。
 *
 * VDFS **不下发**能力表——它由渲染器按节点的访问位与详情定义声明的动作**自行计算**
 * （见 `VdfsFormDetail` 的 `capabilities`）。`DetailForm` 的 `capabilities`
 * 入参即为 `Record<string, boolean>`（当前仅 `mutable` / `test_connection`），不再另设类型。
 */

// ==================== 详情页定义（definition-driven detail） ====================

/** 条件谓词（徽标/动作显隐、字段条件显隐 visible_when）。`all` 存在时为 AND 组合 */
export interface DetailCondition {
  /** 求值键：表单字段名，或特殊键 is_existing / is_default / cap.<name> */
  key: string
  equals?: unknown
  not_equals?: unknown
  truthy?: boolean
  all?: DetailCondition[]
}

/** select 选项 */
export interface DetailOption {
  value: string
  label: string
}

/**
 * 表单字段定义。widget ∈ text|password|number|select|textarea|toggle|datalist|list|map|static
 * 结构化 widget 表单模型约定（渲染器与后端 validate_manifest 两侧一致）：
 * - list：字符串数组，编辑态每行一项
 * - map：字符串键值对，编辑态每行 KEY=VALUE
 * - static：只读展示（info 绑定），options 可作值→标签映射
 */
export interface DetailField {
  key: string
  label: string
  description?: string
  required?: boolean
  widget: string
  /** 条件显隐（不满足时整行不渲染） */
  visible_when?: DetailCondition
  placeholder?: string
  min?: number
  max?: number
  step?: number
  rows?: number
  options?: DetailOption[]
  suggestions?: string[]
  options_from_preset?: boolean
  suggestions_from_preset?: boolean
  full_width?: boolean
  default?: unknown
}

/** 分区（可折叠） */
export interface DetailSection {
  title?: string
  collapsed?: boolean
  fields: DetailField[]
}

/** 预设项：选中后按 set 填充字段，options 注入对应字段动态候选 */
export interface DetailPreset {
  value: string
  label: string
  /** 按 fill 策略填充（if_empty/always） */
  set?: Record<string, unknown>
  /** 总是覆盖（如协议校正） */
  set_always?: Record<string, unknown>
  options?: Record<string, string[]>
}

/** 预设联动规格：field 为触发字段；fill ∈ if_empty | always */
export interface DetailPresetSpec {
  field: string
  fill: string
  presets: DetailPreset[]
}

/** 标题区徽标（如「默认」「已停用」） */
export interface DetailBadge {
  when?: DetailCondition
  label: string
  style: string
}

/**
 * 动作按钮。id ∈ save|test|delete|set-default|open-container（payload.kind 指定子类别）或自定义。
 * icon：图标名（可选）——语义动作 id 自带默认图标映射；仅当需要区分同 id 多形态
 * （如「跳过校验保存」）或自定义动作需要图标时显式指定；未知图标名回落为文字按钮。
 */
export interface DetailAction {
  id: string
  label: string
  style: string
  icon?: string
  when?: DetailCondition
  disabled_when?: DetailCondition
  payload?: Record<string, unknown>
  busy_label?: string
}

/**
 * 详情页定义。binding ∈
 * - `upload`：清单型资源，保存交回「id + 完整字段值」（id 由节点名给出或按定义派生）；
 * - `info`  ：只读概览，static 字段取值来自节点顶层的扩展字段（flatten 的 attributes）；
 * - `option`：数据来自外部、保存只交回纯字段值 —— VDFS 的 `form` 节点与级联选项表单
 *             都落在这两种形态上（取值统一由渲染器的显式入参传入）。
 */
export interface DetailDefinition {
  binding: string
  title_from?: string[]
  title_fallback?: string
  subtitle_from?: string[]
  name_from?: string[]
  id_from?: string[]
  sections: DetailSection[]
  presets?: DetailPresetSpec
  badges?: DetailBadge[]
  actions?: DetailAction[]
}

// ==================== 动作区装配（机制唯一实现） ====================

/** 动作分组的视觉分隔符（`VdfsActions` 已内置其渲染） */
export const DETAIL_ACTION_DIVIDER: DetailAction = { id: 'divider', label: '', style: 'divider' }

/**
 * 把「渲染器自有动作」与「机制动作」装配成一行动作区。
 *
 * 动作来自两个来源，职责不同：`own` 随资源形态而变（save / reset / test /
 * open-container…），由渲染器声明；`mechanism` 是页面对**任何已落盘的可写节点**
 * 都能做的默认动作（重命名 / 删除），由页面单点算好
 * （`useVdfs.mechanismActions`）——渲染器不该各算一遍。
 *
 * 规则只有两条，因此只有这一份实现：
 *
 * 1. 两段之间插一个 `DETAIL_ACTION_DIVIDER`（视觉分组）；
 * 2. **同 id 时渲染器声明的那一份胜出** —— 渲染器对「这是哪个动作」更具体
 *    （如后端详情定义给 `delete` 配了更贴切的文案），机制只负责保证它存在。
 *    于是「定义声明了 delete」与「机制兜底提供 delete」不会渲染成两个按钮。
 *
 * 返回的三个数组**等长且按索引对齐**（`VdfsActions` 的入参形状）：自有动作的
 * 忙态取自调用方给的等长数组；机制动作同时最多只有一个在跑，故按 id 判定。
 */
export function mergeDetailActions(
  own: DetailAction[],
  mechanism: DetailAction[],
  ownBusy: boolean[] = [],
  ownDisabled: boolean[] = [],
  mechanismBusyId: string | null = null,
): { actions: DetailAction[]; busy: boolean[]; disabled: boolean[] } {
  const actions: DetailAction[] = []
  const busy: boolean[] = []
  const disabled: boolean[] = []
  own.forEach((a, i) => {
    actions.push(a)
    busy.push(Boolean(ownBusy[i]))
    disabled.push(Boolean(ownDisabled[i]))
  })
  const declared = new Set(own.map((a) => a.id))
  const extra = mechanism.filter((a) => !declared.has(a.id))
  if (extra.length) {
    if (actions.length) {
      actions.push(DETAIL_ACTION_DIVIDER)
      busy.push(false)
      disabled.push(false)
    }
    for (const a of extra) {
      actions.push(a)
      busy.push(a.id === mechanismBusyId)
      disabled.push(false)
    }
  }
  return { actions, busy, disabled }
}

// ==================== 预设联动（机制唯一实现） ====================
//
// 「选中某个预设 ⇒ 联动填充若干字段 / 注入若干动态候选」这一整套规则原先写在
// `DetailForm.vue` 的 computed 与 `applyPreset` 里，只能经**组件挂载**间接覆盖。
// 而它恰是本层最绕的一段（fill 策略 / set_always / 动态候选 / 建议合并）——
// 最该被直接钉住的规则，覆盖却最薄。故下沉为纯函数：**规则在这里，状态在组件**。

/** 判定空值（`set` 的 `if_empty` 策略用；与上传绑定的 id/name 回落链同口径） */
function isDetailValueEmpty(v: unknown): boolean {
  return v == null || v === '' || (Array.isArray(v) && v.length === 0)
}

/** 由触发字段的当前值解出命中的预设（无规格 / 未命中 → `null`） */
export function detailPresetOf(
  spec: DetailPresetSpec | null | undefined,
  triggerValue: unknown,
): DetailPreset | null {
  if (!spec) return null
  return spec.presets.find((p) => p.value === triggerValue) ?? null
}

/**
 * 字段候选：`options_from_preset` 时取当前预设注入的候选（**值即标签**——
 * 预设给的是协议名 / 模型名一类标识，没有另一套展示文案），否则用静态 options。
 */
export function detailPresetFieldOptions(
  f: DetailField,
  preset: DetailPreset | null,
): DetailOption[] {
  if (!f.options_from_preset) return f.options ?? []
  return (preset?.options?.[f.key] ?? []).map((v) => ({ value: v, label: v }))
}

/** datalist 建议：静态 `suggestions` 在前 + 预设动态候选在后（去重） */
export function detailPresetFieldSuggestions(
  f: DetailField,
  preset: DetailPreset | null,
): string[] {
  const staticSug = f.suggestions ?? []
  if (!f.suggestions_from_preset) return staticSug
  const dyn = preset?.options?.[f.key] ?? []
  return [...staticSug, ...dyn.filter((v) => !staticSug.includes(v))]
}

/**
 * 预设变更要写入的**字段补丁**（不直接改表单模型：规则层不持状态）。
 *
 * 两条规则，缺一不可：
 *
 * 1. `set` 按 `spec.fill` 策略填充 —— `if_empty` 只补空字段（不覆盖用户已填的），
 *    `always` 一律覆盖；
 * 2. `set_always` **总是**覆盖，且**不受 `applySet` 影响**（如切换预设时把协议
 *    校正为该预设支持的首个协议——「编辑态不重填」不该把它一并跳过）。
 *
 * @param applySet `false` = 只应用 `set_always`。编辑态预填走这条路：不覆盖用户
 *   已填的一般字段，但仍要校正必须一致的字段。
 * @param valueOf  取字段当前值（表单模型在组件里，规则层只问不存）
 */
export function detailPresetPatch(
  spec: DetailPresetSpec | null | undefined,
  preset: DetailPreset | null,
  valueOf: (key: string) => unknown,
  applySet: boolean,
): Record<string, unknown> {
  const patch: Record<string, unknown> = {}
  if (spec && preset && applySet && preset.set) {
    const ifEmpty = spec.fill !== 'always'
    for (const [k, v] of Object.entries(preset.set)) {
      if (!ifEmpty || isDetailValueEmpty(valueOf(k))) patch[k] = v
    }
  }
  // set_always 后写：同一键同时出现在 set 与 set_always 时，以后者为准
  if (preset?.set_always) {
    for (const [k, v] of Object.entries(preset.set_always)) patch[k] = v
  }
  return patch
}

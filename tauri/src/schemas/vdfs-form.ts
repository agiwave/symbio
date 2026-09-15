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
 * - `config`：配置分区，经 load/save_path 自持读写（插件路由，非 VDFS 通道）；
 * - `info`  ：只读概览，static 字段取值来自节点顶层的扩展字段（flatten 的 attributes）；
 * - `option`：数据来自外部、保存只交回纯字段值 —— VDFS 的 `form` 节点与级联选项表单
 *             都落在这两种形态上（取值统一由渲染器的显式入参传入）。
 */
export interface DetailDefinition {
  binding: string
  load_path?: string
  save_path?: string
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

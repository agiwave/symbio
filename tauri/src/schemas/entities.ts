/**
 * 统一实体协议（前端侧类型契约）
 *
 * 与后端大面积对齐：symbio/src/symbio_core/schemas/entities.rs
 * 覆盖 model / mcp / agent / skill / session 等实体类型。
 * 所有实体共享同一套 entities/* 操作与能力开关，前端据此驱动统一页面。
 *
 * 实体类型（kind）已开放为 string：类型的**存在性/能力/前缀**以后端
 * `entities/providers` 下发的 ProviderInfo 为单一真相源，前端不再硬编码类型清单。
 */

/**
 * 实体能力开关——决定统一页面启用哪些模块 */
export interface EntityCapabilities {
  /** 以上传 zip 为主（文件名即实体目录名） */
  zip_upload: boolean
  /** 是否有独立表单 */
  independent_form: boolean
  /** 列表项是否有实时状态 */
  realtime_status: boolean
  /** 是否可写（可上传新增 / 删除） */
  mutable: boolean
  /** 是否支持连接测试 */
  test_connection: boolean
  /** 是否默认只读 */
  read_only: boolean
}

/**
 * 实体类型（provider）注册信息 —— 前端从后端 `entities/providers` 拉取。
 * 对应后端 symbio_core::schemas::entities::ProviderInfo。
 */
export interface ProviderInfo {
  kind: string
  /** 提供方显示名，用于路径 [provider]/[id].[kind] */
  provider_name: string
  /** 实体操作路径前缀（entitiesOp 拼接 `${prefix}/entities/<op>`） */
  prefix: string
  capabilities: EntityCapabilities
  /** 展示顺序（导航 / 类型选择排序） */
  order: number
  /** 展示标签 */
  label: string
  /** 是否支持在实体管理器内创建/删除（session=false） */
  supports_upload: boolean
  /** 列表简洁模式：仅显示类型图标 + 标题（如设置分区） */
  compact_list?: boolean
  /** 列表项是否显示运行状态图示（如设置分区为 false，隐藏状态点）；缺省 true */
  status_indicator?: boolean
  /** 容器声明：条目内部托管的子实体类型（空/缺省 = 条目不是容器） */
  container_kinds?: ContainerKindInfo[]
}

/** entities/providers 响应 */
export interface ProvidersResponse {
  providers: ProviderInfo[]
}

/**
 * 容器子实体类型声明 —— 该 provider 的条目本身是「容器」，内部托管这些子类型。
 *
 * 如 agent（OAB bundle）内部托管 prompt / skill / mcp。容器实体页的左侧类别导航、
 * 新建路径模板、新建内容模板均由此下发——后端控制，前端零硬编码。
 */
export interface ContainerKindInfo {
  /** 子实体类型（如 prompt / skill / mcp） */
  kind: string
  /** 展示标签 */
  label: string
  /** 语义说明（新建/编辑表单提示文本） */
  description?: string
  /** 新建路径模板（<name> 占位符），如 prompts/<name>.md */
  path_hint?: string
  /** 新建内容模板（编辑器初始内容） */
  default_content?: string
  /** 子实体能力开关 */
  capabilities: EntityCapabilities
}

/** 统一实体概要（列表项） */
export interface EntitySummary {
  kind: string
  /** 提供方（插件）显示名，用于实体路径 [provider]/[id].[kind]；后端 dispatch 统一回填 */
  provider?: string
  name: string
  id: string
  description?: string
  summary?: string
  updated_at?: number
  status: string
  status_detail?: string
  // 类型特有扩展字段（flatten）
  [extra: string]: unknown
}

/** entities/list 响应（请求携带 container 时为容器语义，container 回显容器 id） */
export interface EntitiesListResponse {
  kind: string
  capabilities: EntityCapabilities
  items: EntitySummary[]
  /** 容器作用域（容器语义时下发；顶层列表缺省） */
  container?: string
}

/** entities/upload 响应 */
export interface EntityUploadResponse {
  kind: string
  id: string
  created: boolean
}

/** entities/status 响应 */
export interface EntityStatusResponse {
  kind: string
  id: string
  status: string
  status_detail?: string
}

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
 * 动作按钮。id ∈ save|test|delete|set-default|open-container（payload.kind 指定容器类别）或自定义。
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

/** 详情页定义。binding ∈ upload（实体实体，保存走 entities/upload）| config（配置分区，经 load/save_path 读写）| info（只读概览，字段取值来自 item.config/extra） */
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

/** entities/detail 响应（definition = null 表示无定义） */
export interface DetailDefinitionResponse {
  definition: DetailDefinition | null
}

/** 各类型标签（前端兜底展示用；后端 ProviderInfo.label 为权威，未下发时用此表） */
export const ENTITY_LABELS: Record<string, string> = {
  session: '会话',
  model: '模型',
  agent: '智能体',
  skill: '技能',
  mcp: 'MCP',
  setting: '设置',
}
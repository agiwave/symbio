/**
 * 统一资源协议（前端侧类型契约）
 *
 * 与后端大面积对齐：symbio/src/symbio_core/schemas/resources.rs
 * 覆盖 model / mcp / agent / skill / session 等资源类型。
 * 所有资源共享同一套 resources/* 操作与能力开关，前端据此驱动统一页面。
 *
 * 资源类型（kind）已开放为 string：类型的**存在性/能力/前缀**以后端
 * `resources/providers` 下发的 ProviderInfo 为单一真相源，前端不再硬编码类型清单。
 */

/**
 * 资源能力开关——决定统一页面启用哪些模块 */
export interface ResourceCapabilities {
  /** 以上传 zip 为主（文件名即资源目录名） */
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
 * 资源类型（provider）注册信息 —— 前端从后端 `resources/providers` 拉取。
 * 对应后端 symbio_core::schemas::resources::ProviderInfo。
 */
export interface ProviderInfo {
  kind: string
  /** 提供方显示名，用于路径 [provider]/[id].[kind] */
  provider_name: string
  /** 资源操作路径前缀（resourcesOp 拼接 `${prefix}/resources/<op>`） */
  prefix: string
  capabilities: ResourceCapabilities
  /** 展示顺序（导航 / 类型选择排序） */
  order: number
  /** 展示标签 */
  label: string
  /** 是否支持在资源管理器内创建/删除（session=false） */
  supports_upload: boolean
  /** 列表简洁模式：仅显示类型图标 + 标题（如设置分区） */
  compact_list?: boolean
  /** 列表项是否显示运行状态图示（如设置分区为 false，隐藏状态点）；缺省 true */
  status_indicator?: boolean
  /** 容器声明：条目内部托管的子资源类型（空/缺省 = 条目不是容器） */
  container_kinds?: ContainerKindInfo[]
}

/** resources/providers 响应 */
export interface ProvidersResponse {
  providers: ProviderInfo[]
}

/**
 * 容器子资源类型声明 —— 该 provider 的条目本身是「容器」，内部托管这些子类型。
 *
 * 如 agent（OAB bundle）内部托管 prompt / skill / mcp。容器资源页的左侧类别导航、
 * 新建路径模板、新建内容模板均由此下发——后端控制，前端零硬编码。
 */
export interface ContainerKindInfo {
  /** 子资源类型（如 prompt / skill / mcp） */
  kind: string
  /** 展示标签 */
  label: string
  /** 语义说明（新建/编辑表单提示文本） */
  description?: string
  /** 新建路径模板（<name> 占位符），如 prompts/<name>.md */
  path_hint?: string
  /** 新建内容模板（编辑器初始内容） */
  default_content?: string
  /** 子资源能力开关 */
  capabilities: ResourceCapabilities
}

/** 统一资源概要（列表项） */
export interface ResourceSummary {
  kind: string
  /** 提供方（插件）显示名，用于资源路径 [provider]/[id].[kind]；后端 dispatch 统一回填 */
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

/** resources/list 响应（请求携带 container 时为容器语义，container 回显容器 id） */
export interface ResourcesListResponse {
  kind: string
  capabilities: ResourceCapabilities
  items: ResourceSummary[]
  /** 容器作用域（容器语义时下发；顶层列表缺省） */
  container?: string
}

/** resources/upload 响应 */
export interface ResourceUploadResponse {
  kind: string
  id: string
  created: boolean
}

/** resources/status 响应 */
export interface ResourceStatusResponse {
  kind: string
  id: string
  status: string
  status_detail?: string
}

// ==================== 详情页定义（definition-driven detail） ====================

/** 条件谓词（徽标/动作显隐）。`all` 存在时为 AND 组合 */
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

/** 表单字段定义。widget ∈ text|password|number|select|textarea|toggle|datalist */
export interface DetailField {
  key: string
  label: string
  description?: string
  required?: boolean
  widget: string
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

/** 动作按钮。id ∈ save|test|delete|set-default 或自定义 */
export interface DetailAction {
  id: string
  label: string
  style: string
  when?: DetailCondition
  disabled_when?: DetailCondition
  payload?: Record<string, unknown>
  busy_label?: string
}

/** 详情页定义。binding ∈ upload（实体资源，保存走 resources/upload）| config（配置分区，经 load/save_path 读写） */
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

/** resources/detail 响应（definition = null 表示无定义） */
export interface DetailDefinitionResponse {
  definition: DetailDefinition | null
}

/** 各类型标签（前端兜底展示用；后端 ProviderInfo.label 为权威，未下发时用此表） */
export const RESOURCE_LABELS: Record<string, string> = {
  session: '会话',
  model: '模型',
  agent: '智能体',
  skill: '技能',
  mcp: 'MCP',
  setting: '设置',
}